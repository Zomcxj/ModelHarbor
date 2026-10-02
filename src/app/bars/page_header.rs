use crate::app::save::PageTarget;
use crate::app::App;
use eframe::egui;

impl App {
    /// 页头：本页保存按钮 + 写入路径。
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
            // 一键保存：把同一份界面状态写到每个已安装后端的目标路径，
            // 免去逐页切换逐个点保存。放在「保存」左侧（先全局后本页）。
            // 按钮直接带目标数量：一次会写几个文件必须点之前就看得见——
            // 本页的 provider 集合与别的 agent 不一致时，这一下会把它们一起改掉。
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
            let save_tint = ui.visuals().text_color();
            let save_fill = ui.visuals().selection.bg_fill;
            let save_btn = match self.toolbar_icons.save.as_ref() {
                Some(tex) => egui::Button::image_and_text(
                    egui::Image::from_texture(tex)
                        .fit_to_exact_size(egui::vec2(14.0, 14.0))
                        .tint(save_tint),
                    egui::RichText::new("保存").strong(),
                )
                .fill(save_fill),
                None => egui::Button::new(egui::RichText::new("保存").strong()).fill(save_fill),
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
            // 路径可能很长（吸顶区是固定高度，不能换行）：截断显示，全文放悬停。
            ui.add(
                egui::Label::new(egui::RichText::new(format!("写入: {path}")).weak()).truncate(),
            );
            // 该格式不支持的区块提前提示，避免保存后才发现数据没写入
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
