//! The peripheral bars — top bar / status bar / page header — plus sticky titles and error-text helpers.
//!
//! The public API re-exports from here; `crate::app::bars::X` paths stay unchanged.

mod errors;
mod page_header;
mod status;
mod sticky;
mod top_bar;
mod widgets;

pub(crate) use crate::util::sanitize_network_error;
pub(in crate::app) use errors::{http_status_code, short_err};
/// Unit tests assert its geometry directly; application code only uses `begin` / `end`.
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
        let margin = 3.0; // egui's default clip_rect_margin
                          // Zero scroll: clip_top = content top − margin, the title pinned at its content-flow position.
        assert_eq!(sticky_y(40.0, 40.0 - margin, margin), 40.0);
        // Just scrolled past the visible top: pinned at the visible top, not one pixel more.
        assert_eq!(sticky_y(39.0, 40.0 - margin, margin), 40.0);
        assert_eq!(sticky_y(40.0, 52.0, margin), 55.0);
    }

    #[test]
    fn sticky_does_not_jump_for_a_subpixel_gap() {
        // Content top equals the visible top; sub-pixel differences collapse into the same cell.
        assert_eq!(sticky_y(0.4, -3.0, 3.0), 0.0);
        assert_eq!(sticky_y(0.0, -2.6, 3.0), 0.0);
    }
}
