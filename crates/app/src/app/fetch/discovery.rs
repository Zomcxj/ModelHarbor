//! Model discovery (the "discover models" action in the provider form): probe the `/models` endpoint in one click and check the new finds.
//!
//! Architecture mirrors the latency test in this directory: the blocking [`probe`] runs on a background
//! thread, results come back over a channel and are polled each frame ([`App::poll_discovery`]), with
//! `request_repaint_after` keeping frames alive during the probe. Cancellation uses a generation counter:
//! both start and cancel bump it; a reply with a stale gen is discarded wholesale. Candidate fallback and
//! error classification live in core::discovery — inside `probe`, 404/405 automatically advance to the next
//! candidate and `NotFound` only returns after all candidates fail, so no upper-level candidate loop here.
//! Before probing, [`DiscoveryCache`] is consulted: the same base_url within 24 hours hits directly, no request.

use super::*;
use crate::app::providers_form::{fetch_grid_columns, FETCH_GRID_GAP_X};
use crate::format::ConfigFormat;
use crate::model::ModelRow;
use model_harbor_core::discovery::{
    merge_missing, models_endpoint_candidates, probe, DiscoveryCache, ProbeAuth, ProbeOutcome,
};

/// Timeout of one probe: as generous as "fetch models" (model lists may come from very slow relay stations).
pub(crate) const DISCOVERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Panel data of one successful probe: discovered model list + each item's checked state.
#[derive(Default)]
pub(crate) struct DiscoveryFound {
    /// The discovered model ids (response order preserved).
    pub(crate) models: Vec<String>,
    /// Whether each model is checked (seeding in [`seed_checked`]: new finds all selected).
    pub(crate) checked: Vec<bool>,
}

/// One provider's "discover models" state (key → state; existing = the result panel is open).
#[derive(Default)]
pub(crate) struct DiscoveryState {
    /// Bumped on start / cancel; a reply with a mismatched gen is discarded wholesale (cache writes included).
    pub(crate) generation: u64,
    /// The background probe's reply channel (`Some` = in flight).
    pub(crate) rx: Option<std::sync::mpsc::Receiver<(u64, ProbeOutcome)>>,
    /// The base_url at probe start (trimmed; the cache key written on success).
    pub(crate) base_url: String,
    /// The last failure's classified reason (`ProbeError`'s Display, human-readable; cleared on success).
    pub(crate) error: Option<String>,
    /// The successful discovery's panel data (cleared on failure).
    pub(crate) found: Option<DiscoveryFound>,
}

impl DiscoveryState {
    /// Whether a probe is in flight.
    pub(crate) fn in_flight(&self) -> bool {
        self.rx.is_some()
    }

    /// Whether the result panel is open (presentable content exists: a success list or a failure reason).
    pub(crate) fn has_result(&self) -> bool {
        self.found.is_some() || self.error.is_some()
    }
}

// ---------------------------------------------------------------------------
// Pure logic (with unit tests, see app/tests/discovery_tests.rs)
// ---------------------------------------------------------------------------

/// Probe auth: with a key use Bearer (the default shape of OpenAI-compatible gateways); without one, send
/// no auth header — the server's 401 is classified by core as "API key not configured", which reads truer.
pub(crate) fn probe_auth_for_secret(secret: &str) -> ProbeAuth {
    if secret.trim().is_empty() {
        ProbeAuth::None
    } else {
        ProbeAuth::Bearer
    }
}

/// Can this click use the cache directly: nothing in flight, and the result panel not yet open (panel open = the user wants a refresh).
pub(crate) fn should_use_cache(in_flight: bool, panel_open: bool) -> bool {
    !in_flight && !panel_open
}

/// Hover text of the "discover models" button: lists the candidate endpoints tried in order (404/405 auto-fallback).
pub(crate) fn probe_candidates_label(base_url: &str) -> String {
    let candidates = models_endpoint_candidates(base_url, None);
    if candidates.is_empty() {
        return "先填写 Base URL 再获取模型".to_string();
    }
    format!(
        "将依次尝试候选端点（404 / 405 自动回退）：{}",
        candidates.join(" → ")
    )
}

/// Whether a model already exists: case-insensitive, whitespace-trimmed — same dedup criteria as [`merge_missing`].
pub(crate) fn is_existing_model(existing: &[String], id: &str) -> bool {
    let trimmed = id.trim();
    !trimmed.is_empty()
        && existing
            .iter()
            .any(|known| known.trim().eq_ignore_ascii_case(trimmed))
}

