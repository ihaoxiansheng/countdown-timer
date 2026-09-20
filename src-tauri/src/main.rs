// ============================================================
// 简易倒计时 — Tauri 主程序
//
// 职责分工:
//   timer.rs  纯逻辑(计时、格式化、解析),带单元测试
//   main.rs   平台外壳(窗口、原生右键菜单、推帧、提示音)
//
// 计时状态全部放在 Rust 侧,前端只负责画。这样 macOS 与 Windows
// 跑的是同一套逻辑,两平台行为不会各自漂移。
// ============================================================

// Windows 下隐藏控制台窗口(release 构建才生效,debug 时保留方便看日志)
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod timer;

use std::sync::Mutex;
use std::time::Duration;

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, State, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

use timer::{Phase, Snapshot, Timer};

// ---------- 常量 ----------

/// 窗口最小尺寸。压到刚够容纳时间文案,方便当成小挂件贴在屏幕角落。
/// "重置窗口"菜单项也复用这个尺寸。
const MIN_W: f64 = 110.0;
const MIN_H: f64 = 44.0;

/// 推帧间隔。100ms 足够让秒数跳变看起来即时,又不会白耗 CPU。
const TICK_MS: u64 = 100;

/// 自定义时间输入窗口的标签与尺寸。
/// 标签同时用于判重(已开着就聚焦而不是再建一个)和 capabilities 授权。
///
/// 高度是按 custom.html 的内容量出来的,窗口不可缩放,少一点就会裁掉按钮:
///   上下留白 14×2 = 28、两行提示 ≈ 33、输入框(30px 字号)≈ 41、
///   错误提示行 ≈ 14、按钮行 ≈ 28、四个元素之间三道 8px 间隙 = 24,
/// 合计约 168,取 170 留一点余量。
const CUSTOM_LABEL: &str = "custom";
const CUSTOM_W: f64 = 320.0;
const CUSTOM_H: f64 = 170.0;

/// 结束提醒的计时帧数。响铃共 3 声,分布在前 2 秒内:
/// 归零时第 1 声,0.8 秒后第 2 声,1.6 秒后第 3 声。
/// 保留 20 帧(2.0 秒)余量,确保最后一声响完。
const ALERT_DURATION_TICKS: u32 = 20;

// ---------- 共享状态 ----------

/// 跨线程共享的应用状态。
/// Tauri 的命令处理器可能在不同线程被调用,所以用 Mutex 包起来。
struct AppState {
    timer: Mutex<Timer>,
    /// 结束提醒的剩余计时帧数。大于 0 表示正在响铃提醒。
    alert_left: Mutex<u32>,
}

impl AppState {
    fn new() -> Self {
        Self {
            timer: Mutex::new(Timer::new()),
            alert_left: Mutex::new(0),
        }
    }
}

// ---------- 子进程工具 ----------

