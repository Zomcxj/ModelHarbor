//! The "Local Usage" state machine: background scan + 3s change detection + debounce.
//!
//! Follows the [`crate::app::balance`] pattern: a background thread plus an `mpsc::Receiver`; the UI
//! thread calls `try_recv` each frame to pick up results — **never call `usage::scan()` on the UI thread**.
//!
//! A rescan is triggered when the **source-file fingerprints change**, not when "3 seconds passed":
//!
//! ```text
//! Every 3 seconds, compare fingerprints
//!   ├─ unchanged ────────────► do nothing (idle CPU ≈ 0)
//!   └─ changed ──► note first-change time, wait 1.5s (coalesce consecutive writes)
//!                  ├─ quiet for 1.5s ──► rescan
//!                  └─ still changing (agent streaming writes) ──► rescan after at most 5s
//! ```
//!
//! Debounce parameters mirror token-monitor (`DEBOUNCE_SECS` / `MAX_WAIT_SECS` /
//! `POLL_INTERVAL_SECS`), **with no extra cooldown** — token-monitor states in its source:
//! "There is deliberately no cooldown on top of the debounce". The debounce already coalesces
//! consecutive writes; adding a cooldown would only make the numbers feel laggy.

pub(in crate::app) mod aggregate;
mod chart;
pub(in crate::app) use chart::ChartKind;
mod component;
// Layout tests assert on the line-spacing budget (the session table is 3x an ordinary table).
#[cfg(test)]
pub(in crate::app) use component::ROW_PITCH;
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

/// Coalescing window for consecutive changes: after the first change is seen, wait this long and only rescan if nothing else changed.
pub(in crate::app) const DEBOUNCE_SECS: f64 = 1.5;

/// Hard trigger cap while writes keep coming: when an agent streams a session file, the fingerprint changes on every poll,
/// and "quiet for 1.5s" may never arrive — so force a rescan after at most this long.
pub(in crate::app) const MAX_WAIT_SECS: f64 = 5.0;

/// Fingerprint poll interval: source files are only stat-ed every 3s (the sole idle-time cost).
pub(in crate::app) const POLL_INTERVAL_SECS: f64 = 3.0;

/// Minimum interval between ledger flushes: at most one flush per detection cycle, so no need to write on every scan.
pub(in crate::app) const SAVE_INTERVAL_SECS: f64 = 30.0;

/// Poll interval while a scan is in flight: a scan takes far longer than a frame, and the UI won't repaint on its own
/// while idle, so we must explicitly request frames to collect results promptly.
const SCAN_POLL_REPAINT_SECS: f64 = 0.25;

/// Minimum interval between repaint requests: when both deadlines have passed, fall back to this beat
/// so a 0s interval (repaint every frame) can't spin the UI idle.
pub(in crate::app) const MIN_REPAINT_SECS: f64 = 0.01;

/// Segmented button: selected state = hover-tinted fill + 2px border, same style as the top-bar page tabs.
///
/// Fixed width (doesn't jump with label length); returns whether it was clicked. The usage view's
/// range / dimension / chart-toggle groups all reuse it instead of restating the selected style.
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
    let accent = visuals.widgets.hovered.bg_fill;
    let corner = ui.visuals().widgets.inactive.corner_radius;
    let response = ui.add(
        egui::Button::new(text)
            .fill(fill)
            .stroke(stroke)
            .min_size(egui::vec2(width, 22.0)),
    );
    // Click acknowledgment: an accent ring expanding from the chip.
    crate::motion::click_pulse(ui, &response, accent, corner);
    response.clicked()
}

