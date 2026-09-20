//! Provider 编辑 / 新增表单、模型获取弹层与表单字段控件。
use super::App;
use crate::app::fetch::{
    fetch_models_remote, model_latency_label, model_probe_button, net_guard_gate, ModelFetchState,
    NEW_PROVIDER_FETCH_KEY,
};
use crate::app::providers::ProviderFormFlags;
use crate::convert;
use crate::credentials;
use crate::format::ConfigFormat;
use crate::model::{ModelRow, ProviderRow};
use crate::ui::{
    card_frame, field_label, merge_drag_target, move_item, numeric_text_edit, secret_text_edit,
    DragHandle,
};
use eframe::egui;
use std::collections::HashSet;

/// npm 包下拉（opencode 专用）；`id_salt` 区分同一页面内的多个表单实例。
pub(super) fn provider_npm_combo(
    ui: &mut egui::Ui,
    p: &mut ProviderRow,
    id_salt: &str,
    empty_has_label: bool,
) {
    const NPM_OPTIONS: [&str; 5] = [
        "",
        "@ai-sdk/openai",
        "@ai-sdk/anthropic",
        "@ai-sdk/google",
        "@ai-sdk/openai-compatible",
    ];
    field_label(ui, 120.0, "npm");
    let current = p.npm.clone();
    let mut selected = NPM_OPTIONS.iter().position(|n| *n == current.as_str());
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(if current.is_empty() {
            "选择 npm 包..."
        } else {
            &current
        })
        .width(220.0)
        .show_ui(ui, |ui| {
            for (i, npm) in NPM_OPTIONS.iter().enumerate() {
                let label = if npm.is_empty() && empty_has_label {
                    "(空)"
                } else {
                    *npm
                };
                if ui.selectable_label(selected == Some(i), label).clicked() {
                    selected = Some(i);
                }
            }
        });
    if let Some(i) = selected {
        p.npm = NPM_OPTIONS[i].to_string();
    }
}

/// api / npm 下拉里的「(空)」标签：表示未指定协议。
pub(super) const EMPTY_API_LABEL: &str = "(空)";

/// 官方预设下拉：只填 key / baseUrl / 协议（opencode 页填 npm），不写密钥、不动模型列表。
///
/// 首项就是「(自定义 / 不套用)」——不选预设时表单与以前完全一样，第三方 / 中转站 / 自建
/// 端点照旧手填；套用后所有字段仍可继续手改。
pub(super) fn provider_preset_combo(
    ui: &mut egui::Ui,
    p: &mut ProviderRow,
    dialect: crate::presets::PresetDialect,
    id_salt: &str,
) {
    field_label(ui, 120.0, "官方预设");
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text("(自定义 / 不套用)")
        .width(240.0)
        .show_ui(ui, |ui| {
            ui.label(
                egui::RichText::new("套用后仅覆盖 key / baseUrl / 协议，其余字段可照旧手填")
                    .small()
                    .weak(),
            );
            ui.separator();
            for preset in crate::presets::PRESETS {
                let text = format!("{}  ·  {}", preset.label, preset.key);
                if ui.selectable_label(false, text).clicked() {
                    crate::presets::apply(p, preset, dialect);
                }
            }
        })
        .response
        .on_hover_text("预设不写密钥、不改动模型列表；第三方 / 中转站 / 自建端点请直接在下方手填");
}

/// api 下拉：omp 官方 9 值 / pi KnownApi 10 值；首项「(空)」与 opencode 页 npm 的空选项同义。
pub(super) fn provider_api_combo(
    ui: &mut egui::Ui,
    p: &mut ProviderRow,
    page: ConfigFormat,
    id_salt: &str,
) {
    // 每个后端自己的协议词表：omp 9 值 / pi 10 值 / ZCode 3 值（多一个 chat）/
    // WorkBuddy 3 值（协议落到 URL 后缀，见 workbuddy 后端）。
    let options: &[&str] = match page {
        ConfigFormat::OhMyPi => &convert::OMP_APIS,
        ConfigFormat::ZCode => &convert::ZCODE_APIS,
        ConfigFormat::WorkBuddy => &convert::WORKBUDDY_APIS,
        _ => &convert::PI_APIS,
    };
    // ZCode 的 api.type 词表与内部表示差一个 `chat`，显示与写回都要转换。
    let zcode = page == ConfigFormat::ZCode;
    let to_display = |api: &str| {
        if zcode {
            convert::api_to_zcode_api(api)
        } else {
            api.to_string()
        }
    };
    // 「(空)」= 未指定协议。四页共用同一份数据，故以 npm / pi_api / raw.api
    // 是否都为空判定，显示值统一走 effective_api()，与写盘、延迟测试同口径。
    let explicit = p.has_explicit_api();
    let current = p.effective_api();
    field_label(ui, 120.0, "api");
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(if explicit {
            to_display(&current)
        } else {
            EMPTY_API_LABEL.to_string()
        })
        .width(180.0)
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(!explicit, EMPTY_API_LABEL)
                .on_hover_text("不指定协议：写盘时按兼容层 openai-completions 处理")
                .clicked()
            {
                p.clear_api();
            }
            for &api in options {
                // 选中态按「转换后」的值比较：ZCode 页存的是 openai-completions，
                // 但选项文本是 openai-chat-completions。
                let shown = to_display(api);
                if ui
                    .selectable_label(explicit && to_display(&current) == shown, shown)
                    .clicked()
                {
                    // 写回内部表示：ZCode 的 chat 值先映射回 pi 词表。
                    p.pi_api = if zcode {
                        convert::zcode_api_to_api(api)
                    } else {
                        api.to_string()
                    };
                    p.npm = convert::api_to_npm(api);
                    // WorkBuddy：选非 Chat 协议就默认勾上「自定义协议」，
                    // 让界面与保存结果一致（保存时后端仍会强制对齐一次）。
                    if page == ConfigFormat::WorkBuddy {
                        set_workbuddy_custom(p, api != "openai-completions");
                    }
                }
            }
        });
}

