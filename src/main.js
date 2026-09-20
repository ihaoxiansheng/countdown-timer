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

// #time 里的字符盒子(每个字符一个 span),由 setTimeText() 重建。
// 三角的水平位置直接取冒号盒子排版后的实测坐标,不再做任何宽度推算。
let charEls = [];

// 当前已写入 #time 的文案。文案没变就不重建 DOM(后端每 100ms 推一帧)。
let currentText = "";

// ---------- 版面常量 ----------
// 与 macOS 版保持同一套比例:留白和进度条都用固定磅值封顶,
// 窗口小的时候按比例收缩,放大后不再继续扩张。
// 这是刻意保留的手感:拉高窗口时数字与边框的缝隙会随之变大。

const kBarMinHeight = 2;        // 进度条最小高度(px)
const kBarMaxHeight = 3;        // 进度条最大高度(px)
const kPadXMax = 8;             // 左右留白上限(px)
const kPadYMax = 5;             // 上下留白上限(px)

// 度量基准字号:先在这个字号下量一次字符宽度,再按比例换算出实际可用字号。
// 这样避免每次 layout 都反复试探字号。
const kBaseFontSize = 100;

// 数字大写高度(capHeight)相对字号的比例。
// 刻意用常量而不是实测值:两个平台的字体不同,实测 capHeight 会让同样大小的
// 窗口在 macOS 与 Windows 上算出不同字号,三角的换算基准也跟着漂。
const kCapHeightRatio = 0.72;

// 播放三角的几何比例,单位是"相对数字 capHeight 的倍数"
const kGlyphHeightRatio = 0.82;
const kGlyphWidthRatio = 0.86;

// 隐藏的度量元素(懒创建)。
// 刻意不用 canvas:canvas 的 font 字符串不认 font-variant-numeric,量出来的
// 宽度和真正渲染的等宽数字对不上;Windows 上整套字体栈缺失、回退到通用
// sans-serif 之后差得更多。用真实 DOM 元素量,才和屏幕上的排版一致。
let measureEl = null;

// 基准字号下的字符宽度,启动时量一次,所有帧复用。
// digitW 取 0-9 里最宽的那个:没有等宽特性时(Windows 回退字体)"1" 比 "0" 窄,
// 统一按最宽值当上限,字号就不会因为 00:09 → 00:10 而抖动。
let baseMetrics = null;

/**
 * 懒创建度量元素。字体相关属性全部从 #time 的计算样式里拷过来,
 * 这样 CSS 改了字体栈,度量结果跟着变,两边不会脱钩。
 */
function ensureMeasureEl() {
    if (measureEl) return measureEl;

    const cs = getComputedStyle(timeEl);
    measureEl = document.createElement("span");
    measureEl.style.position = "absolute";
    measureEl.style.left = "-9999px";      // 挪出可视区,不参与版面
    measureEl.style.top = "0";
    measureEl.style.visibility = "hidden";
    measureEl.style.whiteSpace = "pre";
    measureEl.style.fontFamily = cs.fontFamily;
    measureEl.style.fontWeight = cs.fontWeight;
    measureEl.style.fontVariantNumeric = cs.fontVariantNumeric;
    measureEl.style.fontFeatureSettings = cs.fontFeatureSettings;
    measureEl.style.fontSize = `${kBaseFontSize}px`;
    measureEl.style.lineHeight = "1";
    document.body.appendChild(measureEl);
    return measureEl;
}

/**
 * 在基准字号下测量单个字符的真实渲染宽度(px)。
 */
function measureChar(ch) {
    const el = ensureMeasureEl();
    el.textContent = ch;
    return el.getBoundingClientRect().width;
}

/**
 * 初始化基准度量:最宽的数字宽度 + 冒号宽度。
 * 冒号是标点,比数字窄得多,必须单独量,不能拿数字宽度顶替。
 */
function initBaseMetrics() {
    if (baseMetrics) return;

    let digitW = 0;
    for (let d = 0; d <= 9; d++) {
        digitW = Math.max(digitW, measureChar(String(d)));
    }
    baseMetrics = { digitW, colonW: measureChar(":") };
}

// 非数字、非冒号字符的宽度缓存(比如空闲态的 "—"),量一次就记下来
const otherCharWidths = new Map();

