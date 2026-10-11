//! 「本机用量」的状态机：后台扫描 + 3 秒增量检测 + debounce。
//!
//! 照 [`crate::app::balance`] 的模式：后台线程 + `mpsc::Receiver`，UI 线程每帧
//! `try_recv` 收结果，**绝不在 UI 线程调 `usage::scan()`**（debug 实测 14.6s，
//! 会把界面卡死）。
//!
//! 重扫的触发条件是**源文件指纹变了**，不是「过了 3 秒」：
//!
//! ```text
//! 每 3 秒比对一次指纹
//!   ├─ 没变化 ──────────────► 什么都不做（空闲时 CPU ≈ 0）
//!   └─ 有变化 ──► 记下首次变化时刻，等 1.5 秒（合并连续写入）
//!                  ├─ 1.5 秒内没再变 ──► 重扫
//!                  └─ 一直在变（agent 流式写文件）──► 最多等 5 秒就重扫
//! ```
//!
//! debounce 参数照 token-monitor 的实测值（`DEBOUNCE_SECS` / `MAX_WAIT_SECS` /
//! `POLL_INTERVAL_SECS`），**没有额外的冷却** —— token-monitor 在源码里明确写了
//! 「There is deliberately no cooldown on top of the debounce」：debounce 已经
//! 把连续写入合并掉了，再加冷却只会让用户觉得数字不跟手。

pub(in crate::app) mod aggregate;
mod chart;
pub(in crate::app) use chart::ChartKind;
mod component;
mod overview;
pub(in crate::app) mod range;
mod scan_sources;
pub(in crate::app) mod view;

use crate::app::App;
use crate::format::ConfigFormat;
use crate::usage::{DailyBucket, DailyMap, Ledger, SessionSnapshot};
use eframe::egui;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError};

/// 连续变化合并窗口：首次发现变化后等这么久，期间没再变才重扫。
pub(in crate::app) const DEBOUNCE_SECS: f64 = 1.5;

/// 持续写入时的强制触发上限：agent 流式写会话文件时，指纹每 3 秒都在变，
/// 光靠「静止 1.5 秒」可能永远等不到，所以最多等这么久就重扫一次。
pub(in crate::app) const MAX_WAIT_SECS: f64 = 5.0;

/// 指纹检测间隔：每 3 秒才 stat 一遍源文件（空闲时这是唯一的开销）。
pub(in crate::app) const POLL_INTERVAL_SECS: f64 = 3.0;

/// 账本落盘的最小间隔：扫描可能 3-5 秒一次，但没必要每次都写盘。
pub(in crate::app) const SAVE_INTERVAL_SECS: f64 = 30.0;

/// 扫描在途时的轮询间隔：扫描要 2.4-14 秒，界面空闲时不会自己重绘，
/// 所以要主动要帧才能及时收结果。
const SCAN_POLL_REPAINT_SECS: f64 = 0.25;

/// 重绘请求的最小间隔：两个期限都已经过去时退化成这个节拍，
/// 避免 0 秒（每帧重绘）把界面拖成空转。
pub(in crate::app) const MIN_REPAINT_SECS: f64 = 0.01;

/// 段控按钮：选中态 = 悬浮色底 + 2px 描边，与顶栏页签同一套样式。
///
/// 宽度固定（不随标签长短跳动），返回是否被点击。用量视图里的区间 / 维度 /
/// 图表开关三组按钮都用它，免得到处重写一遍选中态。
pub(in crate::app) fn chip(ui: &mut egui::Ui, selected: bool, label: &str, width: f32) -> bool {
    let visuals = ui.visuals();
    let (fill, stroke) = if selected {
        (
            visuals.widgets.hovered.bg_fill,
            egui::Stroke::new(2.0f32, visuals.widgets.hovered.bg_stroke.color),
        )
    } else {
        (egui::Color32::TRANSPARENT, egui::Stroke::NONE)
    };
    let text = egui::RichText::new(label);
    let text = if selected { text.strong() } else { text };
    ui.add(
        egui::Button::new(text)
            .fill(fill)
            .stroke(stroke)
            .min_size(egui::vec2(width, 22.0)),
    )
    .clicked()
}

/// 系列颜色：强调色按黄金角转一圈，第 0 个系列就是强调色本身。
///
/// 用黄金角（137.508°）而不是均分：均分时系列一多，相邻两色就贴在一起；
/// 黄金角让任意前缀的几个系列都散得开。
///
/// 饱和度与明度按明暗主题分开夹：亮色主题下高饱和色在浅底上会「发飘」，
/// 深色主题下低明度色在深底上又糊成一片。
///
/// 趋势图的堆叠系列与总览卡的构成条共用它，两处的颜色才对得上。
pub(in crate::app) fn series_color(
    index: usize,
    accent: egui::Color32,
    dark: bool,
) -> egui::Color32 {
    // 第 0 个系列原样用强调色：下面那套 s/v 夹取是为了保证「转出来」的颜色
    // 在深浅底上都看得清，但强调色本身是主题作者调过的，不该被动。
    // （`Hsva` 在线性空间算，往返一趟会把强调色推偏几个色阶。）
    if index == 0 {
        return accent;
    }
    let mut hsva = egui::ecolor::Hsva::from(accent);
    hsva.h = (hsva.h + index as f32 * 0.381_966_1).fract();
    if dark {
        hsva.s = hsva.s.clamp(0.35, 0.95);
        hsva.v = hsva.v.max(0.55);
    } else {
        hsva.s = hsva.s.clamp(0.35, 0.95);
        hsva.v = hsva.v.min(0.90);
    }
    egui::Color32::from(hsva)
}

