use crate::app::App;
use eframe::egui;

impl App {
    /// 令牌面板内容：列出当前页面所有站点（按 origin 归并），填 / 改 / 删面板访问令牌。
    ///
    /// 令牌是**站点级**的：同一站点的多个 provider 共用一份，所以这里按站点一行，
    /// 并标注哪些 provider 在用。只用于只读查询，不写进任何 agent 配置文件。
    /// 由 [`super::App::ui_tokens_window`] 装进悬浮窗渲染，不再占正文布局。
    pub(in crate::app) fn ui_tokens_panel(&mut self, ui: &mut egui::Ui) {
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
    pub(in crate::app) fn station_needs_user_id(&self, provider_keys: &[String]) -> bool {
        provider_keys.iter().any(|key| {
            self.balance
                .get(key)
                .and_then(|state| state.display_result(crate::app::balance::local_midnight_unix()))
                .is_some_and(|result| match result {
                    Err(err) => err.contains(crate::app::balance::NEEDS_USER_ID_MARK),
                    Ok(info) => info
                        .note
                        .as_deref()
                        .is_some_and(|note| note.contains(crate::app::balance::NEEDS_USER_ID_MARK)),
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
}
