//! Scroll-area edge treatment: the "more content this way" fade at an overflowing viewport edge.
//!
//! Rows used to end square at the viewport edge, which reads as "the list ends here" — with a scrollbar the
//! only hint that more exists. A short gradient over the panel fill says so without a hard line.
//!
//! Painting happens **after** the scroll area, so the band sits above the content, takes no layout space,
//! and never clips real content.

use eframe::egui;

/// Paint the fade bands at a vertically scrolling viewport's overflowing edges.
///
/// Does nothing when the content fits, and paints each edge only while content continues past it, so a
/// fully scrolled list keeps a clean bottom edge.
pub fn paint_edge_fades<R>(
    ui: &egui::Ui,
    out: &egui::containers::scroll_area::ScrollAreaOutput<R>,
) {
    let rect = out.inner_rect;
    if rect.height() <= 0.0 {
        return;
    }
    let fill = ui.visuals().panel_fill;
    // Same hue, zero alpha: blending to a transparent black / white would tint the fade on the other
    // theme family.
    let clear = fill.gamma_multiply(0.0);
    let band = (rect.height() * 0.12).clamp(6.0, 16.0);
    let more_above = out.state.offset.y > 1.0;
    let more_below = out.state.offset.y + rect.height() < out.content_size.y - 1.0;
    if more_above {
        let band_rect =
            egui::Rect::from_min_max(rect.left_top(), egui::pos2(rect.right(), rect.top() + band));
        vertical_fade(ui, band_rect, fill, clear);
    }
    if more_below {
        let band_rect = egui::Rect::from_min_max(
            rect.left_bottom() - egui::vec2(0.0, band),
            rect.right_bottom(),
        );
        vertical_fade(ui, band_rect, clear, fill);
    }
}

/// A two-color vertical gradient (top color to bottom color) across `rect`.
fn vertical_fade(ui: &egui::Ui, rect: egui::Rect, top: egui::Color32, bottom: egui::Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    ui.painter().add(egui::Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fade band is painted only at an edge that still has content beyond it.
    #[test]
    fn edge_fade_appears_only_when_content_overflows() {
        let ctx = egui::Context::default();
        crate::theme::Theme::Dark.apply_style(&ctx, crate::theme::UiStyle::default(), false);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(200.0, 60.0),
            )),
            ..Default::default()
        };
        fn count_meshes(shape: &egui::Shape, out: &mut usize) {
            match shape {
                egui::Shape::Mesh(_) => *out += 1,
                egui::Shape::Vec(items) => items.iter().for_each(|s| count_meshes(s, out)),
                _ => {}
            }
        }
        let run = |rows: usize| {
            let out = ctx.run(input.clone(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let out = egui::ScrollArea::vertical()
                        .id_salt("fade_probe")
                        .auto_shrink([false, false])
                        .max_height(40.0)
                        .min_scrolled_height(40.0)
                        .show(ui, |ui| {
                            for i in 0..rows {
                                ui.label(format!("row {i}"));
                            }
                        });
                    paint_edge_fades(ui, &out);
                });
            });
            let mut meshes = 0;
            for clipped in &out.shapes {
                count_meshes(&clipped.shape, &mut meshes);
            }
            meshes
        };
        assert!(run(20) > 0, "内容溢出时底部必须有渐隐带");
        assert_eq!(run(1), 0, "内容装得下时不该有渐隐带");
    }
}
