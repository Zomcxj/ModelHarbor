use eframe::egui;

/// This frame's requested custom grab cursor.
///
/// Widgets only raise requests; `App::update` lands them on Win32 at frame end
/// (see `crate::cursor::set_custom_cursor`).
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum GrabCursor {
    #[default]
    None,
    /// Hovering something draggable: the system's own hand cursor (`IDC_HAND`).
    Palm,
    /// Held down: the closed fist.
    Fist,
}

fn grab_cursor_id() -> egui::Id {
    egui::Id::new("model_harbor_grab_cursor")
}

/// Request this frame's grab cursor. Within one frame, the stronger state wins (Fist > Palm > None).
pub fn request_grab_cursor(ctx: &egui::Context, want: GrabCursor) {
    if want == GrabCursor::None {
        return;
    }
    ctx.data_mut(|d| {
        let cur = d
            .get_temp::<GrabCursor>(grab_cursor_id())
            .unwrap_or_default();
        let next = if cur == GrabCursor::Fist || want == GrabCursor::Fist {
            GrabCursor::Fist
        } else {
            GrabCursor::Palm
        };
        d.insert_temp(grab_cursor_id(), next);
    });
}

/// Take and clear this frame's request (called once at frame end, so state never leaks into the next).
pub fn take_grab_cursor(ctx: &egui::Context) -> GrabCursor {
    ctx.data_mut(|d| {
        let cur = d
            .get_temp::<GrabCursor>(grab_cursor_id())
            .unwrap_or_default();
        d.remove::<GrabCursor>(grab_cursor_id());
        cur
    })
}

/// Hovering: palm; pressed: fist.
///
/// For widgets like drag handles that start dragging on press — a press immediately judges Fist via
/// `is_pointer_button_down_on()`, never waiting for `dragged()` to cross the drag threshold.
pub fn grab_cursor_for(resp: &egui::Response) -> GrabCursor {
    if resp.is_pointer_button_down_on() || resp.dragged() {
        GrabCursor::Fist
    } else if resp.hovered() {
        GrabCursor::Palm
    } else {
        GrabCursor::None
    }
}
