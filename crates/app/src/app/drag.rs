use super::*;

impl super::App {
    /// Whether any drag source is currently dragging.
    ///
    /// Tabs (backend icons) are drag sources too; the custom grab cursor takes effect window-wide.
    pub(in crate::app) fn is_dragging_anything(&self) -> bool {
        self.agent_drag_src.is_some()
            || self.provider_drag_src.is_some()
            || self.model_drag_src.is_some()
            || self.tab_drag_src.is_some()
    }

    /// The drag ghost's label: the thing being dragged, as the user knows it.
    ///
    /// Agent / provider keys and model ids are looked up rather than trusted: a card can be deleted while its
    /// ghost is still on screen.
    fn drag_ghost_label(&self) -> Option<String> {
        if let Some(k) = &self.agent_drag_src {
            return self.agents.iter().any(|a| &a.key == k).then(|| k.clone());
        }
        if let Some(k) = &self.provider_drag_src {
            return self
                .providers
                .iter()
                .any(|p| &p.key == k)
                .then(|| k.clone());
        }
        if let Some(k) = &self.model_drag_src {
            // `provider_key\u{1f}model_id`: the ghost names the model, not the provider.
            let model = k.split_once('\u{1f}').map(|(_, m)| m).unwrap_or(k.as_str());
            return Some(model.to_string());
        }
        // Tabs are drag sources too (top bar reordering). They used to paint **no ghost at all**: the chain
        // ended before this case, so dragging a tab gave no feedback beyond the drop-target ring.
        self.tab_drag_src.map(|format| format.label().to_string())
    }

    pub(in crate::app) fn paint_drag_ghost(&self, ctx: &egui::Context) {
        let Some(label) = self.drag_ghost_label() else {
            return;
        };
        if label.is_empty() {
            return;
        }
        // `pointer_interact_pos`, not `pointer_hover_pos`: during a drag the pointer can briefly report
        // "gone" (leaving the window, crossing a panel edge) and the ghost would blink out mid-drag.
        let Some(pointer) = ctx.pointer_interact_pos() else {
            return;
        };
        let layer_id = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("drag_ghost"));
        let painter = ctx.layer_painter(layer_id);
        let visuals = &ctx.style().visuals;
        let shape = crate::theme::active_style(ctx);
        // Follow the theme's shape preset and type scale: the ghost used to pin radius 8 / font 14 regardless.
        let corner: egui::CornerRadius = shape.radius().into();
        let font_id = egui::FontId::proportional(crate::theme::TEXT_BODY);
        let text_color = visuals.text_color();
        // A lifted chip rather than a panel-colored box: a fill equal to the page reads as a hole cut in the
        // list, not as the thing being carried.
        let fill = visuals.widgets.hovered.weak_bg_fill;
        let stroke = egui::Stroke::new(1.0_f32, visuals.widgets.hovered.bg_stroke.color);
        let galley = painter.layout(label, font_id.clone(), text_color, f32::INFINITY);
        let padding = egui::vec2(crate::theme::SPACE_3, crate::theme::SPACE_2);
        let size = galley.size() + padding * 2.0;
        let ghost_rect = egui::Rect::from_min_size(pointer + egui::vec2(20.0, -size.y / 2.0), size);
        // Every preset gets a shadow here, unlike cards: cards sit in the page and take its shape vocabulary,
        // while the ghost floats over arbitrary content (cards, tables, text) and must separate from all of it.
        let shadow = egui::epaint::Shadow {
            offset: [0, 6],
            blur: 16,
            spread: 0,
            color: egui::Color32::from_black_alpha(if visuals.dark_mode { 120 } else { 60 }),
        };
        painter.add(shadow.as_shape(ghost_rect, corner));
        painter.rect_filled(ghost_rect, corner, fill);
        painter.rect_stroke(ghost_rect, corner, stroke, egui::StrokeKind::Inside);
        // Paint the galley already laid out for measuring instead of layouts again.
        painter.galley(
            ghost_rect.left_center() + egui::vec2(padding.x, 0.0),
            galley,
            text_color,
        );
    }
}
