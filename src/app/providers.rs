//! Providers 区块：厂商卡片列表、拖拽落点聚合、表单字段可见性标志与思考档位方言。
use super::balance;
use super::App;
use crate::app::bars::{short_err, sticky_begin, sticky_end};
use crate::app::fetch::{latency_color, matrix_label};
use crate::credentials;
use crate::format::ConfigFormat;
use crate::ui::{card_frame, card_list, move_item, DragHandle};
use eframe::egui;

/// provider / model 表单的字段可见性与方言标签（opencode / pi / omp / DSH 共用）。
#[derive(Clone, Copy)]
pub(super) struct ProviderFormFlags {
    pub(super) show_oc: bool,
    pub(super) show_omp: bool,
    pub(super) show_dsh: bool,
    pub(super) show_provider_base_url: bool,
    pub(super) show_provider_timeout: bool,
    pub(super) show_model_name: bool,
    pub(super) show_model_context: bool,
    pub(super) show_model_output: bool,
    pub(super) show_model_input: bool,
    pub(super) show_model_variants: bool,
    pub(super) show_model_reasoning: bool,
    pub(super) show_model_tool_call: bool,
    pub(super) show_model_store: bool,
    pub(super) base_label: &'static str,
    pub(super) api_key_label: &'static str,
    pub(super) context_label: &'static str,
    pub(super) output_label: &'static str,
    pub(super) input_label: &'static str,
}

impl ProviderFormFlags {
    pub(super) fn new(app: &App) -> Self {
        let show_oc = app.current_page == ConfigFormat::Opencode;
        let show_dsh = app.current_page == ConfigFormat::DeepSeekHarness;
        Self {
            show_oc,
            show_omp: app.current_page == ConfigFormat::OhMyPi,
            show_dsh,
            show_provider_base_url: app.page_has_provider_field("base_url"),
            // opencode 的 options.timeout 始终显示（文件未写该字段时默认 180000ms）
            show_provider_timeout: show_oc || app.page_has_provider_field("timeout"),
            show_model_name: app.page_has_model_field("name"),
            show_model_context: app.page_has_model_field("context"),
            show_model_output: app.page_has_model_field("output"),
            show_model_input: app.page_has_model_field("input"),
            show_model_variants: app.page_has_model_field("variants"),
            show_model_reasoning: app.page_has_model_field("reasoning"),
            show_model_tool_call: app.page_has_model_field("tool_call"),
            show_model_store: app.page_has_model_field("store"),
            base_label: if show_oc {
                "options.baseURL"
            } else if show_dsh {
                "baseURL"
            } else {
                "baseUrl"
            },
            api_key_label: if show_oc {
                "options.apiKey"
            } else if show_dsh {
                "apiKeyEnv"
            } else {
                "apiKey"
            },
            context_label: if show_oc {
                "limit.context"
            } else {
                "contextWindow"
            },
            output_label: if show_oc { "limit.output" } else { "maxTokens" },
            input_label: if show_oc { "modalities.input" } else { "input" },
        }
    }
}

