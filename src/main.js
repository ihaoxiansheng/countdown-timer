// ============================================================
// 简易倒计时 — 前端逻辑
// 计时状态与所有业务规则都放在 Rust 侧,这里只负责:
//   1. 把后端推来的状态画到界面上
//   2. 把用户的点击/按键转成命令发回后端
// 这样 macOS 与 Windows 两个平台的行为完全一致,不会各自漂移。
// ============================================================

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// ---------- DOM 引用 ----------

const rootEl = document.getElementById("root");
const timeEl = document.getElementById("time");
const glyphEl = document.getElementById("play-glyph");
const barFillEl = document.getElementById("bar-fill");

// ---------- 版面常量 ----------
// 与 macOS 版保持同一套比例:留白和进度条都用固定磅值封顶,
// 窗口小的时候按比例收缩,放大后不再继续扩张。
// 这是刻意保留的手感:拉高窗口时数字与边框的缝隙会随之变大。

const kBarMinHeight = 2;        // 进度条最小高度(px)
const kBarMaxHeight = 3;        // 进度条最大高度(px)
const kPadXMax = 8;             // 左右留白上限(px)
const kPadYMax = 5;             // 上下留白上限(px)

// 度量基准字号:先在这个字号下量一次文案宽度,再按比例换算出实际可用字号。
// 这样避免每次 layout 都反复试探字号,和 Swift 版的换算逻辑完全一致。
const kBaseFontSize = 100;

// 播放三角的几何比例,单位是"相对数字 capHeight 的倍数"
const kGlyphHeightRatio = 0.82;
const kGlyphWidthRatio = 0.86;

// 用于精确测量文案宽度与 capHeight 的 canvas(懒创建)
let measureCanvas = null;

// 基准字号下固定参考文案的度量结果,启动时算一次,所有帧复用。
// 用 "88:88" 作为参考(5 字符),等宽数字保证所有 5 字符串(00:10, 01:23 等)宽度相同。
let baseMetrics5 = null;
let baseMetrics7 = null;  // "8:88:88"(7 字符),用于 1 小时以上的文案

/**
 * 测量指定字号下文案的实际宽度与 capHeight。
 * 返回 { width, capHeight },单位 px。
 */
function measureText(text, fontSize) {
    if (!measureCanvas) {
        measureCanvas = document.createElement("canvas");
    }
    const ctx = measureCanvas.getContext("2d");
    // 字体串要和 CSS 的 #time 保持一致:font-weight、font-variant-numeric。
    // font-variant-numeric 在 canvas font 字符串里不直接支持,但等宽数字的
    // CSS 特性已经让浏览器把字体选成等宽变体,measureText 会反映这个宽度。
    ctx.font = `500 ${fontSize}px -apple-system, BlinkMacSystemFont, "SF Pro Display", "Helvetica Neue", "PingFang SC", sans-serif`;
    const metrics = ctx.measureText(text);
    // actualBoundingBoxAscent 是从基线到字形顶部的实际距离,对于纯数字等同于 capHeight
    const capHeight = metrics.actualBoundingBoxAscent || fontSize * 0.72;
    return { width: metrics.width, capHeight };
}

/**
 * 初始化基准度量:在基准字号下测量固定参考文案。
 * 用 "88:88"(5 字符)和 "8:88:88"(7 字符)作为参考,
 * 等宽数字保证所有相同字符数的串宽度一致,字号就不会抖动。
 */
function initBaseMetrics() {
    if (!baseMetrics5) {
        baseMetrics5 = measureText("88:88", kBaseFontSize);
        baseMetrics7 = measureText("8:88:88", kBaseFontSize);
    }
}

// ---------- 状态渲染 ----------

/**
 * 把后端推来的一帧状态画到界面上。
 * @param {object} s 后端 Snapshot:{ text, progress, phase }
 *   phase 取值:idle / running / paused / overtime / overtimePaused /
 *              stopwatch / stopwatchPaused
 */
function render(s) {
    timeEl.textContent = s.text;

    // 三种暂停(倒计时暂停、超时暂停、秒表暂停)共用一套视觉
    const paused = s.phase === "paused"
        || s.phase === "overtimePaused"
        || s.phase === "stopwatchPaused";

    // 超时态:归零后的红色
    const overtime = s.phase === "overtime" || s.phase === "overtimePaused";

    // 走时态:倒计时进行中或秒表进行中,数字转绿
    const running = s.phase === "running" || s.phase === "stopwatch";

    // 状态类名挂在 #root 上,数字与进度条各自在 CSS 里取色。
    // 优先级:暂停 > 超时 > 走时;都不满足则落到默认的黑字空闲态。
    rootEl.classList.toggle("state-paused", paused);
    rootEl.classList.toggle("state-overtime", !paused && overtime);
    rootEl.classList.toggle("state-running", !paused && !overtime && running);

    // 进度条填充比例。超时铺满、秒表留空,由后端算好,这里只做钳位。
    barFillEl.style.width = `${Math.min(1, Math.max(0, s.progress)) * 100}%`;

    // 文案长度会变(00:00 ↔ 1:00:00),每帧都要重算字号
    layout();
}