/// Windows 的 CREATE_NO_WINDOW 标志。
///
/// `#![windows_subsystem = "windows"]` 只保证主程序自己不带控制台,
/// 管不到它 spawn 出来的子进程:默认情况下系统会给每个控制台子进程
/// (powershell、cmd 等)分配一个新的控制台窗口,于是屏幕上闪一个黑框。
/// 建进程时带上这个标志就不会再分配控制台。
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 建一个不会弹控制台窗口的子进程命令。
///
/// 所有调外部命令的地方都必须走这里,不要直接用 `Command::new`,
/// 否则 Windows 上会闪黑框(见 CREATE_NO_WINDOW 的说明)。
fn hidden_command(program: &str) -> std::process::Command {
    // macOS 分支不需要改动这个命令,所以 mut 在那边用不上,加 allow 免掉告警
    #[allow(unused_mut)]
    let mut cmd = std::process::Command::new(program);

    #[cfg(target_os = "windows")]
    {
        // CommandExt 只在 Windows 上存在,所以 use 也放在条件块里
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd
}

// ---------- 提示音 ----------

/// 播放系统提示音。两个平台各用各自的原生方式,不引入额外的音频依赖。
fn play_alert_sound() {
    #[cfg(target_os = "macos")]
    {
        // afplay 是 macOS 自带的命令行播放器,Glass 是系统提示音之一
        let _ = hidden_command("afplay")
            .arg("/System/Library/Sounds/Glass.aiff")
            .spawn();
    }

    #[cfg(target_os = "windows")]
    {
        // rundll32 直接调用 user32 的 MessageBeep,是 Windows 上最轻的发声方式:
        // 不需要 PowerShell 启动开销(约 200ms),也没有控制台可闪。
        // 参数 0x40 = MB_ICONASTERISK,对应系统的"星号"提示音。
        let _ = hidden_command("rundll32")
            .args(["user32.dll,MessageBeep", "0x40"])
            .spawn();
    }
}

// ---------- 前端命令 ----------

/// 取一次当前状态。前端启动时调一次,避免首帧空白。
#[tauri::command]
fn snapshot(state: State<AppState>) -> Snapshot {
    state.timer.lock().unwrap().snapshot()
}

/// 开始 / 暂停 / 继续
#[tauri::command]
fn toggle(state: State<AppState>) {
    state.timer.lock().unwrap().toggle();
}

/// 重置到设定值并停止计时
#[tauri::command]
fn reset(state: State<AppState>) {
    state.timer.lock().unwrap().reset();
    // 重置同时把提醒掐掉,否则会继续闪
    *state.alert_left.lock().unwrap() = 0;
}

/// 左键单击。
/// 超时阶段第一次点击先把闪烁提醒停掉(正计时继续走),再点才是暂停。
/// 这样"到点了想让它安静"和"想停表"是两个独立动作,不会误触。
#[tauri::command]
fn click_toggle(state: State<AppState>) {
    let mut alert_left = state.alert_left.lock().unwrap();
    if *alert_left > 0 {
        *alert_left = 0;
        return;
    }
    drop(alert_left);
    state.timer.lock().unwrap().toggle();
}

/// 右键:弹出原生菜单。
/// 菜单每次都重建,这样"开始/暂停"的文案和预设的勾选状态永远是最新的。
#[tauri::command]
fn show_menu(app: AppHandle, window: WebviewWindow, state: State<AppState>) -> Result<(), String> {
    let menu = build_context_menu(&app, &state).map_err(|e| e.to_string())?;
    window.popup_menu(&menu).map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- 右键菜单 ----------

/// 构建右键菜单。
/// 顺序:开始/暂停 → 重置 → 设置时间 → 开始正向计时 → 结束提醒音
///      → 重置窗口 → 窗口置顶 → 退出
fn build_context_menu(app: &AppHandle, state: &State<AppState>) -> tauri::Result<Menu<tauri::Wry>> {
    let timer = state.timer.lock().unwrap();
    let phase = timer.phase();
    let has_duration = timer.has_duration();
    let selected = timer.selected_minutes;
    let alert_enabled = timer.alert_enabled;
    drop(timer);

    // 开始/暂停:文案随状态变,让菜单自己说明下一步会发生什么
    let (toggle_label, toggle_enabled) = match phase {
        Phase::Running => ("暂停", true),
        Phase::Paused => ("继续", true),
        Phase::Idle => ("开始", has_duration),
        Phase::Overtime => ("暂停超时", true),
        Phase::OvertimePaused => ("继续超时", true),
        Phase::Stopwatch => ("暂停秒表", true),
        Phase::StopwatchPaused => ("继续秒表", true),
    };

    let item_toggle = MenuItem::with_id(app, "toggle", toggle_label, toggle_enabled, None::<&str>)?;
    let item_reset = MenuItem::with_id(app, "reset", "重置", true, None::<&str>)?;
    let item_stopwatch =
        MenuItem::with_id(app, "stopwatch", "开始正向计时", true, None::<&str>)?;

    // 设置时间子菜单:第一行是自定义输入,后面平铺所有预设档位
    let item_custom = MenuItem::with_id(app, "custom", "自定义…", true, None::<&str>)?;
    let sub_time = Submenu::with_id(app, "time", "设置时间", true)?;
    sub_time.append(&item_custom)?;
    sub_time.append(&PredefinedMenuItem::separator(app)?)?;

    // 预设项用 CheckMenuItem,给当前选中的档位打勾
    for minutes in timer::PRESET_MINUTES {
        let checked = selected.map_or(false, |m| (m - minutes).abs() < 0.01);
        let item = CheckMenuItem::with_id(
            app,
            format!("preset:{}", minutes),
            timer::preset_title(minutes),
            true,
            checked,
            None::<&str>,
        )?;
        sub_time.append(&item)?;
    }

    let item_reset_window = MenuItem::with_id(app, "reset_window", "重置窗口", true, None::<&str>)?;

    // 结束提醒音:CheckMenuItem,默认不勾选
    let item_alert = CheckMenuItem::with_id(
        app,
        "alert_sound",
        "结束提醒音",
        true,
        alert_enabled,
        None::<&str>,
    )?;

    // 窗口置顶用 CheckMenuItem,勾选状态直接读窗口的实际值
    let on_top = app
        .get_webview_window("main")
        .and_then(|w| w.is_always_on_top().ok())
        .unwrap_or(true);
    let item_on_top = CheckMenuItem::with_id(app, "on_top", "窗口置顶", true, on_top, None::<&str>)?;

    let item_quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;

    let menu = Menu::new(app)?;
    menu.append(&item_toggle)?;
    menu.append(&item_reset)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&sub_time)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&item_stopwatch)?;
    menu.append(&item_alert)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&item_reset_window)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&item_on_top)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&item_quit)?;

    Ok(menu)
}

