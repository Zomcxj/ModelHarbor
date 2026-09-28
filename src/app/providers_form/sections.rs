use super::*;

/// 编辑表单（`render_provider_form`）与新增表单（`ui_new_provider_form`）共用的
/// provider 头部字段：key / 协议下拉 / timeout / compat / DSH 重试 / baseURL / 密钥。
///
/// 两份表单曾逐行抄写并各自漂移——新增表单漏了 DSH 的 timeoutMs 与 retryPolicy，
/// baseURL 的显隐也和编辑表单不一致。统一走这里的同一套门控后，差异只剩三件事，
/// 全由 [`ProviderHeaderCtx`] 表达：占位提示与 key 重复检查（新增表单在「确认」时才查）、
/// npm 空选项的标签、以及 `relaxed` 门控。
pub(super) struct ProviderHeaderCtx<'a> {
    pub(super) page: ConfigFormat,
    pub(super) npm_salt: String,
    pub(super) api_salt: String,
    /// 新增表单 = true：字段是给**全新** provider 填的，不受「已加载文件里有没有
    /// 这个键」的影响（`show_provider_base_url` 这类内容型开关放宽为常显）。
    pub(super) relaxed: bool,
    pub(super) npm_empty_label: bool,
    pub(super) show_api_keys: bool,
    /// key 重复检查用的「其他 provider key」集合；None = 不查（新增表单）。
    pub(super) other_keys: Option<&'a HashSet<String>>,
}

pub(super) fn provider_header_fields(
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
            field_label(ui, 120.0, "timeout");
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
        // Qwen 的凭据框自己画标签（它只剩一个密钥框，变量名自动推导）。
        if !flags.show_qwen {
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
            // 同时写两个会让 Kimi Code **启动失败**。曾经在这里摆一个 `api_key_env`
            // 勾选框切换两种模式，但那个勾选框读起来像一个「是否启用 api_key_env」的
            // 开关、右边的框又没有标签，用户看不出它在干什么；而且它占掉一整格，
            // 密钥框只能缩到 200（其它后端都是 408）。改成与其它后端完全同形的一个框：
            // 文件里已有的 `api_key_env` 原样沿用（`api_key` 留空就照写它，见后端
            // `provider_entry_from_row`），填了密钥就写 `api_key` 并清掉变量名。
            let hint = if ctx.relaxed {
                "sk-xxx"
            } else if !p.api_key_env.trim().is_empty() {
                // 密钥在环境变量里，框里没有值可显示：把变量名当占位符写出来，
                // 免得看起来像「这里漏填了密钥」。
                p.api_key_env.trim()
            } else {
                ""
            };
            secret_text_edit(ui, &mut p.api_key, ctx.show_api_keys, 408.0, hint);
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
pub(super) struct FetchSectionCtx<'a> {
    pub(super) model_fetch: &'a mut HashMap<String, ModelFetchState>,
    pub(super) model_fetch_open: &'a mut HashSet<String>,
    pub(super) latency: &'a HashMap<String, LatencyState>,
    pub(super) current_page: ConfigFormat,
    pub(super) fetch_key: &'a str,
    pub(super) popup_salt: &'a str,
    pub(super) scroll_salt: egui::Id,
}

pub(super) fn models_fetch_section(
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
pub(super) struct VariantsCtx<'a> {
    pub(super) label: &'static str,
    pub(super) names: &'a [&'static str],
    pub(super) open_set: &'a mut HashSet<String>,
    pub(super) open_key: String,
    /// 编辑表单的模型行会归一已存的档位串；「添加 Model」子表单只收集新输入。
    pub(super) normalize: bool,
}