/**
 * 按当前窗口尺寸算出数字字号与各处间距。
 * 取"高度受限"与"宽度受限"中更小的那个缩放比,保证数字不会溢出。
 * 换算逻辑完全对齐 Swift 版:在基准字号下量一次固定参考文案,所有帧复用这个宽度。
 * 等宽数字保证相同字符数的串宽度一致(88:88 = 00:10),字号稳定不抖动。
 */
function layout() {
    initBaseMetrics();  // 懒初始化基准度量

    const w = rootEl.clientWidth;
    const h = rootEl.clientHeight;
    if (w === 0 || h === 0) return;

    // 进度条与留白:按比例算,再用固定磅值封顶
    const barH = Math.max(kBarMinHeight, Math.min(kBarMaxHeight, h * 0.03));
    const padX = Math.min(kPadXMax, w * 0.04);
    const padY = Math.min(kPadYMax, h * 0.05);

    // 数字可用的高度带与宽度
    const bandH = Math.max(h - barH - padY * 2, 1);
    const usableW = Math.max(w - padX * 2, 1);

    // 按当前文案字符数选用对应的基准度量:5 字符用 "88:88",7 字符用 "8:88:88"。
    // 等宽数字保证相同字符数的串宽度完全一致,字号就不会因 00:09 → 00:10 而抖动。
    const text = timeEl.textContent;
    const charCount = text.length;
    const base = charCount <= 5 ? baseMetrics5 : baseMetrics7;
    const scale = Math.min(bandH / base.capHeight, usableW / base.width);
    const fontSize = kBaseFontSize * scale;
    const capH = base.capHeight * scale;

    timeEl.style.fontSize = `${fontSize}px`;
    // 数字垂直居中于高度带:底部留出进度条与下留白的位置
    timeEl.style.bottom = `${barH + padY}px`;
    timeEl.style.height = `${bandH}px`;

    barFillEl.parentElement.style.height = `${barH}px`;

    // 播放三角跟着 capHeight 走,始终盖在数字正中
    const glyphH = capH * kGlyphHeightRatio;
    glyphEl.style.height = `${glyphH}px`;
    glyphEl.style.width = `${glyphH * kGlyphWidthRatio}px`;
    // 垂直方向:三角中心对齐数字带的中心。
    // 水平方向不在这里写,由 CSS 的 left:50% + translateX(-50%) 负责,
    // 那样三角宽度变化时不用重算偏移量。
    glyphEl.style.bottom = `${barH + padY + (bandH - glyphH) / 2}px`;
}

// ---------- 与后端通信 ----------

// 后端每 100ms 推一帧状态过来,渲染完全由它驱动
listen("tick", (event) => render(event.payload));

// 窗口尺寸变化时重排(拖边框、重置窗口都会触发)
window.addEventListener("resize", layout);

// ---------- 鼠标交互 ----------

// 左键单击 = 开始/暂停。
// data-tauri-drag-region 已经接走了拖动,所以这里只需处理"没发生拖动"的情况:
// 按下与抬起之间位移小于阈值才算点击,避免手抖导致点击失效。
const kDragThreshold = 3;   // px
let downX = 0;
let downY = 0;

rootEl.addEventListener("mousedown", (e) => {
    if (e.button !== 0) return;
    downX = e.screenX;
    downY = e.screenY;
});

rootEl.addEventListener("mouseup", (e) => {
    if (e.button !== 0) return;
    const moved = Math.hypot(e.screenX - downX, e.screenY - downY);
    if (moved <= kDragThreshold) {
        invoke("click_toggle");
    }
});

// 右键:交给后端弹原生菜单(菜单项由 Rust 侧构建,两平台外观都是原生的)
rootEl.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    invoke("show_menu");
});

// ---------- 键盘快捷键 ----------

// 空格 = 开始/暂停,R 或 Esc = 重置。与 macOS 版一致。
window.addEventListener("keydown", (e) => {
    if (e.code === "Space") {
        e.preventDefault();
        invoke("toggle");
    } else if (e.code === "Escape" || e.code === "KeyR") {
        e.preventDefault();
        invoke("reset");
    }
});

// ---------- 启动 ----------

// 主动取一次当前状态,避免首帧空白(后端的推送要等到下一个 tick)
invoke("snapshot").then(render);
