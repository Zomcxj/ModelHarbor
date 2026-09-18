//! Providers 区块：厂商卡片列表、拖拽落点聚合、表单字段可见性标志与思考档位方言。
use super::balance;
use super::App;
use crate::app::bars::{short_err, sticky_begin, sticky_end};
use crate::app::fetch::{latency_color, matrix_label};
use crate::credentials;
use crate::format::ConfigFormat;
use crate::ui::{card_frame, card_list, move_item, DragHandle};
use eframe::egui;

/// 卡片渲染时向外收集的动作与落点。
///
/// 打包成一个结构而不是一串 `&mut Option<_>`：出参一多，函数签名就超出
/// clippy 的参数上限，而且调用处一长串 `&mut` 也读不出哪个对应哪个。
#[derive(Default)]
pub(super) struct CardActions {
    /// 要删除的 provider 下标。
    pub(super) remove: Option<usize>,
    /// 要复制的 provider 下标。
    pub(super) copy: Option<usize>,
    /// provider 卡片的拖拽落点。
    pub(super) hover: Option<String>,
    /// model 卡片的拖拽落点。
    pub(super) model_hover: Option<String>,
}

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

    /// 首次使用引导条：三步上手 + 一句查余额前提，可关闭（状态存 settings.json）。
    ///
    /// 只在用户没关过时出现；关掉后不再打扰。
    fn ui_first_run_guide(&mut self, ui: &mut egui::Ui) {
        if self.guide_dismissed {
            return;
        }
        let semantics = crate::theme::semantics(ui);
        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::same(crate::theme::SPACE_3 as i8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("开始使用")
                            .strong()
                            .color(semantics.info),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("知道了").clicked() {
                            self.guide_dismissed = true;
                        }
                    });
                });
                ui.label(
                    egui::RichText::new(
                        "1. 顶栏「配置文件」填路径或点「浏览」加载　→　\
                         2. 展开卡片填 API Key　→　3. 点「保存」写入",
                    )
                    .small(),
                );
                ui.label(
                    egui::RichText::new(
                        "想查账号余额 / 已用 / 今日用量：先在页头「令牌」里填该站点的面板访问令牌。",
                    )
                    .small()
                    .color(ui.visuals().weak_text_color()),
                );
            });
        ui.add_space(crate::theme::SPACE_2);
    }

    /// Providers 区块：标题行吸顶（滚动时始终显示在顶部），内容紧跟其下。
    pub(super) fn ui_providers_section(&mut self, ui: &mut egui::Ui) {
        // 吸顶区是**固定高度矩形**（sticky_end 用 max_rect 建子 ui）：
        // 里面所有内容都必须装在这一个高度里，多出来的行不会撑开矩形，
        // 而是直接画到下面的卡片区上。所以「代理支持」是并回标题行、
        // 而不是另起一行。
        let anchor = sticky_begin(ui, 30.0);
        let matched: Vec<usize> = (0..self.providers.len()).collect();

        if self.providers.is_empty() && !self.show_new_provider {
            self.show_new_provider = true;
        }

        self.ui_first_run_guide(ui);

        // 拖拽落点必须在**所有卡片渲染完之后**统一聚合再写入 self：
        // 卡片各自赋值会被后渲染的卡片用 None 覆盖（模型卡片曾因此丢失绿色落点边框）。
        let mut actions = CardActions::default();
        card_list(ui, &matched, 0.0, |ui, idx| {
            self.render_provider_card(ui, idx, &mut actions);
        });
        if let Some(idx) = actions.remove {
            self.providers.remove(idx);
            self.status = "已删除 provider".into();
        }
        if let Some(idx) = actions.copy {
            let mut p = self.providers[idx].clone();
            p.key = format!("{}_copy", p.key);
            self.providers.push(p);
            self.status = "已复制 provider".into();
        }
        if self.provider_drag_src.is_some() {
            self.provider_drag_target = actions.hover;
        } else {
            self.provider_drag_target = None;
        }
        // 模型拖拽落点：与 provider 同样在外层聚合，保证任意展开顺序下被拖到的
        // 模型卡片都能拿到绿色边框（见 render_provider_form 里的说明）。
        if self.model_drag_src.is_some() {
            self.model_drag_target = actions.model_hover;
        } else {
            self.model_drag_target = None;
        }

        ui.add_space(crate::theme::SPACE_3);
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
                // 查询用户数据：一次查完当前页面全部厂商（只读管理接口，直连不走代理）。
                // 一份结果里能有什么就显示什么：余额 / 已用 / 今日 / 近 7 天 / 签到状态。
                // 查不到的站点不在卡片上显示，只在状态栏汇总（避免一堆红字噪音）。
                if ui
                    .button("查询用户数据")
                    .on_hover_text(
                        "查询全部厂商的账号数据（已用 / 余额 / 签到状态），显示在卡片上\n\
                         只读接口、直连不走代理；同一站点两次查询至少间隔 5 秒",
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
                            // 旧版 new-api 要的用户 ID：没填就是空串（不发该头）。
                            user_id: self.station_user_id(&p.base_url),
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
                        format!("已开始查询 {} 个厂商的用户数据…", started)
                    };
                }
                // 「令牌」「预览」「显示密钥」都搬到了页头「保存」那一行右端
                // （见 `bars::ui_page_header`）：它们都跟「保存 / 看」这个动作有关，
                // 放在 Providers 标题行只有切到该页才看得到。
                //
                // 「代理支持」留在同一行、靠右：它是个安全开关，靠右能与这排视图
                // 按钮分开；右对齐布局里越晚添加越靠左，所以先放开关、再放文字。
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let allow_probe = self.allow_model_test_with_proxy;
                    let mut allow_checked = allow_probe;
                    if ui
                        .checkbox(&mut allow_checked, "代理支持")
                        .on_hover_text(
                            "勾选后即使检测到代理 / VPN 也允许「模型延迟测试」。\n\
                             默认关闭：中转站的多 IP 检测 / 测活风控可能因此封号。\n\
                             本工具探测始终直连、不走系统代理；要警惕的是 VPN / TUN
                             已改变出口 IP。选择写入 settings.json。",
                        )
                        .changed()
                    {
                        self.allow_model_test_with_proxy = allow_checked;
                    }
                    // 只有在没放行时才报「已禁用」：放行后显示中性提示，避免自相矛盾。
                    if let Some(reason) = self.net_guard.clone() {
                        let semantics = crate::theme::semantics(ui);
                        ui.label(
                            egui::RichText::new(if allow_probe {
                                format!("已放行模型测试：{reason}")
                            } else {
                                format!("{}：{reason}", crate::netguard::BLOCK_PREFIX)
                            })
                            .small()
                            .color(if allow_probe {
                                semantics.warn
                            } else {
                                semantics.err
                            }),
                        )
                        .on_hover_text(
                            "中转站普遍有多 IP 检测 / 测活风控，经代理做推理探测可能被封号。\n\
                             「连通性测试」不做推理，不受影响。",
                        );
                    }
                });
            });
        });
    }

    pub(super) fn render_provider_card(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        actions: &mut CardActions,
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
                        "当前 baseUrl
{}
只提示，不自动改写配置",
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
                                ui.label(
                                    egui::RichText::new(short_err(err))
                                        .color(crate::theme::semantics(ui).err),
                                )
                                .on_hover_text(err);
                            }
                        }
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("删除").clicked() {
                        actions.remove = Some(idx);
                    }
                    if ui.button("复制").clicked() {
                        actions.copy = Some(idx);
                    }
                    // 查询结果紧挨「复制」左侧：余额 / 已用 / 今日 / 签到状态都在这
                    // 一行里（有就输出，没有就不输出）。
                    // 查不到的站点不显示（未开放接口 / WAF / 空数据），也不显示占位余额。
                    if let Some(state) = self.balance.get(&key) {
                        let display_result =
                            state.display_result(super::balance::local_midnight_unix());
                        match (&state.rx, display_result.as_ref()) {
                            (Some(_), _) => {
                                ui.add(egui::Spinner::new().size(14.0));
                                // 字号与连通性结果（`123ms`）一致：默认正文号，不用 .small()。
                                ui.label(egui::RichText::new("查询中…").weak());
                            }
                            (None, Some(Ok(info))) if info.is_displayable() => {
                                // 主数字加粗（第一眼要看到的那个），其余数字降为淡色小字。
                                // 两段仍在同一行：卡片主行高度是固定的。
                                if let Some(headline) = info.headline() {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(headline)
                                                .strong()
                                                .color(crate::theme::semantics(ui).info),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(info.detail());
                                    let rest = info.inline_rest();
                                    if !rest.is_empty() {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(rest)
                                                    .color(ui.visuals().weak_text_color()),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(info.detail());
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                });
            });
            if open {
                self.render_provider_form(ui, idx, &mut actions.model_hover);
            }
        });
        if let Some(src_key) = &self.provider_drag_src {
            if src_key != &key && resp.contains_pointer() && actions.hover.is_none() {
                actions.hover = Some(key.clone());
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
            ui.label(
                egui::RichText::new(
                    "先在 Providers 区新增一个 provider，填上 baseUrl，再回到这里填令牌。",
                )
                .small()
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
            if !self.token_uid_draft.contains_key(origin) {
                let existing = self.tokens.user_id(origin).to_string();
                self.token_uid_draft.insert(origin.clone(), existing);
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
                // 提示文字颜色由主题统一给定（见 `theme::Palette::hint_color`），
                // 不再在这里逐个控件覆盖：一处改、全应用一致。
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
            // 用户 ID：只有部分站点（部署的是旧版 new-api）需要它，
            // 所以放在令牌下一行，并说明什么情况下才填。
            // 先算好再进闭包：`station_needs_user_id` 借整个 self，
            // 不能在已经借了 `token_uid_draft` 的闭包里调用。
            let needs_id = self.station_needs_user_id(keys);
            ui.horizontal(|ui| {
                ui.add_space(crate::theme::SPACE_2);
                ui.label(
                    egui::RichText::new("用户 ID")
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
                if let Some(draft) = self.token_uid_draft.get_mut(origin) {
                    ui.add(
                        egui::TextEdit::singleline(draft)
                            .desired_width(90.0)
                            .hint_text("可留空"),
                    );
                }
                if needs_id {
                    ui.label(
                        egui::RichText::new("上次查询提示缺 New-Api-User，填你的用户 ID")
                            .small()
                            .color(crate::theme::semantics(ui).warn),
                    );
                } else {
                    ui.label(
                        egui::RichText::new("站点提示缺 New-Api-User 时才需填")
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                }
            });
            ui.add_space(crate::theme::SPACE_1);
        }

        // 单条显隐：与全局「显示密钥」是「或」的关系，互不干扰。
        if let Some(origin) = toggle_reveal {
            if !self.token_reveal.remove(&origin) {
                self.token_reveal.insert(origin);
            }
        }
        if let Some(origin) = save {
            let token = self.token_draft.get(&origin).cloned().unwrap_or_default();
            let uid = self
                .token_uid_draft
                .get(&origin)
                .cloned()
                .unwrap_or_default();
            if token.trim().is_empty() {
                self.status = format!("{} 的令牌为空：要清除请点「删除」", origin);
            } else {
                self.tokens.set(&origin, &token);
                // 用户 ID 是可选项：空串 = 不发 New-Api-User（新版站点不需要）。
                self.tokens.set_user_id(&origin, &uid);
                let with_uid = !uid.trim().is_empty();
                match self.tokens.save() {
                    Ok(()) => {
                        self.status = if with_uid {
                            format!("已保存 {} 的面板令牌与用户 ID", origin)
                        } else {
                            format!("已保存 {} 的面板令牌", origin)
                        };
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
            self.token_uid_draft.remove(&origin);
            match self.tokens.save() {
                Ok(()) => {
                    self.status = format!("已删除 {} 的面板令牌", origin);
                    self.forget_station_balance(&origin);
                }
                Err(err) => self.status = format!("令牌删除失败：{err}"),
            }
        }
    }

    /// 该站点的上次查询是否回了「缺 New-Api-User」。
    ///
    /// 从已有的用量结果推导，不额外记状态：错误可能落在 `Err`（令牌侧也失败）
    /// 或成功结果的 `note`（令牌侧成功、只有账号部分失败）两处，两边都要看。
    pub(super) fn station_needs_user_id(&self, provider_keys: &[String]) -> bool {
        provider_keys.iter().any(|key| {
            self.balance
                .get(key)
                .and_then(|state| state.display_result(super::balance::local_midnight_unix()))
                .is_some_and(|result| match result {
                    Err(err) => err.contains(super::balance::NEEDS_USER_ID_MARK),
                    Ok(info) => info
                        .note
                        .as_deref()
                        .is_some_and(|note| note.contains(super::balance::NEEDS_USER_ID_MARK)),
                })
        })
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
