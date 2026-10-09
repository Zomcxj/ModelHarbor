//! 模型探测（Provider 表单内的「探测模型」）：一键探测 `/models` 端点并勾选新增。
//!
//! 架构照搬同目录的延迟测试：阻塞的 [`probe`] 在后台线程执行，结果经通道回传、
//! 每帧轮询（[`App::poll_discovery`]），探测中用 `request_repaint_after` 保帧。
//! 取消用 generation counter：发起与取消都自增，回包 gen 不符即整个丢弃。
//! 候选端点回退与错误分级都在 core::discovery——`probe` 内部对 404/405 自动
//! 换下一个候选、全部候选失败才返回 `NotFound`，这里不再做上层候选循环。
//! 探测前先查 [`DiscoveryCache`]：24 小时内同 base_url 直接命中，不再发请求。

use super::*;
use crate::app::providers_form::{fetch_grid_columns, FETCH_GRID_GAP_X};
use crate::format::ConfigFormat;
use crate::model::ModelRow;
use model_harbor_core::discovery::{
    merge_missing, models_endpoint_candidates, probe, DiscoveryCache, ProbeAuth, ProbeOutcome,
};

/// 单次探测的超时：与「获取模型」同量级的宽松值（模型列表可能来自很慢的中转站）。
pub(crate) const DISCOVERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// 一次成功探测的结果面板数据：发现的模型列表 + 每项的勾选状态。
#[derive(Default)]
pub(crate) struct DiscoveryFound {
    /// 发现的模型 id（保持响应顺序）。
    pub(crate) models: Vec<String>,
    /// 每个模型是否被勾选（种子见 [`seed_checked`]：新增项全选）。
    pub(crate) checked: Vec<bool>,
}

/// 一个 provider 的「探测模型」状态（key → 状态；存在即结果面板开着）。
#[derive(Default)]
pub(crate) struct DiscoveryState {
    /// 发起 / 取消时自增；回包 gen 不符即整个丢弃（含缓存写入）。
    pub(crate) generation: u64,
    /// 后台探测回传通道（`Some` = 正在飞）。
    pub(crate) rx: Option<std::sync::mpsc::Receiver<(u64, ProbeOutcome)>>,
    /// 发起探测时的 base_url（trim 过；探测成功后写缓存的键）。
    pub(crate) base_url: String,
    /// 最近一次失败的分级原因（`ProbeError` 的 Display 人话；成功后清空）。
    pub(crate) error: Option<String>,
    /// 成功发现的结果面板数据（失败时清空）。
    pub(crate) found: Option<DiscoveryFound>,
}

impl DiscoveryState {
    /// 是否有探测在飞。
    pub(crate) fn in_flight(&self) -> bool {
        self.rx.is_some()
    }

    /// 结果面板是否开着（有可展示的内容：成功列表或失败原因）。
    pub(crate) fn has_result(&self) -> bool {
        self.found.is_some() || self.error.is_some()
    }
}

// ---------------------------------------------------------------------------
// 纯逻辑（配单测，见 app/tests/discovery_tests.rs）
// ---------------------------------------------------------------------------

/// 探测鉴权：填了 key 用 Bearer（OpenAI 兼容网关的默认形态），没填就不带
/// 鉴权头——服务端的 401 由 core 归类为「未配置 API Key」，文案更准。
pub(crate) fn probe_auth_for_secret(secret: &str) -> ProbeAuth {
    if secret.trim().is_empty() {
        ProbeAuth::None
    } else {
        ProbeAuth::Bearer
    }
}

/// 这次点击能否直接用缓存：不在飞、且结果面板还没开着（面板开着 = 用户想刷新）。
pub(crate) fn should_use_cache(in_flight: bool, panel_open: bool) -> bool {
    !in_flight && !panel_open
}

/// 「探测模型」按钮的悬停文案：列出将依次尝试的候选端点（404/405 自动回退）。
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

/// 是否已有该模型：大小写不敏感、忽略首尾空白，与 [`merge_missing`] 的去重口径一致。
pub(crate) fn is_existing_model(existing: &[String], id: &str) -> bool {
    let trimmed = id.trim();
    !trimmed.is_empty()
        && existing
            .iter()
            .any(|known| known.trim().eq_ignore_ascii_case(trimmed))
}

/// 默认勾选：全部不勾，由用户手动挑选（已有项画成禁用勾选框，本就不可勾）。
pub(crate) fn seed_checked(discovered: &[String], existing: &[String]) -> Vec<bool> {
    let _ = existing;
    vec![false; discovered.len()]
}

/// 结果 → 将新增的模型列表：只取勾选项，交给 [`merge_missing`] 只填空。
///
/// 返回值正是写盘时会同顺序追加的那批 id（与 merge 的尾部逐条一致）。
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

/// 探测结果的状态栏摘要：成功带模型数，失败带原因。
pub(crate) enum ProbeSummary {
    /// 成功发现 N 个模型。
    Success(usize),
    /// 失败原因（`ProbeError` 的 Display 人话）。
    Failure(String),
}

/// 底部状态栏的「最近探测」文案。
pub(crate) fn probe_status_text(hhmm: &str, summary: &ProbeSummary) -> String {
    match summary {
        ProbeSummary::Success(count) => format!("最近获取：{hhmm}（成功 {count} 个）"),
        ProbeSummary::Failure(reason) => format!("最近获取：{hhmm}（失败：{reason}）"),
    }
}