/// Default checks: all unchecked, picked manually by the user (existing items render as disabled checkboxes — uncheckable anyway).
pub(crate) fn seed_checked(discovered: &[String], existing: &[String]) -> Vec<bool> {
    let _ = existing;
    vec![false; discovered.len()]
}

/// Result → models to be added: only checked items, handed to [`merge_missing`] as fill-only.
///
/// The return value is exactly the batch that will be appended in the same order at write time (matching the merge's tail item by item).
pub(crate) fn planned_additions(
    existing: &[String],
    discovered: &[String],
    checked: &[bool],
) -> Vec<String> {
    let picked: Vec<String> = discovered
        .iter()
        .enumerate()
        .filter(|(i, _)| checked.get(*i).copied().unwrap_or(false))
        .map(|(_, id)| id.clone())
        .collect();
    let mut merged = merge_missing(existing, picked);
    merged.split_off(existing.len())
}

/// Status-line summary of a probe result: success carries the model count, failure the reason.
pub(crate) enum ProbeSummary {
    /// Discovered N models successfully.
    Success(usize),
    /// The failure reason (`ProbeError`'s Display, human-readable).
    Failure(String),
}

/// The status bar's "last discovery" text.
pub(crate) fn probe_status_text(hhmm: &str, summary: &ProbeSummary) -> String {
    match summary {
        ProbeSummary::Success(count) => format!("最近获取：{hhmm}（成功 {count} 个）"),
        ProbeSummary::Failure(reason) => format!("最近获取：{hhmm}（失败：{reason}）"),
    }
}

/// HH:MM (two digits; out-of-range values clamp back into range).
pub(crate) fn format_hhmm(hour: i64, minute: i64) -> String {
    format!("{:02}:{:02}", hour.clamp(0, 23), minute.clamp(0, 59))
}

/// Unix seconds → that day's HH:MM (UTC).
///
/// Compiled only in the non-Windows branch (`probe_clock_hhmm`'s fallback path) and in tests:
/// Windows uses GetLocalTime local time and doesn't need it; without `cfg`, Windows non-test builds
/// would raise dead_code (blocked outright by `-D warnings`).
#[cfg(any(not(windows), test))]
pub(crate) fn hhmm_from_unix_utc(secs: i64) -> String {
    let rem = secs.rem_euclid(86_400);
    format_hhmm(rem / 3_600, rem % 3_600 / 60)
}

/// HH:MM of the last discovery's completion.
///
/// Windows reads local time directly (GetLocalTime, same source as core::billing's local midnight); other
/// platforms can't get the local timezone (see the precedent in `balance::local_midnight_unix`) and fall back to UTC.
pub(crate) fn probe_clock_hhmm() -> String {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::SYSTEMTIME;
        use windows_sys::Win32::System::SystemInformation::GetLocalTime;
        let mut local = SYSTEMTIME::default();
        // SAFETY: `GetLocalTime` writes only this one struct.
        unsafe { GetLocalTime(&mut local) };
        format_hhmm(i64::from(local.wHour), i64::from(local.wMinute))
    }
    #[cfg(not(windows))]
    {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as i64)
            .unwrap_or(0);
        hhmm_from_unix_utc(secs)
    }
}

// ---------------------------------------------------------------------------
// Start / cancel (consumes only the state table, never App — callable inside form borrows)
// ---------------------------------------------------------------------------