/// 写入 WorkBuddy 的「自定义协议」开关（存在 raw 的 `useCustomProtocol`）。
pub(super) fn set_workbuddy_custom(p: &mut ProviderRow, custom: bool) {
    if let serde_json::Value::Object(obj) = &mut p.raw {
        obj.insert("useCustomProtocol".into(), serde_json::Value::Bool(custom));
    }
}

/// 读取 WorkBuddy 的「自定义协议」开关。
pub(super) fn workbuddy_custom(p: &ProviderRow) -> bool {
    p.raw
        .get("useCustomProtocol")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// 思考档位多选：按钮展开、勾选写回逗号分隔文本。
/// `normalize` 为 true 时按规范档位顺序写回，避免重新勾选后被追加到末尾。
pub(super) fn variant_selector(
    ui: &mut egui::Ui,
    variants: &mut String,
    names: &[&'static str],
    open_key: String,
    open_set: &mut HashSet<String>,
    normalize: bool,
) {
    let current = variants.clone();
    let mut selected: Vec<String> = current
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let display = if selected.is_empty() {
        "选择...".to_string()
    } else {
        current.clone()
    };
    let is_open = open_set.contains(&open_key);
    if ui.button(display).clicked() {
        if is_open {
            open_set.remove(&open_key);
        } else {
            open_set.insert(open_key.clone());
        }
    }
    if !is_open {
        return;
    }
    for name in names {
        let mut checked = selected.iter().any(|s| s == name);
        if ui.checkbox(&mut checked, *name).changed() {
            if checked {
                if !selected.iter().any(|s| s == name) {
                    selected.push((*name).to_string());
                }
            } else {
                selected.retain(|s| s != name);
            }
            *variants = if normalize {
                crate::model::ordered_variants(selected.iter().map(String::as_str)).join(", ")
            } else {
                selected.join(", ")
            };
        }
    }
}

/// 「获取模型」面板的分栏参数：列间水平间距、单列最小宽度、最大列数。
pub(super) const FETCH_GRID_GAP_X: f32 = 28.0;

pub(super) const FETCH_GRID_MIN_COL_W: f32 = 150.0;

pub(super) const FETCH_GRID_MAX_COLS: usize = 5;

/// 计算模型勾选面板的横向分栏：列数由**可用宽度**推出来（而非写死 5 列），列宽再把总宽
/// 均分，所以 `列数 * 列宽 + 间距 * (列数 - 1)` 恒等于可用宽度，面板不会横向溢出。
/// 返回 `(列数, 每列宽度)`。
pub(super) fn fetch_grid_columns(id_count: usize, avail_width: f32) -> (usize, f32) {
    if id_count == 0 {
        return (1, avail_width.max(1.0));
    }
    let max_cols = FETCH_GRID_MAX_COLS.min(id_count);
    // 浮点转整数在 Rust 中是饱和转换（NaN -> 0），因此不会 panic；clamp 也只是兜底。
    let cols = (((avail_width + FETCH_GRID_GAP_X) / (FETCH_GRID_MIN_COL_W + FETCH_GRID_GAP_X))
        .floor() as usize)
        .clamp(1, max_cols);
    let col_w = ((avail_width - FETCH_GRID_GAP_X * (cols - 1) as f32) / cols as f32).max(1.0);
    (cols, col_w)
}

/// 模型获取结果的勾选面板：勾选后把其中未配置的模型追加到 `models`。
pub(super) fn model_fetch_popup<H: std::hash::Hash>(
    ui: &mut egui::Ui,
    state: Option<&ModelFetchState>,
    models: &mut Vec<ModelRow>,
    current_page: ConfigFormat,
    id_salt: H,
) {
    let Some(state) = state else {
        ui.label(egui::RichText::new("尚未获取，请先点击「获取模型」").weak());
        return;
    };
    if state.rx.is_some() {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(16.0));
            ui.label(egui::RichText::new("正在获取模型…").weak());
        });
        return;
    }
    let Some(result) = &state.result else {
        return;
    };
    match result {
        Ok(models_remote) if models_remote.is_empty() => {
            ui.label(egui::RichText::new("接口未返回任何模型").weak());
        }
        Ok(models_remote) => {
            let ids = models_remote.clone();
            ui.label(egui::RichText::new("勾选可新增未配置的模型：").weak());
            // 区域高度固定为 22 行，每列超出部分在区域内垂直滚动查看。
            // 注意 1：ScrollArea 内不能用 ui.columns —— columns 会把内容裁到当前可用高度，
            // 导致内容不进入滚动区、无法滚动。改为横向排布 + 纵向子列。
            // 注意 2：列数必须在滚动区**内部**按可用宽度计算，才能把「始终可见的滚动条」
            // 占用的那一条宽度也扣掉，否则整块内容会向右溢出界面。
            let row_h = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
            egui::ScrollArea::vertical()
                .id_salt(id_salt)
                .max_height(row_h * 22.5)
                .auto_shrink([false, true])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| {
                    let (cols, col_w) = fetch_grid_columns(ids.len(), ui.available_width());
                    let per_col = ids.len().div_ceil(cols);
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = FETCH_GRID_GAP_X;
                        for ci in 0..cols {
                            ui.vertical(|ui| {
                                // 定宽列 + 截断：超长模型名悬停看全名，而不是把列撑宽。
                                ui.set_width(col_w);
                                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                                for id in ids.iter().skip(ci * per_col).take(per_col) {
                                    let mut checked = models.iter().any(|m| m.id.trim() == id);
                                    if ui
                                        .checkbox(&mut checked, id)
                                        .on_hover_text(id.as_str())
                                        .changed()
                                        && checked
                                    {
                                        let mut row = ModelRow::new();
                                        row.id = id.clone();
                                        row.name = id.clone();
                                        row.source_format = Some(current_page);
                                        models.push(row);
                                    }
                                }
                            });
                        }
                    });
                });
        }
        Err(err) => {
            ui.label(
                egui::RichText::new(format!("获取失败：{}", err))
                    .color(crate::theme::semantics(ui).err),
            );
        }
    }
}

