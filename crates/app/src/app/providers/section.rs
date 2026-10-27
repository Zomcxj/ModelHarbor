use super::flags::CardActions;
use super::flags::SyncField;
use crate::app::balance;
use crate::app::bars::{sticky_begin, sticky_end};
use crate::app::App;
use crate::credentials;
use crate::ui::card_list;
use eframe::egui;

impl App {
    /// The Providers block: the title row sticks to the top (always visible while scrolling), content right below.
    pub(in crate::app) fn ui_providers_section(&mut self, ui: &mut egui::Ui) {
        // The sticky region is a **fixed-height rect** (sticky_end builds the child ui from max_rect):
        // everything inside must fit within that one height — extra rows would paint straight onto the cards below.
        let anchor = sticky_begin(ui, 30.0);
        let matched: Vec<usize> = (0..self.providers.len()).collect();

        if self.providers.is_empty() && !self.show_new_provider {
            self.show_new_provider = true;
        }

        self.ui_first_run_guide(ui);

        // Drag drop targets must aggregate uniformly **after all cards render**, then write into self.
        let mut actions = CardActions::default();
        let card_gap = if crate::theme::active_style(ui.ctx()).has_card_shadow() {
            crate::theme::SPACE_2
        } else {
            0.0
        };
        card_list(ui, &matched, card_gap, |ui, idx| {
            self.render_provider_card(ui, idx, &mut actions);
        });
        // One model id can be enabled globally only once — handled centrally after all cards render.
        if let Some(picked) = actions.model_enable.take() {
            self.enable_model_exclusively(picked);
        }
        // "Sync": brush the source model's context / output values onto every model on the page.
        // Also applied centrally after all cards render, avoiding a mutable borrow mid-render.
        if let Some((field, value)) = actions.sync.take() {
            self.apply_limit_sync(field, value);
        }
        if let Some(idx) = actions.remove {
            self.providers.remove(idx);
            self.set_status_ok("已删除 provider");
        }
        if let Some(idx) = actions.copy {
            let mut p = self.providers[idx].clone();
            p.key = format!("{}_copy", p.key);
            self.providers.push(p);
            self.set_status_ok("已复制 provider");
        }
        if self.provider_drag_src.is_some() {
            self.provider_drag_target = actions.hover;
        } else {
            self.provider_drag_target = None;
        }
        // Model drag drop targets: aggregated at the outer layer like providers'.
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
                ui.strong(egui::RichText::new("Providers").size(crate::theme::TEXT_HEADING));
                // Three action buttons on the title row's left (same row as the title, left-aligned with content):
                // expand/collapse all, connectivity test, query user data — one icon style.
                // Expand/collapse all (chevrons icon): visible even when every card is collapsed.
                if !self.providers.is_empty() {
                    let all_open = self
                        .providers
                        .iter()
                        .all(|p| !self.provider_collapsed(&p.key));
                    let chevrons_tint = ui.visuals().text_color();
                    if crate::app::bars::toolbar_icon_button(
                        ui,
                        self.toolbar_icons.unfold.as_ref(),
                        false,
                        chevrons_tint,
                    )
                    .on_hover_text(if all_open {
                        "收起全部卡片"
                    } else {
                        "展开全部卡片"
                    })
                    .clicked()
                    {
                        // all_open true = everything is expanded now → the button reads "collapse all"
                        self.set_all_providers_collapsed(all_open);
                        // Batch actions skip the height tween.
                        let ctx = ui.ctx().clone();
                        let open = !all_open;
                        for provider in &self.providers {
                            crate::motion::snap_collapse(
                                &ctx,
                                egui::Id::new(("provider_card", provider.key.clone())),
                                open,
                            );
                        }
                    }
                }

                // Connectivity test (zap icon): batch-test every vendor's connectivity.
                let zap_tint = ui.visuals().text_color();
                if crate::app::bars::toolbar_icon_button(
                    ui,
                    self.toolbar_icons.zap.as_ref(),
                    false,
                    zap_tint,
                )
                .on_hover_text("连通性测试：批量测全部厂商的连通性")
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
                    self.set_status(format!("已开始连通性测试（{} 个厂商）", count));
                }

                // Query user data (database icon, closest to the line's end): queries every vendor on the page in one go.
                // One result shows whatever it holds: balance / used / today / last 7 days / check-in status.
                let database_tint = ui.visuals().text_color();
                if crate::app::bars::toolbar_icon_button(
                    ui,
                    self.toolbar_icons.database.as_ref(),
                    false,
                    database_tint,
                )
                .on_hover_text("查询用户数据：批量查全部厂商的余额/用量/签到")
                .clicked()
                {
                    let targets: Vec<balance::Query> = self
                        .providers
                        .iter()
                        .map(|p| balance::Query {
                            key: p.key.clone(),
                            base_url: p.base_url.clone(),
                            secret: credentials::effective_secret(p),
                            // Station-level panel token: an empty string when unset.
                            pat: self.station_pat(&p.base_url),
                            // The user ID older new-api requires: empty when unset (the header isn't sent).
                            user_id: self.station_user_id(&p.base_url),
                            // sub2api stations: query /v1/usage directly, skipping the panel system.
                            sub2api: self.station_is_sub2api(&p.base_url),
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
                    self.set_status(if started == 0 {
                        "所有厂商都还在冷却中，请几秒后再试".to_string()
                    } else {
                        format!("已开始查询 {} 个厂商的用户数据…", started)
                    });
                }
            });
        });
    }

    /// Brush one model's context / output values onto every model on the page (the "sync" dropdown's first item).
    ///
    /// `value` is the source model's value at click time (extracted during render; this only writes back); empty values don't brush.
    pub(in crate::app) fn apply_limit_sync(&mut self, field: SyncField, value: String) {
        let value = value.trim().to_string();
        if value.is_empty() {
            self.set_status_err(format!("源模型的{}为空，未同步", field.label()));
            return;
        }
        let mut touched = 0usize;
        for p in &mut self.providers {
            for m in &mut p.models {
                match field {
                    SyncField::Context => m.context = value.clone(),
                    SyncField::Output => m.output = value.clone(),
                }
                touched += 1;
            }
        }
        self.set_status_ok(format!(
            "已把{} {value} 同步到 {touched} 个模型",
            field.label()
        ));
    }
}