/// 后台线程交回来的原始扫描结果：会话表、按天分桶、消息数、内核自报耗时。
type ScanPayload = (HashMap<String, SessionSnapshot>, DailyMap, usize, u32);

/// 一次扫描的成品（已并入账本）。
pub(in crate::app) struct UsageReport {
    /// 展示视图：实时扫描结果 ∪ 账本里已消失的会话，按最近活动倒序。
    pub sessions: Vec<SessionSnapshot>,
    /// 按本地日历日分桶（含已删会话留下的记录）。
    ///
    /// 热力图与「今日 / 本周 / 本月 / 总计」都读这里：会话级汇总做不到这件事，
    /// 一个会话可以横跨几十天。
    pub daily: DailyMap,
    /// 本次扫描完成时刻（egui 时间轴秒），用于显示「更新于 N 秒前」。
    pub scanned_at: f64,
    /// 内核读到的消息条数。
    pub message_count: usize,
    /// 内核自报的扫描耗时（毫秒）。
    pub scan_ms: u32,
}

impl UsageReport {
    /// 合计 token。
    ///
    /// 只用于测试：生产路径的合计走按天分桶（`range::range_stats`），
    /// 那里能按区间筛选，而会话级汇总做不到。
    #[cfg(test)]
    pub(in crate::app) fn total(&self) -> i64 {
        self.sessions.iter().map(SessionSnapshot::total).sum()
    }
}

/// 「本机用量」的扫描状态。
pub(in crate::app) struct UsageState {
    /// 最近一次成功扫描的结果（已并入账本）。
    pub result: Option<Result<UsageReport, String>>,
    /// `result` 当前是账本顶上的缓存数据（打开秒显）：后台扫完即替换。
    cached: bool,
    /// 扫描在途时的接收端。
    rx: Option<Receiver<Result<ScanPayload, String>>>,
    /// 账本（跨扫描保留已删会话）。
    ledger: Ledger,
    /// 上次指纹检测时刻（egui 秒）。
    last_poll_at: Option<f64>,
    /// 上次落盘时刻（egui 秒）。
    last_save_at: Option<f64>,
    /// 源文件指纹：path → (mtime_secs, size)。
    fingerprints: scan_sources::Fingerprints,
    /// debounce：首次检测到变化的时刻。
    dirty_since: Option<f64>,
}

impl Default for UsageState {
    fn default() -> Self {
        let mut state = Self {
            result: None,
            cached: false,
            rx: None,
            // 账本按项目既有约定放在配置目录（`Prefs::config_dir()`，
            // 与 settings.json / tokens.json 同目录），不自己拼路径。
            ledger: Ledger::open_default(),
            last_poll_at: None,
            last_save_at: None,
            fingerprints: scan_sources::Fingerprints::new(),
            dirty_since: None,
        };
        // 打开秒显：账本里有上次的数据就直接顶上，后台扫完再替换
        //（对齐 token-monitor：缓存保持可见，后台刷新期间不清屏）。
        state.fill_from_ledger();
        state
    }
}

impl UsageState {
    /// 是否有扫描在途（首次打开还没有结果时，界面据此显示「扫描中」）。
    pub(in crate::app) fn scanning(&self) -> bool {
        self.rx.is_some()
    }

    /// 账本条目数（含已归档的已删会话）。
    pub(in crate::app) fn ledger_len(&self) -> usize {
        self.ledger.len()
    }

    /// 打开秒显：账本里有数据就把上次的结果直接填进展示（后台扫完替换）。
    /// 返回是否填上了；账本为空时清掉旧展示与缓存标记（保持状态一致）。
    pub(in crate::app) fn fill_from_ledger(&mut self) -> bool {
        self.cached = false;
        if self.ledger.is_empty() {
            self.result = None;
            return false;
        }
        self.cached = true;
        self.result = Some(Ok(UsageReport {
            sessions: self.ledger.view(),
            daily: self.ledger.daily().clone(),
            // 缓存态不读 egui 时间轴的 scanned_at：时刻在 `cached_saved_at`。
            scanned_at: 0.0,
            message_count: 0,
            scan_ms: 0,
        }));
        true
    }

    /// 缓存展示中的数据时刻（账本落盘时刻，unix 毫秒）；
    /// `None` = 实时数据或还没有数据。
    pub(in crate::app) fn cached_saved_at(&self) -> Option<i64> {
        if self.cached {
            self.ledger.saved_at()
        } else {
            None
        }
    }

    /// 收下后台扫描结果：并入账本、替换展示视图、清掉缓存标记。
    fn adopt_scan(&mut self, payload: ScanPayload, now: f64) {
        let (sessions, daily, message_count, scan_ms) = payload;
        self.result = Some(Ok(merge_into_ledger(
            &mut self.ledger,
            sessions,
            daily,
            now,
            message_count,
            scan_ms,
        )));
        self.cached = false;
    }
}

/// 指纹比对的结果：这次该不该发起重扫。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum PollOutcome {
    /// 指纹没变：不重扫，并清掉 debounce 状态。
    Idle,
    /// 刚发现变化：记下时刻，还在合并窗口内。
    Debouncing,
    /// 该重扫了。
    Scan,
}

