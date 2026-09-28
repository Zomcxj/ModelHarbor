use crate::app::preview::PREVIEW_RESIZER_ORDER;
use crate::app::App;
use eframe::egui;

/// 预览分隔条必须低于令牌悬浮窗：否则分割线会横穿窗口。
#[test]
fn preview_resizer_sits_below_the_tokens_window() {
    let resizer = PREVIEW_RESIZER_ORDER;
    let window = App::TOKENS_WINDOW_ORDER;
    assert!(
        resizer < window,
        "分隔条层级 {resizer:?} 不低于令牌窗层级 {window:?}"
    );
    // 悬停提示仍要显示在窗上面。
    assert!(window < egui::Order::Tooltip);
    // 分隔条要能接住拖拽：不能掉到背景层（预览面板本身就在 background）。
    assert!(resizer > egui::Order::Background);
}
