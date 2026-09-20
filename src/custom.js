// ============================================================
// 自定义时间输入窗口 — 前端逻辑
//
// 这是一个独立的小窗口,由 Rust 侧按需创建并居中显示,只负责收一段
// 时间文案。解析与状态变更都在 Rust 侧,这里只做三件事:
//   1. 打开时预填当前设定值并聚焦
//   2. 回车 / 点"开始"提交,Esc / 点"取消" / 失焦放弃
//   3. 格式不合法时标红提示,不关窗,让用户接着改
//
// 窗口是"用完即销毁"的:关掉等于 close 而不是 hide,所以不需要在这里
// 重置上一次的输入状态,每次打开都是一张干净的页面。
// ============================================================

const { invoke } = window.__TAURI__.core;
const { getCurrentWindow } = window.__TAURI__.window;

// ---------- DOM 引用 ----------

const panelEl = document.getElementById("panel");
const fieldEl = document.getElementById("field");
const startBtn = document.getElementById("start");
const cancelBtn = document.getElementById("cancel");

// ---------- 关窗 ----------

/**
 * 关掉自己。倒计时窗口自始至终没动过,不受这里影响。
 * 权限来自 capabilities/custom.json 的 core:window:allow-close。
 */
function closeSelf() {
    getCurrentWindow().close();
}

// ---------- 提交 ----------

/// 提交要等后端回话,这个标记挡住连按回车造成的重复提交
let submitting = false;

/**
 * 提交输入。
 * 解析成功就关窗;失败则标红留在原地,让用户接着改,不额外弹提示打断。
 */
async function submit() {
    if (submitting) return;

    const text = fieldEl.value.trim();

    // 空输入等同于放弃
    if (!text) {
        closeSelf();
        return;
    }

    submitting = true;
    let ok = false;
    try {
        ok = await invoke("submit_custom", { text });
    } finally {
        // 无论成功、失败还是抛异常都要解锁,否则一次意外之后就再也提交不了
        submitting = false;
    }

    if (ok) {
        closeSelf();
        return;
    }

    markInvalid();
}

/**
 * 标红提示格式不对。
 *
 * 先移除再加回是为了让连续输错时的提示能重新生效:中间读一次
 * offsetWidth 强制浏览器结帧,否则两次 classList 操作会被合并成"没变化"。
 */
function markInvalid() {
    panelEl.classList.remove("invalid");
    void panelEl.offsetWidth;
    panelEl.classList.add("invalid");

    // 读屏软件靠 aria-invalid 知道这次输入被拒了,光变红它读不出来
    fieldEl.setAttribute("aria-invalid", "true");

    // 焦点可能在"开始"按钮上(点按钮提交的情况),收回输入框方便直接重打
    fieldEl.focus();
    fieldEl.select();
}

/// 用户一动内容就清掉错误态,不然改对了还留着红字
function clearInvalid() {
    panelEl.classList.remove("invalid");
    fieldEl.removeAttribute("aria-invalid");
}

// ---------- 键盘 ----------

// 回车提交。只挂在输入框上:焦点落在按钮上时回车由按钮自己的 click 处理,
// 两边都接会提交两次。
fieldEl.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
        e.preventDefault();
        submit();
    }
});

fieldEl.addEventListener("input", clearInvalid);

// Esc 取消。挂在窗口级而不是输入框上:焦点 Tab 到按钮之后也要能取消。
window.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
        e.preventDefault();
        closeSelf();
    }
});

// ---------- 按钮 ----------

startBtn.addEventListener("click", submit);
cancelBtn.addEventListener("click", closeSelf);

// ---------- 失焦即放弃 ----------

// 点到别的窗口就当放弃:这是个临时输入框,不该留在屏幕中央挡着。
// 监听窗口级 blur 而不是输入框的 blur:后者在窗口内点按钮时也会触发,
// 那时不该关窗。
//
// 这里有两道护栏,针对的是同一个麻烦:窗口刚创建时焦点会在系统与 webview
// 之间倒手(Windows 上尤其明显),那期间的 blur 不代表用户的意思。
//
// 1) focusArmed —— 没真正拿到过焦点就不理 blur。
//    初值取 document.hasFocus(),覆盖脚本执行晚于窗口获得焦点的情况。
// 2) 延迟确认 —— 收到 blur 不立刻关,等一小会儿;这段时间里焦点回来了
//    就当没发生过。拿到焦点之后仍可能被系统抖一次 blur/focus,只靠第 1 条
//    挡不住。
//
// 刻意不用"创建后 N 毫秒内一律忽略 blur"的写法:那会把这段时间里用户
// 真正的切窗口动作整个吞掉,而后面不会再补一次 blur。这个窗口没有边框也
// 没有关闭按钮,一旦吞掉就僵在屏幕中央了。延迟确认不吞事件,只是晚一点
// 下结论,所以没有这个风险。
const kBlurConfirmMs = 150;

let focusArmed = document.hasFocus();
let pendingClose = null;

window.addEventListener("focus", () => {
    focusArmed = true;

    // 焦点回来了,说明刚才那次 blur 是系统在倒手,撤销待执行的关窗
    if (pendingClose !== null) {
        clearTimeout(pendingClose);
        pendingClose = null;
    }
});

window.addEventListener("blur", () => {
    if (!focusArmed) return;
    if (pendingClose !== null) return;   // 已经在等了,不重复排队

    pendingClose = setTimeout(closeSelf, kBlurConfirmMs);
});

// ---------- 启动 ----------

// 预填当前设定值并全选,方便在原有时长上直接改。
// autofocus 已经让输入框拿到焦点,这里再 focus 一次是为了覆盖
// Windows 上窗口显示时机晚于页面加载、autofocus 被丢掉的情况。
invoke("custom_prefill").then((text) => {
    fieldEl.value = text || "";
    fieldEl.focus();
    fieldEl.select();
});