/// 纯逻辑的 debounce 判定（可单测）。
///
/// `changed` 是本次指纹比对发现的变化文件数；`dirty_since` 是上一次「首次发现
/// 变化」的时刻（`None` = 之前是干净的）。
///
/// 语义（带最大等待的尾沿防抖）：
/// - `changed > 0` 只负责**置脏**，不负责触发 —— 源里确实写了新数据，
///   就应该在窗口结束后扫一次，哪怕这期间它又安静下来了（否则「改了、
///   但下一次指纹检测时已经稳定」的那次改动会被永远跳过）；
/// - 已置脏时不再看 `changed`：等到距首次发现变化满 [`DEBOUNCE_SECS`] 就扫；
/// - [`MAX_WAIT_SECS`] 是安全网：只有把检测间隔调到小于防抖窗口时才用得上，
///   保留它是为了在调参时不会出现「流式写入永远等不到」的坑。
pub(in crate::app) fn poll_outcome(
    changed: usize,
    dirty_since: Option<f64>,
    now: f64,
) -> (PollOutcome, Option<f64>) {
    // 只在「干净 → 脏」的那一次记时刻；已置脏就保留最初的时刻，
    // 不让持续写入把窗口起点往后推。
    let since = match (changed > 0, dirty_since) {
        (_, Some(since)) => Some(since),
        (true, None) => Some(now),
        (false, None) => None,
    };
    let Some(since) = since else {
        return (PollOutcome::Idle, None);
    };
    if now - since >= DEBOUNCE_SECS || now - since >= MAX_WAIT_SECS {
        // 扫完就清脏；扫描期间新增的改动会在下一轮检测里重新置脏。
        return (PollOutcome::Scan, None);
    }
    (PollOutcome::Debouncing, Some(since))
}

/// 是否到了该做指纹检测的时刻（3 秒节流）。
pub(in crate::app) fn poll_due(last_poll_at: Option<f64>, now: f64) -> bool {
    match last_poll_at {
        None => true,
        Some(last) => now - last >= POLL_INTERVAL_SECS,
    }
}

/// debounce 期间该请求多久之后重绘。
///
/// 界面空闲时 egui 不重绘，不主动要帧就等不到「1.5 秒后该扫了」的那一刻。
/// 返回 `None` = 不需要额外请求（没在 debounce）。
///
/// 只认**还没到**的期限：把已过去的期限夹到 0 再取最小，会退化成 0.01 秒的
/// 空转重绘。检测间隔（3 秒）比合并窗口（1.5 秒）长，所以「刚过合并窗口、
/// 还没到下一个检测点」这个状态每轮都会出现，夹 0 会让界面连续空绘上百帧。
pub(in crate::app) fn repaint_delay(dirty_since: Option<f64>, now: f64) -> Option<f64> {
    let since = dirty_since?;
    // 取两个期限里更近的**未来**时刻；都过了才退化成最小节拍。
    let nearest = [since + DEBOUNCE_SECS - now, since + MAX_WAIT_SECS - now]
        .into_iter()
        .filter(|remaining| *remaining > 0.0)
        .fold(f64::INFINITY, f64::min);
    Some(if nearest.is_finite() {
        nearest
    } else {
        MIN_REPAINT_SECS
    })
}

impl App {
    /// 每帧调用：收扫描结果 + 按指纹变化决定是否重扫。
    ///
    /// 与 `poll_balance()` 并列放在 `App::update()` 里。扫描全程在后台线程，
    /// 这里只做「收结果 + stat 指纹 + spawn」。
    pub(super) fn poll_usage(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|input| input.time);

        // 1. 先收结果。`try_recv` 不阻塞：没有结果就立刻返回。
        let received = self.usage.rx.as_ref().map(Receiver::try_recv);
        match received {
            Some(Ok(Ok((sessions, daily, message_count, scan_ms)))) => {
                // 并入账本 + 产出展示视图。merge_scan 是 O(会话数)，几十条，UI 线程够用。
                self.usage
                    .adopt_scan((sessions, daily, message_count, scan_ms), now);
                self.usage.rx = None;
                self.save_usage_ledger(now);
            }
            Some(Ok(Err(err))) => {
                // 扫描失败：换上错误页（与无缓存时的行为一致）。缓存标记一并清掉，
                // 否则「有缓存 → 自动重扫」会在持续失败时每帧打转。
                self.usage.result = Some(Err(err));
                self.usage.cached = false;
                self.usage.rx = None;
            }
            Some(Err(TryRecvError::Empty)) => {}
            Some(Err(TryRecvError::Disconnected)) => {
                // 线程异常退出：结束等待，下一轮检测再决定要不要重扫。
                self.usage.rx = None;
            }
            None => {}
        }

        // 2. 首帧（或上次扫描失败后重开）自动扫一次：满足「每次打开软件自动更新」。
        //    缓存秒显也一样：账本只保证「有得看」，扫完才保证「是新的」。
        //    不等 3 秒，否则打开用量视图会先看到一段过期数字。
        if self.usage.rx.is_none() && (self.usage.result.is_none() || self.usage.cached) {
            self.start_usage_scan(now, ctx);
            return;
        }

