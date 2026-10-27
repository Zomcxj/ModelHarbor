//! Shared UI widgets: grab cursor, card frames and decorations, drag handles, form controls, and toggles.
//!
//! The public API is always re-exported from here, so external `crate::ui::X` paths stay unchanged.

mod card;
mod cursor;
mod drag;
mod scroll;
mod widgets;

pub use card::{accent_bar_rect, bevel_segments, card_frame, ACCENT_BAR_HEIGHT};
pub use cursor::{grab_cursor_for, request_grab_cursor, take_grab_cursor, GrabCursor};
pub use drag::{
    drag_handle_dots, merge_drag_target, move_item, DragHandle, DRAG_HANDLE_GAP, DRAG_SOURCE_COLOR,
    DRAG_SOURCE_FILL, DROP_TARGET_COLOR, DROP_TARGET_FILL,
};
pub use scroll::paint_edge_fades;
pub use widgets::{
    card_list, empty_state, field_label, focus_ring, numeric_text_edit, secret_text_edit,
    text_edit, text_input, toggle_knob_center, toggle_knob_center_at, toggle_switch, TOGGLE_HEIGHT,
    TOGGLE_INSET, TOGGLE_KNOB, TOGGLE_WIDTH,
};

#[cfg(test)]
mod tests {
    use super::{
        drag_handle_dots, merge_drag_target, toggle_knob_center, toggle_knob_center_at,
        DRAG_HANDLE_GAP, TOGGLE_HEIGHT, TOGGLE_INSET, TOGGLE_KNOB, TOGGLE_WIDTH,
    };
    use eframe::egui;

