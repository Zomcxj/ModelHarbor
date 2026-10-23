use crate::app::App;
use eframe::egui;

impl App {
    /// The token panel's content: lists the current page's every station (merged by origin), with add / edit /
    /// delete for panel access tokens.
    ///
    /// Tokens are **station-level**: several providers on one station share one; rows are per station, noting
    /// which providers use them. Read-only queries only — never written into any agent config file.
    /// Rendered inside a popover by [`super::App::ui_tokens_window`].
    pub(in crate::app) fn ui_tokens_panel(&mut self, ui: &mut egui::Ui) {
        // Station → the provider keys using it (in first-seen order).
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

        // The popover supplies the title; no duplicate here.
        ui.label(
            egui::RichText::new(
                "在站点面板「个人设置 → 安全设置 → 系统访问令牌」生成；\
                 令牌是站点级的，同一站点的多个 provider 共用一份。\
                 只用于只读查询账号余额。\
                 站点类型可切换：NewAPI 填面板令牌；sub2api 直查无需配置。",
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

        // On open, drafts fill from saved values (masked display; empty default = unset).
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

        // Button actions collect first and write back to self uniformly after the loop.
        let mut save: Option<String> = None;
        let mut remove: Option<String> = None;
        let mut toggle_reveal: Option<String> = None;
        let mut toggle_kind: Option<String> = None;
        for (origin, keys) in &stations {
            let is_sub2api = self.tokens.is_sub2api(origin);
            let configured = self.tokens.has(origin);
            let revealed = self.show_api_keys || self.token_reveal.contains(origin);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(origin).monospace());
                // Station kind: NewAPI (the panel system) or sub2api (querying /v1/usage directly).
                let kind_button = ui.button(if is_sub2api {
                    "类型：sub2api"
                } else {
                    "类型：NewAPI"
                });
                if kind_button.clicked() {
                    toggle_kind = Some(origin.clone());
                }
                kind_button.on_hover_text(
                    "NewAPI：填面板令牌查账号额度；sub2api：直接用 provider 的 \
                     sk- key 查询余额，无需面板令牌，点击切换",
                );
                let semantics = crate::theme::semantics(ui);
                ui.label(
                    egui::RichText::new(if is_sub2api {
                        "sub2api 直查"
                    } else if configured {
                        "已设置"
                    } else {
                        "未设置"
                    })
                    .small()
                    .color(if is_sub2api || configured {
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
            // sub2api stations need no panel token or user ID: hide those two inputs and show the note instead.
            if !is_sub2api {
                ui.horizontal(|ui| {
                    // The hint text's color comes from the theme (`theme::Palette::hint_color`).
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
                // User ID: only some stations (older new-api) need it, so it sits under the token.
                // Compute before entering the closure: `station_needs_user_id` borrows all of self.
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
            } else {
                // sub2api: balances query /v1/usage directly — panel tokens and user IDs are useless here.
                ui.label(
                    egui::RichText::new(
                        "sub2api 站点：余额直接用 provider 的 sk- key 查询 /v1/usage，无需面板令牌与用户 ID",
                    )
                    .small()
                    .color(ui.visuals().weak_text_color()),
                );
            }
            ui.add_space(crate::theme::SPACE_1);
        }

        // Per-row visibility: an "or" with the global "show keys".
        if let Some(origin) = toggle_reveal {
            if !self.token_reveal.remove(&origin) {
                self.token_reveal.insert(origin);
            }
        }
        // Station-kind switch: persists and drops the station's usage cache (the query path changed).
        if let Some(origin) = toggle_kind {
            let next = !self.tokens.is_sub2api(&origin);
            self.tokens.set_site_kind(&origin, next);
            match self.tokens.save() {
                Ok(()) => {
                    self.status = format!(
                        "{} 已切换为 {}",
                        origin,
                        if next {
                            "sub2api（直查 /v1/usage）"
                        } else {
                            "NewAPI（面板令牌）"
                        }
                    );
                    self.forget_station_balance(&origin);
                }
                // A failed write rolls the in-memory flag back, keeping it consistent with disk.
                Err(err) => {
                    self.tokens.set_site_kind(&origin, !next);
                    self.status = format!("站点类型保存失败：{err}");
                }
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
                // The user ID is optional: an empty string = don't send New-Api-User.
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
                    // Errors carry paths only, never token content (see tokens::save_to).
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

    /// Whether the station's last query replied "missing New-Api-User".
    ///
    /// Derived from existing usage results: the error may sit in `Err` or in a success result's `note`.
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

    /// After a token change, drop the station's providers' usage caches.
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
