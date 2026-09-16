// ============================================================
// 计时核心
// 所有业务规则都集中在这里,前端只负责画。这样 macOS 与 Windows
// 两个平台跑的是同一套逻辑,行为不会各自漂移。
//
// 计时不靠"每 tick 累加",而是记录绝对时刻(Instant)再反算,
// 这样定时器抖动或线程被挂起都不会导致时间漂移。
// ============================================================

use std::time::{Duration, Instant};

/// 自定义时长的上限(秒):90:00。超过这个值一律按上限处理。
pub const MAX_CUSTOM_SECS: f64 = 90.0 * 60.0;

/// 右键菜单里可选的倒计时时长(单位:分钟)
pub const PRESET_MINUTES: [f64; 15] = [
    1.0, 2.0, 3.0, 5.0, 10.0, 15.0, 20.0, 25.0, 30.0, 35.0, 40.0, 45.0, 50.0, 60.0, 90.0,
];

/// 运行状态。
/// 后四个状态数字都往上走,区别在语义与配色:
/// Overtime 系列是倒计时归零后的超时(红色 + 进度条铺满),
/// Stopwatch 系列是直接启动的秒表(绿色 + 进度条留空)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,             // 空闲:尚未开始,或已重置
    Running,          // 正在倒计时
    Paused,           // 倒计时已暂停
    Overtime,         // 倒计时已归零,正在向上累计超时
    OvertimePaused,   // 超时正计时已暂停
    Stopwatch,        // 正向计时(秒表):从 00:00 起步,没有目标时长
    StopwatchPaused,  // 秒表已暂停
}

impl Phase {
    /// 是否处于"数字往上走"的状态(超时与秒表共用同一套累计逻辑)
    pub fn is_counting_up(self) -> bool {
        matches!(
            self,
            Phase::Overtime | Phase::OvertimePaused | Phase::Stopwatch | Phase::StopwatchPaused
        )
    }

    /// 是否处于暂停(三种暂停都算),前端据此画播放三角并压暗数字
    pub fn is_paused(self) -> bool {
        matches!(
            self,
            Phase::Paused | Phase::OvertimePaused | Phase::StopwatchPaused
        )
    }

    /// 传给前端的字符串标识。与 main.js 里的 phase 判断一一对应。
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Idle => "idle",
            Phase::Running => "running",
            Phase::Paused => "paused",
            Phase::Overtime => "overtime",
            Phase::OvertimePaused => "overtimePaused",
            Phase::Stopwatch => "stopwatch",
            Phase::StopwatchPaused => "stopwatchPaused",
        }
    }
}

/// 推给前端的一帧状态。字段名用 camelCase,直接对应 JS 里的属性。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// 已经格式化好的时间文案,前端拿来就用
    pub text: String,
    /// 进度条填充比例(0~1)
    pub progress: f64,
    /// 状态标识,前端据此选配色
    pub phase: &'static str,
}

/// 计时器。
///
/// 时间基准的选择:
/// - 倒计时用 `end_at`(结束时刻),每帧反算剩余
/// - 正计时用 `up_from`(起点时刻),每帧反算已走
/// 暂停时把当前值落到 `remaining` / `elapsed_up`,恢复时再把基准时刻往前挪回去。
pub struct Timer {
    phase: Phase,

    /// 本轮设定的总时长(秒),秒表模式下为 0
    total: f64,
    /// 剩余时长(秒)。运行中由 end_at 反算,暂停时保存快照值。
    remaining: f64,
    /// 倒计时的结束时刻。用绝对时刻避免定时器累积误差。
    end_at: Option<Instant>,

    /// 正计时(超时 / 秒表)的起点时刻
    up_from: Option<Instant>,
    /// 正计时已累计的秒数。运行中由 up_from 反算,暂停时保存快照值。
    elapsed_up: f64,

    /// 当前选中的预设分钟数,用于给菜单打勾。自定义时长时为 None。
    pub selected_minutes: Option<f64>,

    /// 结束提醒音开关。默认关闭,用户可在菜单里切换。
    pub alert_enabled: bool,
}

