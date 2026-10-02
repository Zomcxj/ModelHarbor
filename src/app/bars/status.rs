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
                    // 底部状态文字：限制在左侧约七成宽度内；超宽时向左滚动（跑马灯）——
                    // 「一键保存」的汇总经常超出整行版面，静态显示会溢出、还会把右侧
                    // 统计挤走。滚动循环从「起点对齐」开始（先看到消息开头），滚完再重来。
                    let max_w = (ui.available_width() * 0.72).max(160.0);
                    let row_h = ui.available_height();
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(max_w, row_h), egui::Sense::hover());
                    let color = ui.visuals().weak_text_color();
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
                        let span = text_w + 48.0; // 尾部留白，循环衔接
                        let phase = (t * 48.0) % span; // 48 px/s
                        painter.galley(egui::pos2(rect.left() - phase, y), galley, color);
                        // 持续重绘才能动起来；仅溢出时才开动画，不空耗。
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(32));
                    }
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