/**
 * 基准字号下单个字符占的宽度(px)。
 * 数字一律返回"最宽数字"的宽度:回退字体没有等宽数字时 "1" 比 "0" 窄,
 * 统一按最宽值排版,00:09 → 00:10 的瞬间整串宽度才不变,字号也就不抖。
 */
function charWidth(ch) {
    if (ch >= "0" && ch <= "9") return baseMetrics.digitW;
    if (ch === ":") return baseMetrics.colonW;
    if (!otherCharWidths.has(ch)) {
        otherCharWidths.set(ch, measureChar(ch));
    }
    return otherCharWidths.get(ch);
}

/**
 * 基准字号下整串文案的宽度(px)。数字按最宽数字算,同格式文案宽度恒定。
 */
function textWidth(text) {
    let width = 0;
    for (const ch of text) {
        width += charWidth(ch);
    }
    return width;
}

// ---------- 状态渲染 ----------

/// 上一帧的快照,供乐观渲染时判断当前状态用
let lastSnapshot = null;

/**
 * 把后端推来的一帧状态画到界面上。
 * @param {object} s 后端 Snapshot:{ text, progress, phase }
 *   phase 取值:idle / running / paused / overtime / overtimePaused /
 *              stopwatch / stopwatchPaused
 */
function render(s) {
    // 验证后端推送数据的完整性,避免缺字段时静默失败
    if (!s || typeof s.text !== 'string' || typeof s.phase !== 'string' || typeof s.progress !== 'number') {
        console.error('render: 收到不完整的状态数据', s);
        return;
    }

    lastSnapshot = s;  // 存一份供 optimisticToggle 使用
    setTimeText(s.text);

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
 * 把文案写进 #time,一个字符一个 span。
 *
 * 刻意不用一整串文本 + font-variant-numeric:tabular-nums 只统一"数字"宽度,
 * 冒号作为标点比数字窄得多;更要紧的是 Windows 上 -apple-system / SF Pro
 * Display / PingFang SC 整套字体栈都不存在,回退到通用 sans-serif 之后连
 * 等宽数字特性都没有,"1" 比 "0" 明显窄。于是 00:10 的右半边比左半边窄,
 * 整串居中时冒号被挤到右边 —— 这就是截图里冒号和三角一起偏的根因。
 *
 * 逐字符装进定宽盒子(宽度在 layout() 里按实测字符宽度写入)之后,
 * MM:SS 的左右两段宽度恒等,冒号必然落在窗口正中,和字体无关。
 *
 * @param {string} text 形如 "00:00" / "1:00:00" 的时间文案
 */
function setTimeText(text) {
    // 文案没变就不动 DOM:后端每 100ms 推一帧,大部分帧文案是一样的
    if (text === currentText) return;
    currentText = text;

    // 逐个摘掉旧字符盒子。不用 innerHTML,避免每帧重新解析 HTML。
    while (timeEl.firstChild) {
        timeEl.removeChild(timeEl.firstChild);
    }

    charEls = [];
    for (const ch of text) {
        const span = document.createElement("span");
        span.className = "ch";
        span.textContent = ch;
        timeEl.appendChild(span);
        charEls.push(span);
    }
}

/**
 * 挑出三角要盖住的那个冒号元素。
 * MM:SS 只有一个冒号,直接用它(它就在窗口正中)。
 * H:MM:SS 有两个,取离整串水平中心更近的那个,三角才不会跑到边上。
 * @returns {HTMLElement|null} 没有冒号时返回 null
 */
function pickColonEl() {
    const colons = charEls.filter((el) => el.textContent === ":");
    if (colons.length === 0) return null;
    if (colons.length === 1) return colons[0];

    // 整串文案的水平中心:取首字符左边缘与末字符右边缘的中点
    const firstRect = charEls[0].getBoundingClientRect();
    const lastRect = charEls[charEls.length - 1].getBoundingClientRect();
    const textCenter = (firstRect.left + lastRect.right) / 2;

    let best = colons[0];
    let bestDist = Infinity;
    for (const el of colons) {
        const rect = el.getBoundingClientRect();
        const dist = Math.abs(rect.left + rect.width / 2 - textCenter);
        if (dist < bestDist) {
            bestDist = dist;
            best = el;
        }
    }
    return best;
}

/**
 * 按当前窗口尺寸算出数字字号、各字符盒子的宽度与三角的位置。
 * 取"高度受限"与"宽度受限"中更小的那个缩放比,保证数字不会溢出。
 *
 * capHeight 用固定比例常量而不是实测值:两平台字体不同,实测值会让同尺寸
 * 窗口在 macOS 与 Windows 上算出不同字号,三角的换算基准跟着一起漂。
 */
function layout() {
    initBaseMetrics();  // 懒初始化基准字符宽度

    const w = rootEl.clientWidth;
    const h = rootEl.clientHeight;
    if (w === 0 || h === 0) return;
    if (charEls.length === 0) return;  // 首帧文案还没到

    // 进度条与留白:按比例算,再用固定磅值封顶
    const barH = Math.max(kBarMinHeight, Math.min(kBarMaxHeight, h * 0.03));
    const padX = Math.min(kPadXMax, w * 0.04);
    const padY = Math.min(kPadYMax, h * 0.05);

    // 数字可用的高度带与宽度
    const bandH = Math.max(h - barH - padY * 2, 1);
    const usableW = Math.max(w - padX * 2, 1);

    // 基准字号下整串宽度。数字一律按最宽数字计,所以同格式文案宽度恒定,
    // 字号不会因为 00:09 → 00:10 而抖动。
    const baseW = Math.max(textWidth(currentText || ""), 1);

    // 高度受限比与宽度受限比取小者
    const scale = Math.min(bandH / (kBaseFontSize * kCapHeightRatio), usableW / baseW);
    const fontSize = kBaseFontSize * scale;
    const capH = fontSize * kCapHeightRatio;

    timeEl.style.fontSize = `${fontSize}px`;
    // 数字垂直居中于高度带:底部留出进度条与下留白的位置
    timeEl.style.bottom = `${barH + padY}px`;
    timeEl.style.height = `${bandH}px`;

    barFillEl.parentElement.style.height = `${barH}px`;

    // 把实测字符宽度按同一个缩放比写进每个盒子,排版就完全可控了
    for (const el of charEls) {
        el.style.width = `${charWidth(el.textContent) * scale}px`;
    }

    // 三角尺寸跟着 capHeight 走
    const glyphH = capH * kGlyphHeightRatio;
    const glyphW = glyphH * kGlyphWidthRatio;
    glyphEl.style.height = `${glyphH}px`;
    glyphEl.style.width = `${glyphW}px`;

    // 三角水平位置:直接取冒号盒子排版后的实测坐标,不做任何宽度推算。
    // 上面刚写完宽度,这里读 rect 会触发一次同步重排,拿到的就是新版面。
    const colonEl = pickColonEl();
    if (colonEl) {
        const rootRect = rootEl.getBoundingClientRect();
        const colonRect = colonEl.getBoundingClientRect();
        // 冒号中心相对 #root 左边缘的坐标
        const colonCenter = colonRect.left + colonRect.width / 2 - rootRect.left;
        glyphEl.style.left = `${colonCenter - glyphW / 2}px`;
    } else {
        // 没有冒号(异常文案)时落回窗口正中
        glyphEl.style.left = `${w / 2 - glyphW / 2}px`;
    }

    // 垂直方向:三角中心对齐数字带的中心
    glyphEl.style.bottom = `${barH + padY + (bandH - glyphH) / 2}px`;
}

// ---------- 与后端通信 ----------

// 后端每 100ms 推一帧状态过来,渲染完全由它驱动
listen("tick", (event) => render(event.payload));

// 窗口尺寸变化时重排(拖边框、重置窗口都会触发)
window.addEventListener("resize", layout);

// ---------- 鼠标交互 ----------

// 拖动与点击的判定阈值(px)。按下后位移超过它才算拖动,否则算点击。
const kDragThreshold = 4;

let downX = 0;          // 按下时的屏幕坐标
let downY = 0;
let pressing = false;   // 是否正处于一次左键按下
let dragging = false;   // 这一次按下是否已经升级成拖动

/**
 * 按下:只记录起点,不做任何动作。
 *
 * 这里刻意不在按下当帧就启动拖动。窗口原先整块是 data-tauri-drag-region,
 * 那个属性会在 mousedown 当帧进入系统的模态拖动循环;Windows 上从别的
 * 窗口点回来时,这一次 mousedown 同时承担"激活窗口"的职责,激活过程会
 * 把配对的 mouseup 吃掉,系统拖动循环收不到结束信号,窗口就黏在鼠标上,
 * 而那一次点击也随之丢失(表现为暂停/继续没反应)。
 *
 * setPointerCapture 把后续的 move/up 都锁定到这个元素上,即使指针移出
 * 窗口边界也能收到抬起事件,不会再出现"按下了但永远等不到抬起"的状态。
 */
rootEl.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;

    pressing = true;
    dragging = false;
    downX = e.screenX;
    downY = e.screenY;

    try {
        rootEl.setPointerCapture(e.pointerId);
    } catch {
        // 捕获失败不影响主流程:大不了指针移出窗口时少收几个事件
    }
});

