use crate::app::preview::PREVIEW_RESIZER_ORDER;
use crate::app::App;
use eframe::egui;

/// The preview splitter sits below the token popover.
#[test]
fn preview_resizer_sits_below_the_tokens_window() {
    let resizer = PREVIEW_RESIZER_ORDER;
    let window = App::TOKENS_WINDOW_ORDER;
    assert!(
        resizer < window,
        "分隔条层级 {resizer:?} 不低于令牌窗层级 {window:?}"
    );
    // Hover tips display above the window.
    assert!(window < egui::Order::Tooltip);
    // The splitter must catch drags — it can't sink to the background layer (the preview panel itself is background).
    assert!(resizer > egui::Order::Background);
}
