use super::*;

impl super::App {
    /// The station token popover's shell (content in `ui_tokens_panel`).
    ///
    /// The window is confined inside the main window, with fixed height and internal scrolling for overflow.
    pub(in crate::app) fn ui_tokens_window(&mut self, ctx: &egui::Context) {
        if !self.show_tokens {
            return;
        }
        // Fixed height: many stations must not stretch the window unboundedly — content goes to the internal scroll area.
        // The cap is the current window height minus margins.
        let area = ctx.content_rect();
        let height = (area.height() - 140.0).clamp(240.0, 520.0);
        // `.open()` borrows a local.
        let mut open = true;
        // Centered every frame: `default_pos` only applies the first time; per-frame centering uses `current_pos`.
        // With fixed window size, the top-left derives from the content area.
        let size = egui::vec2(620.0, height);
        let centered = area.center() - size / 2.0;
        egui::Window::new("站点面板令牌")
            // Raised to Foreground: the preview splitter lives on Middle — the splitter line must not cross the popover.
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .order(Self::TOKENS_WINDOW_ORDER)
            .fixed_size(size)
            // No dragging outside the main window.
            .constrain_to(area)
            .current_pos(centered)
            .frame(self.floating_window_frame(ctx))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    // Fill the fixed height; the scrollbar appears only when content overflows.
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.ui_tokens_panel(ui);
                    });
            });
        if !open {
            self.show_tokens = false;
        }
    }

    /// The config health popover: gathers the various checks into one list (content in [`crate::app::health`]).
    ///
    /// A read-only list — **no one-click fixes**.
    pub(in crate::app) fn ui_health_window(&mut self, ctx: &egui::Context) {
        if !self.show_health {
            return;
        }
        let area = ctx.content_rect();
        let height = (area.height() - 140.0).clamp(240.0, 560.0);
        let mut open = true;
        let size = egui::vec2(680.0, height);
        let centered = area.center() - size / 2.0;
        egui::Window::new("配置体检")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .order(Self::TOKENS_WINDOW_ORDER)
            .fixed_size(size)
            .constrain_to(area)
            .current_pos(centered)
            .frame(self.floating_window_frame(ctx))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.ui_health_panel(ui);
                    });
            });
        if !open {
            self.show_health = false;
        }
    }

    /// The popovers' shared border style: larger corner radius and padding than cards.
    /// The radius adds one step over the shape preset (capped at 20).
    pub(in crate::app) fn floating_window_frame(&self, ctx: &egui::Context) -> egui::Frame {
        let mut frame = egui::Frame::window(&ctx.style());
        let extra = crate::theme::RADIUS_LG - crate::theme::RADIUS_MD;
        frame.corner_radius = self.ui_style.radius().saturating_add(extra).min(20).into();
        frame.inner_margin = egui::Margin::same(crate::theme::SPACE_4 as i8);
        frame
    }

    /// The health list's content. Recomputed every frame: only walks in-memory providers / agents.
    pub(in crate::app) fn ui_health_panel(&mut self, ui: &mut egui::Ui) {
        let input = health::HealthInput {
            page: self.current_page,
            providers: &self.providers,
            agents: &self.agents,
            source_is_opencode: self.source_format.is_opencode_family(),
        };
        let issues = health::collect(&input);
        let semantics = crate::theme::semantics(ui);
        if issues.is_empty() {
            ui.colored_label(semantics.ok, "没有发现需要处理的问题。");
            ui.add_space(crate::theme::SPACE_2);
            ui.label(
                egui::RichText::new(
                    "体检只检查结构与字段写法（重名、非法数字、可疑 URL、跨网关引用等），\
                     不代表配置一定能在上游跑通。",
                )
                .small()
                .weak(),
            );
            return;
        }
        let (blockers, warnings) = health::count_by_severity(&issues);
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("{} 项", issues.len()));
            if blockers > 0 {
                ui.colored_label(semantics.err, format!("{} 项会阻止保存", blockers));
            }
            if warnings > 0 {
                ui.colored_label(semantics.warn, format!("{} 项需要注意", warnings));
            }
        });
        ui.add_space(crate::theme::SPACE_2);
        for issue in &issues {
            let color = match issue.severity {
                health::Severity::Blocker => semantics.err,
                health::Severity::Warn => semantics.warn,
                health::Severity::Info => semantics.info,
            };
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(color, egui::RichText::new("●").small());
                ui.strong(&issue.title);
                if !issue.where_.is_empty() {
                    ui.label(egui::RichText::new(&issue.where_).monospace().weak());
                }
                ui.label(
                    egui::RichText::new(issue.severity.label())
                        .small()
                        .color(color),
                );
            });
            // Details indent one line, apart from the title, with long text wrapping.
            ui.horizontal_wrapped(|ui| {
                ui.add_space(crate::theme::SPACE_4);
                ui.add(egui::Label::new(egui::RichText::new(&issue.detail).small().weak()).wrap());
            });
            ui.add_space(crate::theme::SPACE_2);
        }
    }
}