/// 菜单点击分发
fn handle_menu_event(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();

    match id {
        "toggle" => state.timer.lock().unwrap().toggle(),

        "reset" => {
            state.timer.lock().unwrap().reset();
            *state.alert_left.lock().unwrap() = 0;
        }

        "stopwatch" => {
            state.timer.lock().unwrap().start_stopwatch();
            *state.alert_left.lock().unwrap() = 0;
        }

        "custom" => open_custom_window(app),

        "reset_window" => reset_window_size(app),

        "on_top" => {
            if let Some(win) = app.get_webview_window("main") {
                let now = win.is_always_on_top().unwrap_or(true);
                let _ = win.set_always_on_top(!now);
            }
        }

        "alert_sound" => {
            let mut timer = state.timer.lock().unwrap();
            timer.alert_enabled = !timer.alert_enabled;
        }

        "quit" => app.exit(0),

        // 预设档位:id 形如 "preset:5"
        other => {
            if let Some(rest) = other.strip_prefix("preset:") {
                if let Ok(minutes) = rest.parse::<f64>() {
                    state
                        .timer
                        .lock()
                        .unwrap()
                        .set_and_start(minutes * 60.0, Some(minutes));
                    *state.alert_left.lock().unwrap() = 0;
                }
            }
        }
    }
}

/// 把窗口缩回最小尺寸(位置保持不变,只收窄收矮)。
/// 拖大之后想恢复成屏幕角落的小挂件,不必再手动拖边框。
fn reset_window_size(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_size(LogicalSize::new(MIN_W, MIN_H));
    }
}

/// 打开自定义时间输入窗口:独立窗口,屏幕居中,输完即关。
///
/// 这里换过两轮做法:
///   1. 最早是拉起外部进程弹原生对话框(macOS 的 osascript、Windows 的
///      VB InputBox)。Windows 上子进程会闪一个控制台黑框,而且 InputBox
///      是另一个进程的顶层窗口,和这个小挂件完全脱节。
///   2. 然后改成把输入条内嵌在倒计时窗口里。黑框没有了,但 220×96 的
///      地方太挤,而且输入框出现的位置跟着挂件跑,不在视线中央。
/// 现在是应用自己的独立窗口:居中、有足够空间、外观可控,倒计时窗口
/// 自始至终不动。
///
/// 窗口已存在时只把它拉到前台,不重复创建。
fn open_custom_window(app: &AppHandle) {
    // 已经开着就聚焦,避免连按菜单叠出一堆窗口
    if let Some(win) = app.get_webview_window(CUSTOM_LABEL) {
        let _ = win.set_focus();
        return;
    }

    let built = WebviewWindowBuilder::new(
        app,
        CUSTOM_LABEL,
        WebviewUrl::App("custom.html".into()),
    )
    .title("自定义时间")
    .inner_size(CUSTOM_W, CUSTOM_H)
    .center()                 // 居中显示,这是这一版的主要目的
    .resizable(false)
    .minimizable(false)
    .maximizable(false)
    .always_on_top(true)      // 倒计时窗口本身也置顶,输入窗口不能被它压住
    .decorations(false)       // 和主窗口一致的无边框风格,圆角由 CSS 画
    .transparent(true)
    .shadow(true)
    .focused(true)
    .build();

    if let Err(e) = built {
        // 建窗口失败没有兜底方案,但也不该让应用崩掉:倒计时还在正常走。
        eprintln!("自定义时间窗口创建失败: {e}");
    }
}