/// Series colors: the accent hue rotates by the golden angle; series 0 is the accent itself.
///
/// Golden angle (137.508°) instead of even spacing: with many series, evenly spaced hues collide;
/// the golden angle keeps any prefix of series well separated.
///
/// Saturation and lightness are clamped per theme: on light themes highly saturated colors float off
/// pale backgrounds; on dark themes low-lightness colors blur into deep backgrounds.
///
/// Shared by the trend chart's stacked series and the overview cards' composition bars so both match.
pub(in crate::app) fn series_color(
    index: usize,
    accent: egui::Color32,
    dark: bool,
) -> egui::Color32 {
    // Series 0 uses the accent color as-is: the s/v clamping below exists to keep "generated" colors
    // readable on both light and dark fills, but the accent itself was tuned by the theme author and
    // must not be touched. (`Hsva` computes in linear space; a round trip would shift the accent
    // several steps off.)
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

/// Raw scan result delivered from the background thread: session table, daily buckets, message counts, engine-reported duration.
type ScanPayload = (HashMap<String, SessionSnapshot>, DailyMap, usize, u32);

/// A finished scan (already merged into the ledger).
pub(in crate::app) struct UsageReport {
    /// Display view: live scan results ∪ sessions gone from the sources but kept in the ledger, most recent activity first.
    pub sessions: Vec<SessionSnapshot>,
    /// Buckets per local calendar day (aggregates and breakdowns keep deleted sessions' usage; details don't).
    ///
    /// The heatmap and "today / week / month / total" all read from here: session-level sums can't do this —
    /// one session may span dozens of days.
    pub daily: DailyMap,
    /// When this scan finished (egui timeline seconds), for the "updated N seconds ago" display.
    pub scanned_at: f64,
    /// Message count reported by the engine.
    pub message_count: usize,
    /// Scan duration self-reported by the engine (milliseconds).
    pub scan_ms: u32,
}

impl UsageReport {
    /// Total tokens.
    ///
    /// Test-only: the production total goes through the daily buckets (`range::range_stats`), which can
    /// filter by range — session-level sums can't.
    #[cfg(test)]
    pub(in crate::app) fn total(&self) -> i64 {
        self.sessions.iter().map(SessionSnapshot::total).sum()
    }
}

/// Scan state for the "Local Usage" page.
pub(in crate::app) struct UsageState {
    /// The last successful scan's result (already merged into the ledger).
    pub result: Option<Result<UsageReport, String>>,
    /// `result` currently holds the ledger-topped cached data (instant display on open): replaced once the background scan finishes.
    cached: bool,
    /// Receiver while a scan is in flight.
    rx: Option<Receiver<Result<ScanPayload, String>>>,
    /// The ledger (keeps deleted sessions across scans).
    ledger: Ledger,
    /// Time of the last fingerprint check (egui seconds).
    last_poll_at: Option<f64>,
    /// Time of the last ledger flush (egui seconds).
    last_save_at: Option<f64>,
    /// Source-file fingerprints: path → (mtime_secs, size).
    fingerprints: scan_sources::Fingerprints,
    /// Debounce: the moment the first change was detected.
    dirty_since: Option<f64>,
}

impl Default for UsageState {
    fn default() -> Self {
        let mut state = Self {
            result: None,
            cached: false,
            rx: None,
            // The ledger sits in the config dir per the project's existing convention (`Prefs::config_dir()`,
            // same directory as settings.json / tokens.json); no hand-rolled paths.
            ledger: Ledger::open_default(),
            last_poll_at: None,
            last_save_at: None,
            fingerprints: scan_sources::Fingerprints::new(),
            dirty_since: None,
        };
        // Instant display: with data in the ledger, surface it immediately and replace after the background
        // scan (aligned with token-monitor: the cache stays visible, the screen isn't cleared mid-refresh).
        state.fill_from_ledger();
        state
    }
}

impl UsageState {
    /// Whether a scan is in flight (the UI shows "scanning" on first open when there's no result yet).
    pub(in crate::app) fn scanning(&self) -> bool {
        self.rx.is_some()
    }

    /// Count of sessions gone from the sources, existing only in the ledger.
    pub(in crate::app) fn ledger_archived_len(&self) -> usize {
        self.ledger.archived_len()
    }

    /// Instant display: with ledger data, fill the last result straight into the display (replaced after the background scan).
    /// Returns whether anything was filled; an empty ledger clears the old display and cache flag (state stays consistent).
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
            // In cached mode, don't read the egui timeline's scanned_at: the moment lives in `cached_saved_at`.
            scanned_at: 0.0,
            message_count: 0,
            scan_ms: 0,
        }));
        true
    }

    /// Data timestamp while showing the cache (the ledger's flush time, unix millis);
    /// `None` = live data or no data yet.
    pub(in crate::app) fn cached_saved_at(&self) -> Option<i64> {
        if self.cached {
            self.ledger.saved_at()
        } else {
            None
        }
    }

    /// Accept a background scan result: merge into the ledger, replace the display view, clear the cache flag.
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

/// Fingerprint comparison verdict: whether to kick off a rescan this time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum PollOutcome {
    /// Fingerprints unchanged: no rescan, and the debounce state is cleared.
    Idle,
    /// Change just noticed: record the moment; still inside the coalescing window.
    Debouncing,
    /// Time to rescan.
    Scan,
}