        // 3. 扫描在途：不叠加扫描，但要按扫描轮询间隔要帧才能收到结果。
        if self.usage.rx.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(SCAN_POLL_REPAINT_SECS));
            return;
        }

        // 4. 3 秒节流：空闲时唯一的开销就是这一步。
        if !poll_due(self.usage.last_poll_at, now) {
            // 还在 debounce 里就按剩余时间要一帧，否则界面空闲时不会重绘。
            self.request_usage_repaint(ctx, now);
            return;
        }
        self.usage.last_poll_at = Some(now);

        // 5. 指纹比对：只有源真的变了才可能重扫。
        let current = scan_sources::fingerprint();
        let changed = scan_sources::changed_files(&self.usage.fingerprints, &current);
        // 指纹立刻记下最新的：变化持续时每轮都要刷新，否则下一轮拿「首次变化前」
        // 的快照去比会一直认为有变化（结果一样，但账目更清楚）。
        self.usage.fingerprints = current;
        let (outcome, dirty_since) = poll_outcome(changed, self.usage.dirty_since, now);
        self.usage.dirty_since = dirty_since;
        match outcome {
            PollOutcome::Idle => {}
            PollOutcome::Debouncing => self.request_usage_repaint(ctx, now),
            PollOutcome::Scan => self.start_usage_scan(now, ctx),
        }
    }

    /// debounce 期间按剩余时间请求重绘。
    ///
    /// 取「到 debounce 期限」与「到下一个检测点」里更近的那个：扫描只能在检测点
    /// 发起，所以只等期限会白绘一帧再重排，而只等检测点又会让 [`MAX_WAIT_SECS`]
    /// 在把检测间隔调小时失去作用。
    fn request_usage_repaint(&self, ctx: &egui::Context, now: f64) {
        let Some(deadline) = repaint_delay(self.usage.dirty_since, now) else {
            return;
        };
        let to_next_poll = match self.usage.last_poll_at {
            Some(last) => (last + POLL_INTERVAL_SECS - now).max(0.0),
            None => 0.0,
        };
        let delay = deadline.min(to_next_poll).max(MIN_REPAINT_SECS);
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(delay));
    }

    /// 发起一次后台扫描。UI 线程只负责 spawn 与收结果。
    fn start_usage_scan(&mut self, now: f64, ctx: &egui::Context) {
        let (tx, rx) = std::sync::mpsc::channel();
        // 已删除并永久忽略的会话：名单拷给后台线程，扫描时直接跳过，
        // 否则删掉的会话会在下一轮扫描里复活。
        let ignored = self.usage.ledger.ignored().clone();
        std::thread::spawn(move || {
            // 全量扫描（release 2.4-2.7s）：只能在后台线程跑。
            let payload = crate::usage::scan(&ignored).map(|scanned| {
                (
                    scanned.sessions,
                    scanned.daily,
                    scanned.message_count,
                    scanned.processing_time_ms,
                )
            });
            let _ = tx.send(payload);
        });
        self.usage.rx = Some(rx);
        self.usage.dirty_since = None;
        self.usage.last_poll_at = Some(now);
        // 扫描在途：按轮询间隔继续要帧，结果回来后界面自动换上新数据
        //（指示器已删，用户反馈）。
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(SCAN_POLL_REPAINT_SECS));
    }

    /// 注入一次「扫描完成」的结果（仅测试用）。
    ///
    /// 排版测试只关心表格怎么画，不想真去扫盘（2.4 秒起、且依赖本机数据）。
    /// 走 `merge_into_ledger` 而不是直接塞 `result`：与生产路径同构，
    /// 这样测试里的 `daily` 与 `sessions` 会经过同一套账本合并逻辑。
    #[cfg(test)]
    pub(in crate::app) fn set_usage_result_for_test(
        &mut self,
        sessions: Vec<SessionSnapshot>,
        daily: DailyMap,
    ) {
        // 用内存账本：否则会读用户真实账本，行数不定、断言不稳。
        // 换账本意味着构造时秒显的缓存数据一并作废。
        self.usage.ledger = Ledger::in_memory();
        self.usage.cached = false;
        let scanned: HashMap<String, SessionSnapshot> = sessions
            .into_iter()
            .map(|s| (SessionSnapshot::key(s.client, &s.session_id), s))
            .collect();
        let message_count = scanned.values().map(|s| s.message_count).sum::<i64>() as usize;
        let report = merge_into_ledger(
            &mut self.usage.ledger,
            scanned,
            daily,
            0.0,
            message_count,
            0,
        );
        self.usage.result = Some(Ok(report));
    }

    /// 账本落盘（按 [`SAVE_INTERVAL_SECS`] 节流）。
    ///
    /// 落盘失败不影响功能：账本留在内存里，本次会话的数字照样正确。
    fn save_usage_ledger(&mut self, now: f64) {
        if !self.usage.ledger.is_dirty() {
            return;
        }
        let due = match self.usage.last_save_at {
            None => true,
            Some(last) => now - last >= SAVE_INTERVAL_SECS,
        };
        if !due {
            return;
        }
        self.save_usage_ledger_unthrottled(now);
    }

    /// 立即落盘（不节流）：删除会话是显式动作，不该等攒批间隔。
    fn save_usage_ledger_unthrottled(&mut self, now: f64) {
        if !self.usage.ledger.is_dirty() {
            return;
        }
        let _ = self.usage.ledger.save();
        self.usage.last_save_at = Some(now);
    }

    /// 删除一个会话：从账本移除并**永久忽略**，立即刷新展示数据并落盘。
    ///
    /// 磁盘上的会话文件不动 —— 删的只是 ModelHarbor 自己的统计；之后的
    /// 扫描也跳过它（名单存在账本里，见 [`Ledger::ignore_session`]）。
    pub(in crate::app) fn forget_usage_session(&mut self, key: &str, now: f64) {
        self.usage.ledger.ignore_session(key);
        if let Some(Ok(report)) = self.usage.result.as_mut() {
            report
                .sessions
                .retain(|session| SessionSnapshot::key(session.client, &session.session_id) != key);
            report.daily = self.usage.ledger.daily().clone();
        }
        self.usage_delete_pending = None;
        self.save_usage_ledger_unthrottled(now);
    }

    /// 当前展示视图（各维度聚合的输入）。
    ///
    /// 扫描失败 / 还没扫完时返回空切片：界面会画「没有数据」而不是上一轮的旧数字。
    pub(in crate::app) fn usage_sessions(&self) -> &[SessionSnapshot] {
        match self
            .usage
            .result
            .as_ref()
            .and_then(|result| result.as_ref().ok())
        {
            Some(report) => &report.sessions,
            None => &[],
        }
    }

    /// 按天分桶（热力图与区间统计的输入）。
    ///
    /// 与 [`Self::usage_sessions`] 并列：两者服务于不同视图，都取自同一次扫描结果。
    /// 扫描失败 / 还没扫完时返回 `None`，界面据此画「没有数据」。
    pub(in crate::app) fn usage_daily(&self) -> Option<&DailyMap> {
        self.usage
            .result
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .map(|report| &report.daily)
    }

    /// 按当前 agent 筛选过滤后的按天分桶。
    ///
    /// 筛选方式是**只保留该 agent 的分解，并把合计换成它**：桶里的
    /// `by_client` 是现成的（按 label 分），拿它替掉 `totals` 即可，
    /// 不用重新扫一遍。热力图、区间统计、四个维度全都读这个结果，
    /// 所以「看单个 agent」和「看全部」走的是同一条代码路径。
    ///
    /// 模型与来源 agent 也跟着收窄：只看 pi 时，模型表里不该出现
    /// 只有 opencode 用过的模型。
    pub(in crate::app) fn usage_daily_filtered(&self) -> DailyMap {
        let Some(daily) = self.usage_daily() else {
            return DailyMap::new();
        };
        filter_daily_by_client(daily, self.usage_filter)
    }

    /// 当前筛选的 agent 名；未筛选时给「全部 agent」。
    pub(in crate::app) fn usage_filter_label(&self) -> String {
        match self.usage_filter {
            Some(client) => client.label().to_string(),
            None => "全部 agent".to_string(),
        }
    }

    /// 今天（本地日索引）。
    ///
    /// 区间边界都相对它算，所以集中在这里读一次系统时间，而不是每个函数各读一次
    /// （跨零点时同一帧内读到不同值会画出不一致的界面）。
    pub(in crate::app) fn usage_today(&self) -> i64 {
        let offset = crate::app::balance::local_utc_offset_secs();
        aggregate::local_day_index(unix_now_secs(), offset)
    }
}