impl App {
    pub(super) fn render_provider_form(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        model_hover_target: &mut Option<String>,
    ) {
        let prev_key = self.providers[idx].key.clone();
        let other_keys: HashSet<String> = self
            .providers
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != idx)
            .map(|(_, p)| p.key.trim().to_string())
            .collect();
        let (variants_label, variant_names) = self.dialect_variants();
        let ProviderFormFlags {
            show_oc,
            show_omp,
            show_dsh,
            show_zcode,
            show_wb,
            show_provider_base_url,
            show_provider_timeout,
            show_model_name,
            show_model_context,
            show_model_output,
            show_model_input,
            show_model_variants,
            show_model_reasoning,
            show_model_tool_call,
            show_model_store,
            show_model_disabled,
            base_label,
            api_key_label,
            context_label,
            output_label,
            input_label,
            ..
        } = ProviderFormFlags::new(self);
        let p = &mut self.providers[idx];
        ui.horizontal_wrapped(|ui| {
            field_label(ui, 120.0, "key");
            let key_resp = ui.add(egui::TextEdit::singleline(&mut p.key).desired_width(120.0));
            if !p.key.trim().is_empty() && other_keys.contains(p.key.trim()) {
                key_resp.on_hover_text("key 与其他 provider 重复，保存将被阻止");
                ui.label(
                    egui::RichText::new("⚠ 重复")
                        .small()
                        .color(crate::theme::semantics(ui).err),
                );
            }
            if show_oc {
                let salt = format!("provider_npm_{}", p.key);
                provider_npm_combo(ui, p, &salt, true);
            }
            if !show_oc {
                let salt = format!("provider_api_{}", p.key);
                provider_api_combo(ui, p, self.current_page, &salt);
            }
            // timeout / timeoutMs 与 npm/api 同排（第一行）。
            if show_oc && show_provider_timeout {
                field_label(ui, 120.0, "options.timeout");
                numeric_text_edit(ui, &mut p.timeout, 70.0, "180000");
            }
            if show_dsh {
                field_label(ui, 120.0, "timeoutMs");
                numeric_text_edit(ui, &mut p.dsh_timeout_ms, 70.0, "180000");
            }
            // pi / omp 的 compat 与 api 同排显示（紧跟 api 之后）。
            if !show_oc && !show_dsh && !show_zcode && !show_wb {
                field_label(ui, 120.0, "compat");
                ui.checkbox(&mut p.compat, "supportsDeveloperRole");
                // pi / omp 相互映射字段：加载 opencode/dsh 时缺省不勾选。
                let requires_label = if show_omp {
                    "requiresReasoningContentForAllAssistantTurns"
                } else {
                    "requiresReasoningContentOnAssistantMessages"
                };
                ui.checkbox(&mut p.requires_reasoning_content, requires_label);
            }
            if show_dsh {
                field_label(ui, 120.0, "retryPolicy.mode");
                ui.add(egui::TextEdit::singleline(&mut p.dsh_retry_mode).desired_width(100.0));
                field_label(ui, 120.0, "maxRetries");
                numeric_text_edit(ui, &mut p.dsh_max_retries, 55.0, "3");
            }
        });
        ui.horizontal_wrapped(|ui| {
            if show_provider_base_url {
                field_label(ui, 120.0, base_label);
                ui.add(egui::TextEdit::singleline(&mut p.base_url).desired_width(200.0));
            }
            // WorkBuddy 的协议由 URL 后缀 + 这个开关表达（文件里没有协议字段）：
            // 不勾选 = 自动补 /chat/completions，勾选 = URL 原样使用。
            if show_wb {
                let mut value = workbuddy_custom(p);
                let resp = ui.checkbox(&mut value, "自定义协议").on_hover_text(
                    "不勾选：保存后由 WorkBuddy 自动补 /chat/completions；\n\
                         勾选：URL 原样使用，需自行写全路径（如 .../v1/messages）。\n\
                         选择非 Chat 协议时保存会自动勾上并补后缀。",
                );
                if resp.changed() {
                    set_workbuddy_custom(p, value);
                }
            }
            field_label(ui, 120.0, api_key_label);
            if show_dsh {
                ui.add(egui::TextEdit::singleline(&mut p.api_key_env).desired_width(192.0));
                field_label(ui, 120.0, "API Key");
                secret_text_edit(ui, &mut p.api_key_secret, self.show_api_keys, 408.0, "");
            } else {
                secret_text_edit(ui, &mut p.api_key, self.show_api_keys, 408.0, "");
            }
        });