/// Start one model discovery ([`probe`] runs on a background thread).
///
/// The 24-hour cache is consulted first: a hit with the result panel still closed uses the cache directly
/// (no request); an open panel means the user wants a refresh, so the network is probed again. Returns
/// the message to write into the bottom status bar.
pub(crate) fn start_discovery(
    states: &mut HashMap<String, DiscoveryState>,
    cache: &mut DiscoveryCache,
    key: &str,
    base_url: &str,
    secret: &str,
    existing: &[String],
) -> Option<String> {
    let base = base_url.trim().to_string();
    let state = states.entry(key.to_string()).or_default();
    // In flight: the button is disabled; defensively ignore duplicate clicks.
    if state.in_flight() {
        return None;
    }
    state.base_url = base.clone();
    // The panel is still closed (first probe / panel was closed) → check the 24-hour cache first.
    if should_use_cache(state.in_flight(), state.has_result()) {
        if let Some((models, _)) = cache.fresh(&base, None) {
            let models = models.clone();
            let count = models.len();
            state.found = Some(DiscoveryFound {
                checked: seed_checked(&models, existing),
                models,
            });
            state.error = None;
            return Some(format!("命中 24 小时内的获取缓存：{count} 个模型"));
        }
    }
    // Re-probe over the network: bump gen to invalidate old replies, clearing the last round's panel.
    state.generation += 1;
    let generation = state.generation;
    state.found = None;
    state.error = None;
    let secret = secret.trim().to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let outcome = probe(
            &base,
            Some(&secret),
            probe_auth_for_secret(&secret),
            DISCOVERY_TIMEOUT,
        );
        let _ = tx.send((generation, outcome));
    });
    state.rx = Some(rx);
    None
}

/// Cancel an in-flight discovery: bump the generation to invalidate replies, and drop the channel.
///
/// Returns the message to write into the status bar (untouched when nothing is in flight).
pub(crate) fn cancel_discovery(
    states: &mut HashMap<String, DiscoveryState>,
    key: &str,
) -> Option<String> {
    let state = states.get_mut(key)?;
    if !state.in_flight() {
        return None;
    }
    state.generation += 1;
    state.rx = None;
    Some("已取消模型获取".to_string())
}

// ---------------------------------------------------------------------------
// Per-frame polling
// ---------------------------------------------------------------------------

impl App {
    /// Poll model-probe results each frame: mismatched gens are discarded; success writes the cache and opens
    /// the result panel; failure shows the classified reason in the panel and status bar.
    pub(in crate::app) fn poll_discovery(&mut self) {
        let mut finished: Vec<(String, u64, String, ProbeOutcome)> = Vec::new();
        for (key, state) in self.discovery.iter_mut() {
            let Some(rx) = &state.rx else { continue };
            match rx.try_recv() {
                Ok((generation, outcome)) => {
                    state.rx = None;
                    finished.push((key.clone(), generation, state.base_url.clone(), outcome));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                // The probe thread died / the channel was dropped (cancel): end the wait.
                Err(std::sync::mpsc::TryRecvError::Disconnected) => state.rx = None,
            }
        }
        for (key, generation, base_url, outcome) in finished {
            // Cancelled or superseded by a newer probe: discard wholesale (the cache isn't written either).
            if self
                .discovery
                .get(&key)
                .is_some_and(|state| state.generation != generation)
            {
                continue;
            }
            let hhmm = probe_clock_hhmm();
            match outcome {
                ProbeOutcome::Success { models, .. } => {
                    let count = models.len();
                    // A closed panel still gets the cache write and status note; an open one opens the result list.
                    self.discovery_cache.insert(&base_url, None, models.clone());
                    let existing = self.existing_model_ids(&key);
                    if let Some(state) = self.discovery.get_mut(&key) {
                        state.found = Some(DiscoveryFound {
                            checked: seed_checked(&models, &existing),
                            models,
                        });
                        state.error = None;
                    }
                    self.set_status_ok(probe_status_text(&hhmm, &ProbeSummary::Success(count)));
                }
                ProbeOutcome::Failure(err) => {
                    let reason = err.to_string();
                    if let Some(state) = self.discovery.get_mut(&key) {
                        state.found = None;
                        state.error = Some(reason.clone());
                    }
                    self.set_status_err(probe_status_text(&hhmm, &ProbeSummary::Failure(reason)));
                }
            }
        }
    }

    /// The "existing model ids" seeding checks against: the add form uses new_provider; others look up by key
    /// (not found — e.g. the form closed — counts as all-new).
    fn existing_model_ids(&self, key: &str) -> Vec<String> {
        let models = if key == NEW_PROVIDER_FETCH_KEY {
            Some(&self.new_provider.models)
        } else {
            self.providers
                .iter()
                .find(|p| p.key.trim() == key)
                .map(|p| &p.models)
        };
        models
            .map(|rows| rows.iter().map(|m| m.id.trim().to_string()).collect())
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Result panel
// ---------------------------------------------------------------------------

/// One frame's interaction result of the panel: whether "close" was clicked, plus the message for the status bar.
pub(crate) struct DiscoveryPanelAction {
    /// The user clicked "close"; the caller removes this provider's discovery state.
    pub(crate) close: bool,
    /// The message to write into the bottom status bar (e.g. "added N models").
    pub(crate) status: Option<String>,
}

/// Discovery result panel: header stats + check grid + bottom "add N".
///
/// Probing shows a spinner; failure shows the classified reason; success lists every find, with existing
/// items rendered as disabled checkboxes (never added twice).
pub(crate) fn discovery_panel(
    ui: &mut egui::Ui,
    state: &mut DiscoveryState,
    models: &mut Vec<ModelRow>,
    current_page: ConfigFormat,
    scroll_salt: egui::Id,
    existing: &[String],
) -> DiscoveryPanelAction {
    let mut close = false;
    let mut status = None;
    ui.horizontal(|ui| {
        ui.strong("获取结果");
        if let Some(found) = &state.found {
            let additions = planned_additions(existing, &found.models, &found.checked).len();
            ui.label(
                egui::RichText::new(format!(
                    "共 {} 个，待新增 {} 个（只填空，不覆盖已有）",
                    found.models.len(),
                    additions
                ))
                .weak(),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("关闭").clicked() {
                close = true;
            }
        });
    });
    if state.rx.is_some() {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(16.0));
            ui.label(
                egui::RichText::new("正在获取模型列表…（404 / 405 会自动换下一个候选端点）").weak(),
            );
        });
    } else if let Some(reason) = &state.error {
        ui.label(
            egui::RichText::new(format!("探测失败：{reason}"))
                .color(crate::theme::semantics(ui).err),
        );
    } else if let Some(found) = &mut state.found {
        render_found(
            ui,
            found,
            models,
            current_page,
            scroll_salt,
            existing,
            &mut status,
        );
    }
    DiscoveryPanelAction { close, status }
}

