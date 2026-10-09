//! 界面通用小件：抓取光标、卡片外框与装饰、拖拽把手、表单控件与滑动开关。
//!
//! 公共 API 一律从这里再导出，外部 `crate::ui::X` 路径不变。

mod card;
mod cursor;
mod drag;
mod widgets;

pub use card::{accent_bar_rect, bevel_segments, card_frame, ACCENT_BAR_HEIGHT};
pub use cursor::{grab_cursor_for, request_grab_cursor, take_grab_cursor, GrabCursor};
pub use drag::{
    drag_handle_dots, merge_drag_target, move_item, DragHandle, DRAG_HANDLE_GAP, DRAG_SOURCE_COLOR,
    DRAG_SOURCE_FILL, DROP_TARGET_COLOR, DROP_TARGET_FILL,
};
pub use widgets::{
    card_list, field_label, numeric_text_edit, secret_text_edit, toggle_knob_center,
    toggle_knob_center_at, toggle_switch, TOGGLE_HEIGHT, TOGGLE_INSET, TOGGLE_KNOB, TOGGLE_WIDTH,
};

#[cfg(test)]
mod tests {
    use super::{
        drag_handle_dots, merge_drag_target, toggle_knob_center, toggle_knob_center_at,
        DRAG_HANDLE_GAP, TOGGLE_HEIGHT, TOGGLE_INSET, TOGGLE_KNOB, TOGGLE_WIDTH,
    };
    use eframe::egui;

