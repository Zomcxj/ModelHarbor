use eframe::egui;

/// 本帧请求的自定义抓取光标。
///
/// 控件只「提出请求」，由 `App::update` 在帧末统一落到 Win32（见
/// `crate::cursor::set_custom_cursor`）。这样跨平台构建不需要 `cfg`，
/// 而且同一帧里多个热区只产生一次系统调用。
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum GrabCursor {
    #[default]
    None,
    /// 悬停在可拖动处：系统自带的手形光标（`IDC_HAND`）。
    Palm,
    /// 已按住：握起的拳头。
    Fist,
}

fn grab_cursor_id() -> egui::Id {
    egui::Id::new("model_harbor_grab_cursor")
}

/// 请求本帧的抓取光标。同一帧里取「更强」的一态（Fist > Palm > None）：
/// 拖动中的把手不应被旁边另一个只是悬停的热区降级成手掌。
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

/// 取出并清空本帧请求（帧末调用一次，避免状态泄漏到下一帧）。
pub fn take_grab_cursor(ctx: &egui::Context) -> GrabCursor {
    ctx.data_mut(|d| {
        let cur = d
            .get_temp::<GrabCursor>(grab_cursor_id())
            .unwrap_or_default();
        d.remove::<GrabCursor>(grab_cursor_id());
        cur
    })
}

/// 悬停态：手掌；按下态：拳头。
///
/// 用于拖动把手这类「按下才开始拖」的控件——`dragged()` 要越过拖动阈值才为真，
/// 按下但还没移动的那几帧会没有反馈，所以用 `is_pointer_button_down_on()` 判按下。
pub fn grab_cursor_for(resp: &egui::Response) -> GrabCursor {
    if resp.is_pointer_button_down_on() || resp.dragged() {
        GrabCursor::Fist
    } else if resp.hovered() {
        GrabCursor::Palm
    } else {
        GrabCursor::None
    }
}