impl Timer {
    pub fn new() -> Self {
        Self {
            phase: Phase::Idle,
            total: 0.0,
            remaining: 0.0,
            end_at: None,
            up_from: None,
            elapsed_up: 0.0,
            selected_minutes: None,
            alert_enabled: false,  // 默认关闭提醒音
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// 是否已设定过时长。菜单里的"开始"项据此决定是否可点。
    pub fn has_duration(&self) -> bool {
        self.total > 0.0
    }

    /// 当前设定的总时长(秒)。自定义输入框用它预填。
    pub fn total(&self) -> f64 {
        self.total
    }

    // ---------- 状态推进 ----------

    /// 每帧调用:按当前状态刷新时间值,并处理倒计时归零的转场。
    /// - Returns: true 表示这一帧刚好跨过零点(需要触发提示音)
    pub fn tick(&mut self) -> bool {
        match self.phase {
            Phase::Running => {
                let end_at = match self.end_at {
                    Some(v) => v,
                    None => return false,
                };
                let now = Instant::now();

                if now >= end_at {
                    // 归零瞬间:把超出的那一点点时间作为正计时起点的偏移,衔接不丢秒
                    let overshoot = now.duration_since(end_at).as_secs_f64();
                    self.remaining = 0.0;
                    // 起点往前挪 overshoot,这样显示值从 overshoot 而不是 0 起算
                    self.up_from = Some(now - Duration::from_secs_f64(overshoot));
                    self.elapsed_up = overshoot;
                    self.phase = Phase::Overtime;
                    return true;
                }

                self.remaining = end_at.duration_since(now).as_secs_f64();
                false
            }
            Phase::Overtime | Phase::Stopwatch => {
                if let Some(from) = self.up_from {
                    self.elapsed_up = from.elapsed().as_secs_f64();
                }
                false
            }
            // 三种暂停与空闲都不推进时间
            _ => false,
        }
    }

    // ---------- 操作 ----------

    /// 设定时长并立即开始倒计时。
    /// - Parameter secs: 时长(秒),内部会按 MAX_CUSTOM_SECS 封顶
    /// - Parameter minutes: 对应的预设分钟数,自定义时传 None
    pub fn set_and_start(&mut self, secs: f64, minutes: Option<f64>) {
        let capped = secs.min(MAX_CUSTOM_SECS);
        self.selected_minutes = minutes;
        self.total = capped;
        self.remaining = capped;
        // 换新时长时清掉上一轮的正计时残留
        self.up_from = None;
        self.elapsed_up = 0.0;
        self.start();
    }

    /// 开始 / 继续。空闲与三种暂停状态都走这里。
    pub fn start(&mut self) {
        match self.phase {
            Phase::Idle | Phase::Paused => {
                // 剩余为 0 时不该启动,否则会立刻又归零
                if self.remaining <= 0.0 {
                    return;
                }
                self.end_at = Some(Instant::now() + Duration::from_secs_f64(self.remaining));
                self.phase = Phase::Running;
            }
            Phase::OvertimePaused => {
                // 把起点往前挪已累计的秒数,接着往上走
                self.up_from = Some(Instant::now() - Duration::from_secs_f64(self.elapsed_up));
                self.phase = Phase::Overtime;
            }
            Phase::StopwatchPaused => {
                self.up_from = Some(Instant::now() - Duration::from_secs_f64(self.elapsed_up));
                self.phase = Phase::Stopwatch;
            }
            // 已经在跑了,不重复启动
            _ => {}
        }
    }

    /// 暂停。按当前是倒计时还是正计时,分别把走到的位置落到快照字段。
    pub fn pause(&mut self) {
        match self.phase {
            Phase::Running => {
                if let Some(end_at) = self.end_at {
                    let now = Instant::now();
                    self.remaining = if now >= end_at {
                        0.0
                    } else {
                        end_at.duration_since(now).as_secs_f64()
                    };
                }
                self.phase = Phase::Paused;
            }
            Phase::Overtime => {
                if let Some(from) = self.up_from {
                    self.elapsed_up = from.elapsed().as_secs_f64();
                }
                self.phase = Phase::OvertimePaused;
            }
            Phase::Stopwatch => {
                if let Some(from) = self.up_from {
                    self.elapsed_up = from.elapsed().as_secs_f64();
                }
                self.phase = Phase::StopwatchPaused;
            }
            _ => {}
        }
    }

    /// 开始 / 暂停 / 继续,一个动作走完所有状态
    pub fn toggle(&mut self) {
        match self.phase {
            Phase::Running | Phase::Overtime | Phase::Stopwatch => self.pause(),
            Phase::Idle | Phase::Paused | Phase::OvertimePaused | Phase::StopwatchPaused => {
                self.start()
            }
        }
    }

    /// 启动秒表:从 00:00 起步向上累计,没有目标时长。
    /// 进度条留空(与倒计时逐渐消耗相反)。
    pub fn start_stopwatch(&mut self) {
        self.selected_minutes = None;
        self.total = 0.0;
        self.remaining = 0.0;
        self.up_from = Some(Instant::now());
        self.elapsed_up = 0.0;
        self.phase = Phase::Stopwatch;
    }

    /// 重置:回到设定值并停止计时,正计时累计一并清零
    pub fn reset(&mut self) {
        self.end_at = None;
        self.up_from = None;
        self.elapsed_up = 0.0;
        self.remaining = self.total;
        self.phase = Phase::Idle;
    }

    // ---------- 输出 ----------

    /// 取当前状态的快照,推给前端渲染
    pub fn snapshot(&self) -> Snapshot {
        let (text, progress) = if self.phase.is_counting_up() {
            match self.phase {
                // 超时:显示已超出的时长,进度条整条铺满(红色)
                Phase::Overtime | Phase::OvertimePaused => {
                    (format_elapsed(self.elapsed_up), 1.0)
                }
                // 秒表:显示已走时长,进度条留空(没有目标时长,谈不上进度)
                _ => (format_elapsed(self.elapsed_up), 0.0),
            }
        } else {
            let progress = if self.total > 0.0 {
                self.remaining / self.total
            } else {
                0.0
            };
            (format_remaining(self.remaining), progress)
        };

        Snapshot {
            text,
            progress,
            phase: self.phase.as_str(),
        }
    }
}

// ============================================================
// 格式化与解析
// ============================================================

/// 把整数秒拼成时间文案:不足 1 小时用 MM:SS,达到 1 小时用 H:MM:SS
fn format_clock(total: i64) -> String {
    let total = total.max(0);
    let hour = total / 3600;
    let minute = (total % 3600) / 60;
    let second = total % 60;
    if hour > 0 {
        format!("{}:{:02}:{:02}", hour, minute, second)
    } else {
        format!("{:02}:{:02}", minute, second)
    }
}

/// 剩余秒数格式化(倒计时)。
/// 向上取整:刚开始就显示完整时长,只有真正走完最后一秒才显示 00:00。
pub fn format_remaining(secs: f64) -> String {
    format_clock(secs.max(0.0).ceil() as i64)
}

/// 已用秒数格式化(正计时)。
/// 向下取整:从 00:00 起步,满一秒才跳到 00:01。
pub fn format_elapsed(secs: f64) -> String {
    format_clock(secs.max(0.0).floor() as i64)
}

/// 预设时长的菜单文案:1 → "1分",1.5 → "1分半"
pub fn preset_title(minutes: f64) -> String {
    let whole = minutes as i64;
    let has_half = minutes - whole as f64 > 0.01;
    if has_half {
        format!("{}分半", whole)
    } else {
        format!("{}分", whole)
    }
}

/// 解析手动输入的时间文本。
/// 支持三种写法:"MM:SS"、"H:MM:SS",以及不带冒号的纯数字(按分钟处理)。
/// 允许 "5:" / ":30" 这类省略写法,空缺的段按 0 计。
/// - Returns: 总秒数;格式非法或结果不大于 0 时返回 None
pub fn parse_time_input(raw: &str) -> Option<f64> {
    // 统一全角冒号,并去掉首尾空白
    let text = raw.replace('：', ":").trim().to_string();
    if text.is_empty() {
        return None;
    }

    let parts: Vec<&str> = text.split(':').collect();

    // 没有冒号:整串按分钟数理解,例如 "90" → 90 分钟
    if parts.len() == 1 {
        let minutes: f64 = parts[0].parse().ok()?;
        if minutes <= 0.0 {
            return None;
        }
        return Some(minutes * 60.0);
    }

    // 最多三段(时:分:秒),逐段按 60 进制累加
    if parts.len() > 3 {
        return None;
    }
    let mut total = 0.0_f64;
    for part in parts {
        let value: f64 = if part.is_empty() {
            0.0
        } else {
            part.parse().ok()?
        };
        if value < 0.0 {
            return None;
        }
        total = total * 60.0 + value;
    }
    if total > 0.0 {
        Some(total)
    } else {
        None
    }
}

// ============================================================
// 单元测试
// 计时与解析是纯逻辑,值得用测试锁住行为,避免以后改动时悄悄改坏。
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 时间格式化_不足一小时用两段() {
        assert_eq!(format_clock(0), "00:00");
        assert_eq!(format_clock(59), "00:59");
        assert_eq!(format_clock(60), "01:00");
        assert_eq!(format_clock(3599), "59:59");
    }

    #[test]
    fn 时间格式化_满一小时用三段() {
        assert_eq!(format_clock(3600), "1:00:00");
        assert_eq!(format_clock(5400), "1:30:00");
    }

    #[test]
    fn 剩余时间向上取整_开始就显示完整时长() {
        // 5 分钟整设定值,浮点上略有亏损时仍应显示 05:00
        assert_eq!(format_remaining(299.999), "05:00");
        // 只有真正走完才显示 00:00
        assert_eq!(format_remaining(0.001), "00:01");
        assert_eq!(format_remaining(0.0), "00:00");
    }

    #[test]
    fn 已用时间向下取整_从零起步() {
        assert_eq!(format_elapsed(0.0), "00:00");
        assert_eq!(format_elapsed(0.999), "00:00");
        assert_eq!(format_elapsed(1.0), "00:01");
    }

    #[test]
    fn 解析_纯数字按分钟() {
        assert_eq!(parse_time_input("90"), Some(5400.0));
        assert_eq!(parse_time_input("1"), Some(60.0));
    }

    #[test]
    fn 解析_两段与三段() {
        assert_eq!(parse_time_input("05:30"), Some(330.0));
        assert_eq!(parse_time_input("1:20:00"), Some(4800.0));
    }

    #[test]
    fn 解析_全角冒号与省略写法() {
        assert_eq!(parse_time_input("05：30"), Some(330.0));
        assert_eq!(parse_time_input("5:"), Some(300.0));
        assert_eq!(parse_time_input(":30"), Some(30.0));
    }

    #[test]
    fn 解析_非法输入返回空() {
        assert_eq!(parse_time_input(""), None);
        assert_eq!(parse_time_input("abc"), None);
        assert_eq!(parse_time_input("0"), None);
        assert_eq!(parse_time_input("1:2:3:4"), None);
    }

    #[test]
    fn 预设文案_半分钟带半字() {
        assert_eq!(preset_title(1.0), "1分");
        assert_eq!(preset_title(1.5), "1分半");
        assert_eq!(preset_title(90.0), "90分");
    }

    #[test]
    fn 秒表从零起步且进度条留空() {
        let mut t = Timer::new();
        t.start_stopwatch();
        assert_eq!(t.phase(), Phase::Stopwatch);
        let s = t.snapshot();
        assert_eq!(s.text, "00:00");
        // 秒表没有目标时长,进度条应当留空
        assert_eq!(s.progress, 0.0);
    }

    #[test]
    fn 自定义时长按上限封顶() {
        let mut t = Timer::new();
        // 传入 200 分钟,应被压到 90 分钟
        t.set_and_start(200.0 * 60.0, None);
        assert_eq!(t.total(), MAX_CUSTOM_SECS);
    }

    #[test]
    fn 暂停后继续_剩余时间不跳变() {
        let mut t = Timer::new();
        t.set_and_start(300.0, Some(5.0));
        assert_eq!(t.phase(), Phase::Running);

        t.pause();
        assert_eq!(t.phase(), Phase::Paused);
        let before = t.snapshot().text;

        // 暂停期间时间不应流动
        std::thread::sleep(Duration::from_millis(50));
        t.tick();
        assert_eq!(t.snapshot().text, before);

        t.start();
        assert_eq!(t.phase(), Phase::Running);
    }

    #[test]
    fn 归零后转入超时且进度条铺满() {
        let mut t = Timer::new();
        // 设一个极短的时长,等它自然走完
        t.set_and_start(0.05, None);
        std::thread::sleep(Duration::from_millis(80));

        // 跨零的那一帧应当返回 true(用于触发提示音)
        assert!(t.tick());
        assert_eq!(t.phase(), Phase::Overtime);
        // 超时阶段进度条整条铺满
        assert_eq!(t.snapshot().progress, 1.0);
    }

    #[test]
    fn 重置回到设定值() {
        let mut t = Timer::new();
        t.set_and_start(300.0, Some(5.0));
        t.reset();
        assert_eq!(t.phase(), Phase::Idle);
        assert_eq!(t.snapshot().text, "05:00");
        // 重置只停表,不清掉设定值,所以还能再次开始
        assert!(t.has_duration());
    }
}
