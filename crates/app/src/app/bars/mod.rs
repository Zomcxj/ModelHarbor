//! 顶栏 / 状态栏 / 页头等外围栏位，以及吸顶标题与错误文本处理小工具。
//!
//! 公共 API 从这里再导出，`crate::app::bars::X` 路径不变。

mod errors;
mod page_header;
mod status;
mod sticky;
mod top_bar;
mod widgets;

pub(crate) use crate::util::sanitize_network_error;
pub(in crate::app) use errors::{http_status_code, short_err};
/// 仅单测直接断言其几何性质；应用代码只用 `begin` / `end`。
#[cfg(test)]
pub(in crate::app) use sticky::sticky_y;
pub(in crate::app) use sticky::{sticky_begin, sticky_end};
pub(in crate::app) use widgets::toolbar_icon_button;

#[cfg(test)]
mod tests {
    use super::{sticky_y, toolbar_icon_button};
    use eframe::egui;

    #[test]
    fn toolbar_icon_button_has_a_fixed_hit_target() {
        let ctx = egui::Context::default();
        let mut states = Vec::new();
        for open in [false, true] {
            ctx.begin_pass(egui::RawInput::default());
            egui::CentralPanel::default().show(&ctx, |ui| {
                let response = toolbar_icon_button(ui, None, open, egui::Color32::WHITE);
                states.push(response.rect.size());
            });
            let _ = ctx.end_pass();
        }
        assert_eq!(states[0], states[1]);
        assert_eq!(states[0], egui::vec2(24.0, 24.0));
    }

    #[test]
    fn sticky_stays_put_until_it_hits_the_clip() {
        let margin = 3.0; // egui 默认 clip_rect_margin
                          // 滚动为零时 clip_top = 内容顶 - margin，标题钉在内容流位置。
        assert_eq!(sticky_y(40.0, 40.0 - margin, margin), 40.0);
        // 刚滚过可视顶：钉在可视顶，一格都不多滚。
        assert_eq!(sticky_y(39.0, 40.0 - margin, margin), 40.0);
        assert_eq!(sticky_y(40.0, 52.0, margin), 55.0);
    }

    #[test]
    fn sticky_does_not_jump_for_a_subpixel_gap() {
        // 内容顶就是可视顶，亚像素差收成同一格。
        assert_eq!(sticky_y(0.4, -3.0, 3.0), 0.0);
        assert_eq!(sticky_y(0.0, -2.6, 3.0), 0.0);
    }
}
