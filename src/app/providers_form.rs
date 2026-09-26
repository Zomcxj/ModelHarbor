//! Provider 编辑 / 新增表单、模型获取弹层与表单字段控件。
use super::App;
use crate::app::fetch::{
    fetch_models_remote, model_latency_label, model_probe_button, net_guard_gate, LatencyState,
    ModelFetchState, NEW_PROVIDER_FETCH_KEY,
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
use std::collections::{HashMap, HashSet};

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
                egui::RichText::new(
                    "套用后仅覆盖 key / baseUrl / 协议，其余字段可照旧手填；\n\
                     不写密钥、不改动模型列表。第三方 / 中转站 / 自建端点请直接在下方手填",
                )
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
        });
}

/// api 下拉：omp 官方 9 值 / pi KnownApi 10 值；首项「(空)」与 opencode 页 npm 的空选项同义。
pub(super) fn provider_api_combo(
    ui: &mut egui::Ui,
    p: &mut ProviderRow,
    page: ConfigFormat,
    id_salt: &str,
) {
    // 每个后端自己的协议词表：omp 9 值 / pi 10 值 / ZCode 3 值（多一个 chat）/
    // WorkBuddy 3 值（协议落到 URL 后缀，见 workbuddy 后端）/
    // QwenCode 3 值（协议落到 pid + wireApi，见 qwen_code 后端）/
    // KimiCode 6 值（逐字就是 `type` 字段，见 kimi_code 后端）。
    let options: &[&str] = match page {
        ConfigFormat::OhMyPi => &convert::OMP_APIS,
        ConfigFormat::ZCode => &convert::ZCODE_APIS,
        ConfigFormat::WorkBuddy => &convert::WORKBUDDY_APIS,
        ConfigFormat::QwenCode => &convert::QWEN_APIS,
        ConfigFormat::KimiCode => &convert::KIMI_APIS,
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
    // 「(空)」= 未指定协议。协议在 opencode 系 / pi / omp / DSH 之间共用一份数据，
    // 故以 npm / pi_api / raw.api 是否都为空判定，显示值统一走 effective_api()，
    // 与写盘、延迟测试同口径。
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
            ui.label(
                egui::RichText::new("(空) = 不指定协议：写盘时按兼容层 openai-completions 处理")
                    .small()
                    .weak(),
            );
            ui.separator();
            if ui.selectable_label(!explicit, EMPTY_API_LABEL).clicked() {
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
                }
            }
        });
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

/// 编辑表单（`render_provider_form`）与新增表单（`ui_new_provider_form`）共用的
/// provider 头部字段：key / 协议下拉 / timeout / compat / DSH 重试 / baseURL / 密钥。
///
/// 两份表单曾逐行抄写并各自漂移——新增表单漏了 DSH 的 timeoutMs 与 retryPolicy，
/// baseURL 的显隐也和编辑表单不一致。统一走这里的同一套门控后，差异只剩三件事，
/// 全由 [`ProviderHeaderCtx`] 表达：占位提示与 key 重复检查（新增表单在「确认」时才查）、
/// npm 空选项的标签、以及 `relaxed` 门控。
struct ProviderHeaderCtx<'a> {
    page: ConfigFormat,
    npm_salt: String,
    api_salt: String,
    /// 新增表单 = true：字段是给**全新** provider 填的，不受「已加载文件里有没有
    /// 这个键」的影响（`show_provider_base_url` 这类内容型开关放宽为常显）。
    relaxed: bool,
    npm_empty_label: bool,
    show_api_keys: bool,
    /// key 重复检查用的「其他 provider key」集合；None = 不查（新增表单）。
    other_keys: Option<&'a HashSet<String>>,
}