impl App {
    /// 当前页面的思考档位标签。
    pub(super) fn dialect_variants(&self) -> (&'static str, &'static [&'static str]) {
        match self.current_page {
            ConfigFormat::Opencode => (
                "variants",
                &["none", "low", "medium", "high", "xhigh", "max", "ultra"],
            ),
            ConfigFormat::Pi => (
                "thinkingLevelMap",
                &[
                    "off", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
                ],
            ),
            ConfigFormat::OhMyPi => (
                "thinking.efforts",
                &["minimal", "low", "medium", "high", "xhigh", "max", "ultra"],
            ),
            ConfigFormat::DeepSeekHarness => (
                "reasoningEfforts",
                &["minimal", "low", "medium", "high", "xhigh", "max", "ultra"],
            ),
        }
    }

    /// Providers 区块：标题行吸顶（滚动时始终显示在顶部），内容紧跟其下。
    pub(super) fn ui_providers_section(&mut self, ui: &mut egui::Ui) {
        let anchor = sticky_begin(ui, 30.0);
        let matched: Vec<usize> = (0..self.providers.len()).collect();

        if self.providers.is_empty() && !self.show_new_provider {
            self.show_new_provider = true;
        }

        let mut to_remove: Option<usize> = None;
        let mut to_copy: Option<usize> = None;
        // 拖拽落点必须在**所有卡片渲染完之后**统一聚合再写入 self：
        // 卡片各自赋值会被后渲染的卡片用 None 覆盖（模型卡片曾因此丢失绿色落点边框）。
        let mut hover_target: Option<String> = None;
        let mut model_hover_target: Option<String> = None;
        card_list(ui, &matched, 0.0, |ui, idx| {
            self.render_provider_card(
                ui,
                idx,
                &mut to_remove,
                &mut to_copy,
                &mut hover_target,
                &mut model_hover_target,
            );
        });
        if let Some(idx) = to_remove {
            self.providers.remove(idx);
            self.status = "已删除 provider".into();
        }
        if let Some(idx) = to_copy {
            let mut p = self.providers[idx].clone();
            p.key = format!("{}_copy", p.key);
            self.providers.push(p);
            self.status = "已复制 provider".into();
        }
        if self.provider_drag_src.is_some() {
            self.provider_drag_target = hover_target;
        } else {
            self.provider_drag_target = None;
        }
        // 模型拖拽落点：与 provider 同样在外层聚合，保证任意展开顺序下被拖到的
        // 模型卡片都能拿到绿色边框（见 render_provider_form 里的说明）。
        if self.model_drag_src.is_some() {
            self.model_drag_target = model_hover_target;
        } else {
            self.model_drag_target = None;
        }

        ui.add_space(10.0);
        if ui.button("新增 Provider").clicked() {
            self.show_new_provider = !self.show_new_provider;
        }
        if self.show_new_provider {
            self.ui_new_provider_form(ui);
        }
        sticky_end(ui, anchor, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Providers");
                if !self.providers.is_empty() {
                    let all_open = self
                        .providers
                        .iter()
                        .all(|p| !self.provider_collapsed(&p.key));
                    if ui
                        .button(if all_open {
                            "收起全部卡片"
                        } else {
                            "展开全部卡片"
                        })
                        .clicked()
                    {
                        // all_open 为真 = 现在全部展开 → 按钮是「收起全部」
                        self.set_all_providers_collapsed(all_open);
                    }
                }
                // 连通性测试：放在标题行右侧，收起全部卡片时也始终可见。
                if ui
                    .button("连通性测试")
                    .on_hover_text("并发测试当前页面全部厂商的接口连通性")
                    .clicked()
                {
                    let targets: Vec<(String, String, String, String)> = self
                        .providers
                        .iter()
                        .map(|p| {
                            let api = p.effective_api();
                            (
                                p.key.clone(),
                                p.base_url.clone(),
                                credentials::effective_secret(p),
                                api,
                            )
                        })
                        .collect();
                    let count = targets.len();
                    for (key, base, secret, api) in targets {
                        Self::start_provider_latency(&mut self.latency, &key, &base, &secret, &api);
                    }
                    self.status = format!("已开始连通性测试（{} 个厂商）", count);
                }
                // 用量查询：一次查完当前页面全部厂商（只读管理接口，直连不走代理）。
                // 查不到的站点不在卡片上显示，只在状态栏汇总（避免一堆红字噪音）。
                if ui
                    .button("查询用量")
                    .on_hover_text(
                        "查询全部厂商的「已用 / 余额」，显示在各卡片的厂商名右侧\n\
                         只读管理接口：优先 /api/usage/token/ + /api/log/token，\
                         再回退 /dashboard/billing/*；直连不走代理\n\
                         同一 provider 两次查询至少间隔 5 秒\n\
                         未开放接口或没有有效数据时不会在卡片上显示\n\
                         公益站占位额度不显示余额；余额带「约」字时仅供参考",
                    )
                    .clicked()
                {
                    let targets: Vec<balance::Query> = self
                        .providers
                        .iter()
                        .map(|p| balance::Query {
                            key: p.key.clone(),
                            base_url: p.base_url.clone(),
                            secret: credentials::effective_secret(p),
                            // 站点级面板令牌：没设置就是空串（只查 sk- 那两个接口）。
                            pat: self.station_pat(&p.base_url),
                        })
                        .collect();
                    let now = ui.input(|i| i.time);
                    let mut started = 0usize;
                    for query in targets {
                        if Self::start_balance_query(&mut self.balance, query, now).is_none() {
                            started += 1;
                        }
                    }
                    self.balance_batch = started > 0;
                    self.status = if started == 0 {
                        "所有厂商都还在冷却中，请几秒后再试".to_string()
                    } else {
                        format!("已开始查询 {} 个厂商的用量…", started)
                    };
                }
                // 令牌：站点面板访问令牌（PAT）管理；填了才能查账号级真实余额。
                if ui
                    .button("令牌")
                    .on_hover_text(
                        "管理站点的面板访问令牌（PAT）\n\
                         在站点面板「个人设置 → 安全设置 → 系统访问令牌」生成\n\
                         令牌是站点级的：同一站点的多个 provider 共用一份\n\
                         只用于只读查询账号余额（/api/user/self），不参与配置保存\n\
                         存在 %USERPROFILE%\\.modelharbor\\tokens.json（含凭证，勿提交、勿共享）",
                    )
                    .clicked()
                {
                    self.show_tokens = !self.show_tokens;
                    if self.show_tokens {
                        // 重新打开时按已保存的值重填草稿（避免残留上次未保存的改动）。
                        self.token_draft.clear();
                    }
                }
                // 网络守卫：检测到系统代理 / VPN 时默认禁用模型延迟测试
                //（中转站的「多 IP 检测 / 测活封号」可能因此触发）。
                // 只有在没放行时才报“已禁用”：放行后显示中性提示，避免自相矛盾。
                let allow_probe = self.allow_model_test_with_proxy;
                if let Some(reason) = self.net_guard.clone() {
                    let semantics = crate::theme::semantics(ui);
                    ui.label(
                        egui::RichText::new(if allow_probe {
                            format!("已放行模型测试：{reason}")
                        } else {
                            format!("{}：{reason}", crate::netguard::BLOCK_PREFIX)
                        })
                        .small()
                        .color(if allow_probe { semantics.warn } else { semantics.err }),
                    )
                    .on_hover_text(
                        "中转站常见多 IP 检测 / 测活风控，经代理做推理探测可能被封号；\n\
                         默认已禁用「模型延迟测试」。若你的探测本来就直连\n\
                         （本工具不走系统代理，仅 VPN / TUN 改变出口 IP），可勾选右侧开关放行。\n\
                         （厂商「连通性测试」与「查询用量」不受影响：它们不做推理。）",
                    );
                }
                // 放行开关：写进 settings.json，重启后仍生效。
                // 不用 `self.allow_model_test_with_proxy` 直接取地址：借用冲突且不便回写。
                let mut allow_checked = allow_probe;
                if ui
                    .checkbox(&mut allow_checked, "代理下测试模型")
                    .on_hover_text(
                        "勾选后即使检测到系统代理 / VPN 也允许「模型延迟测试」。\n\
                         默认关闭：中转站的多 IP 检测 / 测活风控可能因此封号。\n\
                         本工具的探测请求始终直连、不走系统代理，\n\
                         所以仅开看系统代理（如 Clash）时勾选是安全的；\n\
                         真正需要警惕的是 VPN / TUN 已改变出口 IP 的情况。\n\
                         该选择写入 settings.json（allow_model_test_with_proxy）。",
                    )
                    .changed()
                {
                    self.allow_model_test_with_proxy = allow_checked;
                }
                // 全局 API Key 显示/隐藏：一键切换全部密钥的明文/掩码。
                // 文案带「密钥」二字，与区块「隐藏/展开」、卡片 ▼/▶ 折叠按钮明确区分。
                if ui
                    .button(if self.show_api_keys {
                        "隐藏密钥"
                    } else {
                        "显示密钥"
                    })
                    .on_hover_text(if self.show_api_keys {
                        "点击掩码全部 API Key（默认状态）"
                    } else {
                        "点击显示全部 API Key 明文（注意防窥）"
                    })
                    .clicked()
                {
                    self.show_api_keys = !self.show_api_keys;
                }
                // 配置预览：右侧面板实时展示当前页面的序列化内容，可编辑并应用回组件。
                if ui
                    .button(if self.show_preview {
                        "关闭预览"
                    } else {
                        "预览"
                    })
                    .on_hover_text(
                        "在右侧打开当前页面「待保存文档」预览；可直接编辑，改动实时应用并自动保存",
                    )
                    .clicked()
                {
                    self.show_preview = !self.show_preview;
                    if self.show_preview {
                        // 打开时以组件状态重建待保存文档
                        self.reset_preview_draft();
                    }
                }
            });
        });
    }

    pub(super) fn render_provider_card(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        to_remove: &mut Option<usize>,
        to_copy: &mut Option<usize>,
        hover_target: &mut Option<String>,
        model_hover_target: &mut Option<String>,
    ) {
        let key = self.providers[idx].key.clone();
        let open = !self.provider_collapsed(&key);
        let highlight = if self.provider_drag_target.as_deref() == Some(key.as_str()) {
            2
        } else if self.provider_drag_src.as_deref() == Some(key.as_str()) {
            1
        } else {
            0
        };
        let resp = card_frame(ui, open, highlight, |ui| {
            ui.horizontal(|ui| {
                let h = ui.add(DragHandle);
                if h.drag_started() {
                    self.provider_drag_src = Some(key.clone());
                    self.provider_drag_target = None;
                }
                if h.drag_stopped() {
                    if self.provider_drag_src == Some(key.clone()) {
                        if let Some(dst) = self.provider_drag_target.clone() {
                            let s = self.providers.iter().position(|p| p.key == key);
                            let d = self.providers.iter().position(|p| p.key == dst);
                            if let (Some(s), Some(d)) = (s, d) {
                                move_item(&mut self.providers, s, d);
                            }
                        }
                    }
                    self.provider_drag_src = None;
                    self.provider_drag_target = None;
                }
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(if open { "▼" } else { "▶" }).size(14.0),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    self.set_provider_collapsed(&key, open);
                }
                ui.strong(&self.providers[idx].key);
                // baseUrl 体检提示：`//v1` 这类笔误在卡片上直接可见（只提示，不自动改写）。
                let suspicions = crate::util::url_suspicions(&self.providers[idx].base_url);
                if !suspicions.is_empty() {
                    ui.label(
                        egui::RichText::new(format!("⚠ baseUrl: {}", suspicions.join("、")))
                            .small()
                            .color(crate::theme::semantics(ui).err),
                    )
                    .on_hover_text(format!(
                        "当前 baseUrl\n{}\n\n请核对协议头、重复斜杠与末尾斜杠；工具只提示，不会自动改写配置。",
                        self.providers[idx].base_url
                    ));
                }
                // 连通性测试结果：显示在厂商名字右侧，卡片收起时也可见。
                if let Some(state) = self.latency.get(&key) {
                    if state.provider_rx.is_some() {
                        matrix_label(ui, &key);
                    } else if let Some(res) = &state.provider {
                        match res {
                            Ok(ms) => {
                                ui.label(
                                    egui::RichText::new(format!("{}ms", ms))
                                        .color(latency_color(*ms, crate::theme::semantics(ui))),
                                );
                            }
                            Err(err) => {
                                ui.label(egui::RichText::new(short_err(err)).color(crate::theme::semantics(ui).err))
                                    .on_hover_text(err);
                            }
                        }
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("删除").clicked() {
                        *to_remove = Some(idx);
                    }
                    if ui.button("复制").clicked() {
                        *to_copy = Some(idx);
                    }
                    // 用量查询结果紧挨「复制」左侧（右对齐布局里越晚添加越靠左）。
                    // 查不到的站点不显示（未开放接口 / WAF / 空数据），也不显示占位余额。
                    if let Some(state) = self.balance.get(&key) {
                        let display_result =
                            state.display_result(super::balance::local_midnight_unix());
                        match (&state.rx, display_result.as_ref()) {
                            (Some(_), _) => {
                                ui.add(egui::Spinner::new().size(14.0));
                                // 字号与连通性结果（`123ms`）一致：默认正文号，不用 .small()。
                                ui.label(egui::RichText::new("查询用量…").weak());
                            }
                            (None, Some(Ok(info))) if info.is_displayable() => {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(info.inline_full())
                                            .color(crate::theme::semantics(ui).info),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(info.detail());
                            }
                            _ => {}
                        }
                    }
                });
            });
            if open {
                self.render_provider_form(ui, idx, model_hover_target);
            }
        });
        if let Some(src_key) = &self.provider_drag_src {
            if src_key != &key && resp.contains_pointer() && hover_target.is_none() {
                *hover_target = Some(key.clone());
            }
        }
    }

    /// 令牌面板内容：列出当前页面所有站点（按 origin 归并），填 / 改 / 删面板访问令牌。
    ///
    /// 令牌是**站点级**的：同一站点的多个 provider 共用一份，所以这里按站点一行，
    /// 并标注哪些 provider 在用。只用于只读查询，不写进任何 agent 配置文件。
    /// 由 [`super::App::ui_tokens_window`] 装进悬浮窗渲染，不再占正文布局。
    pub(super) fn ui_tokens_panel(&mut self, ui: &mut egui::Ui) {
        // 站点 → 使用它的 provider key（按首次出现顺序，保持与卡片列表一致）。
        let mut stations: Vec<(String, Vec<String>)> = Vec::new();
        for provider in &self.providers {
            let origin = crate::tokens::station_key(&provider.base_url);
            if origin.is_empty() {
                continue;
            }
            match stations.iter_mut().find(|(name, _)| name == &origin) {
                Some((_, keys)) => keys.push(provider.key.clone()),
                None => stations.push((origin, vec![provider.key.clone()])),
            }
        }

        // 标题由悬浮窗提供，此处不再重复。
        ui.label(
            egui::RichText::new(
                "在站点面板「个人设置 → 安全设置 → 系统访问令牌」生成；\
                 令牌是站点级的，同一站点的多个 provider 共用一份。\
                 只用于只读查询账号余额。",
            )
            .small()
            .color(ui.visuals().weak_text_color()),
        );
        if stations.is_empty() {
            ui.label(
                egui::RichText::new("当前页面没有带 baseUrl 的 provider")
                    .color(ui.visuals().weak_text_color()),
            );
            return;
        }

        // 打开面板时按已保存值补齐草稿（掩码显示；缺省为空 = 未设置）。
        for (origin, _) in &stations {
            if !self.token_draft.contains_key(origin) {
                let existing = self.tokens.get(origin).to_string();
                self.token_draft.insert(origin.clone(), existing);
            }
        }

        // 按钮动作先收集、循环后统一写入 self（避免渲染中的借用冲突）。
        let mut save: Option<String> = None;
        let mut remove: Option<String> = None;
        let mut toggle_reveal: Option<String> = None;
        for (origin, keys) in &stations {
            let configured = self.tokens.has(origin);
            let revealed = self.show_api_keys || self.token_reveal.contains(origin);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(origin).monospace());
                let semantics = crate::theme::semantics(ui);
                ui.label(
                    egui::RichText::new(if configured { "已设置" } else { "未设置" })
                        .small()
                        .color(if configured {
                            semantics.ok
                        } else {
                            semantics.warn
                        }),
                );
                if keys.len() > 1 {
                    ui.label(
                        egui::RichText::new(format!("{} 个 provider 共用", keys.len()))
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    )
                    .on_hover_text(keys.join("、"));
                }
            });
            ui.horizontal(|ui| {
                if let Some(draft) = self.token_draft.get_mut(origin) {
                    ui.add(
                        egui::TextEdit::singleline(draft)
                            .password(!revealed)
                            .desired_width(280.0)
                            .hint_text(if configured {
                                "留空不改变；要清除请点「删除」"
                            } else {
                                "粘贴面板访问令牌"
                            }),
                    );
                }
                if ui.button(if revealed { "隐藏" } else { "显示" }).clicked() {
                    toggle_reveal = Some(origin.clone());
                }
                if ui.button("保存").clicked() {
                    save = Some(origin.clone());
                }
                if ui
                    .add_enabled(configured, egui::Button::new("删除"))
                    .clicked()
                {
                    remove = Some(origin.clone());
                }
            });
            ui.add_space(2.0);
        }

        // 单条显隐：与全局「显示密钥」是「或」的关系，互不干扰。
        if let Some(origin) = toggle_reveal {
            if !self.token_reveal.remove(&origin) {
                self.token_reveal.insert(origin);
            }
        }
        if let Some(origin) = save {
            let token = self.token_draft.get(&origin).cloned().unwrap_or_default();
            if token.trim().is_empty() {
                self.status = format!("{} 的令牌为空：要清除请点「删除」", origin);
            } else {
                self.tokens.set(&origin, &token);
                match self.tokens.save() {
                    Ok(()) => {
                        self.status = format!("已保存 {} 的面板令牌", origin);
                        self.forget_station_balance(&origin);
                    }
                    // 错误只带路径，不带令牌内容（见 tokens::save_to）。
                    Err(err) => self.status = format!("令牌保存失败：{err}"),
                }
            }
        }
        if let Some(origin) = remove {
            self.tokens.remove(&origin);
            self.token_draft.remove(&origin);
            match self.tokens.save() {
                Ok(()) => {
                    self.status = format!("已删除 {} 的面板令牌", origin);
                    self.forget_station_balance(&origin);
                }
                Err(err) => self.status = format!("令牌删除失败：{err}"),
            }
        }
    }

    /// 令牌变更后丢弃该站点各 provider 的用量缓存：下次查询重新取账号数据，
    /// 避免换了令牌还继续显示旧账号余额。
    fn forget_station_balance(&mut self, origin: &str) {
        let affected: Vec<String> = self
            .providers
            .iter()
            .filter(|provider| crate::tokens::station_key(&provider.base_url) == origin)
            .map(|provider| provider.key.clone())
            .collect();
        for key in affected {
            self.balance.remove(&key);
        }
    }

    /// provider key 重命名后同步 UI 状态（卡片折叠集合 + 弹窗键）。
    pub(super) fn sync_provider_rename(&mut self, old: &str, new: &str) {
        if old == new || new.is_empty() {
            return;
        }
        self.rename_collapsed_card("providers", old, new);
        // 下标型弹窗键直接关闭（避免前缀歧义），需要时重新打开即可
        let variant_prefix = format!("variant_open_{}_", old);
        let show_key = format!("show_new_model_{}", old);
        let new_variant_key = format!("new_model_variant_{}", old);
        self.variant_open
            .retain(|k| !k.starts_with(&variant_prefix) && k != &show_key && k != &new_variant_key);
    }
}
