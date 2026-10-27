//! The status line's level: how the bottom bar should read the current message.
//!
//! `App::status` used to be a bare `String`, so the bar painted every message in the same faint gray —
//! "已保存" and "令牌保存失败" looked identical. The level below is decided at the moment the message is
//! written (the only place that knows the outcome) and drives the bar's color.

use eframe::egui;

/// How the status line reads: neutral information, a completed action, or a failure.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(in crate::app) enum StatusLevel {
    /// Neutral information (progress notes, counts, what just happened): the faint body color.
    #[default]
    Info,
    /// A completed action: the semantic success color.
    Ok,
    /// A failure the user should notice: the semantic error color.
    Err,
}

impl super::App {
    /// Write the status line with an explicit level (the single place that touches both fields).
    pub(in crate::app) fn set_status_level(&mut self, level: StatusLevel, msg: impl Into<String>) {
        self.status = msg.into();
        self.status_level = level;
    }

    /// Write a neutral information status.
    pub(in crate::app) fn set_status(&mut self, msg: impl Into<String>) {
        self.set_status_level(StatusLevel::Info, msg);
    }

    /// Write a "just completed" status.
    pub(in crate::app) fn set_status_ok(&mut self, msg: impl Into<String>) {
        self.set_status_level(StatusLevel::Ok, msg);
    }

    /// Write a failure status.
    pub(in crate::app) fn set_status_err(&mut self, msg: impl Into<String>) {
        self.set_status_level(StatusLevel::Err, msg);
    }
}

/// The status bar's text color for a level: faint for information, the semantic success / error colors
/// for a completed action or a failure.
pub(in crate::app) fn status_color(ui: &egui::Ui, level: StatusLevel) -> egui::Color32 {
    match level {
        StatusLevel::Info => ui.visuals().weak_text_color(),
        StatusLevel::Ok => crate::theme::semantics(ui).ok,
        StatusLevel::Err => crate::theme::semantics(ui).err,
    }
}

#[cfg(test)]
impl super::App {
    /// The current status text (test accessor).
    pub(in crate::app) fn status_text(&self) -> &str {
        &self.status
    }

    /// The current status level (test accessor).
    pub(in crate::app) fn status_level(&self) -> StatusLevel {
        self.status_level
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;

    /// Each setter records the text and its level; the default is neutral.
    #[test]
    fn setters_record_text_and_level() {
        let mut app = App::default();
        assert_eq!(app.status_level(), StatusLevel::Info, "默认应是中性");
        app.set_status("加载中");
        assert_eq!(app.status_text(), "加载中");
        assert_eq!(app.status_level(), StatusLevel::Info);
        app.set_status_ok("已保存");
        assert_eq!(app.status_level(), StatusLevel::Ok);
        app.set_status_err("保存失败");
        assert_eq!(app.status_text(), "保存失败");
        assert_eq!(app.status_level(), StatusLevel::Err);
        // A later plain write resets the level rather than keeping the stale one.
        app.set_status("再说一句");
        assert_eq!(app.status_level(), StatusLevel::Info);
    }

    /// The level decides the bar's color: faint for information, the semantic ok / err colors otherwise.
    #[test]
    fn level_selects_the_bar_color() {
        let ctx = egui::Context::default();
        crate::theme::Theme::Dark.apply_style(&ctx, crate::theme::UiStyle::default(), false);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(200.0, 40.0),
            )),
            ..Default::default()
        };
        let mut seen: Vec<(
            StatusLevel,
            egui::Color32,
            egui::Color32,
            egui::Color32,
            egui::Color32,
        )> = Vec::new();
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let sem = crate::theme::semantics(ui);
                let weak = ui.visuals().weak_text_color();
                for level in [StatusLevel::Info, StatusLevel::Ok, StatusLevel::Err] {
                    seen.push((level, status_color(ui, level), weak, sem.ok, sem.err));
                }
            });
        });
        assert_eq!(seen.len(), 3, "三级都应取到颜色");
        for (level, got, weak, ok, err) in seen {
            match level {
                StatusLevel::Info => assert_eq!(got, weak, "信息文案应是弱化色"),
                StatusLevel::Ok => assert_eq!(got, ok, "完成文案应是成功色"),
                StatusLevel::Err => assert_eq!(got, err, "失败文案应是错误色"),
            }
        }
    }
}