fn provider_header_fields(
    ui: &mut egui::Ui,
    p: &mut ProviderRow,
    flags: &ProviderFormFlags,
    ctx: &ProviderHeaderCtx<'_>,
) {
    let on = |flag: bool| ctx.relaxed || flag;
    ui.horizontal_wrapped(|ui| {
        field_label(ui, 120.0, "key");
        let key_edit = egui::TextEdit::singleline(&mut p.key).desired_width(120.0);
        let key_edit = if ctx.relaxed {
            key_edit.hint_text("openai")
        } else {
            key_edit
        };
        let key_resp = ui.add(key_edit);
        if let Some(other_keys) = ctx.other_keys {
            if !p.key.trim().is_empty() && other_keys.contains(p.key.trim()) {
                key_resp.on_hover_text("key 与其他 provider 重复，保存将被阻止");
                ui.label(
                    egui::RichText::new("⚠ 重复")
                        .small()
                        .color(crate::theme::semantics(ui).err),
                );
            }
        }
        if flags.show_oc {
            provider_npm_combo(ui, p, &ctx.npm_salt, ctx.npm_empty_label);
        }
        if !flags.show_oc {
            provider_api_combo(ui, p, ctx.page, &ctx.api_salt);
        }
        // timeout / timeoutMs 与 npm/api 同排（第一行）。
        if flags.show_oc && flags.show_provider_timeout {
            field_label(ui, 120.0, "options.timeout");
            numeric_text_edit(ui, &mut p.timeout, 70.0, "180000");
        }
        if flags.show_dsh {
            field_label(ui, 120.0, "timeoutMs");
            numeric_text_edit(ui, &mut p.dsh_timeout_ms, 70.0, "180000");
        }
        // QwenCode 的 timeout 落在条目的 generationConfig 里。
        if flags.show_qwen {
            field_label(ui, 120.0, "generationConfig.timeout");
            numeric_text_edit(ui, &mut p.timeout, 70.0, "180000");
        }
        // pi / omp 的 compat 与 api 同排显示（紧跟 api 之后）。
        if !flags.show_oc
            && !flags.show_dsh
            && !flags.show_zcode
            && !flags.show_wb
            && !flags.show_qwen
            && !flags.show_kimi
        {
            field_label(ui, 120.0, "compat");
            ui.checkbox(&mut p.compat, "supportsDeveloperRole");
            // pi / omp 相互映射字段：加载 opencode/dsh 时缺省不勾选。
            let requires_label = if flags.show_omp {
                "requiresReasoningContentForAllAssistantTurns"
            } else {
                "requiresReasoningContentOnAssistantMessages"
            };
            ui.checkbox(&mut p.requires_reasoning_content, requires_label);
        }
        if flags.show_dsh {
            field_label(ui, 120.0, "retryPolicy.mode");
            ui.add(egui::TextEdit::singleline(&mut p.dsh_retry_mode).desired_width(100.0));
            field_label(ui, 120.0, "maxRetries");
            numeric_text_edit(ui, &mut p.dsh_max_retries, 55.0, "3");
        }
    });
    ui.horizontal_wrapped(|ui| {
        if on(flags.show_provider_base_url) {
            field_label(ui, 120.0, flags.base_label);
            let url_edit = egui::TextEdit::singleline(&mut p.base_url).desired_width(200.0);
            let url_edit = if ctx.relaxed {
                url_edit.hint_text("https://api.openai.com/v1")
            } else {
                url_edit
            };
            ui.add(url_edit);
        }
        // WorkBuddy 的协议由 URL 后缀表达（文件里没有协议字段），**不给手动开关**：
        // 上面选的协议决定保存时补什么后缀，选非 chat/completions 就是自定义协议。
        // 曾经有个「自定义协议」勾选框，但它与协议选择表达同一件事，两个控件可以
        // 互相矛盾（勾了却选着 chat、或没勾却选了 messages），保存时还得强制对齐一次。
        // Kimi 与 Qwen 的凭据框各自画标签：Kimi 的标签是「环境变量名」勾选框本身，
        // Qwen 只剩一个密钥框。
        if !flags.show_kimi && !flags.show_qwen {
            field_label(ui, 120.0, flags.api_key_label);
        }
        if flags.show_dsh {
            let env_edit = egui::TextEdit::singleline(&mut p.api_key_env).desired_width(192.0);
            let env_edit = if ctx.relaxed {
                env_edit.hint_text("DEEPSEEK_API_KEY")
            } else {
                env_edit
            };
            ui.add(env_edit);
            field_label(ui, 120.0, "API Key");
            let hint = if ctx.relaxed { "实际密钥" } else { "" };
            secret_text_edit(ui, &mut p.api_key_secret, ctx.show_api_keys, 408.0, hint);
        } else if flags.show_qwen {
            // QwenCode 的密钥存在顶层 `env[<envKey>]` 里：条目上写变量名、`env` 里写
            // 值。变量名**不单独给框**——留空就按 provider key 自动推导（见后端
            // `env_key_name`），文件里已有的变量名原样沿用；界面上只填密钥值本身。
            field_label(ui, 120.0, "API Key");
            let hint = if ctx.relaxed { "实际密钥" } else { "" };
            secret_text_edit(ui, &mut p.api_key, ctx.show_api_keys, 408.0, hint);
        } else if flags.show_kimi {
            // KimiCode 的 `api_key`（内联密钥）与 `api_key_env`（环境变量名）**互斥**：
            // 同时写两个会让 Kimi Code **启动失败**。两个框摆在一起永远有一个是空的，
            // 用户还得猜哪个生效——收敛成一个框：勾上 `api_key_env`，框里填的就是
            // 变量名（不掩码，它不是密钥）；不勾就是内联密钥 `api_key`。切换时已敲的
            // 文本跟着搬走，互斥由「只有一个框」从结构上保证，不再依赖现场清空。
            let mut env_mode = p.kimi_env_mode;
            if ui.checkbox(&mut env_mode, "api_key_env").changed() {
                let text = if env_mode {
                    std::mem::take(&mut p.api_key)
                } else {
                    std::mem::take(&mut p.api_key_env)
                };
                if env_mode {
                    p.api_key_env = text;
                    p.api_key.clear();
                } else {
                    p.api_key = text;
                    p.api_key_env.clear();
                }
                p.kimi_env_mode = env_mode;
            }
            if p.kimi_env_mode {
                let edit = egui::TextEdit::singleline(&mut p.api_key_env).desired_width(200.0);
                let edit = if ctx.relaxed {
                    edit.hint_text("MY_API_KEY")
                } else {
                    edit
                };
                ui.add(edit);
            } else {
                let hint = if ctx.relaxed { "sk-xxx" } else { "" };
                secret_text_edit(ui, &mut p.api_key, ctx.show_api_keys, 200.0, hint);
            }
        } else {
            let hint = if ctx.relaxed { "sk-xxx" } else { "" };
            secret_text_edit(ui, &mut p.api_key, ctx.show_api_keys, 408.0, hint);
        }
    });
}