/**
 * 移动:位移超过阈值才升级成拖动,并把后续过程交给系统。
 * 一次按下只升级一次,所以 begin_drag 不会被重复调用。
 */
rootEl.addEventListener("pointermove", (e) => {
    if (!pressing || dragging) return;

    const moved = Math.hypot(e.screenX - downX, e.screenY - downY);
    if (moved > kDragThreshold) {
        dragging = true;
        // 交给系统拖动循环后,这一次按下不再被当作点击。
        // 系统接手期间不会再给 webview 发 pointer 事件,所以这里先把
        // 按下状态清掉,避免拖完之后 pressing 还挂着。
        pressing = false;
        invoke("begin_drag");
    }
});

/**
 * 抬起:没升级成拖动就算一次点击 = 开始/暂停。
 */
rootEl.addEventListener("pointerup", (e) => {
    if (e.button !== 0) return;

    try {
        rootEl.releasePointerCapture(e.pointerId);
    } catch {
        // 没捕获成功过,这里自然也释放不了,忽略即可
    }

    if (!pressing || dragging) {
        pressing = false;
        return;
    }
    pressing = false;

    // 先本地抢一帧,再发命令。后端 100ms 才推一帧,等它回话最坏要差一整帧,
    // 手感上就是"点了没反应"。乐观渲染让状态变化在当帧就看得见,
    // 下一帧后端的真实状态会覆盖它,两者不一致时以后端为准。
    optimisticToggle();
    invoke("click_toggle");
});