        ui.add_space(crate::theme::SPACE_2);
        ui.add_space(crate::theme::SPACE_2);
        let mut fetch_request: Option<(String, String, String, String)> = None;
        let mut close_fetch = false;
        // 本帧用户点下的探测请求（provider key, model id），UI 循环外统一发起。
        let mut probe_request: Option<(String, String)> = None;
        ui.horizontal(|ui| {
            ui.strong("Models");
            let fetch_api = p.effective_api();
            let fetch_secret = credentials::effective_secret(p);
            if ui.button("获取模型").clicked() {
                fetch_request = Some((
                    p.key.clone(),
                    p.base_url.clone(),
                    fetch_secret.clone(),
                    fetch_api.clone(),
                ));
            }
            if self.model_fetch_open.contains(&p.key) && ui.button("关闭").clicked() {
                close_fetch = true;
            }
            // 模型延迟已无批量入口：每个模型行右侧各有一个「测试」按钮，
            // 一次只测一个（同模型 60s、同厂商 10s 节流，规避测活风控）。
            if self
                .latency
                .get(&p.key)
                .is_some_and(|s| s.model_rx.is_some())
            {
                ui.label(egui::RichText::new("延迟测试中…").small());
            }
        });
        if let Some((key, base, secret, api)) = fetch_request {
            // 后台线程拉取模型列表（避免阻塞 UI），结果经通道回传。
            let url = Self::models_url(&base, &api);
            let secret = secret.trim().to_string();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let result = fetch_models_remote(&url, &secret, &api);
                let _ = tx.send(result);
            });
            self.model_fetch.insert(
                key.clone(),
                ModelFetchState {
                    rx: Some(rx),
                    result: None,
                },
            );
            self.model_fetch_open.insert(key);
        }
        if close_fetch {
            self.model_fetch_open.remove(&p.key);
        }
        if self.model_fetch_open.contains(&p.key) {
            let fetch_key = p.key.clone();
            card_frame(
                ui,
                false,
                0,
                egui::Id::new(("model_fetch", fetch_key.clone())),
                |ui| {
                    model_fetch_popup(
                        ui,
                        self.model_fetch.get(&fetch_key),
                        &mut p.models,
                        self.current_page,
                        ("model_fetch_scroll", fetch_key.as_str()),
                    );
                },
            );
        }
        let mut rm: Option<usize> = None;
        let mut model_hover_here: Option<String> = None;
        let mut model_drag_stopped = false;
        for j in 0..p.models.len() {
            let model_key = format!("{}\u{1f}{}", p.key, p.models[j].id);
            let model_highlight = if self.model_drag_target.as_deref() == Some(model_key.as_str()) {
                2
            } else if self.model_drag_src.as_deref() == Some(model_key.as_str()) {
                1
            } else {
                0
            };
            let other_ids: HashSet<String> = p
                .models
                .iter()
                .enumerate()
                .filter(|(j2, _)| *j2 != j)
                .map(|(_, m)| m.id.trim().to_string())
                .collect();
            let model_response = card_frame(
                ui,
                true,
                model_highlight,
                egui::Id::new(("model_card", model_key.clone())),
                |ui| {
                    // 停用的模型整行压淡：一眼能扫出哪些不生效（选择器里也是灰的）。
                    if show_model_disabled && p.models[j].disabled {
                        ui.set_opacity(0.45);
                    }
                    ui.horizontal(|ui| {
                        let handle = ui.add(DragHandle);
                        if handle.drag_started() {
                            self.model_drag_src = Some(model_key.clone());
                            self.model_drag_target = None;
                        }
                        if handle.drag_stopped() {
                            model_drag_stopped = true;
                        }
                        // 单模型延迟测试：按钮在拖动按钮右侧，结果显示在按钮右侧。
                        let model_id = p.models[j].id.trim().to_string();
                        // 启用/停用（仅 WorkBuddy 认这个字段）：`disabled: true` 让该模型
                        // 在 WorkBuddy 的选择器里变灰、不可选，但**仍留在列表里**。
                        // 放在标题行是为了能一眼扫出哪些被停用；停用的行整体压淡。
                        if show_model_disabled {
                            let on = !p.models[j].disabled;
                            let mut enabled = on;
                            let cb = ui.checkbox(&mut enabled, "启用");
                            if enabled != on {
                                p.models[j].disabled = !enabled;
                            }
                            cb.on_hover_text(
                                "WorkBuddy 的选择器**按模型 id 全局去重**：同一个 id 只会列出一行，\
                                 其余同名条目会被忽略。\n\
                                 停用后该条目在选择器里变灰、不可选，但仍在列表里（不会被删）。\n\
                                 ⚠ 不要为了区分同名模型去改 id —— id 同时就是发给上游的模型名，\
                                 改了会直接请求失败。",
                            );
                        }
                        // 只借两个字段（不是 `&self` 方法）：此处 `p` 还借着 providers，
                        // 且外层闭包需要独占 `*self`，整结构借用编译不过。
                        let gate =
                            net_guard_gate(&self.net_guard, self.allow_model_test_with_proxy);
                        if model_probe_button(
                            ui,
                            &self.probe,
                            &p.key,
                            ui.input(|i| i.time),
                            gate.as_deref(),
                        ) {
                            probe_request = Some((p.key.clone(), model_id.clone()));
                        }
                        let latency = self.latency.get(&p.key);
                        model_latency_label(ui, latency, &model_id);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("删").clicked() {
                                rm = Some(j);
                            }
                        });
                    });
                    ui.horizontal_wrapped(|ui| {
                        field_label(ui, 120.0, "id:");
                        let id_resp = ui.add(
                            egui::TextEdit::singleline(&mut p.models[j].id).desired_width(120.0),
                        );
                        if !p.models[j].id.trim().is_empty()
                            && other_ids.contains(p.models[j].id.trim())
                        {
                            // 保存**不会**因为模型 id 重复而失败（只有 provider key 重复才拦），
                            // 所以这里不能说「保存将被阻止」。WorkBuddy 是唯一按 id 全局去重的
                            // 后端：重复的 id 里只有第一条会在它的选择器里生效。
                            let hint = if show_model_disabled {
                                "id 与同 provider 内其他模型重复。\n\
                                 WorkBuddy 的选择器**按模型 id 全局去重**，同一个 id 只会列出一行，\
                                 重复条目里只有第一条生效。\n\
                                 保留多条请把其余条目「停用」（变灰不可选），\
                                 不要改 id —— id 就是发给上游的模型名。"
                            } else {
                                "id 与同 provider 内其他模型重复；保存仍会写入，\
                                 但同名模型在部分后端只会生效一次。"
                            };
                            id_resp.on_hover_text(hint);
                            ui.label(
                                egui::RichText::new("⚠ 重复")
                                    .small()
                                    .color(crate::theme::semantics(ui).warn),
                            );
                        }
                        if show_model_name {
                            field_label(ui, 120.0, "name:");
                            ui.add(
                                egui::TextEdit::singleline(&mut p.models[j].name)
                                    .desired_width(120.0),
                            );
                        }
                        if show_model_reasoning && (show_oc || !show_dsh) {
                            ui.checkbox(&mut p.models[j].reasoning, "reasoning");
                        }
                        if show_model_tool_call && show_oc {
                            ui.checkbox(&mut p.models[j].tool_call, "tool_call");
                        }
                        if show_model_store && show_oc {
                            ui.checkbox(&mut p.models[j].store, "store");
                        }
                        if show_model_context {
                            field_label(ui, 120.0, context_label);
                            numeric_text_edit(ui, &mut p.models[j].context, 53.0, "");
                        }
                        if show_model_output {
                            field_label(ui, 120.0, output_label);
                            numeric_text_edit(ui, &mut p.models[j].output, 53.0, "");
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        if show_model_input {
                            field_label(ui, 120.0, input_label);
                            ui.add(
                                egui::TextEdit::singleline(&mut p.models[j].modalities_input)
                                    .desired_width(80.0),
                            );
                        }
                        if show_oc {
                            field_label(ui, 120.0, "modalities.output");
                            ui.add(
                                egui::TextEdit::singleline(&mut p.models[j].modalities_output)
                                    .desired_width(80.0),
                            );
                        }
                        if show_model_variants {
                            field_label(ui, 120.0, variants_label);
                        }
                        let variant_key = format!("variant_open_{}_{}", p.key, j);
                        variant_selector(
                            ui,
                            &mut p.models[j].variants,
                            variant_names,
                            variant_key,
                            &mut self.variant_open,
                            true,
                        );
                    });
                },
            );
            if let Some(src) = &self.model_drag_src {
                if src != &model_key
                    && model_response.contains_pointer()
                    && model_hover_here.is_none()
                {
                    model_hover_here = Some(model_key.clone());
                }
            }
        }
        if let Some((provider_key, model_id)) = probe_request {
            let now = ui.input(|i| i.time);
            let base_url = p.base_url.clone();
            let secret = credentials::effective_secret(p);
            let api = p.effective_api();
            // 先取出门控值：`p` 还借着 providers，整结构借用的方法在这里也会冲突。
            let gate = net_guard_gate(&self.net_guard, self.allow_model_test_with_proxy);
            self.status = Self::run_model_probe(
                &mut self.probe,
                &mut self.latency,
                gate.as_deref(),
                &provider_key,
                &model_id,
                now,
                &base_url,
                &secret,
                &api,
            );
        }
        // 只登记「本 provider 内被拖到的模型」，跨卡片的聚合交给调用方
        // （ui_providers_section 在全部卡片渲染完之后统一写入 self.model_drag_target）。
        merge_drag_target(model_hover_target, model_hover_here);
        if model_drag_stopped {
            if let Some(src) = self.model_drag_src.take() {
                let target = self.model_drag_target.take();
                if let Some(dst) = target {
                    let source = p
                        .models
                        .iter()
                        .position(|m| format!("{}\u{1f}{}", p.key, m.id) == src);
                    let destination = p
                        .models
                        .iter()
                        .position(|m| format!("{}\u{1f}{}", p.key, m.id) == dst);
                    if let (Some(source), Some(destination)) = (source, destination) {
                        move_item(&mut p.models, source, destination);
                    }
                }
            }
        }
        if let Some(j) = rm {
            p.models.remove(j);
            // 删除后下标错位：关闭该 provider 的档位弹窗，避免状态串到其他模型
            let prefix = format!("variant_open_{}_", p.key);
            self.variant_open.retain(|k| !k.starts_with(&prefix));
        }
        ui.add_space(crate::theme::SPACE_2);
        let show_new_model_key = format!("show_new_model_{}", p.key);
        let show_new_model = self.variant_open.contains(&show_new_model_key);
        let btn_text = if show_new_model {
            "收起"
        } else {
            "添加 Model"
        };
        if ui
            .add_sized([120.0, 20.0], egui::Button::new(btn_text))
            .clicked()
        {
            if show_new_model {
                self.variant_open.remove(&show_new_model_key);
            } else {
                self.variant_open.insert(show_new_model_key.clone());
            }
        }
        if show_new_model {
            ui.horizontal_wrapped(|ui| {
                field_label(ui, 120.0, "id:");
                ui.add(egui::TextEdit::singleline(&mut p.new_model.id).desired_width(120.0));
                field_label(ui, 120.0, "name:");
                ui.add(egui::TextEdit::singleline(&mut p.new_model.name).desired_width(120.0));
                if show_oc || !show_dsh {
                    ui.checkbox(&mut p.new_model.reasoning, "reasoning");
                }
                if show_oc {
                    ui.checkbox(&mut p.new_model.tool_call, "tool_call");
                    ui.checkbox(&mut p.new_model.store, "store");
                }
                field_label(ui, 120.0, context_label);
                numeric_text_edit(ui, &mut p.new_model.context, 53.0, "");
                field_label(ui, 120.0, output_label);
                numeric_text_edit(ui, &mut p.new_model.output, 53.0, "");
            });
            ui.horizontal_wrapped(|ui| {
                field_label(ui, 120.0, input_label);
                ui.add(
                    egui::TextEdit::singleline(&mut p.new_model.modalities_input)
                        .desired_width(80.0),
                );
                if show_oc {
                    field_label(ui, 120.0, "modalities.output");
                    ui.add(
                        egui::TextEdit::singleline(&mut p.new_model.modalities_output)
                            .desired_width(80.0),
                    );
                }
                field_label(ui, 120.0, variants_label);
                let variant_key = format!("new_model_variant_{}", p.key);
                variant_selector(
                    ui,
                    &mut p.new_model.variants,
                    variant_names,
                    variant_key,
                    &mut self.variant_open,
                    false,
                );
            });
            ui.horizontal(|ui| {
                ui.add_space(crate::theme::SPACE_7);
                if ui.button("添加").clicked() && !p.new_model.id.trim().is_empty() {
                    p.models.push(p.new_model.clone());
                    p.new_model = ModelRow::new();
                    self.variant_open.remove(&show_new_model_key);
                }
            });
        }
        // key 重命名后同步展开状态与弹窗键
        let new_key = self.providers[idx].key.clone();
        if new_key != prev_key {
            self.sync_provider_rename(&prev_key, &new_key);
        }
    }

    pub(super) fn ui_new_provider_form(&mut self, ui: &mut egui::Ui) {
        let (variants_label, variant_names) = self.dialect_variants();
        let ProviderFormFlags {
            show_oc,
            show_omp,
            show_dsh,
            show_zcode,
            show_wb,
            base_label,
            api_key_label,
            context_label,
            output_label,
            input_label,
            ..
        } = ProviderFormFlags::new(self);
        ui.group(|ui| {
            // 官方预设（可选）：一键填 key / baseUrl / 协议；不套用则完全手填。
            ui.horizontal_wrapped(|ui| {
                let dialect = if show_oc {
                    crate::presets::PresetDialect::Opencode
                } else {
                    crate::presets::PresetDialect::PiLike
                };
                provider_preset_combo(ui, &mut self.new_provider, dialect, "new_provider_preset");
            });
            ui.horizontal_wrapped(|ui| {
                field_label(ui, 120.0, "key");
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_provider.key)
                        .hint_text("openai")
                        .desired_width(120.0),
                );
                if show_oc {
                    let p = &mut self.new_provider;
                    provider_npm_combo(ui, p, "new_provider_npm", false);
                }
                if !show_oc {
                    let p = &mut self.new_provider;
                    provider_api_combo(ui, p, self.current_page, "new_provider_api");
                }
                // timeout 与 npm/api 同排（第一行）。
                if show_oc {
                    field_label(ui, 120.0, "options.timeout");
                    numeric_text_edit(ui, &mut self.new_provider.timeout, 70.0, "180000");
                }
                // pi / omp 的 compat 与 api 同排显示（紧跟 api 之后）。
                // ZCode / WorkBuddy 没有 compat 字段，不显示。
                if !show_oc && !show_dsh && !show_zcode && !show_wb {
                    field_label(ui, 120.0, "compat");
                    ui.checkbox(&mut self.new_provider.compat, "supportsDeveloperRole");
                    let requires_label = if show_omp {
                        "requiresReasoningContentForAllAssistantTurns"
                    } else {
                        "requiresReasoningContentOnAssistantMessages"
                    };
                    ui.checkbox(
                        &mut self.new_provider.requires_reasoning_content,
                        requires_label,
                    );
                }
            });
            ui.horizontal_wrapped(|ui| {
                field_label(ui, 120.0, base_label);
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_provider.base_url)
                        .hint_text("https://api.openai.com/v1")
                        .desired_width(200.0),
                );
                // WorkBuddy：与已有 provider 卡片一致的「自定义协议」开关。
                if show_wb {
                    let mut value = workbuddy_custom(&self.new_provider);
                    let resp = ui.checkbox(&mut value, "自定义协议").on_hover_text(
                        "不勾选：保存后由 WorkBuddy 自动补 /chat/completions；\n\
                             勾选：URL 原样使用，需自行写全路径（如 .../v1/messages）。",
                    );
                    if resp.changed() {
                        set_workbuddy_custom(&mut self.new_provider, value);
                    }
                }
                field_label(ui, 120.0, api_key_label);
                if show_dsh {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_provider.api_key_env)
                            .hint_text("DEEPSEEK_API_KEY")
                            .desired_width(192.0),
                    );
                    field_label(ui, 120.0, "API Key");
                    secret_text_edit(
                        ui,
                        &mut self.new_provider.api_key_secret,
                        self.show_api_keys,
                        408.0,
                        "实际密钥",
                    );
                } else {
                    secret_text_edit(
                        ui,
                        &mut self.new_provider.api_key,
                        self.show_api_keys,
                        408.0,
                        "sk-xxx",
                    );
                }
            });
            ui.add_space(crate::theme::SPACE_2);
            let mut fetch_request: Option<(String, String, String)> = None;
            let mut close_fetch = false;
            ui.horizontal(|ui| {
                ui.strong("Models");
                let fetch_api = self.new_provider.effective_api();
                let fetch_secret = credentials::effective_secret(&self.new_provider);
                if ui.button("获取模型").clicked() {
                    fetch_request = Some((
                        self.new_provider.base_url.clone(),
                        fetch_secret.clone(),
                        fetch_api.clone(),
                    ));
                }
                if self.model_fetch_open.contains(NEW_PROVIDER_FETCH_KEY)
                    && ui.button("关闭").clicked()
                {
                    close_fetch = true;
                }
                // 模型延迟已无批量入口：每个模型行右侧各有一个「测试」按钮。
                if self
                    .latency
                    .get(NEW_PROVIDER_FETCH_KEY)
                    .is_some_and(|s| s.model_rx.is_some())
                {
                    ui.label(egui::RichText::new("延迟测试中…").small());
                }
            });
            if let Some((base, secret, api)) = fetch_request {
                self.start_model_fetch(NEW_PROVIDER_FETCH_KEY, &base, &secret, &api);
                self.model_fetch_open
                    .insert(NEW_PROVIDER_FETCH_KEY.to_string());
            }
            if close_fetch {
                self.model_fetch_open.remove(NEW_PROVIDER_FETCH_KEY);
            }
            if self.model_fetch_open.contains(NEW_PROVIDER_FETCH_KEY) {
                card_frame(
                    ui,
                    false,
                    0,
                    egui::Id::new(("new_provider_fetch", NEW_PROVIDER_FETCH_KEY)),
                    |ui| {
                        model_fetch_popup(
                            ui,
                            self.model_fetch.get(NEW_PROVIDER_FETCH_KEY),
                            &mut self.new_provider.models,
                            self.current_page,
                            "new_provider_fetch_scroll",
                        );
                    },
                );
            }
            let mut rm_new: Option<usize> = None;
            let mut move_new_request: Option<(usize, usize)> = None;
            // 本帧用户点下的探测请求（新 provider 固定用 NEW_PROVIDER_FETCH_KEY 做节流键）。
            let mut probe_request: Option<(String, String)> = None;
            for j in 0..self.new_provider.models.len() {
                let model_count = self.new_provider.models.len();
                card_frame(
                    ui,
                    true,
                    0,
                    egui::Id::new(("new_provider_model", j)),
                    |ui| {
                        ui.horizontal(|ui| {
                            if j > 0 && ui.button("↑").clicked() {
                                move_new_request = Some((j, j - 1));
                            }
                            if j + 1 < model_count && ui.button("↓").clicked() {
                                move_new_request = Some((j, j + 1));
                            }
                            // 单模型延迟测试：按钮在调整按钮右侧，结果显示在按钮右侧。
                            let model_id = self.new_provider.models[j].id.trim().to_string();
                            let gate =
                                net_guard_gate(&self.net_guard, self.allow_model_test_with_proxy);
                            if model_probe_button(
                                ui,
                                &self.probe,
                                NEW_PROVIDER_FETCH_KEY,
                                ui.input(|i| i.time),
                                gate.as_deref(),
                            ) {
                                probe_request =
                                    Some((NEW_PROVIDER_FETCH_KEY.to_string(), model_id.clone()));
                            }
                            let latency = self.latency.get(NEW_PROVIDER_FETCH_KEY);
                            model_latency_label(ui, latency, &model_id);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("删").clicked() {
                                        rm_new = Some(j);
                                    }
                                },
                            );
                        });
                        ui.horizontal_wrapped(|ui| {
                            field_label(ui, 120.0, "id:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.new_provider.models[j].id)
                                    .desired_width(120.0),
                            );
                            field_label(ui, 120.0, "name:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.new_provider.models[j].name)
                                    .desired_width(120.0),
                            );
                            if show_oc || !show_dsh {
                                ui.checkbox(
                                    &mut self.new_provider.models[j].reasoning,
                                    "reasoning",
                                );
                            }
                            if show_oc {
                                ui.checkbox(
                                    &mut self.new_provider.models[j].tool_call,
                                    "tool_call",
                                );
                                ui.checkbox(&mut self.new_provider.models[j].store, "store");
                            }
                            field_label(ui, 120.0, context_label);
                            numeric_text_edit(
                                ui,
                                &mut self.new_provider.models[j].context,
                                53.0,
                                "",
                            );
                            field_label(ui, 120.0, output_label);
                            numeric_text_edit(
                                ui,
                                &mut self.new_provider.models[j].output,
                                53.0,
                                "",
                            );
                        });
                    },
                );
            }
            if let Some((provider_key, model_id)) = probe_request {
                let now = ui.input(|i| i.time);
                let base_url = self.new_provider.base_url.clone();
                let secret = credentials::effective_secret(&self.new_provider);
                let api = self.new_provider.effective_api();
                let gate = net_guard_gate(&self.net_guard, self.allow_model_test_with_proxy);
                self.status = Self::run_model_probe(
                    &mut self.probe,
                    &mut self.latency,
                    gate.as_deref(),
                    &provider_key,
                    &model_id,
                    now,
                    &base_url,
                    &secret,
                    &api,
                );
            }
            if let Some((from, to)) = move_new_request {
                move_item(&mut self.new_provider.models, from, to);
            }
            if let Some(j) = rm_new {
                self.new_provider.models.remove(j);
            }
            ui.add_space(crate::theme::SPACE_2);
            let show_new_model_key = format!("new_provider_show_model_{}", self.new_provider.key);
            let show_new_model = self.variant_open.contains(&show_new_model_key);
            if ui
                .button(if show_new_model {
                    "收起"
                } else {
                    "添加 Model"
                })
                .clicked()
            {
                if show_new_model {
                    self.variant_open.remove(&show_new_model_key);
                } else {
                    self.variant_open.insert(show_new_model_key.clone());
                }
            }
            if show_new_model {
                ui.horizontal_wrapped(|ui| {
                    field_label(ui, 120.0, "id:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_provider.new_model.id)
                            .desired_width(120.0),
                    );
                    field_label(ui, 120.0, "name:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_provider.new_model.name)
                            .desired_width(120.0),
                    );
                    if show_oc || !show_dsh {
                        ui.checkbox(&mut self.new_provider.new_model.reasoning, "reasoning");
                    }
                    if show_oc {
                        ui.checkbox(&mut self.new_provider.new_model.tool_call, "tool_call");
                        ui.checkbox(&mut self.new_provider.new_model.store, "store");
                    }
                    field_label(ui, 120.0, context_label);
                    numeric_text_edit(ui, &mut self.new_provider.new_model.context, 53.0, "");
                    field_label(ui, 120.0, output_label);
                    numeric_text_edit(ui, &mut self.new_provider.new_model.output, 53.0, "");
                });
                ui.horizontal_wrapped(|ui| {
                    field_label(ui, 120.0, input_label);
                    ui.add(
                        egui::TextEdit::singleline(
                            &mut self.new_provider.new_model.modalities_input,
                        )
                        .desired_width(80.0),
                    );
                    if show_oc {
                        field_label(ui, 120.0, "modalities.output");
                        ui.add(
                            egui::TextEdit::singleline(
                                &mut self.new_provider.new_model.modalities_output,
                            )
                            .desired_width(80.0),
                        );
                    }
                    field_label(ui, 120.0, variants_label);
                    variant_selector(
                        ui,
                        &mut self.new_provider.new_model.variants,
                        variant_names,
                        "new_provider_new_model_variant".to_string(),
                        &mut self.variant_open,
                        false,
                    );
                });
                ui.horizontal(|ui| {
                    ui.add_space(crate::theme::SPACE_7);
                    if ui.button("添加").clicked()
                        && !self.new_provider.new_model.id.trim().is_empty()
                    {
                        self.new_provider
                            .models
                            .push(self.new_provider.new_model.clone());
                        self.new_provider.new_model = ModelRow::new();
                        self.variant_open.remove(&show_new_model_key);
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.add_space(crate::theme::SPACE_7);
                if ui.button("确认").clicked() {
                    let key = self.new_provider.key.trim().to_string();
                    if key.is_empty() {
                        self.status = "请填写 provider key".into();
                    } else if self.providers.iter().any(|p| p.key.trim() == key) {
                        self.status = format!("provider key \"{}\" 已存在", key);
                    } else {
                        let np = self.new_provider.clone();
                        self.providers.push(np);
                        self.new_provider = ProviderRow::new();
                        self.show_new_provider = false;
                        self.clear_new_provider_state();
                        self.status = "已添加 provider".into();
                    }
                }
                if ui.button("取消").clicked() {
                    self.new_provider = ProviderRow::new();
                    self.show_new_provider = false;
                    self.clear_new_provider_state();
                }
            });
        });
    }

    /// 关闭新增 provider 表单时清理其测试/获取状态，避免下次打开残留旧结果。
    pub(super) fn clear_new_provider_state(&mut self) {
        self.latency.remove(NEW_PROVIDER_FETCH_KEY);
        self.model_fetch.remove(NEW_PROVIDER_FETCH_KEY);
        self.model_fetch_open.remove(NEW_PROVIDER_FETCH_KEY);
        // 表单关掉后探测结果无处显示：释放它的串行位。
        self.probe.release(Some(NEW_PROVIDER_FETCH_KEY));
    }
}