/// 「获取模型」区块（两份表单共用）：按钮行、后台拉取与弹层。
///
/// 拆字段传参而不是拿 `&mut App`：编辑表单里 `p = &mut self.providers[idx]` 还借着一角，
/// `model_fetch` 等字段必须拆开借才能与它共存（与 `start_provider_latency` 同理）。
struct FetchSectionCtx<'a> {
    model_fetch: &'a mut HashMap<String, ModelFetchState>,
    model_fetch_open: &'a mut HashSet<String>,
    latency: &'a HashMap<String, LatencyState>,
    current_page: ConfigFormat,
    fetch_key: &'a str,
    popup_salt: &'a str,
    scroll_salt: egui::Id,
}

fn models_fetch_section(
    ui: &mut egui::Ui,
    ctx: &mut FetchSectionCtx<'_>,
    base_url: &str,
    secret: &str,
    api: &str,
    models: &mut Vec<ModelRow>,
) {
    let mut fetch_request: Option<(String, String, String)> = None;
    let mut close_fetch = false;
    ui.horizontal(|ui| {
        ui.strong("Models");
        if ui.button("获取模型").clicked() {
            fetch_request = Some((base_url.to_string(), secret.to_string(), api.to_string()));
        }
        if ctx.model_fetch_open.contains(ctx.fetch_key) && ui.button("关闭").clicked() {
            close_fetch = true;
        }
        // 模型延迟已无批量入口：每个模型行右侧各有一个「测试」按钮，
        // 一次只测一个（同模型 60s、同厂商 10s 节流，规避测活风控）。
        if ctx
            .latency
            .get(ctx.fetch_key)
            .is_some_and(|s| s.model_rx.is_some())
        {
            ui.label(egui::RichText::new("延迟测试中…").small());
        }
    });
    if let Some((base, secret, api)) = fetch_request {
        let url = App::models_url(&base, &api);
        let secret = secret.trim().to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = fetch_models_remote(&url, &secret, &api);
            let _ = tx.send(result);
        });
        ctx.model_fetch.insert(
            ctx.fetch_key.to_string(),
            ModelFetchState {
                rx: Some(rx),
                result: None,
            },
        );
        ctx.model_fetch_open.insert(ctx.fetch_key.to_string());
    }
    if close_fetch {
        ctx.model_fetch_open.remove(ctx.fetch_key);
    }
    if ctx.model_fetch_open.contains(ctx.fetch_key) {
        let fetch_key = ctx.fetch_key.to_string();
        card_frame(
            ui,
            false,
            0,
            egui::Id::new((ctx.popup_salt, fetch_key.clone())),
            |ui| {
                model_fetch_popup(
                    ui,
                    ctx.model_fetch.get(&fetch_key),
                    models,
                    ctx.current_page,
                    ctx.scroll_salt,
                );
            },
        );
    }
}

