use crate::app::save::PageTarget;
use crate::app::App;
use eframe::egui;

impl App {
    /// The page header: this page's save button + write path.
    pub(in crate::app) fn ui_page_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let fmt = self.current_page;
            let target = self.page_save_path(fmt);
            let (path, can_save) = match &target {
                PageTarget::Current(p) => (p.clone(), true),
                PageTarget::Modified(p) => (p.clone(), true),
                PageTarget::Default(p) => {
                    let ok = self.targets.iter().any(|t| t.backend == fmt && t.available);
                    (p.clone(), ok)
                }
            };
            // Save all: writes the same UI state to every installed backend's target path.
            // The button carries the target count, so the number of files this write touches is visible before clicking.
            let installed: Vec<(String, String)> = self
                .targets
                .iter()
                .filter(|t| t.available && !t.path.trim().is_empty())
                .map(|t| (t.backend.label().to_string(), t.path.clone()))
                .collect();
            let save_all_tint = ui.visuals().text_color();
            let save_all_btn = match self.toolbar_icons.layers.as_ref() {
                Some(tex) => ui.add(egui::Button::image_and_text(
                    egui::Image::from_texture(tex)
                        .fit_to_exact_size(egui::vec2(14.0, 14.0))
                        .tint(save_all_tint),
                    format!("({})", installed.len()),
                )),
                None => ui.button(format!("一键保存 ({})", installed.len())),
            };
            if save_all_btn
                .on_hover_text(format!("一键保存：写入 {} 个已安装目标", installed.len()))
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                self.save_all();
            }
            // On the accent fill the body text color has too little contrast (dark ink on a dark accent,
            // light ink on a light one): the on-accent ink is the one that reads. With nothing to write the
            // button drops the accent entirely — a faded accent still reads as "the primary action is
            // available", while a neutral fill under the disabled dimming reads as "nothing to do".
            let (save_fill, save_ink) = if can_save {
                (
                    ui.visuals().selection.bg_fill,
                    ui.visuals().selection.stroke.color,
                )
            } else {
                (
                    ui.visuals().widgets.inactive.weak_bg_fill,
                    ui.visuals().weak_text_color(),
                )
            };
            let save_btn = match self.toolbar_icons.save.as_ref() {
                Some(tex) => egui::Button::image_and_text(
                    egui::Image::from_texture(tex)
                        .fit_to_exact_size(egui::vec2(14.0, 14.0))
                        .tint(save_ink),
                    egui::RichText::new("保存").strong().color(save_ink),
                )
                .fill(save_fill),
                None => egui::Button::new(egui::RichText::new("保存").strong().color(save_ink))
                    .fill(save_fill),
            };
            if ui
                .add_enabled(can_save, save_btn)
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                self.save_page(fmt);
            }
            if let Some(icon) = self.icon_for(fmt) {
                ui.add(egui::Image::from_texture(icon).fit_to_exact_size(egui::vec2(12.0, 12.0)));
            }
            // Paths can be long (the sticky region has fixed height, no wrapping): truncate for display, full text on hover.
            ui.add(
                egui::Label::new(egui::RichText::new(format!("写入: {path}")).weak()).truncate(),
            );
            // Blocks the format doesn't support get an early note.
            if !fmt.is_opencode_family() && !self.agents.is_empty() {
                ui.label(
                    egui::RichText::new(format!(
                        "⚠ {} 个 agents 不会写入该格式",
                        self.agents.len()
                    ))
                    .small()
                    .color(crate::theme::semantics(ui).warn),
                )
                .on_hover_text("该格式不支持 agent 定义，保存时将忽略");
            }
        });
        ui.separator();
    }
}