/// Pure debounce logic (unit-testable).
///
/// `changed` is the number of changed files found by this fingerprint comparison; `dirty_since` is the
/// moment the change was first noticed (`None` = it was clean before).
///
/// Semantics (trailing-edge debounce with a max wait):
/// - `changed > 0` only **marks dirty**, it does not trigger — new data did land in the sources, so a
///   scan should happen once the window closes, even if the source went quiet again in the meantime
///   (otherwise a change that "was made but had already settled by the next fingerprint check" would
///   be skipped forever);
/// - once dirty, `changed` is ignored: when [`DEBOUNCE_SECS`] has passed since the first sighting, scan;
/// - [`MAX_WAIT_SECS`] is a safety net: only needed if the poll interval is tuned below the debounce
///   window, kept so parameter tweaking can't hit the "streaming writes never settle" trap.
pub(in crate::app) fn poll_outcome(
    changed: usize,
    dirty_since: Option<f64>,
    now: f64,
) -> (PollOutcome, Option<f64>) {
    // Record the moment only on the "clean → dirty" transition; once dirty, the original moment is kept —
    // continuous writes must not push the window's origin later.
    let since = match (changed > 0, dirty_since) {
        (_, Some(since)) => Some(since),
        (true, None) => Some(now),
        (false, None) => None,
    };
    let Some(since) = since else {
        return (PollOutcome::Idle, None);
    };
    if now - since >= DEBOUNCE_SECS || now - since >= MAX_WAIT_SECS {
        // Clear dirty after the scan; changes arriving mid-scan re-dirty on the next check round.
        return (PollOutcome::Scan, None);
    }
    (PollOutcome::Debouncing, Some(since))
}

/// Whether it's time for a fingerprint check (3s throttle).
pub(in crate::app) fn poll_due(last_poll_at: Option<f64>, now: f64) -> bool {
    match last_poll_at {
        None => true,
        Some(last) => now - last >= POLL_INTERVAL_SECS,
    }
}

