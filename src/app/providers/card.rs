use super::flags::CardActions;
use crate::app::bars::short_err;
use crate::app::fetch::{latency_color, matrix_label};
use crate::app::App;
use crate::ui::{card_frame, move_item, DragHandle};
use eframe::egui;

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
                            state.display_result(crate::app::balance::local_midnight_unix());
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
                                                    .size(crate::theme::TEXT_SMALL)
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
            // 折叠 / 展开带高度动画；动画 id 按 key 派生，改名即换 id（状态不串卡）。
            crate::motion::animated_collapse(ui, card_id, open, |ui| {
                self.render_provider_form(
                    ui,
                    idx,
                    &mut actions.model_hover,
                    &mut actions.model_enable,
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