/// 当前时间（Unix 秒）。
///
/// 放在 `mod.rs` 而不是各子模块里各写一份：`view` 与 `overview` 都要用，
/// 同一帧内两处读系统时间会跨零点不一致。
pub(in crate::app) fn unix_now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// 按 agent 筛选按天分桶：只保留该 agent 的用量，并把它当成这一天的新合计。
///
/// 筛选方式是**用现成的分解换掉合计**，不重新扫一遍：`by_client` 本来就是按
/// agent 分好的，拿它替掉 `totals` 即可。于是热力图、区间统计、四个维度表
/// 全都读同一份结果，「看单个 agent」与「看全部」走完全相同的代码路径。
///
/// 模型与会话也跟着收窄：只看 pi 时，模型表里不该出现只有 opencode 用过的模型。
/// 某天该 agent 没有用量就不生成空桶 —— 热力图靠「桶存不存在」区分
/// 「那天没用过」与「那天用了但为 0」。
fn filter_daily_by_client(daily: &DailyMap, filter: Option<ConfigFormat>) -> DailyMap {
    let Some(filter) = filter else {
        return daily.clone();
    };
    let label = filter.label();
    let mut out = DailyMap::new();
    for (date, bucket) in daily {
        let Some(totals) = bucket.by_client.get(label) else {
            continue;
        };
        let mut narrowed = DailyBucket::new(date);
        narrowed.totals = *totals;
        narrowed.by_client.insert(label.to_string(), *totals);
        // 交叉分解的键就是 agent label：拿它同时填 `by_model`，
        // 这样模型表天然只含该 agent 用过的模型。
        if let Some(models) = bucket.by_client_model.get(label) {
            narrowed
                .by_client_model
                .insert(label.to_string(), models.clone());
            for (model, model_totals) in models {
                narrowed.by_model.insert(model.clone(), *model_totals);
            }
        }
        // 会话键是 `"{client}:{session_id}"`（见 `SessionSnapshot::key`）。
        let prefix = format!("{label}:");
        for (session, session_totals) in &bucket.by_session {
            if session.starts_with(&prefix) {
                narrowed.by_session.insert(session.clone(), *session_totals);
            }
        }
        out.insert(date.clone(), narrowed);
    }
    out
}