    /// A focused text input carries the accent focus ring.
    ///
    /// egui's own focus frame draws in `selection.stroke` — kept as the on-accent ink here — so without
    /// the ring a focused input loses its outline entirely on both dark and light panels.
    #[test]
    fn focused_text_input_gets_the_accent_ring() {
        let ctx = egui::Context::default();
        crate::theme::Theme::Dark.apply_style(&ctx, crate::theme::UiStyle::default(), false);
        let accent = ctx.style().visuals.selection.bg_fill;
        let id = egui::Id::new("ring_probe");
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(300.0, 80.0),
            )),
            ..Default::default()
        };
        fn strokes(shape: &egui::Shape, out: &mut Vec<(egui::Color32, f32)>) {
            match shape {
                egui::Shape::Rect(r) => out.push((r.stroke.color, r.stroke.width)),
                egui::Shape::Vec(items) => items.iter().for_each(|s| strokes(s, out)),
                _ => {}
            }
        }
        let mut text = String::new();
        // First frame: the widget exists, focus is requested for the next one.
        let _ = ctx.run(input.clone(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                crate::ui::text_edit(ui, egui::TextEdit::singleline(&mut text).id(id));
                ui.memory_mut(|mem| mem.request_focus(id));
            });
        });
        // Second frame: focused, so the ring is painted.
        let out = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let resp = crate::ui::text_edit(ui, egui::TextEdit::singleline(&mut text).id(id));
                assert!(resp.has_focus(), "第二帧输入框应已取得焦点");
            });
        });
        let mut painted = Vec::new();
        for clipped in &out.shapes {
            strokes(&clipped.shape, &mut painted);
        }
        assert!(
            painted.iter().any(|(c, w)| *c == accent && *w > 1.0),
            "聚焦的输入框必须画出强调色聚焦环：{painted:?}"
        );
    }

    /// The empty state draws its title, hint, and action label.
    #[test]
    fn empty_state_renders_its_text() {
        let ctx = egui::Context::default();
        crate::theme::Theme::Dark.apply_style(&ctx, crate::theme::UiStyle::default(), false);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 200.0),
            )),
            ..Default::default()
        };
        fn texts(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(t) => out.push(t.galley.text().to_owned()),
                egui::Shape::Vec(items) => items.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let mut seen: Vec<String> = Vec::new();
        let mut clicked = true;
        let out = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                clicked = super::empty_state(ui, "没有数据", "先跑一轮对话", None, Some("去扫描"));
            });
        });
        for clipped in &out.shapes {
            texts(&clipped.shape, &mut seen);
        }
        for want in ["没有数据", "先跑一轮对话", "去扫描"] {
            assert!(
                seen.iter().any(|t| t.contains(want)),
                "空态缺少「{want}」：{seen:?}"
            );
        }
        assert!(!clicked, "未点击时不应报告动作被触发");
    }

    /// The ribbon covers only the card's top 3px.
    #[test]
    fn accent_bar_covers_only_the_top_strip() {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(200.0, 60.0));
        let bar = super::accent_bar_rect(rect);
        assert_eq!(bar.height(), super::ACCENT_BAR_HEIGHT);
        assert_eq!(bar.top(), rect.top());
        assert_eq!(bar.left(), rect.left());
        assert_eq!(bar.right(), rect.right());
        // Far smaller than the card height.
        assert!(bar.bottom() < rect.top() + 10.0, "色带侵入内容区：{bar:?}");
    }

    /// The dot matrix's geometric center coincides with the widget rect's center (even when `interact_size` stretches the widget).
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
            // The spacing IS the constant: two columns gap apart, three rows span 2*gap.
            assert!((max_x - min_x - DRAG_HANDLE_GAP).abs() < 0.01);
            assert!((max_y - min_y - 2.0 * DRAG_HANDLE_GAP).abs() < 0.01);
        }
    }

    /// The emboss lines sit inside the rect, avoiding the corners at both ends; the bright edge top/left, dark edge bottom/right.
    #[test]
    fn bevel_lines_stay_inside_and_clear_the_corners() {
        use super::bevel_segments;
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(120.0, 60.0));
        let radius = 4.0;
        let (light, dark) = bevel_segments(rect, radius);
        let [top, left] = light;
        // Both 1px inside the rect (the Frame's border takes that ring).
        assert!(top[0].y > rect.top() && top[0].y < rect.bottom());
        assert!(left[0].x > rect.left() && left[0].x < rect.right());
        assert!((top[0].x - (rect.left() + 1.0 + radius)).abs() < 0.01);
        assert!((top[1].x - (rect.right() - 1.0 - radius)).abs() < 0.01);
        assert!((left[0].y - (rect.top() + 1.0 + radius)).abs() < 0.01);
        assert!((left[1].y - (rect.bottom() - 1.0 - radius)).abs() < 0.01);
        // Dark edge bottom/right, symmetric to the bright edge.
        let [bottom, right] = dark;
        assert!(bottom[0].y < rect.bottom() && bottom[0].y > rect.top());
        assert!(right[0].x < rect.right() && right[0].x > rect.left());
        assert!((bottom[0].x - top[0].x).abs() < 0.01);
        assert!((right[0].y - left[0].y).abs() < 0.01);
    }

    /// When the corner radius exceeds half the rect, lines don't reverse.
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

    /// When several hot zones report states in one frame, take the stronger one (`Fist` beats `Palm`).
    #[test]
    fn grab_cursor_requests_take_the_strongest_state_in_a_frame() {
        use super::{take_grab_cursor, GrabCursor};

        let ctx = egui::Context::default();
        // No requests: default off, and cleared after take.
        assert_eq!(take_grab_cursor(&ctx), GrabCursor::None);
        assert_eq!(take_grab_cursor(&ctx), GrabCursor::None);

        // A None request itself writes no state.
        super::request_grab_cursor(&ctx, GrabCursor::None);
        assert_eq!(take_grab_cursor(&ctx), GrabCursor::None);

        // Palm first, fist later -> fist.
        super::request_grab_cursor(&ctx, GrabCursor::Palm);
        super::request_grab_cursor(&ctx, GrabCursor::Fist);
        assert_eq!(take_grab_cursor(&ctx), GrabCursor::Fist);

        // Fist first, palm later -> still fist.
        super::request_grab_cursor(&ctx, GrabCursor::Fist);
        super::request_grab_cursor(&ctx, GrabCursor::Palm);
        assert_eq!(take_grab_cursor(&ctx), GrabCursor::Fist);

        // Palm only -> palm.
        super::request_grab_cursor(&ctx, GrabCursor::Palm);
        assert_eq!(take_grab_cursor(&ctx), GrabCursor::Palm);

        // After take clears, the next frame starts from zero.
        assert_eq!(take_grab_cursor(&ctx), GrabCursor::None);
    }

    #[test]
    fn merge_drag_target_keeps_earlier_hit() {
        let mut acc = None;
        // The earlier-rendered card missed: the aggregate stays empty
        merge_drag_target(&mut acc, None);
        assert_eq!(acc, None);
        // A hit landing
        merge_drag_target(&mut acc, Some("p\u{1f}m".to_string()));
        assert_eq!(acc.as_deref(), Some("p\u{1f}m"));
        // A later card expanded and missed: don't clear the already-registered landing
        merge_drag_target(&mut acc, None);
        assert_eq!(acc.as_deref(), Some("p\u{1f}m"));
        // Once hit, later hits don't overwrite: the first landing is kept
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

    /// The knob sits fully inside the track in both states (padding included), and the two states separate.
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
        // The two states' centers separate.
        assert!(on.x > off.x, "开态应在右、关态应在左");
        // Vertically centered
        assert!((off.y - rect.center().y).abs() < 0.01);
        assert!((on.y - rect.center().y).abs() < 0.01);
    }

    /// The knob's diameter equals "track height − 2×padding".
    #[test]
    fn toggle_knob_diameter_follows_the_track() {
        assert!((TOGGLE_KNOB - (TOGGLE_HEIGHT - 2.0 * TOGGLE_INSET)).abs() < 0.01);
    }

    /// Off-state knob hugs the left, on-state the right, with symmetric side padding.
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

    /// Offscreen-render the toggle: both states paint a track (rounded rect) + knob (circle),
    /// with the on-state track's fill differing from the off-state's.
    ///
    /// Uses an offscreen `ctx.run` + a shape sweep.
    #[test]
    fn toggle_paints_a_track_and_a_knob_in_both_states() {
        use egui::epaint::Shape;
        // Collect two shape kinds: (rect fill, circle center)
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

    /// Click to toggle: the whole track is the hot zone; one click flips it.
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
        // First frame: get the toggle's actual rect.
        let out = ctx.run(input.clone(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut v = value;
                rect = super::toggle_switch(ui, egui::Id::new("t"), &mut v).rect;
                value = v;
            });
        });
        assert!(rect.is_positive(), "开关应占据正面积");
        let _ = out;
        // Second frame: press and release at the toggle's center -> value flips.
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

    /// Mid-animation frame: with progress in (0,1), the knob sits between the two states.
    #[test]
    fn toggle_knob_sweeps_between_the_two_ends() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(10.0, 20.0),
            egui::vec2(TOGGLE_WIDTH, TOGGLE_HEIGHT),
        );
        let off = toggle_knob_center(rect, false);
        let on = toggle_knob_center(rect, true);
        // Monotonic: greater progress sits further right.
        let mut prev = f32::NEG_INFINITY;
        for step in 0..=10 {
            let t = step as f32 / 10.0;
            let x = toggle_knob_center_at(rect, t).x;
            assert!(x > prev, "进度 {t} 处滑块没有继续向右：{x} <= {prev}");
            prev = x;
        }
        // Endpoints exactly equal the two states.
        assert!((toggle_knob_center_at(rect, 0.0).x - off.x).abs() < 0.01);
        assert!((toggle_knob_center_at(rect, 1.0).x - on.x).abs() < 0.01);
        // Mid-frames are genuinely in the middle.
        let mid = toggle_knob_center_at(rect, 0.5).x;
        assert!(
            mid > off.x + 1.0 && mid < on.x - 1.0,
            "进度 0.5 时滑块应在两态之间：off={} mid={mid} on={}",
            off.x,
            on.x
        );
    }

    /// The knob stays fully inside the track at any progress (out-of-range included).
    #[test]
    fn toggle_knob_stays_inside_at_every_progress() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(10.0, 20.0),
            egui::vec2(TOGGLE_WIDTH, TOGGLE_HEIGHT),
        );
        let radius = TOGGLE_KNOB / 2.0;
        // Out-of-range t (negative / greater than 1) gets clamped.
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

    /// `toggle_progress`'s deterministic path: the final value directly under debug / tests.
    #[test]
    fn toggle_progress_is_binary_when_everything_is_visible() {
        let ctx = egui::Context::default();
        ctx.memory_mut(|mem| mem.set_everything_is_visible(true));
        let id = egui::Id::new("toggle_anim");
        assert_eq!(crate::motion::toggle_progress(&ctx, id, false), 0.0);
        assert_eq!(crate::motion::toggle_progress(&ctx, id, true), 1.0);
    }

    /// The animation advances with time: the knob leaves the start and finally rests at the target.
    ///
    /// Driven by real time (`RawInput::time`; egui advances by time deltas).
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
        // Start: off state; register the id as 0 first.
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
        // Opened: progress immediately leaves 0, but hasn't reached 1.
        time += 1.0 / 60.0;
        let early = progress_at(time, true);
        assert!(early > 0.0, "开启动画没有推进：{early}");
        assert!(early < 1.0, "开启动画一帧就到位，没有滑动过程：{early}");
        // Advance past the animation duration: rests at 1.
        time += crate::motion::TOGGLE_TIME as f64 + 0.05;
        assert_eq!(progress_at(time, true), 1.0, "动画应收敛到 1");
        // Toggling back off also passes through the intermediate state.
        time += 1.0 / 60.0;
        let back = progress_at(time, false);
        assert!(back < 1.0 && back > 0.0, "关闭动画应在中间：{back}");
        time += crate::motion::TOGGLE_TIME as f64 + 0.05;
        assert_eq!(progress_at(time, false), 0.0, "动画应收敛到 0");
    }
}