/**
 * 指针被系统取消(拖动接手、窗口失焦等)时清掉按下状态,
 * 否则 pressing 会一直挂着,下一次抬起会被误判成点击。
 */
rootEl.addEventListener("pointercancel", () => {
    pressing = false;
    dragging = false;
});

// 右键:交给后端弹原生菜单(菜单项由 Rust 侧构建,两平台外观都是原生的)
rootEl.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    invoke("show_menu");
});

/**
 * 本地先行渲染一次"开始/暂停"的结果,纯粹为了手感,不改变任何真实状态。
 *
 * 只处理状态翻转明确的四种情形。超时阶段(overtime / overtimePaused)
 * 刻意跳过:后端的 click_toggle 在响铃期间第一次点击是"停响铃"而不是
 * "暂停",这里猜不准,猜错会让画面先跳一下再被纠正,反而更难受。
 * 空闲态也跳过,因为前端不知道是否已设定过时长。
 */
function optimisticToggle() {
    if (!lastSnapshot) return;

    const next = {
        running: "paused",
        paused: "running",
        stopwatch: "stopwatchPaused",
        stopwatchPaused: "stopwatch",
    }[lastSnapshot.phase];

    if (!next) return;

    render({ ...lastSnapshot, phase: next });
}

// ---------- 键盘快捷键 ----------

// 空格 = 开始/暂停,R 或 Esc = 重置。与 macOS 版一致。
// 自定义时间的输入已经搬到独立窗口(custom.html),那边的按键由它自己处理,
// 这个窗口不再需要判断"是否正在编辑"。
window.addEventListener("keydown", (e) => {
    if (e.code === "Space") {
        e.preventDefault();
        // 和鼠标点击一样先抢一帧,避免等后端那 100ms
        optimisticToggle();
        invoke("toggle");
    } else if (e.code === "Escape" || e.code === "KeyR") {
        e.preventDefault();
        invoke("reset");
    }
});

// ---------- 启动 ----------

// 主动取一次当前状态,避免首帧空白(后端的推送要等到下一个 tick)
invoke("snapshot").then(render);