/// 档位（variants）下拉所需的成套参数：字段标签、词表、展开状态与展开键。
struct VariantsCtx<'a> {
    label: &'static str,
    names: &'a [&'static str],
    open_set: &'a mut HashSet<String>,
    open_key: String,
    /// 编辑表单的模型行会归一已存的档位串；「添加 Model」子表单只收集新输入。
    normalize: bool,
}

/// 上下文（输入）的常用预设值，单位为 **k（1000）**——与仓库里既有的写法一致
/// （`ModelRow::new()` 的 `262000` 就是 262k）。
///
/// 取 `1000` 而不是 `1024`：厂商文档与网关界面普遍按 1000 报数（「上下文 128k」），
/// 而这些字段是**发给上游的声明**，少声明一点是安全的，多声明会被上游直接拒绝。
/// 所以 `262k` 落到 `262000` 而不是 `262144`、`1024k` 落到 `1024000` 而不是 `1048576`。
const CONTEXT_PRESETS: [u32; 7] = [128, 200, 262, 300, 400, 500, 1024];

/// 最大输出（output）的常用预设值，单位同上。
///
/// `131` / `262` 看着不像整数是有意的：对应厂商常声明的 `131072` / `262144`，
/// 按 1000 进制落成 `131000` / `262000`（少声明，安全）。
const OUTPUT_PRESETS: [u32; 4] = [32, 64, 131, 262];

/// 预设值的显示文本（`128k`）与实际写入配置的数字（`128000`）。
///
/// 两者**分开**给出：界面写「128k」是因为它一眼能读懂，而配置里必须是整数——
/// 上游不认 `128k` 这种写法。
fn preset_label(k: u32) -> String {
    format!("{}k", k)
}

fn preset_value(k: u32) -> String {
    (k as u64 * 1000).to_string()
}

/// 当前值在预设表里的下标；不在表里（手填的任意数字、空、非法）返回 `None`。
///
/// 抽成独立函数是为了能单测：它决定下拉收起时显示哪个标签，而「手填的值被误显示成
/// 某个预设」会让用户以为自己选过——这种错只在界面上看得出来，测试里看不见。
fn preset_index(value: &str, presets: &[u32]) -> Option<usize> {
    presets
        .iter()
        .position(|k| preset_value(*k) == value.trim())
}

/// 数值字段右侧的预设下拉：选一项就把该值填进字段。
///
/// 用下拉而不是按钮组：上下文有 7 个预设，平铺会把整行挤爆（这一行本来就有
/// id / name / 三个勾选框），而下拉在收起时只占一个控件的宽度。
///
/// 只**填值**、不锁定：填完仍可继续手动编辑。选中项按当前值反查，所以手填的
/// 非预设值不会误显示成某个预设（下拉显示 `选择...`）。
fn preset_combo(
    ui: &mut egui::Ui,
    value: &mut String,
    presets: &[u32],
    id_salt: &str,
    tooltip: &str,
) {
    // 当前值恰好等于某个预设时，把下拉显示成那一项；否则显示占位文案。
    let selected = preset_index(value, presets);
    let text = match selected {
        Some(i) => preset_label(presets[i]),
        None => "选择...".to_string(),
    };
    // 只有真正点了某一项才写回：初值是 `None` 而不是 `selected`，
    // 否则「打开下拉又点空白处关掉」也会走一次赋值（虽然写的是同一个数，
    // 但会把 `" 262000 "` 这类带空白的值静默改写，属于用户没要求的改动）。
    let mut picked: Option<usize> = None;
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(text)
        .width(72.0)
        .show_ui(ui, |ui| {
            for (i, k) in presets.iter().enumerate() {
                if ui
                    .selectable_label(selected == Some(i), preset_label(*k))
                    .clicked()
                {
                    picked = Some(i);
                }
            }
        })
        .response
        .on_hover_text(tooltip);
    if let Some(i) = picked {
        *value = preset_value(presets[i]);
    }
}

