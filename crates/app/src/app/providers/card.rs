use super::flags::CardActions;
use crate::app::bars::short_err;
use crate::app::fetch::{latency_color, matrix_label};
use crate::app::App;
use crate::ui::{card_frame, move_item, DragHandle};
use eframe::egui;

/// The card head's right group's balance display, for `Sides`'s right closure.
///
/// Right-group data computes into owned values first: `Sides::show`'s two closures live simultaneously,
/// and the left closure needs the mutable `self` borrow.
enum BalanceDisplay {
    /// Querying (spinner + "querying…").
    Loading,
    /// Query finished with presentable data (headline / remaining numbers / hover details).
    ///
    /// `headline` is non-optional: the construction site returns `None` directly when there's no headline.
    Info {
        headline: String,
        rest: String,
        detail: String,
    },
}

impl App {
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
        let card_id = egui::Id::new(("provider_card", key.clone()));
        let resp = card_frame(ui, open, highlight, card_id, |ui| {
            // The head row splits into left and right groups: the left (grab handle / collapse / vendor name /
            // baseUrl note / connectivity result) is variable-length; the right (delete / duplicate / balance)
            // hugs the card's right edge.
            //
            // Uses `Sides::shrink_left().truncate()`: measure the right group first, then constrain the left group
            // to the remaining width, truncating as needed.
            let balance_display = self.balance.get(&key).and_then(|state| {
                let display_result =
                    state.display_result(crate::app::balance::local_midnight_unix());
                match (&state.rx, display_result.as_ref()) {
                    (Some(_), _) => Some(BalanceDisplay::Loading),
                    (None, Some(Ok(info))) if info.is_displayable() => Some(BalanceDisplay::Info {
                        // No headline means nothing displays.
                        headline: info.headline()?,
                        rest: info.inline_rest(),
                        detail: info.detail(),
                    }),
                    _ => None,
                }
            });
            let _ = egui::Sides::new().shrink_left().truncate().show(
                ui,
                |ui| {
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
                    // The baseUrl health note: typos like `//v1` are visible on the card (a hint only, never auto-rewritten).
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
                    // The connectivity test's result: shown right of the vendor name, visible even when the card is collapsed.
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
                },
                |ui| {
                    if ui.button("删除").clicked() {
                        actions.remove = Some(idx);
                    }
                    if ui.button("复制").clicked() {
                        actions.copy = Some(idx);
                    }
                    // The query result sits right of "duplicate": balance / used / today / check-in status all live on this line.
                    // Unqueryable stations show nothing — never a placeholder balance.
                    match &balance_display {
                        Some(BalanceDisplay::Loading) => {
                            ui.add(egui::Spinner::new().size(14.0));
                            // The font size matches the connectivity result (`123ms`).
                            ui.label(egui::RichText::new("查询中…").weak());
                        }
                        Some(BalanceDisplay::Info {
                            headline,
                            rest,
                            detail,
                        }) => {
                            // The headline is bold; the rest are faint small text — both on one line.
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(headline)
                                        .strong()
                                        .color(crate::theme::semantics(ui).info),
                                )
                                .truncate(),
                            )
                            .on_hover_text(detail);
                            if !rest.is_empty() {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(rest)
                                            .size(crate::theme::TEXT_SMALL)
                                            .color(ui.visuals().weak_text_color()),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(detail);
                            }
                        }
                        None => {}
                    }
                },
            );
            // Collapse / expand with height animation; the animation id derives from the key.
            crate::motion::animated_collapse(ui, card_id, open, |ui| {
                self.render_provider_form(
                    ui,
                    idx,
                    &mut actions.model_hover,
                    &mut actions.model_enable,
                    &mut actions.sync,
                );
            });
        });
        if let Some(src_key) = &self.provider_drag_src {
            if src_key != &key && resp.contains_pointer() && actions.hover.is_none() {
                actions.hover = Some(key.clone());
            }
        }
    }
}