/// The success result's check grid and bottom "add N" button.
fn render_found(
    ui: &mut egui::Ui,
    found: &mut DiscoveryFound,
    models: &mut Vec<ModelRow>,
    current_page: ConfigFormat,
    scroll_salt: egui::Id,
    existing: &[String],
    status: &mut Option<String>,
) {
    if found.models.is_empty() {
        ui.label(egui::RichText::new("接口未返回任何模型").weak());
        return;
    }
    ui.label(egui::RichText::new("勾选要新增的模型（已配置的画成禁用，不会重复添加）：").weak());
    // The area's height is fixed at about 12 rows, scrolling vertically inside for overflow. Column count
    // computes inside the scroll area from the available width, deducting the scrollbar's width.
    let row_h = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
    egui::ScrollArea::vertical()
        .id_salt(scroll_salt)
        .max_height(row_h * 12.5)
        .auto_shrink([false, true])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
        .show(ui, |ui| {
            let (cols, col_w) = fetch_grid_columns(found.models.len(), ui.available_width());
            let per_col = found.models.len().div_ceil(cols);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = FETCH_GRID_GAP_X;
                for ci in 0..cols {
                    ui.vertical(|ui| {
                        // Fixed-width columns + truncation: over-long model names show in full on hover, never widening the column.
                        ui.set_width(col_w);
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                        for (i, id) in found
                            .models
                            .iter()
                            .enumerate()
                            .skip(ci * per_col)
                            .take(per_col)
                        {
                            if id.trim().is_empty() {
                                continue;
                            }
                            if is_existing_model(existing, id) {
                                let mut off = false;
                                ui.add_enabled(
                                    false,
                                    egui::Checkbox::new(&mut off, format!("{id}（已有）")),
                                )
                                .on_hover_text("已在模型列表中，探测不重复添加");
                            } else {
                                ui.checkbox(&mut found.checked[i], id.as_str())
                                    .on_hover_text(id.as_str());
                            }
                        }
                    });
                }
            });
        });
    // "Add N": N matches the count that will actually be appended in the same order (merge_missing is fill-only).
    let additions = planned_additions(existing, &found.models, &found.checked);
    let count = additions.len();
    if ui
        .add_enabled(count > 0, egui::Button::new(format!("添加 {count} 个")))
        .clicked()
    {
        for id in &additions {
            let mut row = ModelRow::new();
            row.id = id.clone();
            row.name = id.clone();
            row.source_format = Some(current_page);
            models.push(row);
        }
        *status = Some(format!("已添加 {count} 个模型（只填空，不影响已有）"));
    }
}