/// 模型字段编辑行之一：id / name / reasoning / tool_call / store / context / output。
///
/// `relaxed` = 新增表单语境：`ProviderFormFlags` 里**内容型**开关（文件里出现过该键
/// 才显示）放宽为常显——新模型是全新条目，不该受已加载文件内容影响；
/// 方言门控（oc / dsh）照旧保留。
///
/// `dup` 给出模型 id 的重复判定集合（编辑表单）：命中时在 id 字段旁画 ⚠ 提示——
/// 只有编辑表单有这步，新增表单传 None。
///
/// `salt` 区分同一页里的多个表单实例（编辑表单用 provider key，新增表单用固定串）：
/// 下拉的展开状态按 `Id` 记在 egui 里，两处共用同一个 id 会互相串开合状态。
fn model_core_fields_row(
    ui: &mut egui::Ui,
    m: &mut ModelRow,
    flags: &ProviderFormFlags,
    relaxed: bool,
    dup: Option<(&HashSet<String>, &HashSet<String>)>,
    salt: &str,
) {
    let on = |flag: bool| relaxed || flag;
    ui.horizontal_wrapped(|ui| {
        field_label(ui, 120.0, "id:");
        let id_resp = ui.add(egui::TextEdit::singleline(&mut m.id).desired_width(120.0));
        if let Some((same_provider, global)) = dup {
            if !m.id.trim().is_empty()
                && (same_provider.contains(m.id.trim()) || global.contains(m.id.trim()))
            {
                // 保存**不会**因为模型 id 重复而失败（只有 provider key 重复才拦），
                // 所以这里不能说「保存将被阻止」。WorkBuddy 是唯一按 id 全局去重的
                // 后端：重复的 id 里只有第一条会在它的选择器里生效。
                let hint = if flags.show_model_disabled {
                    "这个模型名在**全局**出现多次（含其他厂商）。\n\
                     WorkBuddy 的选择器按模型 id 全局去重，同名只会列出一行、\
                     只有第一条生效。\n\
                     要用哪一家，就在那一家的卡片上勾「启用」——勾上后同名的\
                     其他条目会自动关闭，只有勾选的写进 models.json。\n\
                     取消勾选不会丢配置：条目仍存在 models.full.json 里，勾回来即可。\n\
                     不要改 id —— id 就是发给上游的模型名，改了会直接请求失败。"
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
        }
        if on(flags.show_model_name) {
            field_label(ui, 120.0, "name:");
            ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(120.0));
        }
        if on(flags.show_model_reasoning) && (flags.show_oc || !flags.show_dsh) {
            ui.checkbox(&mut m.reasoning, "reasoning");
        }
        if on(flags.show_model_tool_call) && flags.show_oc {
            ui.checkbox(&mut m.tool_call, "tool_call");
        }
        if on(flags.show_model_store) && flags.show_oc {
            ui.checkbox(&mut m.store, "store");
        }
        if on(flags.show_model_context) {
            field_label(ui, 120.0, flags.context_label);
            numeric_text_edit(ui, &mut m.context, 53.0, "");
            preset_combo(
                ui,
                &mut m.context,
                &CONTEXT_PRESETS,
                &format!("ctx_preset_{}", salt),
                "常用上下文预设值；选择即填入，之后仍可手动改。",
            );
        }
        if on(flags.show_model_output) {
            field_label(ui, 120.0, flags.output_label);
            numeric_text_edit(ui, &mut m.output, 53.0, "");
            preset_combo(
                ui,
                &mut m.output,
                &OUTPUT_PRESETS,
                &format!("out_preset_{}", salt),
                "常用最大输出预设值；选择即填入，之后仍可手动改。",
            );
        }
    });
}

/// 模型字段编辑行之二：模态（input / output）与档位（variants）下拉。
fn model_modalities_row(
    ui: &mut egui::Ui,
    m: &mut ModelRow,
    flags: &ProviderFormFlags,
    variants: &mut VariantsCtx<'_>,
    relaxed: bool,
) {
    let on = |flag: bool| relaxed || flag;
    ui.horizontal_wrapped(|ui| {
        if on(flags.show_model_input) {
            field_label(ui, 120.0, flags.input_label);
            ui.add(egui::TextEdit::singleline(&mut m.modalities_input).desired_width(80.0));
        }
        if flags.show_oc {
            field_label(ui, 120.0, "modalities.output");
            ui.add(egui::TextEdit::singleline(&mut m.modalities_output).desired_width(80.0));
        }
        if on(flags.show_model_variants) {
            field_label(ui, 120.0, variants.label);
        }
        variant_selector(
            ui,
            &mut m.variants,
            variants.names,
            variants.open_key.clone(),
            variants.open_set,
            variants.normalize,
        );
    });
}

/// 「添加 Model」子表单（两份表单共用）：字段行 + 添加按钮。
///
/// 添加后模型进入 `p.models`、子表单清空并收起（`show_key` 是展开状态的键）。
fn new_model_subform(
    ui: &mut egui::Ui,
    p: &mut ProviderRow,
    flags: &ProviderFormFlags,
    variants: &mut VariantsCtx<'_>,
    show_key: &str,
) {
    // `show_key` 按构造就逐表单唯一（编辑表单带 provider key），正好当预设下拉的
    // id 盐用——不必再单传一个参数。
    model_core_fields_row(ui, &mut p.new_model, flags, true, None, show_key);
    model_modalities_row(ui, &mut p.new_model, flags, variants, true);
    ui.horizontal(|ui| {
        ui.add_space(crate::theme::SPACE_7);
        if ui.button("添加").clicked() && !p.new_model.id.trim().is_empty() {
            p.models.push(p.new_model.clone());
            p.new_model = ModelRow::new();
            variants.open_set.remove(show_key);
        }
    });
}

impl App {
    pub(super) fn render_provider_form(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        model_hover_target: &mut Option<String>,
        model_enable_target: &mut Option<(usize, usize)>,
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
        let flags = ProviderFormFlags::new(self);
        let show_model_disabled = flags.show_model_disabled;
        // WorkBuddy 的重复判定是**全局**的（按裸 id 去重，跨厂商也只生效一次），
        // 所以它的「重复」提示要看所有 provider，而不是只看同一张卡片。
        // 必须在 `p = &mut self.providers[idx]` **之前**算好：之后 self.providers
        // 已被可变借用，再读一遍会冲突。
        let global_dup_ids: HashSet<String> = if show_model_disabled {
            let mut seen: HashSet<String> = HashSet::new();
            let mut dup: HashSet<String> = HashSet::new();
            for pv in self.providers.iter() {
                for m in pv.models.iter() {
                    let id = m.id.trim().to_string();
                    if !id.is_empty() && !seen.insert(id.clone()) {
                        dup.insert(id);
                    }
                }
            }
            dup
        } else {
            HashSet::new()
        };
        let p = &mut self.providers[idx];
        let header_ctx = ProviderHeaderCtx {
            page: self.current_page,
            npm_salt: format!("provider_npm_{}", p.key),
            api_salt: format!("provider_api_{}", p.key),
            relaxed: false,
            npm_empty_label: true,
            show_api_keys: self.show_api_keys,
            other_keys: Some(&other_keys),
        };
        provider_header_fields(ui, p, &flags, &header_ctx);

        ui.add_space(crate::theme::SPACE_2);
        ui.add_space(crate::theme::SPACE_2);
        // 本帧用户点下的探测请求（provider key, model id），UI 循环外统一发起。
        let mut probe_request: Option<(String, String)> = None;
        let fetch_api = p.effective_api();
        let fetch_secret = credentials::effective_secret(p);
        let mut fetch_ctx = FetchSectionCtx {
            model_fetch: &mut self.model_fetch,
            model_fetch_open: &mut self.model_fetch_open,
            latency: &self.latency,
            current_page: self.current_page,
            fetch_key: &p.key,
            popup_salt: "model_fetch",
            scroll_salt: egui::Id::new(("model_fetch_scroll", p.key.clone())),
        };
        models_fetch_section(
            ui,
            &mut fetch_ctx,
            &p.base_url,
            &fetch_secret,
            &fetch_api,
            &mut p.models,
        );
        let mut rm: Option<usize> = None;
        let mut model_hover_here: Option<String> = None;
        let mut model_drag_stopped = false;
        // 本帧被勾上的启用开关，用 `(provider 下标, 模型下标)` 记录。同一模型 id
        // 全局只能开一个，互斥在全部卡片渲染完后统一处理（见 `ui_providers_section`）。
        let mut enable_request: Option<(usize, usize)> = None;
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
                        // 右对齐区放在**行尾**：`with_layout` 会吃掉本行剩余宽度，
                        // 放在中间会把后面的探测按钮挤出可视区。
                        // 先加的靠最右，所以「删」在右、「启用」紧贴其左（用户指定的位置）。
                        let enabled_now = !p.models[j].disabled;
                        let mut enable_clicked: Option<bool> = None;
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("删").clicked() {
                                rm = Some(j);
                            }
                            if show_model_disabled {
                                // 滑动开关代替原来的小勾选框：在密集的模型卡片里，
                                // 勾选框太容易被当成装饰，开关的轨道与滑块一眼可辨。
                                // 先加开关、后加文字，右对齐布局下读作「启用 [开关] 删」。
                                //
                                // id 用**行的稳定键**（`model_key` = provider 键 + 模型 id），
                                // 不能用 egui 自动 id：同一行里延迟标签是条件渲染的
                                // （只在测过之后才占位），自动 id 会随它出现而漂移，
                                // 滑动动画就会串到别的行上。
                                let mut enabled = enabled_now;
                                let toggle = crate::ui::toggle_switch(
                                    ui,
                                    egui::Id::new(("model_enable", model_key.as_str())),
                                    &mut enabled,
                                )
                                .on_hover_text(
                                    "WorkBuddy 的选择器**按模型 id 全局去重**：同一个模型名\
                                     无论挂在哪个厂商下，都只会列出一行、只有第一条生效。\n\
                                     所以同一 id 全局只能开一个——打开这个，同名的其他条目\
                                     会自动关闭。\n\
                                     只有开启的会写进 WorkBuddy 的 models.json；关闭**不会\
                                     删除配置**，条目仍保存在同目录的 models.full.json 里，\n\
                                     开回来即可恢复。想换一家厂商的同一个模型，直接开它即可。\n\
                                     ⚠ 不要为了区分同名模型去改 id —— id 同时就是发给上游的\
                                     模型名，改了会直接请求失败。",
                                );
                                ui.label("启用");
                                if toggle.clicked() {
                                    enable_clicked = Some(enabled);
                                }
                            }
                        });
                        if let Some(on) = enable_clicked {
                            p.models[j].disabled = !on;
                            if on {
                                enable_request = Some((idx, j));
                            }
                        }
                    });
                    // 用 `(provider key, 模型下标)` 当下拉 id 盐，而不是模型 id：
                    // id 允许为空或重复（保存不拦），拿它做盐会让两行的下拉共用同一个
                    // Id，展开一个另一个跟着开。
                    model_core_fields_row(
                        ui,
                        &mut p.models[j],
                        &flags,
                        false,
                        Some((&other_ids, &global_dup_ids)),
                        &format!("{}#{}", p.key, j),
                    );
                    let mut variants = VariantsCtx {
                        label: variants_label,
                        names: variant_names,
                        open_set: &mut self.variant_open,
                        open_key: format!("variant_open_{}_{}", p.key, j),
                        normalize: true,
                    };
                    model_modalities_row(ui, &mut p.models[j], &flags, &mut variants, false);
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
        if let Some(picked) = enable_request {
            model_enable_target.get_or_insert(picked);
        }
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
            let mut variants = VariantsCtx {
                label: variants_label,
                names: variant_names,
                open_set: &mut self.variant_open,
                open_key: format!("new_model_variant_{}", p.key),
                normalize: false,
            };
            new_model_subform(ui, p, &flags, &mut variants, &show_new_model_key);
        }
        // key 重命名后同步展开状态与弹窗键
        let new_key = self.providers[idx].key.clone();
        if new_key != prev_key {
            self.sync_provider_rename(&prev_key, &new_key);
        }
    }

    pub(super) fn ui_new_provider_form(&mut self, ui: &mut egui::Ui) {
        let (variants_label, variant_names) = self.dialect_variants();
        let flags = ProviderFormFlags::new(self);
        ui.group(|ui| {
            // 官方预设（可选）：一键填 key / baseUrl / 协议；不套用则完全手填。
            ui.horizontal_wrapped(|ui| {
                let dialect = if flags.show_oc {
                    crate::presets::PresetDialect::Opencode
                } else {
                    crate::presets::PresetDialect::PiLike
                };
                provider_preset_combo(ui, &mut self.new_provider, dialect, "new_provider_preset");
            });
            let header_ctx = ProviderHeaderCtx {
                page: self.current_page,
                npm_salt: "new_provider_npm".to_string(),
                api_salt: "new_provider_api".to_string(),
                relaxed: true,
                npm_empty_label: false,
                show_api_keys: self.show_api_keys,
                other_keys: None,
            };
            provider_header_fields(ui, &mut self.new_provider, &flags, &header_ctx);
            ui.add_space(crate::theme::SPACE_2);
            let mut fetch_ctx = FetchSectionCtx {
                model_fetch: &mut self.model_fetch,
                model_fetch_open: &mut self.model_fetch_open,
                latency: &self.latency,
                current_page: self.current_page,
                fetch_key: NEW_PROVIDER_FETCH_KEY,
                popup_salt: "new_provider_fetch",
                scroll_salt: egui::Id::new("new_provider_fetch_scroll"),
            };
            let fetch_api = self.new_provider.effective_api();
            let fetch_secret = credentials::effective_secret(&self.new_provider);
            models_fetch_section(
                ui,
                &mut fetch_ctx,
                &self.new_provider.base_url,
                &fetch_secret,
                &fetch_api,
                &mut self.new_provider.models,
            );
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
                        model_core_fields_row(
                            ui,
                            &mut self.new_provider.models[j],
                            &flags,
                            true,
                            None,
                            &format!("new_provider#{}", j),
                        );
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
                let mut variants = VariantsCtx {
                    label: variants_label,
                    names: variant_names,
                    open_set: &mut self.variant_open,
                    open_key: "new_provider_new_model_variant".to_string(),
                    normalize: false,
                };
                new_model_subform(
                    ui,
                    &mut self.new_provider,
                    &flags,
                    &mut variants,
                    &show_new_model_key,
                );
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

#[cfg(test)]
mod tests {
    use super::{preset_index, preset_label, preset_value, CONTEXT_PRESETS, OUTPUT_PRESETS};

    /// 写入配置的必须是**纯数字**：上游不认 `128k` 这种写法，而 `numeric_text_edit`
    /// 也会把它标红（非法数字 → 保存时字段被忽略，等于白填）。
    #[test]
    fn every_preset_writes_a_plain_integer() {
        for k in CONTEXT_PRESETS.iter().chain(OUTPUT_PRESETS.iter()) {
            let value = preset_value(*k);
            assert!(
                value.chars().all(|c| c.is_ascii_digit()),
                "{} 的写入值不是纯数字：{}",
                k,
                value
            );
            assert!(
                crate::util::parse_number_text(&value).is_some(),
                "{} 的写入值过不了表单的数字校验：{}",
                k,
                value
            );
        }
    }

    /// 标签与写入值必须成对：`128k` ↔ `128000`。两者分开算，写错了界面会显示一个
    /// 数字、填进去另一个，而用户只会看到自己选的「128k」。
    #[test]
    fn the_label_and_the_written_value_agree() {
        assert_eq!(preset_label(128), "128k");
        assert_eq!(preset_value(128), "128000");
        assert_eq!(preset_label(1024), "1024k");
        // 1024k 取 1000 进制（1024000）而不是 1048576：这些字段是发给上游的声明，
        // 少声明是安全的、多声明会被直接拒绝。
        assert_eq!(preset_value(1024), "1024000");
        // 131 / 262 对应厂商的 131072 / 262144，同样按 1000 进制落数。
        assert_eq!(preset_label(131), "131k");
        assert_eq!(preset_value(131), "131000");
        assert_eq!(preset_value(262), "262000");
    }

    /// 预设表要覆盖用户点名的那几个值，且从小到大排好（下拉里的顺序就是它）。
    #[test]
    fn the_tables_hold_the_requested_values_in_order() {
        assert_eq!(CONTEXT_PRESETS, [128, 200, 262, 300, 400, 500, 1024]);
        assert_eq!(OUTPUT_PRESETS, [32, 64, 131, 262]);
        for table in [&CONTEXT_PRESETS[..], &OUTPUT_PRESETS[..]] {
            assert!(
                table.windows(2).all(|w| w[0] < w[1]),
                "预设必须严格递增：{:?}",
                table
            );
        }
    }

    /// 仓库里的占位值 `262000` 应当正好命中一个预设，否则新模型的下拉会显示
    /// 「选择...」而看不出自己其实已经是 262k。
    #[test]
    fn the_built_in_placeholder_matches_a_preset() {
        let placeholder = crate::model::ModelRow::new().context;
        assert_eq!(placeholder, preset_value(262));
        let output = crate::model::ModelRow::new().output;
        assert_eq!(output, preset_value(131));
    }

    /// 手填的非预设值不能被误认成某个预设——否则下拉会显示一个用户没选过的标签，
    /// 让人以为自己选过。
    #[test]
    fn a_hand_typed_value_is_not_mistaken_for_a_preset() {
        for value in ["", "  ", "1000", "100000", "999999", "abc", "128000000"] {
            assert_eq!(
                preset_index(value, &CONTEXT_PRESETS),
                None,
                "{} 不该命中任何预设",
                value
            );
        }
    }

    /// 反查要能对上：每个预设值本身，以及两侧带空白的写法，都要认出来。
    #[test]
    fn the_current_value_is_recognised_after_a_round_trip() {
        for (i, k) in CONTEXT_PRESETS.iter().enumerate() {
            let value = preset_value(*k);
            assert_eq!(preset_index(&value, &CONTEXT_PRESETS), Some(i));
            assert_eq!(
                preset_index(&format!("  {}  ", value), &CONTEXT_PRESETS),
                Some(i),
                "两侧空白不该影响识别"
            );
        }
    }
}
