use crate::app::{status_color, App};
use eframe::egui;

impl App {
    pub(in crate::app) fn ui_status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("bottom")
            .exact_height(32.0)
            .show(ctx, |ui| {
                // Bottom text: flush low and left-aligned; right-side stats flush right.
                ui.with_layout(egui::Layout::left_to_right(egui::Align::BOTTOM), |ui| {
                    if let Some(err) = &self.load_error {
                        ui.label(
                            egui::RichText::new(format!("⚠ 加载失败: {}", err))
                                .color(crate::theme::semantics(ui).err),
                        );
                    }
                    // The bottom status text is capped at about 70% of the left width, scrolling left when over-long (a marquee),
                    // with each loop starting aligned at its origin.
                    let max_w = (ui.available_width() * 0.72).max(160.0);
                    let row_h = ui.available_height();
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(max_w, row_h), egui::Sense::hover());
                    // The text color follows the message's level: information stays faint, a completed action
                    // reads green, a failure red (previously every message was the same gray).
                    let color = status_color(ui, self.status_level);
                    let galley = ui.painter().layout_no_wrap(
                        self.status.clone(),
                        egui::FontId::proportional(crate::theme::TEXT_SMALL),
                        color,
                    );
                    let painter = ui.painter().with_clip_rect(rect);
                    let y = rect.bottom() - galley.size().y;
                    let text_w = galley.size().x;
                    if text_w <= rect.width() {
                        painter.galley(egui::pos2(rect.left(), y), galley, color);
                    } else {
                        let t = ui.ctx().input(|i| i.time) as f32;
                        let span = text_w + 48.0; // Trailing whitespace, joining the loop
                        let phase = (t * 48.0) % span; // 48 px/s
                        painter.galley(egui::pos2(rect.left() - phase, y), galley, color);
                        // Continuous repaints drive the scrolling; the animation runs only on overflow.
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(32));
                    }
                    // Right side: current page + counts
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::BOTTOM), |ui| {
                        ui.horizontal(|ui| {
                            let wk = ui.visuals().weak_text_color();
                            // Current page: backend icon + name
                            if let Some(icon) = self.icon_for(self.current_page) {
                                ui.add(
                                    egui::Image::from_texture(icon)
                                        .fit_to_exact_size(egui::vec2(12.0, 12.0)),
                                );
                            }
                            ui.label(
                                egui::RichText::new(self.current_page.label())
                                    .size(crate::theme::TEXT_SMALL)
                                    .weak(),
                            );
                            ui.separator();
                            // The agents count (bot icon)
                            if let Some(tex) = self.toolbar_icons.bot.as_ref() {
                                ui.add(
                                    egui::Image::from_texture(tex)
                                        .fit_to_exact_size(egui::vec2(12.0, 12.0))
                                        .tint(wk),
                                )
                                .on_hover_text("agents");
                            }
                            ui.label(
                                egui::RichText::new(self.agents.len().to_string())
                                    .size(crate::theme::TEXT_SMALL)
                                    .weak(),
                            );
                            ui.add_space(crate::theme::SPACE_2);
                            // The providers count (server icon)
                            if let Some(tex) = self.toolbar_icons.server.as_ref() {
                                ui.add(
                                    egui::Image::from_texture(tex)
                                        .fit_to_exact_size(egui::vec2(12.0, 12.0))
                                        .tint(wk),
                                )
                                .on_hover_text("providers");
                            }
                            ui.label(
                                egui::RichText::new(self.providers.len().to_string())
                                    .size(crate::theme::TEXT_SMALL)
                                    .weak(),
                            );
                        });
                    });
                });
            });
    }
}
