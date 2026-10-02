use crate::app::App;
use eframe::egui;

impl App {
    pub(in crate::app) fn ui_status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("bottom")
            .exact_height(32.0)
            .show(ctx, |ui| {
                // 底部文字：靠下（不垂直居中）且左对齐，右侧统计仍靠右。
                ui.with_layout(egui::Layout::left_to_right(egui::Align::BOTTOM), |ui| {
                    if let Some(err) = &self.load_error {
                        ui.label(
                            egui::RichText::new(format!("⚠ 加载失败: {}", err))
                                .color(crate::theme::semantics(ui).err),
                        );
                    }
                    ui.label(
                        egui::RichText::new(&self.status)
                            .size(crate::theme::TEXT_SMALL)
                            .weak(),
                    );
                    // 右侧：当前页 + 数量统计，随时可见页面身份
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::BOTTOM), |ui| {
                        ui.horizontal(|ui| {
                            let wk = ui.visuals().weak_text_color();
                            // 当前页：后端官方图标 + 名称
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
                            // agents 计数（bot 图标）
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
                            // providers 计数（server 图标）
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
