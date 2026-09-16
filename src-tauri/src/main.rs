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
use tauri::{AppHandle, Emitter, LogicalSize, Manager, State, WebviewWindow};

use timer::{Phase, Snapshot, Timer};

// ---------- 常量 ----------

/// 窗口最小尺寸。压到刚够容纳时间文案,方便当成小挂件贴在屏幕角落。
/// "重置窗口"菜单项也复用这个尺寸。
const MIN_W: f64 = 110.0;
const MIN_H: f64 = 44.0;

/// 推帧间隔。100ms 足够让秒数跳变看起来即时,又不会白耗 CPU。
const TICK_MS: u64 = 100;

/// 结束提醒的闪烁总次数与响铃轮数。与 macOS 版一致:
/// 闪 12 次(约 6 秒)后停下并保持红色,前 3 轮各响一声。
const ALERT_FLASH_TOTAL: u32 = 12;

// ---------- 共享状态 ----------

/// 跨线程共享的应用状态。
/// Tauri 的命令处理器可能在不同线程被调用,所以用 Mutex 包起来。
struct AppState {
    timer: Mutex<Timer>,
    /// 结束提醒的剩余闪烁次数。大于 0 表示正在响铃闪烁。
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

// ---------- 提示音 ----------

/// 播放系统提示音。两个平台各用各自的原生方式,不引入额外的音频依赖。
fn play_alert_sound() {
    #[cfg(target_os = "macos")]
    {
        // afplay 是 macOS 自带的命令行播放器,Glass 是系统提示音之一
        let _ = std::process::Command::new("afplay")
            .arg("/System/Library/Sounds/Glass.aiff")
            .spawn();
    }

    #[cfg(target_os = "windows")]
    {
        // PowerShell 的 Beep 不依赖任何音频文件,系统静音时也不会报错
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", "[console]::beep(880,200)"])
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

        "custom" => prompt_custom_time(app),

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

/// 弹出输入框自定义时间,确认后立即开始倒计时。
/// 输入支持 "MM:SS"、"H:MM:SS",也支持纯数字(按分钟算)。
/// 上限 90:00:超过的一律按 90:00 处理,不报错也不打断操作。
fn prompt_custom_time(app: &AppHandle) {
    // 对话框必须在主线程之外等待结果,否则会和事件循环互相阻塞,
    // 所以这里用回调式 API,拿到结果再回来改状态。
    let app = app.clone();
    std::thread::spawn(move || {
        let current = {
            let state = app.state::<AppState>();
            let timer = state.timer.lock().unwrap();
            if timer.total() > 0.0 {
                timer::format_remaining(timer.total())
            } else {
                String::new()
            }
        };

        // Tauri 没有内置的文本输入对话框,各平台调各自的原生方式
        let input = ask_text_input(&current);

        let Some(text) = input else { return };

        match timer::parse_time_input(&text) {
            Some(secs) => {
                let state = app.state::<AppState>();
                // 自定义时长不对应任何预设档位,传 None 清掉勾选
                state.timer.lock().unwrap().set_and_start(secs, None);
                *state.alert_left.lock().unwrap() = 0;
            }
            None => {
                // 格式不合法:提示一次,不改动当前计时状态
                show_message("时间格式不正确", "请按 05:30 或 1:20:00 的格式输入。");
            }
        }
    });
}

/// 弹出文本输入框,取用户输入的时间文案。
/// - Returns: 用户确认时返回输入内容,取消时返回 None
#[cfg(target_os = "macos")]
fn ask_text_input(prefill: &str) -> Option<String> {
    // osascript 的 display dialog 自带输入框,是 macOS 上最省事的原生方案
    let script = format!(
        r#"display dialog "输入 分:秒(如 05:30),只填数字则按分钟计算。
最长 90:00,超出会自动按 90:00 计。" default answer "{}" with title "自定义时间" buttons {{"取消", "开始"}} default button "开始"
return text returned of result"#,
        prefill.replace('"', r#"\""#)
    );

    let out = std::process::Command::new("osascript")
        .args(["-e", &script])
        .output()
        .ok()?;

    // 用户点取消时 osascript 返回非 0,这里直接当作放弃
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(target_os = "windows")]
fn ask_text_input(prefill: &str) -> Option<String> {
    // Windows 用 VB 的 InputBox,同样不需要额外依赖
    let script = format!(
        r#"Add-Type -AssemblyName Microsoft.VisualBasic
[Microsoft.VisualBasic.Interaction]::InputBox("输入 分:秒(如 05:30),只填数字则按分钟计算。`n最长 90:00,超出会自动按 90:00 计。", "自定义时间", "{}")"#,
        prefill.replace('"', r#""""#)
    );

    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .ok()?;

    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    // InputBox 取消时返回空串
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// 弹一个只有"好"按钮的提示框
#[cfg(target_os = "macos")]
fn show_message(title: &str, body: &str) {
    let script = format!(
        r#"display dialog "{}" with title "{}" buttons {{"好"}} default button "好""#,
        body, title
    );
    let _ = std::process::Command::new("osascript")
        .args(["-e", &script])
        .output();
}

#[cfg(target_os = "windows")]
fn show_message(title: &str, body: &str) {
    let script = format!(
        r#"Add-Type -AssemblyName System.Windows.Forms
[System.Windows.Forms.MessageBox]::Show("{}", "{}")"#,
        body, title
    );
    let _ = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output();
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
                *state.alert_left.lock().unwrap() = ALERT_FLASH_TOTAL;
                play_alert_sound();
            }
        }

        // 响铃提醒:只响声,不闪烁。原版虽有 isAlerting 翻转逻辑,
        // 但 draw() 里 isOvertime 恒为真,所以 isAlerting 不改变颜色,
        // 闪烁是"哑"的——这里也对齐成只响铃,phase 不动。
        let snap = state.timer.lock().unwrap().snapshot();
        let mut alert_left = state.alert_left.lock().unwrap();
        if *alert_left > 0 {
            *alert_left -= 1;

            // 前 3 轮各响一声:每 20 帧(2 秒)响一次
            if *alert_left > ALERT_FLASH_TOTAL.saturating_sub(3) * 5 && *alert_left % 20 == 0 {
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
            show_menu
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
