use super::*;

impl super::App {
    /// 站点面板令牌的悬浮窗外壳（内容见 `ui_tokens_panel`）。
    ///
    /// 窗口被限制在主窗口内，高度固定、内容超出用滚轮。
    pub(in crate::app) fn ui_tokens_window(&mut self, ctx: &egui::Context) {
        if !self.show_tokens {
            return;
        }
        // 固定高度：站点多了也不让窗口无限撑高，内容交给内部滚动区。
        // 上限取当前窗口高度减去边距。
        let area = ctx.content_rect();
        let height = (area.height() - 140.0).clamp(240.0, 520.0);
        // `.open()` 要借一个局部变量。
        let mut open = true;
        // 每帧居中：`default_pos` 只在首次生效，每帧居中用 `current_pos`。
        // 窗口尺寸固定，左上角由内容区算出。
        let size = egui::vec2(620.0, height);
        let centered = area.center() - size / 2.0;
        egui::Window::new("站点面板令牌")
            // 提到 Foreground：预览分隔条在 Middle 层，分割线不横穿悬浮窗。
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .order(Self::TOKENS_WINDOW_ORDER)
            .fixed_size(size)
            // 不允许拖到主窗口外。
            .constrain_to(area)
            .current_pos(centered)
            .frame(self.floating_window_frame(ctx))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    // 撑满固定高度，滚动条只内容超出时出现。
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.ui_tokens_panel(ui);
                    });
            });
        if !open {
            self.show_tokens = false;
        }
    }

    /// 配置体检悬浮窗：把各类检查汇总成一张清单（内容见 [`crate::app::health`]）。
    ///
    /// 只读清单，**不提供一键修复**。
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

    /// 悬浮窗统一的边框样式：比卡片更大的圆角与内边距。
    /// 圆角在形状预设基础上加一档（上限 20）。
    pub(in crate::app) fn floating_window_frame(&self, ctx: &egui::Context) -> egui::Frame {
        let mut frame = egui::Frame::window(&ctx.style());
        let extra = crate::theme::RADIUS_LG - crate::theme::RADIUS_MD;
        frame.corner_radius = self.ui_style.radius().saturating_add(extra).min(20).into();
        frame.inner_margin = egui::Margin::same(crate::theme::SPACE_4 as i8);
        frame
    }

    /// 体检清单的内容。每帧重算：只遍历内存里的 providers / agents。
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
            // 详情缩进一行，与标题分开，长文本自动换行。
            ui.horizontal_wrapped(|ui| {
                ui.add_space(crate::theme::SPACE_4);
                ui.add(egui::Label::new(egui::RichText::new(&issue.detail).small().weak()).wrap());
            });
            ui.add_space(crate::theme::SPACE_2);
        }
    }
}