/// HH:MM（两位数；超出范围的值夹回合法区间）。
pub(crate) fn format_hhmm(hour: i64, minute: i64) -> String {
    format!("{:02}:{:02}", hour.clamp(0, 23), minute.clamp(0, 59))
}

/// Unix 秒 → 当日 HH:MM（UTC）。
///
/// 仅在非 Windows 运行分支（`probe_clock_hhmm` 的回退路径）与测试中编译：
/// Windows 走 GetLocalTime 本地时间，不需要它，不加 `cfg` 会在 Windows
/// 非 test 构建下触发 dead_code（`-D warnings` 直接拦）。
#[cfg(any(not(windows), test))]
pub(crate) fn hhmm_from_unix_utc(secs: i64) -> String {
    let rem = secs.rem_euclid(86_400);
    format_hhmm(rem / 3_600, rem % 3_600 / 60)
}

/// 最近探测的完成时刻 HH:MM。
///
/// Windows 直接读本地时间（GetLocalTime，与 core::billing 的本地 0 点同源）；
/// 其余平台拿不到本地时区（见 `balance::local_midnight_unix` 的先例），退回 UTC。
pub(crate) fn probe_clock_hhmm() -> String {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::SYSTEMTIME;
        use windows_sys::Win32::System::SystemInformation::GetLocalTime;
        let mut local = SYSTEMTIME::default();
        // SAFETY: `GetLocalTime` 只写这一个结构体。
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
// 发起 / 取消（只吃状态表，不碰 App，便于在表单借用中调用）
// ---------------------------------------------------------------------------

/// 发起一次模型探测（后台线程跑 [`probe`]）。
///
/// 探测前先查 24 小时缓存：命中且结果面板还没开着就直接用缓存（不发请求）；
/// 面板开着说明用户想刷新，走网络重新探测。返回需要写入底部状态栏的消息。
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
    // 正在飞：按钮已禁用，防御性忽略重复点击。
    if state.in_flight() {
        return None;
    }
    state.base_url = base.clone();
    // 面板还关着（首次探测 / 关闭过面板）→ 先查 24 小时内的缓存。
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
    // 走网络重新探测：gen 自增让旧回包作废，清掉上一轮的面板内容。
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

/// 取消进行中的探测：自增 generation 让回包作废，并丢弃通道。
///
/// 返回需要写入状态栏的消息（没有在飞的探测时不打扰状态栏）。
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
// 每帧轮询
// ---------------------------------------------------------------------------

impl App {
    /// 每帧轮询模型探测结果：gen 不符即丢弃；成功写缓存并打开结果面板，
    /// 失败在面板与状态栏给分级原因。
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
                // 探测线程异常退出 / 通道被丢弃（取消）：终止等待。
                Err(std::sync::mpsc::TryRecvError::Disconnected) => state.rx = None,
            }
        }
        for (key, generation, base_url, outcome) in finished {
            // 已取消或被更新的探测取代：整个丢弃（缓存也不写）。
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
                    // 面板被关掉也照写缓存与状态栏；面板还在则打开结果列表。
                    self.discovery_cache.insert(&base_url, None, models.clone());
                    let existing = self.existing_model_ids(&key);
                    if let Some(state) = self.discovery.get_mut(&key) {
                        state.found = Some(DiscoveryFound {
                            checked: seed_checked(&models, &existing),
                            models,
                        });
                        state.error = None;
                    }
                    self.status = probe_status_text(&hhmm, &ProbeSummary::Success(count));
                }
                ProbeOutcome::Failure(err) => {
                    let reason = err.to_string();
                    if let Some(state) = self.discovery.get_mut(&key) {
                        state.found = None;
                        state.error = Some(reason.clone());
                    }
                    self.status = probe_status_text(&hhmm, &ProbeSummary::Failure(reason));
                }
            }
        }
    }

    /// 种子勾选用的「已有模型 id」：新增表单用 new_provider，其余按 key 找
    /// （找不到——比如表单已关——视为全部新增）。
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
// 结果面板
// ---------------------------------------------------------------------------

/// 结果面板一帧的交互结果：是否点了「关闭」、要写入状态栏的消息。
pub(crate) struct DiscoveryPanelAction {
    /// 用户点了「关闭」，调用方移除该 provider 的探测状态。
    pub(crate) close: bool,
    /// 需要写入底部状态栏的消息（如「已添加 N 个模型」）。
    pub(crate) status: Option<String>,
}

/// 探测结果面板：头部统计 + 勾选网格 + 底部「添加 N 个」。
///
/// 探测中显示 spinner；失败显示分级原因；成功列出全部发现项，
/// 已有项画成禁用勾选框（不会重复添加）。
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

/// 成功结果的勾选网格与底部「添加 N 个」按钮。
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
    // 区域高度固定为 12 行左右，超出部分在区域内垂直滚动。
    // 列数在滚动区内部按可用宽度计算，以扣除滚动条占用的宽度。
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
                        // 定宽列 + 截断：超长模型名悬停看全名，不把列撑宽。
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
    // 「添加 N 个」：N 与实际会同顺序追加的条数一致（merge_missing 只填空）。
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
