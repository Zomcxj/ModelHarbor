//! The theme menu's interaction regression: feed pointer events to a headless `egui::Context`,
//! replicating `src/app/bars.rs`'s top-bar "appearance" button + `Popup::menu` structure,
//! checking whether the `Context`'s style really changed after clicking.

use eframe::egui;
use model_harbor_app::theme::Theme;

fn input(events: Vec<egui::Event>) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(600.0, 400.0),
        )),
        events,
        ..Default::default()
    }
}

fn press(pos: egui::Pos2, down: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: down,
        modifiers: egui::Modifiers::default(),
    }
}

/// Run one frame of the top bar (replicating bars.rs's appearance panel); returns this frame's theme item's rect.
fn frame(
    ctx: &egui::Context,
    theme: &mut Theme,
    events: Vec<egui::Event>,
    btn_rect: &mut egui::Rect,
) -> Vec<(Theme, egui::Rect)> {
    let mut items = Vec::new();
    let _ = ctx.run(input(events), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let look_btn = ui.button("外观");
            *btn_rect = look_btn.rect;
            egui::Popup::menu(&look_btn)
                // Consistent with bars.rs.
                .layout(egui::Layout::top_down(egui::Align::LEFT))
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    ui.set_min_width(200.0);
                    // The nine themes lay out in a wrapping horizontal layout.
                    ui.horizontal_wrapped(|ui| {
                        for t in Theme::ALL {
                            let resp = ui.selectable_label(*theme == t, t.label());
                            items.push((t, resp.rect));
                            if resp.clicked() {
                                *theme = t;
                                t.apply(ctx);
                            }
                        }
                    });
                });
        });
    });
    items
}

#[test]
fn clicking_a_theme_menu_item_applies_the_theme() {
    let ctx = egui::Context::default();
    let mut theme = Theme::Dark;
    theme.apply(&ctx);
    let mut btn_rect = egui::Rect::ZERO;

    // 1) Let the buttons lay out, capturing the position
    let _ = frame(&ctx, &mut theme, vec![], &mut btn_rect);
    let btn_center = btn_rect.center();
    assert!(btn_center.x > 0.0, "按钮应有实际尺寸：{btn_rect:?}");

    // 2) Hover → press → release: opens the menu
    let _ = frame(
        &ctx,
        &mut theme,
        vec![egui::Event::PointerMoved(btn_center)],
        &mut btn_rect,
    );
    let _ = frame(
        &ctx,
        &mut theme,
        vec![press(btn_center, true)],
        &mut btn_rect,
    );
    let _ = frame(
        &ctx,
        &mut theme,
        vec![press(btn_center, false)],
        &mut btn_rect,
    );

    // 3) The menu is open; find the "light" item's rect
    let items = frame(&ctx, &mut theme, vec![], &mut btn_rect);
    let (target, item_rect) = items
        .iter()
        .find(|(t, _)| *t == Theme::Light)
        .copied()
        .expect("菜单应已展开并含「浅色」项");
    assert!(item_rect.width() > 0.0, "菜单项应有尺寸：{item_rect:?}");

    // 4) Click it: hover → press → release
    let item_center = item_rect.center();
    let _ = frame(
        &ctx,
        &mut theme,
        vec![egui::Event::PointerMoved(item_center)],
        &mut btn_rect,
    );
    let _ = frame(
        &ctx,
        &mut theme,
        vec![press(item_center, true)],
        &mut btn_rect,
    );
    let _ = frame(
        &ctx,
        &mut theme,
        vec![press(item_center, false)],
        &mut btn_rect,
    );

    assert_eq!(theme.key(), target.key(), "点击菜单项应切换主题");

    // Beyond the button's text, the actual style must change too.
    let reference = egui::Context::default();
    Theme::Light.apply(&reference);
    assert_eq!(
        ctx.style().visuals.panel_fill,
        reference.style().visuals.panel_fill,
        "点击后实际样式没变：只有按钮文字变了"
    );
    assert!(!ctx.style().visuals.dark_mode, "浅色主题应为 light 模式");
}

/// Menu items' left alignment and width: each item's fill should hug its text width, never stretching across the popup.
#[test]
fn menu_items_hug_their_text_instead_of_filling_the_popup() {
    let ctx = egui::Context::default();
    let mut theme = Theme::Dark;
    theme.apply(&ctx);
    let mut btn_rect = egui::Rect::ZERO;

    let _ = frame(&ctx, &mut theme, vec![], &mut btn_rect);
    let btn_center = btn_rect.center();
    let _ = frame(
        &ctx,
        &mut theme,
        vec![egui::Event::PointerMoved(btn_center)],
        &mut btn_rect,
    );
    let _ = frame(
        &ctx,
        &mut theme,
        vec![press(btn_center, true)],
        &mut btn_rect,
    );
    let _ = frame(
        &ctx,
        &mut theme,
        vec![press(btn_center, false)],
        &mut btn_rect,
    );

    let items = frame(&ctx, &mut theme, vec![], &mut btn_rect);
    assert!(!items.is_empty(), "菜单应已展开");
    // Within one line, left-to-right; after wrapping, the left edge returns to the line's start.
    let mut last_row_top = items[0].1.top();
    let mut last_left = f32::NEG_INFINITY;
    for (t, rect) in &items {
        if (rect.top() - last_row_top).abs() > 1.0 {
            // It wrapped: the left edge returns to the line's start, further left than the previous line's last item.
            assert!(
                rect.left() < last_left,
                "{} 换行后没回到行首：{:?}",
                t.key(),
                rect
            );
            last_row_top = rect.top();
        } else {
            assert!(
                rect.left() > last_left,
                "{} 没有从左到右排列：{:?}",
                t.key(),
                rect
            );
        }
        last_left = rect.left();
    }
    // Distinguish "hugging text" from "stretching" by absolute width: stretched, each item roughly equals the popup's width.
    let widths: Vec<f32> = items.iter().map(|(_, r)| r.width()).collect();
    let max = widths.iter().cloned().fold(0.0_f32, f32::max);
    assert!(
        max < 100.0,
        "菜单项被撑满了（宽度 {widths:?}），填充没有贴文字宽度"
    );
}