/// 输入窗口启动时取预填文案:当前设定过时长就填它,方便直接改。
#[tauri::command]
fn custom_prefill(state: State<AppState>) -> String {
    let timer = state.timer.lock().unwrap();
    if timer.total() > 0.0 {
        timer::format_remaining(timer.total())
    } else {
        String::new()
    }
}

/// 输入窗口确认后回调这里,把文案解析成时长并立即开始。
///
/// 输入支持 "MM:SS"、"H:MM:SS",也支持纯数字(按分钟算)。
/// 上限 90:00:超过的一律按 90:00 处理,不报错也不打断操作。
/// - Returns: true 表示解析成功已开始;false 表示格式不合法,
///   前端据此把输入框标红,不再另外弹窗打断操作。
#[tauri::command]
fn submit_custom(state: State<AppState>, text: String) -> bool {
    match timer::parse_time_input(&text) {
        Some(secs) => {
            // 自定义时长不对应任何预设档位,传 None 清掉勾选
            state.timer.lock().unwrap().set_and_start(secs, None);
            *state.alert_left.lock().unwrap() = 0;
            true
        }
        None => false,
    }
}

/// 前端确认"这是一次拖动而不是点击"之后,才请求进入窗口拖动。
///
/// 早先窗口靠整块 `data-tauri-drag-region` 拖动,由 WebView 在 mousedown
/// 当下就进系统的模态拖动循环。Windows 上从别的窗口点回来时,这一次
/// mousedown 同时承担"激活窗口"的职责,配对的 mouseup 被激活流程吃掉,
/// 系统拖动循环收不到结束信号,于是窗口黏在鼠标上一直跟着走。
///
/// 现在拖动区域交给前端判定:按下先不拖,位移超过阈值才调这里,
/// 单纯的点击就完全不会碰到拖动循环。
#[tauri::command]
fn begin_drag(window: WebviewWindow) -> Result<(), String> {
    window.start_dragging().map_err(|e| e.to_string())
}

// ---------- 推帧循环 ----------

/// 后台线程:每 TICK_MS 推一帧状态给前端。
///
/// 用独立线程而不是前端 setInterval 的原因:窗口最小化或前端被节流时,
/// 浏览器会把定时器降频到每秒甚至更慢,而计时器不能因此走慢。
fn spawn_tick_loop(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(TICK_MS));

        let state = app.state::<AppState>();

        // 推进时间,并检查这一帧是否刚好跨过零点
        let crossed_zero = state.timer.lock().unwrap().tick();
        let alert_enabled = state.timer.lock().unwrap().alert_enabled;

        if crossed_zero {
            // 归零:启动响铃提醒(如果开关打开的话)
            if alert_enabled {
                *state.alert_left.lock().unwrap() = ALERT_DURATION_TICKS;
                play_alert_sound();
            }
        }

        // 响铃提醒:只响声,不闪烁。原版虽有 isAlerting 翻转逻辑,
        // 但 draw() 里 isOvertime 恒为真,所以 isAlerting 不改变颜色。
        let snap = state.timer.lock().unwrap().snapshot();
        let mut alert_left = state.alert_left.lock().unwrap();
        if *alert_left > 0 {
            *alert_left -= 1;

            // 补充响铃:在倒数到 12 帧(0.8 秒后)和 4 帧(1.6 秒后)时各响一声,
            // 加上归零那一声,共 3 声,分布在前 2 秒内。
            if *alert_left == 12 || *alert_left == 4 {
                play_alert_sound();
            }
        }
        drop(alert_left);

        let _ = app.emit("tick", snap);
    });
}

// ---------- 入口 ----------

fn main() {
    tauri::Builder::default()
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            snapshot,
            toggle,
            reset,
            click_toggle,
            show_menu,
            custom_prefill,
            submit_custom,
            begin_drag
        ])
        .on_menu_event(|app, event| handle_menu_event(app, event.id().as_ref()))
        .setup(|app| {
            let win = app.get_webview_window("main").unwrap();

            // 最小尺寸在配置里已声明,这里再兜一次,避免某些平台忽略配置
            let _ = win.set_min_size(Some(LogicalSize::new(MIN_W, MIN_H)));

            spawn_tick_loop(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Tauri 应用启动失败");
}