/// 把扫描结果并进账本并产出展示视图。
///
/// 拆成自由函数是为了让「并入 → 视图」这段纯逻辑可以被单测直接调用
/// （不依赖 `App` 与 egui）。
pub(in crate::app) fn merge_into_ledger(
    ledger: &mut Ledger,
    scanned: HashMap<String, SessionSnapshot>,
    daily: DailyMap,
    now: f64,
    message_count: usize,
    scan_ms: u32,
) -> UsageReport {
    ledger.merge_scan(scanned);
    ledger.merge_daily(daily);
    UsageReport {
        sessions: ledger.view(),
        daily: ledger.daily().clone(),
        scanned_at: now,
        message_count,
        scan_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::usage::range::Range;
    use crate::usage::DayTotals;
    use std::collections::BTreeMap;

    /// 无筛选：原样返回（不做无谓的深拷贝）。
    #[test]
    fn filter_without_client_keeps_everything() {
        let mut daily = DailyMap::new();
        daily.insert("2026-01-01".into(), bucket_with_two_clients());
        let filtered = filter_daily_by_client(&daily, None);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered["2026-01-01"].totals.total(), 300);
    }

    /// 筛到单个 agent 时：合计换成该 agent 的，分解也跟着收窄。
    #[test]
    fn filter_narrows_totals_models_and_sessions() {
        let mut daily = DailyMap::new();
        daily.insert("2026-01-01".into(), bucket_with_two_clients());
        let filtered = filter_daily_by_client(&daily, Some(ConfigFormat::Pi));
        let bucket = &filtered["2026-01-01"];
        // 合计 = pi 自己的 200，不是全部的 300。
        assert_eq!(bucket.totals.total(), 200);
        assert_eq!(bucket.by_client.len(), 1, "只剩 pi 一项");
        assert!(bucket.by_client.contains_key(&pi_label()));
        // 模型表不能出现只有 opencode 用过的模型。
        assert!(bucket.by_model.contains_key("claude-opus-5"));
        assert!(
            !bucket.by_model.contains_key("gpt-5"),
            "别的 agent 的模型不该混进来"
        );
        // 会话按 `"{label}:"` 前缀收窄。
        assert_eq!(bucket.by_session.len(), 1);
        assert!(bucket.by_session.contains_key(&pi_session("session-a")));
    }

    /// 某天该 agent 没有用量：整桶不生成（热力图靠它区分「没用过」与「用了 0」）。
    #[test]
    fn filter_drops_days_without_that_client() {
        let mut daily = DailyMap::new();
        daily.insert("2026-01-01".into(), bucket_with_two_clients());
        daily.insert("2026-01-02".into(), bucket_with_one_client());
        // 两天都碰过 opencode → 两天都留下，但合计各自只剩 opencode 那份。
        let filtered = filter_daily_by_client(&daily, Some(ConfigFormat::Opencode));
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered["2026-01-01"].totals.total(), 100, "不含 pi 的 200");
        assert_eq!(filtered["2026-01-02"].totals.total(), 100);

        // 完全没用过的 agent：一天都不留。
        let none = filter_daily_by_client(&daily, Some(ConfigFormat::KimiCode));
        assert!(none.is_empty(), "没用过的 agent 不该生成空桶");
    }

    /// 筛选后逐日合计 == 筛选后区间合计（不丢不重）。
    #[test]
    fn filter_keeps_per_day_sum_consistent() {
        let mut daily = DailyMap::new();
        daily.insert("2026-01-01".into(), bucket_with_two_clients());
        daily.insert("2026-01-02".into(), bucket_with_one_client());
        let filtered = filter_daily_by_client(&daily, Some(ConfigFormat::Opencode));
        // `today` 要晚于桶里的日期：`Range::All` 会排除未来日期（防脏账本）。
        let today = aggregate::days_from_civil(2026, 1, 3);
        let summed: i64 = filtered.values().map(|bucket| bucket.totals.total()).sum();
        let from_range =
            crate::app::usage::range::totals_in_range(&filtered, Range::All, today).total();
        assert_eq!(summed, from_range);
        assert_eq!(summed, 200, "两天各 100 的 opencode 用量");
        // 逐日合计 == 各天 by_client 之和（分解与合计口径一致）。
        for bucket in filtered.values() {
            let per_client: i64 = bucket.by_client.values().map(|t| t.total()).sum();
            assert_eq!(bucket.totals.total(), per_client, "{}", bucket.date);
        }
    }

    /// 造一个含两个 agent 的桶：pi 200 + opencode 100，各自有模型与会话。
    ///
    /// 键一律走 [`ConfigFormat::label`]，不硬编码字符串：分解表的键就是
    /// label（见 `DailyBucket::add_message`），写错大小写会让筛选静默落空。
    fn bucket_with_two_clients() -> DailyBucket {
        let mut bucket = DailyBucket::new("2026-01-01");
        let pi = day_totals(200, 1);
        let opencode = day_totals(100, 1);
        bucket.totals = day_totals(300, 2);
        bucket.by_client.insert(pi_label(), pi);
        bucket.by_client.insert(opencode_label(), opencode);
        bucket.by_client_model.insert(
            pi_label(),
            BTreeMap::from([("claude-opus-5".to_string(), pi)]),
        );
        bucket.by_client_model.insert(
            opencode_label(),
            BTreeMap::from([("gpt-5".to_string(), opencode)]),
        );
        bucket.by_model.insert("claude-opus-5".into(), pi);
        bucket.by_model.insert("gpt-5".into(), opencode);
        bucket.by_session.insert(pi_session("session-a"), pi);
        bucket
            .by_session
            .insert(opencode_session("session-b"), opencode);
        bucket
    }

    /// 造一个只有 opencode 的桶。
    fn bucket_with_one_client() -> DailyBucket {
        let mut bucket = DailyBucket::new("2026-01-02");
        let opencode = day_totals(100, 1);
        bucket.totals = opencode;
        bucket.by_client.insert(opencode_label(), opencode);
        bucket.by_client_model.insert(
            opencode_label(),
            BTreeMap::from([("gpt-5".to_string(), opencode)]),
        );
        bucket.by_model.insert("gpt-5".into(), opencode);
        bucket
            .by_session
            .insert(opencode_session("session-b"), opencode);
        bucket
    }

    fn pi_label() -> String {
        ConfigFormat::Pi.label().to_string()
    }

    fn opencode_label() -> String {
        ConfigFormat::Opencode.label().to_string()
    }

    /// 会话键与 [`SessionSnapshot::key`] 同构：`client:session_id`。
    fn pi_session(id: &str) -> String {
        SessionSnapshot::key(ConfigFormat::Pi, id)
    }

    fn opencode_session(id: &str) -> String {
        SessionSnapshot::key(ConfigFormat::Opencode, id)
    }

    /// 输入 `input`，消息 `messages` 条，其余为 0。
    fn day_totals(input: i64, messages: i64) -> DayTotals {
        DayTotals {
            input,
            messages,
            ..DayTotals::default()
        }
    }

    #[test]
    fn poll_due_throttles_to_the_interval() {
        assert!(poll_due(None, 0.0), "首次应立刻检测");
        assert!(!poll_due(Some(10.0), 10.0));
        assert!(!poll_due(Some(10.0), 12.9));
        assert!(poll_due(Some(10.0), 13.0));
        assert!(poll_due(Some(10.0), 20.0));
    }

    #[test]
    fn no_change_while_clean_means_idle() {
        let (outcome, since) = poll_outcome(0, None, 200.0);
        assert_eq!(outcome, PollOutcome::Idle);
        assert_eq!(since, None);
    }

    #[test]
    fn first_change_starts_the_debounce_window() {
        let (outcome, since) = poll_outcome(3, None, 100.0);
        assert_eq!(outcome, PollOutcome::Debouncing);
        assert_eq!(since, Some(100.0), "记下首次变化时刻");
    }

    /// 关键回归：置脏之后即使源又安静下来（下一轮 `changed == 0`），
    /// 也必须把那次改动扫出来，不能直接回到 Idle。
    #[test]
    fn a_settled_change_is_still_scanned() {
        // 100 秒发现变化（还没到窗口）
        let (outcome, since) = poll_outcome(4, None, 100.0);
        assert_eq!(outcome, PollOutcome::Debouncing);
        // 101.5 秒时源已经安静（changed == 0），但改动仍待处理 → 该扫了。
        let (outcome, since) = poll_outcome(0, since, 101.5);
        assert_eq!(outcome, PollOutcome::Scan, "已置脏的改动不能被丢掉");
        assert_eq!(since, None, "扫完清脏");
    }

    #[test]
    fn change_settling_for_the_debounce_window_triggers_a_scan() {
        // 100 秒首次发现变化 → 101.4 秒还没到窗口
        let (outcome, since) = poll_outcome(1, Some(100.0), 101.4);
        assert_eq!(outcome, PollOutcome::Debouncing);
        assert_eq!(since, Some(100.0));
        // 101.5 秒（正好 DEBOUNCE_SECS）触发
        let (outcome, _) = poll_outcome(1, Some(100.0), 101.5);
        assert_eq!(outcome, PollOutcome::Scan);
    }

    #[test]
    fn continuous_writes_are_capped_by_the_max_wait() {
        // 检测间隔缩到小于防抖窗口时，MAX_WAIT_SECS 是唯一能救场的兜底：
        // 每轮都还有新变化（changed > 0），窗口起点不被推后。
        let mut since = None;
        let mut scanned_at = None;
        for tick in 0..40 {
            let now = 100.0 + tick as f64 * 0.25;
            let (outcome, next) = poll_outcome(5, since, now);
            since = next;
            if outcome == PollOutcome::Scan {
                scanned_at = Some(now);
                break;
            }
        }
        let scanned_at = scanned_at.expect("持续写入也必须触发重扫");
        assert!(
            scanned_at - 100.0 >= DEBOUNCE_SECS,
            "防抖窗口没到之前不该扫：{scanned_at}"
        );
        assert!(
            scanned_at - 100.0 <= MAX_WAIT_SECS,
            "最晚到 MAX_WAIT_SECS 就该扫：{scanned_at}"
        );
    }

    #[test]
    fn debounce_window_start_is_not_pushed_back_by_later_changes() {
        // 这是「永远等不到」的经典 bug：如果每轮都刷新 dirty_since，
        // 流式写入会让 waited 永远是 0。
        let (_, since) = poll_outcome(1, None, 100.0);
        assert_eq!(since, Some(100.0));
        let (outcome, since) = poll_outcome(1, since, 101.0);
        assert_eq!(since, Some(100.0), "窗口起点不能被推后");
        assert_eq!(outcome, PollOutcome::Debouncing);
        let (outcome, _) = poll_outcome(1, since, 101.5);
        assert_eq!(outcome, PollOutcome::Scan);
    }

    /// 每 3 秒检测一次时的真实节奏：改动在下一次检测就该被扫出来。
    #[test]
    fn realistic_poll_cadence_triggers_on_the_next_check() {
        let mut since = None;
        let mut scanned_at = None;
        for tick in 0..10 {
            let now = 100.0 + tick as f64 * POLL_INTERVAL_SECS;
            // 第一轮检测到变化，之后源安静（changed == 0）。
            let changed = usize::from(tick == 0);
            let (outcome, next) = poll_outcome(changed, since, now);
            since = next;
            if outcome == PollOutcome::Scan {
                scanned_at = Some(now);
                break;
            }
        }
        assert_eq!(
            scanned_at,
            Some(100.0 + POLL_INTERVAL_SECS),
            "改动在下一次检测（3 秒）就该被扫出来"
        );
    }

    #[test]
    fn repaint_delay_asks_for_the_nearest_deadline() {
        // 刚发现变化：等 1.5 秒（比 5 秒上限近）。
        assert_eq!(repaint_delay(Some(100.0), 100.0), Some(DEBOUNCE_SECS));
        // 已等了 4.6 秒：离上限只剩 0.4 秒。
        let delay = repaint_delay(Some(100.0), 104.6).expect("应要一帧");
        assert!((delay - 0.4).abs() < 1e-9, "实际 {delay}");
        // 已经过了两个期限：给一个正的最小值，不能是 0 / 负数。
        assert_eq!(repaint_delay(Some(100.0), 200.0), Some(0.01));
        // 没在 debounce：不要帧。
        assert_eq!(repaint_delay(None, 100.0), None);
    }

    /// 端到端：并入账本 → 视图；已删会话保留在展示里。
    #[test]
    fn merge_into_ledger_keeps_deleted_sessions_in_the_view() {
        use crate::format::ConfigFormat;
        let snapshot = |session: &str, input: i64| SessionSnapshot {
            client: ConfigFormat::Pi,
            session_id: session.to_string(),
            model_id: "m".to_string(),
            project: None,

            input,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
            message_count: 1,
            cost: 0.0,
            first_seen_ms: 1_000,
            last_seen_ms: 2_000,
            archived: false,
        };
        let scan_of = |items: Vec<SessionSnapshot>| {
            items
                .into_iter()
                .map(|s| (SessionSnapshot::key(s.client, &s.session_id), s))
                .collect::<HashMap<_, _>>()
        };

        let mut ledger = Ledger::in_memory();
        let first = merge_into_ledger(
            &mut ledger,
            scan_of(vec![snapshot("a", 100)]),
            DailyMap::new(),
            1.0,
            7,
            42,
        );
        assert_eq!(first.sessions.len(), 1);
        assert_eq!(first.message_count, 7);
        assert_eq!(first.scan_ms, 42);
        assert_eq!(first.scanned_at, 1.0);
        assert_eq!(first.total(), 100);

        // 下一轮 a 消失了 → 仍留在视图里，并标记 archived。
        let second = merge_into_ledger(&mut ledger, scan_of(vec![]), DailyMap::new(), 2.0, 0, 10);
        assert_eq!(second.sessions.len(), 1, "已删除的会话必须保留");
        assert!(second.sessions[0].archived);
        assert_eq!(second.total(), 100, "总量不因源里消失而缩水");
    }

    /// 按天分桶也要穿过 `merge_into_ledger` 进到报告里（热力图靠它）。
    #[test]
    fn merge_into_ledger_carries_daily_buckets() {
        let mut bucket = crate::usage::DailyBucket::new("2026-03-15");
        bucket.totals.input = 500;
        bucket.totals.messages = 3;
        let daily: DailyMap = [("2026-03-15".to_string(), bucket)].into_iter().collect();

        let mut ledger = Ledger::in_memory();
        let report = merge_into_ledger(&mut ledger, HashMap::new(), daily, 1.0, 3, 10);
        assert_eq!(report.daily.len(), 1, "按天分桶要出现在报告里");
        assert_eq!(report.daily["2026-03-15"].totals.input, 500);

        // 下一轮扫描没有新数据：账本里的按天记录要留着（已删会话的用量靠它）。
        let second = merge_into_ledger(&mut ledger, HashMap::new(), DailyMap::new(), 2.0, 0, 10);
        assert_eq!(second.daily.len(), 1, "历史按天记录不因本轮没扫到而消失");
        assert_eq!(second.daily["2026-03-15"].totals.input, 500);
    }

    /// 打开秒显：账本非空 → 直接可展示（不用等扫描），后台扫完回到实时态。
    #[test]
    fn fill_from_ledger_shows_cached_data_until_scan_lands() {
        use crate::format::ConfigFormat;
        let snapshot = |session: &str, input: i64| SessionSnapshot {
            client: ConfigFormat::Pi,
            session_id: session.to_string(),
            model_id: "m".to_string(),
            project: None,
            input,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
            message_count: 1,
            cost: 0.0,
            first_seen_ms: 1_000,
            last_seen_ms: 2_000,
            archived: false,
        };
        let scan_of = |items: Vec<SessionSnapshot>| {
            items
                .into_iter()
                .map(|s| (SessionSnapshot::key(s.client, &s.session_id), s))
                .collect::<HashMap<_, _>>()
        };

        let mut state = UsageState {
            result: None,
            cached: false,
            rx: None,
            ledger: Ledger::in_memory(),
            last_poll_at: None,
            last_save_at: None,
            fingerprints: scan_sources::Fingerprints::new(),
            dirty_since: None,
        };
        state.ledger.merge_scan(scan_of(vec![snapshot("a", 100)]));
        assert!(state.fill_from_ledger(), "账本非空应当填上");
        let report = state
            .result
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .expect("缓存秒显应当有可展示数据");
        assert_eq!(report.sessions.len(), 1);
        assert!(state.cached, "展示的是账本顶上的缓存");

        // 后台扫描收尾：替换展示并清掉缓存标记。
        state.adopt_scan(
            (scan_of(vec![snapshot("a", 150)]), DailyMap::new(), 1, 5),
            3.0,
        );
        assert!(!state.cached, "扫完要回到实时态");
        assert!(state.cached_saved_at().is_none());
        let report = state.result.as_ref().and_then(|r| r.as_ref().ok()).unwrap();
        assert_eq!(report.sessions[0].input, 150, "展示要换成最新扫描");
    }

    /// 账本为空 → 不填充：界面维持「还没有扫描结果 · 扫描中」。
    #[test]
    fn fill_from_ledger_skips_empty_ledger() {
        // 显式构造，不碰用户真实账本（default() 会读它并秒显）。
        let mut state = UsageState {
            result: None,
            cached: false,
            rx: None,
            ledger: Ledger::in_memory(),
            last_poll_at: None,
            last_save_at: None,
            fingerprints: scan_sources::Fingerprints::new(),
            dirty_since: None,
        };
        assert!(!state.fill_from_ledger());
        assert!(state.result.is_none());
        assert!(!state.cached);
        assert!(state.cached_saved_at().is_none());
    }

    #[test]
    fn report_total_sums_every_session() {
        use crate::format::ConfigFormat;
        let snapshot = |client: ConfigFormat, input: i64, archived: bool| SessionSnapshot {
            client,
            session_id: "s".into(),
            model_id: "m".into(),
            project: None,

            input,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            reasoning: 0,
            message_count: 1,
            cost: 0.0,
            first_seen_ms: 0,
            last_seen_ms: 1,
            archived,
        };
        let report = UsageReport {
            sessions: vec![
                snapshot(ConfigFormat::Pi, 10, false),
                snapshot(ConfigFormat::Opencode, 100, true),
            ],
            daily: DailyMap::new(),
            scanned_at: 0.0,
            message_count: 2,
            scan_ms: 5,
        };
        assert_eq!(report.total(), 110);
    }
}
