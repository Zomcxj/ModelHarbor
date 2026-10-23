use super::*;

/// The provider header fields shared by the edit form (`render_provider_form`) and the add form
/// (`ui_new_provider_form`): key / protocol dropdown / timeout / compat / DSH retry / baseURL / key.
///
/// The two forms' differences are expressed by [`ProviderHeaderCtx`]: placeholder hints and key-duplicate
/// checks, the npm empty-option label, and the `relaxed` gate.
pub(super) struct ProviderHeaderCtx<'a> {
    pub(super) page: ConfigFormat,
    pub(super) npm_salt: String,
    pub(super) api_salt: String,
    /// Add form = true: the fields fill a brand-new provider, and content toggles relax to always visible.
    pub(super) relaxed: bool,
    pub(super) npm_empty_label: bool,
    pub(super) show_api_keys: bool,
    /// The set of "other provider keys" for duplicate checks; None = don't check (the add form).
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
            provider_npm_combo(ui, p, flags, &ctx.npm_salt, ctx.npm_empty_label);
        }
        if !flags.show_oc {
            provider_api_combo(ui, p, ctx.page, &ctx.api_salt);
        }
        // timeout / timeoutMs share the first row with npm/api.
        if flags.show_oc && flags.show_provider_timeout {
            field_label(ui, 120.0, flags.timeout_label);
            numeric_text_edit(ui, &mut p.timeout, 70.0, "180000");
        }
        if flags.show_dsh {
            field_label(ui, 120.0, "timeoutMs");
            numeric_text_edit(ui, &mut p.dsh_timeout_ms, 70.0, "180000");
        }
        // QwenCode's timeout lands in the entry's generationConfig.
        if flags.show_qwen {
            field_label(ui, 120.0, "timeout");
            numeric_text_edit(ui, &mut p.timeout, 70.0, "180000");
        }
        // pi / omp's compat displays on the api's row (right after api).
        if !flags.show_oc
            && !flags.show_dsh
            && !flags.show_zcode
            && !flags.show_wb
            && !flags.show_qwen
            && !flags.show_kimi
        {
            field_label(ui, 120.0, "compat");
            ui.checkbox(&mut p.compat, "supportsDeveloperRole");
            // pi / omp map to each other: unchecked by default when loading opencode/dsh.
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
        // WorkBuddy's protocol is expressed by the URL suffix (the file has no protocol field), so no manual
        // toggle: the protocol chosen above decides what suffix gets appended at save.
        // Qwen's credential box draws its own label (only one key box remains).
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
            // QwenCode's key lives in the top-level `env[<envKey>]`: the entry carries the variable name, `env` the value.
            // The variable name gets no box of its own; left empty it derives from the provider key (see the backend's `env_key_name`).
            field_label(ui, 120.0, "API Key");
            let hint = if ctx.relaxed { "实际密钥" } else { "" };
            secret_text_edit(ui, &mut p.api_key, ctx.show_api_keys, 408.0, hint);
        } else if flags.show_kimi {
            // KimiCode's `api_key` (inline key) and `api_key_env` (environment variable name) are mutually exclusive —
            // writing both makes Kimi Code fail to start. A pre-existing `api_key_env` in the file is kept as-is
            // (`api_key` stays empty and writes it — see the backend's `provider_entry_from_row`); filling a key
            // writes `api_key` and clears the variable name.
            let hint = if ctx.relaxed {
                "sk-xxx"
            } else if !p.api_key_env.trim().is_empty() {
                // The key lives in an environment variable: write the variable name as the placeholder.
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

/// The "discover models" block (shared by both forms): the button row + the background probe
/// (connectivity / auth / list / latency in one, with 404/405 auto-fallback across candidates) + the result
/// check panel.
///
/// Fields pass individually rather than taking `&mut App`: the caller still holds `self.providers[idx]`.
/// Returns messages for the bottom status bar (cache hit / models added / cancelled).
pub(super) struct FetchSectionCtx<'a> {
    pub(super) latency: &'a HashMap<String, LatencyState>,
    pub(super) current_page: ConfigFormat,
    pub(super) fetch_key: &'a str,
    /// The "discover models" state (key → state; existing = the result panel is open).
    /// The "discover models" state (key → state; existing = the result panel is open).
    pub(super) discovery: &'a mut HashMap<String, DiscoveryState>,
    /// The discovery-result cache (24 hours, keyed by base_url; holds no credentials).
    pub(super) discovery_cache: &'a mut DiscoveryCache,
}

pub(super) fn models_fetch_section(
    ui: &mut egui::Ui,
    ctx: &mut FetchSectionCtx<'_>,
    base_url: &str,
    secret: &str,
    models: &mut Vec<ModelRow>,
) -> Option<String> {
    let mut probe_request: Option<(String, String)> = None;
    let mut cancel_probe = false;
    let mut status: Option<String> = None;
    ui.horizontal(|ui| {
        ui.strong("Models");
        // Model discovery: a background thread runs core::discovery's probe (404/405 auto-fallback across candidates),
        // and results are checked into the panel below. The old "fetch list" entry was replaced by it (a superset:
        // connectivity + auth + latency + fill-only).
        let probing = ctx
            .discovery
            .get(ctx.fetch_key)
            .is_some_and(|state| state.in_flight());
        if probing {
            ui.add_enabled(false, egui::Button::new("获取中…"));
            if ui.button("取消").clicked() {
                cancel_probe = true;
            }
            // Discovery results return over a channel: even the idle state (no animated widgets) needs frames to poll.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        } else if ui
            .button("获取模型")
            .on_hover_text(probe_candidates_label(base_url))
            .clicked()
        {
            probe_request = Some((base_url.to_string(), secret.to_string()));
        }
        // No batch entry for model latency: each model row carries its own "test" button, one at a time
        // (same model 60s, same vendor 10s throttle).
        if ctx
            .latency
            .get(ctx.fetch_key)
            .is_some_and(|s| s.model_rx.is_some())
        {
            ui.label(egui::RichText::new("延迟测试中…").small());
        }
    });
    // Start / cancel: fill-only merge and the like live in fetch::discovery.
    let existing_ids: Vec<String> = models.iter().map(|m| m.id.trim().to_string()).collect();
    if let Some((base, secret)) = probe_request {
        if let Some(msg) = start_discovery(
            ctx.discovery,
            ctx.discovery_cache,
            ctx.fetch_key,
            &base,
            &secret,
            &existing_ids,
        ) {
            status = Some(msg);
        }
    }
    if cancel_probe {
        if let Some(msg) = cancel_discovery(ctx.discovery, ctx.fetch_key) {
            status = Some(msg);
        }
    }
    // The discovery result panel: open whenever the state exists (probing / failed / success list).
    if let Some(state) = ctx.discovery.get_mut(ctx.fetch_key) {
        let action = discovery_panel(
            ui,
            state,
            models,
            ctx.current_page,
            egui::Id::new(("discovery_scroll", ctx.fetch_key)),
            &existing_ids,
        );
        if action.close {
            ctx.discovery.remove(ctx.fetch_key);
        }
        if action.status.is_some() {
            status = action.status;
        }
    }
    status
}

/// The complete parameter set for the tiers (variants) dropdown: field label, vocabulary, expand state, and expand key.
pub(super) struct VariantsCtx<'a> {
    pub(super) label: &'static str,
    pub(super) names: &'a [&'static str],
    pub(super) open_set: &'a mut HashSet<String>,
    pub(super) open_key: String,
    /// The edit form's model rows normalize stored tier strings; the "add model" sub-form only collects new input.
    pub(super) normalize: bool,
}
