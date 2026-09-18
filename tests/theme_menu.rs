//! 主题菜单的交互回归：点菜单项必须**真的应用主题**（只换按钮文字 = bug）。
//!
//! 复刻 `src/app/bars.rs` 里顶栏「主题」按钮 + `Popup::menu` 的结构，
//! 用 headless 的 `egui::Context` 喂指针事件，检查点击后 `Context` 的样式是否真的换了。

use eframe::egui;
use model_harbor::theme::Theme;

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

/// 跑一帧顶栏（复刻 bars.rs 的「外观」面板），返回本帧主题项的矩形。
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
                // 与 bars.rs 一致：菜单默认 top_down_justified 会让填充撑满宽度。
                .layout(egui::Layout::top_down(egui::Align::LEFT))
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    ui.set_min_width(180.0);
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
    items
}

#[test]
fn clicking_a_theme_menu_item_applies_the_theme() {
    let ctx = egui::Context::default();
    let mut theme = Theme::Dark;
    theme.apply(&ctx);
    let mut btn_rect = egui::Rect::ZERO;

    // 1) 让按钮排好版，拿到它的位置
    let _ = frame(&ctx, &mut theme, vec![], &mut btn_rect);
    let btn_center = btn_rect.center();
    assert!(btn_center.x > 0.0, "按钮应有实际尺寸：{btn_rect:?}");

    // 2) 悬停 → 按下 → 松开：打开菜单
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

    // 3) 菜单已展开，找到「浅色」那一项的矩形
    let items = frame(&ctx, &mut theme, vec![], &mut btn_rect);
    let (target, item_rect) = items
        .iter()
        .find(|(t, _)| *t == Theme::Light)
        .copied()
        .expect("菜单应已展开并含「浅色」项");
    assert!(item_rect.width() > 0.0, "菜单项应有尺寸：{item_rect:?}");

    // 4) 点它：悬停 → 按下 → 松开
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

    // 关键：不只是按钮文字变了，**实际样式**也要换
    // （egui 0.33 每个主题各存一份 style，主题变了必须显式再 apply 一次）
    let reference = egui::Context::default();
    Theme::Light.apply(&reference);
    assert_eq!(
        ctx.style().visuals.panel_fill,
        reference.style().visuals.panel_fill,
        "点击后实际样式没变：只有按钮文字变了"
    );
    assert!(!ctx.style().visuals.dark_mode, "浅色主题应为 light 模式");
}

/// 菜单项的左对齐与宽度：每项填充应贴着文字宽度，不能撑满整个弹出宽度。
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
    // 所有项左边缘对齐（同一列）。
    let left = items[0].1.left();
    for (t, rect) in &items {
        assert!(
            (rect.left() - left).abs() < 1.0,
            "{} 项没左对齐：{:?}",
            t.key(),
            rect
        );
    }
    // 五个主题名都是两个汉字，宽度本来就一样；能区分「贴文字」与「撑满」的是
    // **绝对宽度**：撑满时每项会等于弹出宽度（180），贴文字时只有三十几像素。
    let widths: Vec<f32> = items.iter().map(|(_, r)| r.width()).collect();
    let max = widths.iter().cloned().fold(0.0_f32, f32::max);
    assert!(
        max < 100.0,
        "菜单项被撑满了（宽度 {widths:?}），填充没有贴文字宽度"
    );
}
