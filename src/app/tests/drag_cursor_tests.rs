use crate::app::App;
use crate::format::ConfigFormat;

#[test]
fn every_drag_source_activates_the_grab_cursor() {
    let mut app = App::default();
    assert!(!app.is_dragging_anything(), "静止时不该点亮抓取光标");

    app.tab_drag_src = Some(ConfigFormat::ZCode);
    assert!(
        app.is_dragging_anything(),
        "拖动页签（后端图标）必须点亮抓取光标"
    );
    app.tab_drag_src = None;

    app.provider_drag_src = Some("p".to_string());
    assert!(app.is_dragging_anything(), "拖动 provider 卡片必须点亮");
    app.provider_drag_src = None;

    app.agent_drag_src = Some("a".to_string());
    assert!(app.is_dragging_anything(), "拖动 agent 卡片必须点亮");
    app.agent_drag_src = None;

    app.model_drag_src = Some("p\u{1f}m".to_string());
    assert!(app.is_dragging_anything(), "拖动 model 卡片必须点亮");
    app.model_drag_src = None;

    assert!(!app.is_dragging_anything(), "全部松开后必须熄灭抓取光标");
}