/// How soon the debounce phase should request a repaint.
///
/// egui doesn't repaint while idle; without requesting frames we'd never reach "coalescing window
/// expired, time to scan". Returns `None` = no extra request needed (not debouncing).
///
/// Only **not-yet-due** deadlines count: clamping an overdue deadline to 0 and taking the min would
/// degenerate into 0.01s busy repaints. The poll interval (3s) is longer than the coalescing window
/// (1.5s), so the state "window expired, next poll not yet due" occurs every cycle — clamping to 0
/// would spin hundreds of wasted repaint frames.
pub(in crate::app) fn repaint_delay(dirty_since: Option<f64>, now: f64) -> Option<f64> {
    let since = dirty_since?;
    // Take the nearer **future** of the two deadlines; only when both have passed fall back to the minimum beat.
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
    /// Called every frame: collect scan results + decide on a rescan from fingerprint changes.
    ///
    /// Sits beside `poll_balance()` in `App::update()`. The scan runs entirely on a background thread;
    /// this only does "collect results + stat fingerprints + spawn".
    pub(super) fn poll_usage(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|input| input.time);

        // 1. Collect results first. `try_recv` doesn't block: returns immediately with no result.
        let received = self
            .usage
            .rx
            .as_ref()
            .map(std::sync::mpsc::Receiver::try_recv);
        match received {
            Some(Ok(Ok(payload))) => {
                // Merge into the ledger + produce the display view. merge_scan is O(sessions) — dozens — fine on the UI thread.
                self.usage.adopt_scan(payload, now);
                self.save_usage_ledger(now);
            }
            Some(Ok(Err(err))) => {
                // Scan failed: swap in the error page (same behavior as having no cache). Clear the cache flag too,
                // otherwise "has cache → auto rescan" would spin every frame on persistent failure.
                self.usage.result = Some(Err(err));
                self.usage.cached = false;
            }
            Some(Err(TryRecvError::Empty)) => {}
            Some(Err(TryRecvError::Disconnected)) => {
                // Thread died abnormally: end the wait; the next check round decides whether to rescan.
                self.usage.rx = None;
            }
            None => {}
        }

        // 2. First frame (or reopening after a failed scan) scans once automatically: fulfilling "auto-refresh on every open".
        //    Instant cache display is the same: the ledger only guarantees "something to look at"; the scan is what
        //    guarantees "it's fresh". Waiting 3 seconds would show stale numbers first when opening the view.
        if self.usage.rx.is_none() && (self.usage.result.is_none() || self.usage.cached) {
            self.start_usage_scan(now, ctx);
            return;
        }

        // 3. Scan in flight: don't stack scans, but request frames at the polling interval to receive results.
        if self.usage.rx.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(SCAN_POLL_REPAINT_SECS));
            return;
        }

        // 4. 3-second throttle: this step is the only idle-time cost.
        if !poll_due(self.usage.last_poll_at, now) {
            // While debouncing, request a frame for the remaining time; otherwise the idle UI never repaints.
            self.request_usage_repaint(ctx, now);
            return;
        }
        self.usage.last_poll_at = Some(now);

        // 5. Fingerprint comparison: only real source changes can lead to a rescan.
        let current = scan_sources::fingerprint();
        let changed = scan_sources::changed_files(&self.usage.fingerprints, &current);
        // Record the newest fingerprint immediately: while changes keep coming, every round must refresh, or the
        // next round would compare against a pre-first-change snapshot and always see a difference (same outcome,
        // but the bookkeeping is cleaner).
        self.usage.fingerprints = current;
        let (outcome, dirty_since) = poll_outcome(changed, self.usage.dirty_since, now);
        self.usage.dirty_since = dirty_since;
        match outcome {
            // Even when idle, request a frame for "the next check point": egui doesn't repaint on its own while idle,
            // and without frame requests the periodic check starves — later source changes would never be noticed
            // (only mouse movement would revive it). That's exactly how the "half a minute without refresh" defect arose.
            PollOutcome::Idle => {
                let delay = match self.usage.last_poll_at {
                    Some(last) => (last + POLL_INTERVAL_SECS - now).max(MIN_REPAINT_SECS),
                    None => MIN_REPAINT_SECS,
                };
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(delay));
            }
            PollOutcome::Debouncing => self.request_usage_repaint(ctx, now),
            PollOutcome::Scan => self.start_usage_scan(now, ctx),
        }
    }

    /// Request repaints during the debounce phase, by remaining time.
    ///
    /// Takes the nearer of "the debounce deadline" and "the next check point": scans can only start at a
    /// check point, so waiting only for the deadline wastes one frame before reordering, while waiting only
    /// for the check point renders [`MAX_WAIT_SECS`] useless when the poll interval is tuned down. When both
    /// deadlines have passed, the only meaningful frame is the check point itself: taking the min anyway
    /// would degenerate into [`MIN_REPAINT_SECS`]-beat busy repaints (most visible when the poll interval
    /// far exceeds the coalescing window).
    fn request_usage_repaint(&self, ctx: &egui::Context, now: f64) {
        let Some(deadline) = repaint_delay(self.usage.dirty_since, now) else {
            return;
        };
        let to_next_poll = match self.usage.last_poll_at {
            Some(last) => (last + POLL_INTERVAL_SECS - now).max(0.0),
            None => 0.0,
        };
        let delay = if deadline <= MIN_REPAINT_SECS {
            to_next_poll
        } else {
            deadline.min(to_next_poll)
        };
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(
            delay.max(MIN_REPAINT_SECS),
        ));
    }

    /// Start one background scan. The UI thread only spawns and collects.
    fn start_usage_scan(&mut self, now: f64, ctx: &egui::Context) {
        let (tx, rx) = std::sync::mpsc::channel::<Result<ScanPayload, String>>();
        // Deleted and permanently ignored sessions: the list is copied to the background thread and skipped
        // during the scan, otherwise deleted sessions would revive on the next scan (their contributions already
        // merged into the daily buckets are kept as usual — see [`Ledger::ignore_session`]).
        let ignored = self.usage.ledger.ignored().clone();
        std::thread::spawn(move || {
            // A full scan takes far longer than a frame: it can only run on a background thread.
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
        // Scan in flight: keep requesting frames at the polling interval; the UI swaps in new data when results arrive
        // (the spinner was removed per user feedback).
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(SCAN_POLL_REPAINT_SECS));
    }

    /// Inject a "scan finished" result (tests only).
    ///
    /// Layout tests care about how tables draw, not real disk scans (and must not depend on local data).
    /// Goes through `merge_into_ledger` instead of stuffing `result` directly: isomorphic to the production
    /// path, so the tests' `daily` and `sessions` pass through the same ledger merge logic.
    #[cfg(test)]
    pub(in crate::app) fn set_usage_result_for_test(
        &mut self,
        sessions: Vec<SessionSnapshot>,
        daily: DailyMap,
    ) {
        // Use an in-memory ledger: otherwise the user's real ledger loads, with unpredictable rows and flaky assertions.
        // Swapping the ledger also invalidates any cache data filled during construction.
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

    /// Test hook: pre-seed source fingerprints and the clean state so the next [`Self::poll_usage`]
    /// lands directly in the idle branch (no waiting for real file changes, no background scan triggered).
    #[cfg(test)]
    pub(in crate::app) fn seed_usage_idle_for_test(&mut self) {
        self.usage.fingerprints = scan_sources::fingerprint();
        self.usage.dirty_since = None;
        self.usage.last_poll_at = None;
        self.usage.rx = None;
    }

    /// Flush the ledger (throttled by [`SAVE_INTERVAL_SECS`]).
    ///
    /// Flush failure doesn't affect the feature: the ledger stays in memory and this session's numbers stay correct.
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

    /// Flush immediately (no throttle): deleting a session is an explicit action and shouldn't wait for the batching interval.
    fn save_usage_ledger_unthrottled(&mut self, now: f64) {
        if !self.usage.ledger.is_dirty() {
            return;
        }
        let _ = self.usage.ledger.save();
        self.usage.last_save_at = Some(now);
    }

    /// Delete a session: first remove it **from the source**, then withdraw its details from the statistics.
    ///
    /// The two halves are one action:
    /// - **Source** (the agent's own database / session files): really deleted — see [`crate::usage::delete_session`].
    ///   On failure, [`DeleteError::blocks_ledger`] decides whether the statistics half proceeds:
    ///   recoverable failures (agent running, database write error) do **neither half** — once the row
    ///   disappears from the list the user can never click it again, and the source session would be
    ///   stuck undeletable; unrecoverable failures (backend can't delete sources) have no retry to speak
    ///   of, so statistics proceed as usual.
    /// - **Ledger**: the session snapshot and per-day `by_session` details are really deleted; per-dimension
    ///   totals and breakdowns stay untouched, so totals, the heatmap, and the time table don't shrink from
    ///   deletion. The key also enters the ignore list, blocking "scans already in flight" from merging it back.
    pub(in crate::app) fn forget_usage_session(&mut self, key: &str, now: f64) {
        let source = crate::usage::delete_session(key);
        self.apply_session_removal(key, source, now);
    }

    /// The deletion's second half (split from source deletion: tests run the same logic with both "source deleted" and
    /// "source blocked" inputs, without depending on which agents this machine really has installed).
    pub(in crate::app) fn apply_session_removal(
        &mut self,
        key: &str,
        source: Result<crate::usage::Deleted, crate::usage::DeleteError>,
        now: f64,
    ) {
        if let Err(err) = &source {
            if err.blocks_ledger() {
                self.usage_source_error = Some(err.message());
                self.usage_delete_pending = None;
                return;
            }
        }
        self.usage_source_error = source.err().map(|err| err.message());
        self.usage.ledger.ignore_session(key);
        if let Some(Ok(report)) = self.usage.result.as_mut() {
            report
                .sessions
                .retain(|session| SessionSnapshot::key(session.client, &session.session_id) != key);
            // The in-memory display data withdraws its details in sync: don't wait for the next scan,
            // otherwise the ledger has dropped them while the reports still carry them — inconsistent.
            for bucket in report.daily.values_mut() {
                bucket.remove_session_detail(key);
            }
        }
        self.usage_delete_pending = None;
        self.save_usage_ledger_unthrottled(now);
    }

    /// The current display view (input to every dimension's aggregation).
    ///
    /// Returns an empty slice on scan failure / before the first scan finishes: the UI paints "no data"
    /// instead of last round's stale numbers.
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

    /// The daily buckets (input to the heatmap and range stats).
    ///
    /// Parallel to [`Self::usage_sessions`]: the two serve different views, both taken from the same scan
    /// result. Returns `None` on scan failure / before completion, and the UI paints "no data" accordingly.
    pub(in crate::app) fn usage_daily(&self) -> Option<&DailyMap> {
        self.usage
            .result
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .map(|report| &report.daily)
    }

    /// Daily buckets filtered by the current agent selection.
    ///
    /// The filter **keeps only that agent's breakdown and swaps it in as the total**: the bucket's
    /// `by_client` is ready-made (keyed by label), so it simply replaces `totals` — no rescan needed.
    /// The heatmap, range stats, and all four dimensions read this one result, so "view one agent" and
    /// "view all" share a single code path.
    ///
    /// Models and source agents narrow too: when only pi is selected, the model table must not list models
    /// only opencode ever used.
    pub(in crate::app) fn usage_daily_filtered(&self) -> DailyMap {
        let Some(daily) = self.usage_daily() else {
            return DailyMap::new();
        };
        filter_daily_by_client(daily, self.usage_filter)
    }

    /// The currently filtered agent name; "all agents" when unfiltered.
    pub(in crate::app) fn usage_filter_label(&self) -> String {
        match self.usage_filter {
            Some(client) => client.label().to_string(),
            None => "全部 agent".to_string(),
        }
    }

    /// Today (local day index).
    ///
    /// All range bounds are computed relative to it, so the system clock is read once here instead of once
    /// per function (a frame straddling midnight reading different values would paint an inconsistent UI).
    pub(in crate::app) fn usage_today(&self) -> i64 {
        let offset = crate::app::balance::local_utc_offset_secs();
        aggregate::local_day_index(unix_now_secs(), offset)
    }
}

/// Current time (Unix seconds).
///
/// Lives in `mod.rs` rather than duplicated per submodule: both `view` and `overview` need it, and
/// two reads of the system clock within one frame could straddle midnight and disagree.
pub(in crate::app) fn unix_now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// Filter daily buckets by agent: keep only that agent's usage and treat it as the day's new total.
///
/// The filter **swaps the ready-made breakdown in for the total** instead of rescanning: `by_client` is
/// already grouped per agent, so it replaces `totals` directly. The heatmap, range stats, and all four
/// dimension tables then read the same result — "view one agent" and "view all" share one code path.
///
/// Models and sessions narrow too: when only pi is selected, the model table must not list models
/// only opencode ever used. A day with no usage for the agent gets no bucket at all — the heatmap
/// distinguishes "didn't use that day" from "used but zero" by a bucket's mere existence.
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
        // The cross-breakdown key is the agent label: use it to fill `by_model` too, so the model table naturally
        // holds only models that agent used.
        if let Some(models) = bucket.by_client_model.get(label) {
            narrowed
                .by_client_model
                .insert(label.to_string(), models.clone());
            for (model, model_totals) in models {
                narrowed.by_model.insert(model.clone(), *model_totals);
            }
        }
        // The session key is `"{client}:{session_id}"` (see `SessionSnapshot::key`).
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

/// Merge a scan result into the ledger and produce the display view.
///
/// Split into free functions so the "merge → view" logic can be unit-tested directly
/// (without `App` or egui).
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

    /// The idle branch must request a frame for "the next check point": egui doesn't repaint on its own while
    /// idle, and without frame requests the periodic check starves — later source changes would never be
    /// noticed (the historical "half a minute without refresh" defect), and per-frame animations like the
    /// scrollbar fade would stay stuck at transparent.
    #[test]
    fn idle_poll_requests_a_repaint_for_the_next_check() {
        let mut app = crate::app::App::default();
        app.set_usage_result_for_test(Vec::new(), DailyMap::new());
        app.seed_usage_idle_for_test();
        let ctx = egui::Context::default();
        // egui 0.33 routes repaint requests through callbacks (the `repaint_delay` field is gone).
        // The first frame egui itself sends one delay=0 follow-up request, which only sustains one more frame;
        // the idle polling must request frames for "the next check point" itself, or it starves all the same.
        let requested = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&requested);
        ctx.set_request_repaint_callback(move |info| {
            sink.lock().unwrap().push(info.delay);
        });
        // First pass: viewport / font initialization etc. triggers one delay=0 follow-up request, and the merge
        // logic only fires the callback for smaller delays — consume that one first.
        let _ = ctx.run(
            egui::RawInput {
                time: Some(100.0),
                ..Default::default()
            },
            |ctx| app.poll_usage(ctx),
        );
        requested.lock().unwrap().clear();
        // Second pass (one poll interval later): a check point arrives again, sources unchanged → idle branch.
        let _ = ctx.run(
            egui::RawInput {
                time: Some(100.0 + POLL_INTERVAL_SECS + 0.5),
                ..Default::default()
            },
            |ctx| app.poll_usage(ctx),
        );
        let delays = requested.lock().unwrap();
        let poll_request = delays
            .iter()
            .copied()
            .find(|d| *d > std::time::Duration::ZERO);
        let Some(delay) = poll_request else {
            panic!(
                "空闲轮询没有请求定时重绘（只见到 {:?}）：周期检测将饿死，界面永不刷新",
                *delays
            );
        };
        assert!(
            delay <= std::time::Duration::from_secs_f64(POLL_INTERVAL_SECS + 0.5),
            "重绘请求晚于下一个检测点：{delay:?}"
        );
    }

    /// Unfiltered: returned as-is (no pointless deep copy).
    #[test]
    fn filter_without_client_keeps_everything() {
        let mut daily = DailyMap::new();
        daily.insert("2026-01-01".into(), bucket_with_two_clients());
        let filtered = filter_daily_by_client(&daily, None);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered["2026-01-01"].totals.total(), 300);
    }

    /// Filtered to one agent: the total becomes that agent's, and breakdowns narrow with it.
    #[test]
    fn filter_narrows_totals_models_and_sessions() {
        let mut daily = DailyMap::new();
        daily.insert("2026-01-01".into(), bucket_with_two_clients());
        let filtered = filter_daily_by_client(&daily, Some(ConfigFormat::Pi));
        let bucket = &filtered["2026-01-01"];
        // The total = pi's own 200, not the grand 300.
        assert_eq!(bucket.totals.total(), 200);
        assert_eq!(bucket.by_client.len(), 1, "只剩 pi 一项");
        assert!(bucket.by_client.contains_key(&pi_label()));
        // The model table must not list models only opencode used.
        assert!(bucket.by_model.contains_key("claude-opus-5"));
        assert!(
            !bucket.by_model.contains_key("gpt-5"),
            "别的 agent 的模型不该混进来"
        );
        // Sessions narrow by the `"{label}:"` prefix.
        assert_eq!(bucket.by_session.len(), 1);
        assert!(bucket.by_session.contains_key(&pi_session("session-a")));
    }

    /// A day with no usage for the agent: no bucket at all (the heatmap distinguishes "never used" from "used zero" by this).
    #[test]
    fn filter_drops_days_without_that_client() {
        let mut daily = DailyMap::new();
        daily.insert("2026-01-01".into(), bucket_with_two_clients());
        daily.insert("2026-01-02".into(), bucket_with_one_client());
        // Both days touched opencode → both days remain, but each total keeps only opencode's share.
        let filtered = filter_daily_by_client(&daily, Some(ConfigFormat::Opencode));
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered["2026-01-01"].totals.total(), 100, "不含 pi 的 200");
        assert_eq!(filtered["2026-01-02"].totals.total(), 100);

        // An agent never used: not a single day remains.
        let none = filter_daily_by_client(&daily, Some(ConfigFormat::KimiCode));
        assert!(none.is_empty(), "没用过的 agent 不该生成空桶");
    }

    /// After filtering, per-day totals == the filtered range total (nothing lost, nothing doubled).
    #[test]
    fn filter_keeps_per_day_sum_consistent() {
        let mut daily = DailyMap::new();
        daily.insert("2026-01-01".into(), bucket_with_two_clients());
        daily.insert("2026-01-02".into(), bucket_with_one_client());
        let filtered = filter_daily_by_client(&daily, Some(ConfigFormat::Opencode));
        // `today` must be later than the buckets' dates: `Range::All` excludes future dates (guards against dirty ledgers).
        let today = aggregate::days_from_civil(2026, 1, 3);
        let summed: i64 = filtered.values().map(|bucket| bucket.totals.total()).sum();
        let from_range =
            crate::app::usage::range::totals_in_range(&filtered, Range::All, today).total();
        assert_eq!(summed, from_range);
        assert_eq!(summed, 200, "两天各 100 的 opencode 用量");
        // Per-day totals == the sum of each day's by_client (breakdown and total share one accounting basis).
        for bucket in filtered.values() {
            let per_client: i64 = bucket.by_client.values().map(|t| t.total()).sum();
            assert_eq!(bucket.totals.total(), per_client, "{}", bucket.date);
        }
    }

    /// Build a bucket with two agents: pi 200 + opencode 100, each with models and sessions.
    ///
    /// Keys always go through [`ConfigFormat::label`], never hard-coded strings: the breakdown table's keys
    /// ARE labels (see `DailyBucket::add_message`), and a wrong case would make filtering silently no-op.
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

    /// Build an opencode-only bucket.
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

    /// Session keys are isomorphic to [`SessionSnapshot::key`]: `client:session_id`.
    fn pi_session(id: &str) -> String {
        SessionSnapshot::key(ConfigFormat::Pi, id)
    }

    fn opencode_session(id: &str) -> String {
        SessionSnapshot::key(ConfigFormat::Opencode, id)
    }

    /// Input `input`, `messages` messages, everything else 0.
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

    /// Key regression: once dirty, even if the source goes quiet again (next round `changed == 0`),
    /// that change must still be scanned out — no direct return to Idle.
    #[test]
    fn a_settled_change_is_still_scanned() {
        // Change found at 100s (window not yet due)
        let (outcome, since) = poll_outcome(4, None, 100.0);
        assert_eq!(outcome, PollOutcome::Debouncing);
        // At window expiry the source is already quiet (changed == 0), but the change is still pending → time to scan.
        let (outcome, since) = poll_outcome(0, since, 100.0 + DEBOUNCE_SECS);
        assert_eq!(outcome, PollOutcome::Scan, "已置脏的改动不能被丢掉");
        assert_eq!(since, None, "扫完清脏");
    }

    #[test]
    fn change_settling_for_the_debounce_window_triggers_a_scan() {
        // First change at 100s → 0.1s before the window expires, not yet due
        let (outcome, since) = poll_outcome(1, Some(100.0), 100.0 + DEBOUNCE_SECS - 0.1);
        assert_eq!(outcome, PollOutcome::Debouncing);
        assert_eq!(since, Some(100.0));
        // Triggers exactly at DEBOUNCE_SECS
        let (outcome, _) = poll_outcome(1, Some(100.0), 100.0 + DEBOUNCE_SECS);
        assert_eq!(outcome, PollOutcome::Scan);
    }

    #[test]
    fn continuous_writes_are_capped_by_the_max_wait() {
        // When the poll interval shrinks below the debounce window, MAX_WAIT_SECS is the only rescue:
        // every round still has new changes (changed > 0), so the window origin never gets pushed back.
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
        // This is the classic "never arrives" bug: if dirty_since refreshed every round,
        // streaming writes would keep waited at 0 forever.
        let (_, since) = poll_outcome(1, None, 100.0);
        assert_eq!(since, Some(100.0));
        let (outcome, since) = poll_outcome(1, since, 101.0);
        assert_eq!(since, Some(100.0), "窗口起点不能被推后");
        assert_eq!(outcome, PollOutcome::Debouncing);
        let (outcome, _) = poll_outcome(1, since, 101.5);
        assert_eq!(outcome, PollOutcome::Scan);
    }

    /// The real rhythm at a 3-second poll: the change must be scanned out at the very next check.
    #[test]
    fn realistic_poll_cadence_triggers_on_the_next_check() {
        let mut since = None;
        let mut scanned_at = None;
        for tick in 0..10 {
            let now = 100.0 + tick as f64 * POLL_INTERVAL_SECS;
            // First round detects the change; the source then goes quiet (changed == 0).
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
        // Change just noticed: wait the coalescing window (nearer than the cap).
        assert_eq!(repaint_delay(Some(100.0), 100.0), Some(DEBOUNCE_SECS));
        // 4.6 seconds already waited: only 0.4s left to the cap.
        let delay = repaint_delay(Some(100.0), 104.6).expect("应要一帧");
        assert!((delay - 0.4).abs() < 1e-9, "实际 {delay}");
        // Both deadlines passed: give a positive minimum, never 0 / negative.
        assert_eq!(repaint_delay(Some(100.0), 200.0), Some(0.01));
        // Not debouncing: no frame request.
        assert_eq!(repaint_delay(None, 100.0), None);
    }

    /// End to end: merge into the ledger → view; sessions gone from the sources stay in the display (archived).
    #[test]
    fn merge_into_ledger_keeps_deleted_sessions_in_the_view() {
        use crate::format::ConfigFormat;
        let snapshot = |session: &str, input: i64| SessionSnapshot {
            client: ConfigFormat::Pi,
            session_id: session.to_string(),
            model_id: "m".to_string(),
            project: None,
            title: None,

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

        // Next round a disappeared → not in the view, but still in the ledger (totals don't shrink).
        let second = merge_into_ledger(&mut ledger, scan_of(vec![]), DailyMap::new(), 2.0, 0, 10);
        assert!(second.sessions.is_empty(), "源里已消失的不进列表");
        assert_eq!(ledger.len(), 1, "用量在账本里留着");
        assert_eq!(ledger.archived_len(), 1);
    }

    /// Daily buckets must also pass through `merge_into_ledger` into the report (the heatmap depends on them).
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

        // The next scan has no new data: the ledger's daily records must stay (deleted sessions' usage lives there).
        let second = merge_into_ledger(&mut ledger, HashMap::new(), DailyMap::new(), 2.0, 0, 10);
        assert_eq!(second.daily.len(), 1, "历史按天记录不因本轮没扫到而消失");
        assert_eq!(second.daily["2026-03-15"].totals.input, 500);
    }

    /// Instant display: non-empty ledger → immediately presentable (no waiting for a scan); the background scan then returns to live mode.
    #[test]
    fn fill_from_ledger_shows_cached_data_until_scan_lands() {
        use crate::format::ConfigFormat;
        let snapshot = |session: &str, input: i64| SessionSnapshot {
            client: ConfigFormat::Pi,
            session_id: session.to_string(),
            model_id: "m".to_string(),
            project: None,
            title: None,
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

        // Background scan wrap-up: replace the display and clear the cache flag.
        state.adopt_scan(
            (scan_of(vec![snapshot("a", 150)]), DailyMap::new(), 1, 5),
            3.0,
        );
        assert!(!state.cached, "扫完要回到实时态");
        assert!(state.cached_saved_at().is_none());
        let report = state.result.as_ref().and_then(|r| r.as_ref().ok()).unwrap();
        assert_eq!(report.sessions[0].input, 150, "展示要换成最新扫描");
    }

    /// Empty ledger → no fill: the UI keeps "no scan results yet · scanning".
    #[test]
    fn fill_from_ledger_skips_empty_ledger() {
        // Explicit construction; never touches the user's real ledger (default() would read and surface it).
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
            title: None,

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