    /// 色带只覆盖卡片顶部 3px。
    #[test]
    fn accent_bar_covers_only_the_top_strip() {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(200.0, 60.0));
        let bar = super::accent_bar_rect(rect);
        assert_eq!(bar.height(), super::ACCENT_BAR_HEIGHT);
        assert_eq!(bar.top(), rect.top());
        assert_eq!(bar.left(), rect.left());
        assert_eq!(bar.right(), rect.right());
        // 远小于卡片高度。
        assert!(bar.bottom() < rect.top() + 10.0, "色带侵入内容区：{bar:?}");
    }

    /// 点阵的几何中心与控件矩形中心重合（控件被 `interact_size` 撑高时也一样）。
    #[test]
    fn drag_handle_dots_are_centered_in_the_rect() {
        for rect in [
            egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(14.0, 18.0)),
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(14.0, 24.0)),
            egui::Rect::from_min_size(egui::pos2(3.0, 7.0), egui::vec2(14.0, 30.0)),
        ] {
            let dots = drag_handle_dots(rect);
            assert_eq!(dots.len(), 6);
            let min_x = dots.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
            let max_x = dots.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
            let min_y = dots.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
            let max_y = dots.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
            let cx = (min_x + max_x) / 2.0;
            let cy = (min_y + max_y) / 2.0;
            assert!(
                (cx - rect.center().x).abs() < 0.01 && (cy - rect.center().y).abs() < 0.01,
                "点阵中心 ({cx}, {cy}) 与控件中心 {:?} 不重合",
                rect.center()
            );
            // 间距就是常量：两列相距 gap，三行跨 2*gap。
            assert!((max_x - min_x - DRAG_HANDLE_GAP).abs() < 0.01);
            assert!((max_y - min_y - 2.0 * DRAG_HANDLE_GAP).abs() < 0.01);
        }
    }

    /// 浮雕线落在矩形内侧，两端避开圆角；亮边在上 / 左，暗边在下 / 右。
    #[test]
    fn bevel_lines_stay_inside_and_clear_the_corners() {
        use super::bevel_segments;
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(120.0, 60.0));
        let radius = 4.0;
        let (light, dark) = bevel_segments(rect, radius);
        let [top, left] = light;
        // 都在矩形内侧 1px（Frame 的描边占掉那一圈）。
        assert!(top[0].y > rect.top() && top[0].y < rect.bottom());
        assert!(left[0].x > rect.left() && left[0].x < rect.right());
        assert!((top[0].x - (rect.left() + 1.0 + radius)).abs() < 0.01);
        assert!((top[1].x - (rect.right() - 1.0 - radius)).abs() < 0.01);
        assert!((left[0].y - (rect.top() + 1.0 + radius)).abs() < 0.01);
        assert!((left[1].y - (rect.bottom() - 1.0 - radius)).abs() < 0.01);
        // 暗边在下 / 右，与亮边对称。
        let [bottom, right] = dark;
        assert!(bottom[0].y < rect.bottom() && bottom[0].y > rect.top());
        assert!(right[0].x < rect.right() && right[0].x > rect.left());
        assert!((bottom[0].x - top[0].x).abs() < 0.01);
        assert!((right[0].y - left[0].y).abs() < 0.01);
    }

    /// 圆角超过矩形一半时，线段不反向。
    #[test]
    fn bevel_lines_stay_ordered_with_a_huge_radius() {
        use super::bevel_segments;
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(10.0, 8.0));
        let (light, dark) = bevel_segments(rect, 40.0);
        let [top, left] = light;
        assert!(top[0].x <= top[1].x, "上边线反向了：{top:?}");
        assert!(left[0].y <= left[1].y, "左边线反向了：{left:?}");
        let [bottom, right] = dark;
        assert!(bottom[0].x <= bottom[1].x, "下边线反向了：{bottom:?}");
        assert!(right[0].y <= right[1].y, "右边线反向了：{right:?}");
    }

    /// 同一帧里多个热区报状态时取更强的一态（`Fist` 优先于 `Palm`）。
    #[test]
    fn grab_cursor_requests_take_the_strongest_state_in_a_frame() {
        use super::{take_grab_cursor, GrabCursor};

        let ctx = egui::Context::default();
        // 没有任何请求：默认熄灭，且 take 之后清空。
        assert_eq!(take_grab_cursor(&ctx), GrabCursor::None);
        assert_eq!(take_grab_cursor(&ctx), GrabCursor::None);

        // None 请求本身不写入任何状态。
        super::request_grab_cursor(&ctx, GrabCursor::None);
        assert_eq!(take_grab_cursor(&ctx), GrabCursor::None);

        // 手掌先到、拳头后到 -> 拳头。
        super::request_grab_cursor(&ctx, GrabCursor::Palm);
        super::request_grab_cursor(&ctx, GrabCursor::Fist);
        assert_eq!(take_grab_cursor(&ctx), GrabCursor::Fist);

        // 拳头先到、手掌后到 -> 仍是拳头。
        super::request_grab_cursor(&ctx, GrabCursor::Fist);
        super::request_grab_cursor(&ctx, GrabCursor::Palm);
        assert_eq!(take_grab_cursor(&ctx), GrabCursor::Fist);

        // 只有手掌 -> 手掌。
        super::request_grab_cursor(&ctx, GrabCursor::Palm);
        assert_eq!(take_grab_cursor(&ctx), GrabCursor::Palm);

        // take 清空后，下一帧从零开始。
        assert_eq!(take_grab_cursor(&ctx), GrabCursor::None);
    }

    #[test]
    fn merge_drag_target_keeps_earlier_hit() {
        let mut acc = None;
        // 先渲染的卡片未命中：聚合结果保持为空
        merge_drag_target(&mut acc, None);
        assert_eq!(acc, None);
        // 命中落点
        merge_drag_target(&mut acc, Some("p\u{1f}m".to_string()));
        assert_eq!(acc.as_deref(), Some("p\u{1f}m"));
        // 后面还有卡片展开且未命中：不清掉已命中的落点
        merge_drag_target(&mut acc, None);
        assert_eq!(acc.as_deref(), Some("p\u{1f}m"));
        // 已命中时后续命中不覆盖：保持首个落点
        merge_drag_target(&mut acc, Some("other".to_string()));
        assert_eq!(acc.as_deref(), Some("p\u{1f}m"));
    }

    #[test]
    fn merge_drag_target_stays_none_without_hit() {
        let mut acc = None;
        merge_drag_target(&mut acc, None);
        merge_drag_target(&mut acc, None);
        assert_eq!(acc, None);
    }

    /// 滑块在两态下都完整落在轨道内（含内边距），且左右分开。
    #[test]
    fn toggle_knob_stays_inside_the_track() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(10.0, 20.0),
            egui::vec2(TOGGLE_WIDTH, TOGGLE_HEIGHT),
        );
        let radius = TOGGLE_KNOB / 2.0;
        let off = toggle_knob_center(rect, false);
        let on = toggle_knob_center(rect, true);
        for (label, c) in [("off", off), ("on", on)] {
            assert!(
                c.x - radius >= rect.left() - 0.01 && c.x + radius <= rect.right() + 0.01,
                "{label} 态滑块越出轨道：{:?}",
                c
            );
            assert!(
                c.y - radius >= rect.top() - 0.01 && c.y + radius <= rect.bottom() + 0.01,
                "{label} 态滑块越出轨道（纵向）：{:?}",
                c
            );
        }
        // 两态圆心分开。
        assert!(on.x > off.x, "开态应在右、关态应在左");
        // 垂直居中
        assert!((off.y - rect.center().y).abs() < 0.01);
        assert!((on.y - rect.center().y).abs() < 0.01);
    }

    /// 滑块直径等于「轨道高 − 2×内边距」。
    #[test]
    fn toggle_knob_diameter_follows_the_track() {
        assert!((TOGGLE_KNOB - (TOGGLE_HEIGHT - 2.0 * TOGGLE_INSET)).abs() < 0.01);
    }

    /// 关态滑块贴左、开态贴右，且左右内边距对称。
    #[test]
    fn toggle_knob_inset_is_symmetric() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(TOGGLE_WIDTH, TOGGLE_HEIGHT),
        );
        let radius = TOGGLE_KNOB / 2.0;
        let off = toggle_knob_center(rect, false);
        let on = toggle_knob_center(rect, true);
        let left_gap = off.x - radius - rect.left();
        let right_gap = rect.right() - (on.x + radius);
        assert!(
            (left_gap - right_gap).abs() < 0.01,
            "左右内边距应一致：{left_gap} vs {right_gap}"
        );
        assert!((left_gap - TOGGLE_INSET).abs() < 0.01);
    }

    /// 离屏渲染开关：两态都画出轨道（圆角矩形）+ 滑块（圆），
    /// 开态轨道的填充色与关态不同。
    ///
    /// 用离屏 `ctx.run` + 形状遍历。
    #[test]
    fn toggle_paints_a_track_and_a_knob_in_both_states() {
        use egui::epaint::Shape;
        // 收集 (矩形填充色, 圆心) 两类形状
        fn collect(shape: &Shape, rects: &mut Vec<egui::Color32>, knobs: &mut Vec<egui::Pos2>) {
            match shape {
                Shape::Rect(r) => rects.push(r.fill),
                Shape::Circle(c) => knobs.push(c.center),
                Shape::Vec(items) => {
                    for item in items {
                        collect(item, rects, knobs);
                    }
                }
                _ => {}
            }
        }
        let run = |on: bool| {
            let ctx = egui::Context::default();
            crate::theme::Theme::Dark.apply_style(
                &ctx,
                crate::theme::UiStyle::from_key("cloud"),
                false,
            );
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(200.0, 60.0),
                )),
                ..Default::default()
            };
            let mut value = on;
            let out = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut v = value;
                    super::toggle_switch(ui, egui::Id::new("t"), &mut v);
                    value = v;
                });
            });
            let mut rects = Vec::new();
            let mut knobs = Vec::new();
            for clipped in &out.shapes {
                collect(&clipped.shape, &mut rects, &mut knobs);
            }
            (rects, knobs, value)
        };

        let (off_rects, off_knobs, off_value) = run(false);
        let (on_rects, on_knobs, on_value) = run(true);
        assert!(!off_value, "关态渲染不应自行切换");
        assert!(on_value, "开态渲染不应自行切换");
        assert!(!off_knobs.is_empty(), "两态都应画出滑块圆点");
        assert_eq!(on_knobs.len(), off_knobs.len());
        assert!(!off_rects.is_empty(), "两态都应画出轨道");
        assert_ne!(
            on_rects, off_rects,
            "开/关的轨道填充必须不同，否则看不出状态"
        );
    }

    /// 点击切换：整块轨道都是热区，点一下就翻转。
    #[test]
    fn toggle_click_flips_the_value() {
        let ctx = egui::Context::default();
        crate::theme::Theme::Dark.apply_style(
            &ctx,
            crate::theme::UiStyle::from_key("cloud"),
            false,
        );
        let mut value = false;
        let mut rect = egui::Rect::NOTHING;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(200.0, 60.0),
            )),
            ..Default::default()
        };
        // 第一帧：拿到开关的实际矩形。
        let out = ctx.run(input.clone(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut v = value;
                rect = super::toggle_switch(ui, egui::Id::new("t"), &mut v).rect;
                value = v;
            });
        });
        assert!(rect.is_positive(), "开关应占据正面积");
        let _ = out;
        // 第二帧：在开关中心按下并抬起 -> 值翻转。
        let center = rect.center();
        let mut input2 = input.clone();
        input2.events = vec![
            egui::Event::PointerMoved(center),
            egui::Event::PointerButton {
                pos: center,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
            egui::Event::PointerButton {
                pos: center,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            },
        ];
        let _ = ctx.run(input2, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut v = value;
                super::toggle_switch(ui, egui::Id::new("t"), &mut v);
                value = v;
            });
        });
        assert!(value, "点击后应从关变开");
    }

    /// 动画中间帧：进度在 (0,1) 之间时，滑块落在两态之间。
    #[test]
    fn toggle_knob_sweeps_between_the_two_ends() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(10.0, 20.0),
            egui::vec2(TOGGLE_WIDTH, TOGGLE_HEIGHT),
        );
        let off = toggle_knob_center(rect, false);
        let on = toggle_knob_center(rect, true);
        // 单调：进度越大越靠右。
        let mut prev = f32::NEG_INFINITY;
        for step in 0..=10 {
            let t = step as f32 / 10.0;
            let x = toggle_knob_center_at(rect, t).x;
            assert!(x > prev, "进度 {t} 处滑块没有继续向右：{x} <= {prev}");
            prev = x;
        }
        // 端点精确等于两态。
        assert!((toggle_knob_center_at(rect, 0.0).x - off.x).abs() < 0.01);
        assert!((toggle_knob_center_at(rect, 1.0).x - on.x).abs() < 0.01);
        // 中间帧确实在中间。
        let mid = toggle_knob_center_at(rect, 0.5).x;
        assert!(
            mid > off.x + 1.0 && mid < on.x - 1.0,
            "进度 0.5 时滑块应在两态之间：off={} mid={mid} on={}",
            off.x,
            on.x
        );
    }

    /// 任意进度（含越界值）下滑块都完整留在轨道内。
    #[test]
    fn toggle_knob_stays_inside_at_every_progress() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(10.0, 20.0),
            egui::vec2(TOGGLE_WIDTH, TOGGLE_HEIGHT),
        );
        let radius = TOGGLE_KNOB / 2.0;
        // 越界的 t（负数 / 大于 1）会被 clamp。
        for t in [-1.0, -0.001, 0.0, 0.25, 0.5, 0.75, 1.0, 1.001, 2.0] {
            let c = toggle_knob_center_at(rect, t);
            assert!(
                c.x - radius >= rect.left() - 0.01 && c.x + radius <= rect.right() + 0.01,
                "进度 {t} 处滑块越出轨道：{c:?}"
            );
            assert!(
                c.y - radius >= rect.top() - 0.01 && c.y + radius <= rect.bottom() + 0.01,
                "进度 {t} 处滑块纵向越出轨道：{c:?}"
            );
        }
    }

    /// `toggle_progress` 的确定性路径：调试 / 单测下直接给终值。
    #[test]
    fn toggle_progress_is_binary_when_everything_is_visible() {
        let ctx = egui::Context::default();
        ctx.memory_mut(|mem| mem.set_everything_is_visible(true));
        let id = egui::Id::new("toggle_anim");
        assert_eq!(crate::motion::toggle_progress(&ctx, id, false), 0.0);
        assert_eq!(crate::motion::toggle_progress(&ctx, id, true), 1.0);
    }

    /// 动画随时间推进：滑块离开起点，最终停到终点。
    ///
    /// 用真实时间（`RawInput::time`）驱动（egui 按时间差推进）。
    #[test]
    fn toggle_animates_towards_the_target_over_time() {
        let ctx = egui::Context::default();
        crate::theme::Theme::Dark.apply_style(
            &ctx,
            crate::theme::UiStyle::from_key("cloud"),
            false,
        );
        let id = egui::Id::new("toggle_anim");
        let base = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(200.0, 60.0),
            )),
            ..Default::default()
        };
        // 起始：关态，先把 id 登记成 0。
        let mut time = 0.0f64;
        let progress_at = |time: f64, on: bool| {
            let mut input = base.clone();
            input.time = Some(time);
            let mut out = 0.0f32;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    out = crate::motion::toggle_progress(ui.ctx(), id, on);
                });
            });
            out
        };
        assert_eq!(progress_at(time, false), 0.0, "首帧未登记 -> 直接给终值 0");
        // 打开：进度立刻开始离开 0，但还没到 1。
        time += 1.0 / 60.0;
        let early = progress_at(time, true);
        assert!(early > 0.0, "开启动画没有推进：{early}");
        assert!(early < 1.0, "开启动画一帧就到位，没有滑动过程：{early}");
        // 推进超过动画时长：停在 1。
        time += crate::motion::TOGGLE_TIME as f64 + 0.05;
        assert_eq!(progress_at(time, true), 1.0, "动画应收敛到 1");
        // 关回去同样要经过中间态。
        time += 1.0 / 60.0;
        let back = progress_at(time, false);
        assert!(back < 1.0 && back > 0.0, "关闭动画应在中间：{back}");
        time += crate::motion::TOGGLE_TIME as f64 + 0.05;
        assert_eq!(progress_at(time, false), 0.0, "动画应收敛到 0");
    }
}
