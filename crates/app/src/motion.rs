//! UI motion: card border hover transitions, card-body collapse / expand height animation, and toggle knob tweening.
//!
//! All durations are 0.1–0.2 seconds.

use eframe::egui::{self, Color32};

/// Hover border transition duration (seconds).
pub const HOVER_TIME: f32 = 0.12;

/// Collapse / expand duration (seconds).
pub const COLLAPSE_TIME: f32 = 0.18;

/// Toggle knob travel duration (seconds).
pub const TOGGLE_TIME: f32 = 0.14;

/// Per-digit number roll duration (seconds).
///
/// One step longer than hover / collapse: the roll must let people see "the old digits roll out, the new
/// roll in" — too fast reads as a flicker. The reference implementation (token-monitor v0.70.0's digit
/// roll) is also in the half-second range.
pub const DIGIT_ROLL_TIME: f32 = 0.45;

/// Click acknowledgment pulse duration (seconds).
pub const PULSE_TIME: f32 = 0.22;

/// Theme cross-fade duration (seconds).
///
/// Only for the palette interpolation on a theme switch: long enough to read as a transition, short
/// enough that the new colors settle before the click feels stale.
pub const THEME_TIME: f32 = 0.22;

/// Compile-time upper-bound check on animation durations.
const _: () = {
    assert!(HOVER_TIME <= 0.2, "悬停过渡时长超过 0.2s，界面会显得拖沓");
    assert!(
        COLLAPSE_TIME <= 0.25,
        "折叠动画时长超过 0.25s，界面会显得拖沓"
    );
    assert!(TOGGLE_TIME <= 0.2, "开关滑动时长超过 0.2s，点击会显得迟钝");
    assert!(TOGGLE_TIME > 0.0, "开关滑动时长不能为零，否则没有滑动过程");
    assert!(
        DIGIT_ROLL_TIME <= 0.6,
        "数字滚动时长超过 0.6s，读数会显得迟滞"
    );
    assert!(
        DIGIT_ROLL_TIME > 0.0,
        "数字滚动时长不能为零，否则没有滚动过程"
    );
    assert!(
        PULSE_TIME <= 0.25,
        "click pulse over 0.25s reads as lag, not acknowledgment"
    );
    assert!(PULSE_TIME > 0.0, "click pulse duration must be positive");
    assert!(
        THEME_TIME <= 0.25,
        "theme cross-fade over 0.25s reads as lag, not a transition"
    );
    assert!(
        THEME_TIME > 0.0,
        "theme cross-fade duration must be positive"
    );
};

/// Per-channel interpolation in gamma space (premultiplied alpha interpolated as-is).
///
/// `t` is clamped to `[0, 1]`.
pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgba_premultiplied(
        mix(a.r(), b.r()),
        mix(a.g(), b.g()),
        mix(a.b(), b.b()),
        mix(a.a(), b.a()),
    )
}

/// The single easing curve for every duration-driven animation.
///
/// All of them used to run at `linear` (egui's `animate_bool_with_time` default): motion that neither
/// accelerates nor decelerates reads as mechanical, and a widget sliding at a constant rate looks like it is
/// being dragged rather than settling. Ease-out means "fast start, gentle stop", which is what makes a
/// transition feel like it *arrives*.
///
/// One curve for hover / collapse / toggle / theme cross-fade / click pulse / digit roll, rather than a curve
/// per feature: mixing curves across one interface is what makes the parts feel assembled instead of designed.
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Theme cross-fade progress since `start` (0 = the previous palette, 1 = the new one).
///
/// Returns the **raw** (linear) fraction: callers needing the eased blend apply [`ease_out`] themselves, and
/// the `>= 1.0` completion check must read the raw value. Under `everything_is_visible` (debug / offscreen
/// tests) the fade is skipped entirely.
pub fn fade_progress(ctx: &egui::Context, start: f64) -> f32 {
    if ctx.memory(|mem| mem.everything_is_visible()) {
        return 1.0;
    }
    let now = ctx.input(|input| input.time);
    let t = ((now - start) as f32 / THEME_TIME).clamp(0.0, 1.0);
    if t < 1.0 {
        ctx.request_repaint();
    }
    t
}

/// Hover interpolation factor (0 = not hovered, 1 = hovered).
///
/// Under `everything_is_visible` (debug mode) the final value returns directly.
pub fn hover_t(ctx: &egui::Context, id: egui::Id, hovered: bool) -> f32 {
    if ctx.memory(|mem| mem.everything_is_visible()) {
        return hovered as u8 as f32;
    }
    ctx.animate_bool_with_time_and_easing(id, hovered, HOVER_TIME, ease_out)
}

/// Collapse/expand openness (0 = fully closed, 1 = fully open).
pub fn collapse_openness(ctx: &egui::Context, id: egui::Id, open: bool) -> f32 {
    if ctx.memory(|mem| mem.everything_is_visible()) {
        return open as u8 as f32;
    }
    ctx.animate_bool_with_time_and_easing(id.with("collapse"), open, COLLAPSE_TIME, ease_out)
}

/// Toggle knob progress (0 = left / off, 1 = right / on).
///
/// Unregistered ids return the final value on the first frame, so switches don't slide in from the left
/// when a page opens — only state changes caused by clicks animate.
///
/// Under `everything_is_visible` (debug mode) the final value returns directly.
pub fn toggle_progress(ctx: &egui::Context, id: egui::Id, on: bool) -> f32 {
    if ctx.memory(|mem| mem.everything_is_visible()) {
        return on as u8 as f32;
    }
    ctx.animate_bool_with_time_and_easing(id.with("toggle"), on, TOGGLE_TIME, ease_out)
}

/// Pin the collapse animation to its final value immediately (zero duration). Used for bulk expand / collapse.
pub fn snap_collapse(ctx: &egui::Context, id: egui::Id, open: bool) {
    ctx.animate_bool_with_time(id.with("collapse"), open, 0.0);
}

/// The click acknowledgment pulse: an accent ring expanding past the widget's edge, thinning and fading out.
///
/// Call right after creating a clickable widget. When `response` reports a click, the pulse records the
/// moment and paints for [`PULSE_TIME`]: a rounded-rect ring grows ~6px beyond the widget's edge while
/// thinning and fading — the click's visual receipt, without touching layout. Rapid re-clicks restart the
/// pulse from the new click, so machine-gun clicking stays lively instead of stacking rings.
///
/// Under `everything_is_visible` (debug / offscreen tests) pulses are suppressed entirely: no temp state,
/// no shapes, no repaint requests.
pub fn click_pulse(
    ui: &mut egui::Ui,
    response: &egui::Response,
    color: Color32,
    corner: egui::CornerRadius,
) {
    let ctx = ui.ctx().clone();
    if ctx.memory(|mem| mem.everything_is_visible()) {
        return;
    }
    let id = response.id.with("click_pulse");
    let now = ctx.input(|input| input.time);
    if response.clicked() {
        ctx.data_mut(|data| data.insert_temp(id, now));
    }
    let Some(start) = ctx.data(|data| data.get_temp::<f64>(id)) else {
        return;
    };
    let t = ((now - start) as f32 / PULSE_TIME).clamp(0.0, 1.0);
    if t >= 1.0 {
        ctx.data_mut(|data| data.remove_temp::<f64>(id));
        return;
    }
    ctx.request_repaint();
    // The shared ease-out: the ring snaps out, then dissolves.
    let eased = ease_out(t);
    let grow = 6.0 * eased;
    let fade = 0.55 * (1.0 - eased);
    let stroke = egui::Stroke::new(2.5 * (1.0 - eased) + 0.5, color.gamma_multiply(fade));
    let wide = |v: u8| v.saturating_add(grow as u8);
    let ring_corner = egui::CornerRadius {
        nw: wide(corner.nw),
        ne: wide(corner.ne),
        sw: wide(corner.sw),
        se: wide(corner.se),
    };
    ui.painter().rect_stroke(
        response.rect.expand(grow),
        ring_corner,
        stroke,
        egui::StrokeKind::Inside,
    );
}

/// Per-digit number roll: each digit rolls vertically inside its slot; non-digits (commas, dots, units) stay put.
///
/// Mirrors token-monitor v0.70.0's default animation (digit roll, #1014): totals and breakdowns roll
/// digit-by-digit when switching, instead of the whole string flashing.
///
/// Slots: width from the **final value's** digit width (layout doesn't jump mid-roll), height one line.
/// Tween 0 = only the old digit, 1 = only the new: the old digit slides up out of the slot while the new
/// rolls in from below; overflow is clipped. Added high digits roll in from below too.
///
/// State lives in three temp keys: `id` = this roll's start value, `id/target` = target value,
/// `id/roll_start` = start moment. If the value changes mid-roll: the previous roll pins to its target
/// and the new roll starts from there (no backward jumps, matching the reference's #1025 cap on roll depth).
///
/// The whole string's rect is allocated and returned via [`egui::Ui::allocate_exact_size`]: the same
/// layout behavior as `Label`, vertically centered in the line — sharing the line center with adjacent
/// swatches and labels, never offset half a line height like top-painted text.
///
/// Under `everything_is_visible` (debug mode) and on the first frame, the final value paints directly
/// with no roll, so layout tests get stable text.
pub fn rolling_number(
    ui: &mut egui::Ui,
    id: egui::Id,
    text: &str,
    rich: impl Fn(&str) -> egui::RichText + Copy,
) -> egui::Rect {
    let ctx = ui.ctx().clone();
    let target_id = id.with("target");
    let start_id = id.with("roll_start");
    let now = ctx.input(|input| input.time);

    let mut from = ctx
        .data(|data| data.get_temp::<String>(id))
        .unwrap_or_else(|| text.to_string());
    let mut target = ctx
        .data(|data| data.get_temp::<String>(target_id))
        .unwrap_or_else(|| from.clone());

    if ctx.memory(|mem| mem.everything_is_visible()) {
        // Debug mode: pin the final value, clear state.
        from = text.to_string();
        target = from.clone();
        ctx.data_mut(|data| {
            data.insert_temp(id, from.clone());
            data.remove_temp::<String>(target_id);
            data.remove_temp::<f64>(start_id);
        });
    } else if text != target {
        // The value changed: the previous round (if any) pins to its target, and the new roll starts from there.
        from = target;
        target = text.to_string();
        ctx.data_mut(|data| {
            data.insert_temp(id, from.clone());
            data.insert_temp(target_id, target.clone());
            data.insert_temp(start_id, now);
        });
    } else {
        // Value unchanged: note the currently displayed value as the next change's roll start.
        ctx.data_mut(|data| data.insert_temp(id, text.to_string()));
    }

    let rolling = from != target;
    let t = if rolling {
        let start = ctx
            .data(|data| data.get_temp::<f64>(start_id))
            .unwrap_or(now);
        ((now - start) as f32 / DIGIT_ROLL_TIME).clamp(0.0, 1.0)
    } else {
        1.0
    };
    if rolling && t < 1.0 {
        ctx.request_repaint();
    }
    if rolling && t >= 1.0 {
        // Roll finished: the start value becomes the target; clear timing.
        ctx.data_mut(|data| {
            data.insert_temp(id, target.clone());
            data.remove_temp::<f64>(start_id);
        });
    }
    // The shared ease-out: fast then slow, so the finish doesn't jar.
    let eased = ease_out(t);

    let chars: Vec<char> = target.chars().collect();
    let from_chars: Vec<char> = from.chars().collect();
    // Align digits right-to-left: last digit to last digit.
    let offset = chars.len().saturating_sub(from_chars.len());
    let fallback = ui.visuals().text_color();

    // Measure the whole string first: one galley per digit; only with total width and line height can rects be requested from layout.
    let galleys: Vec<_> = chars
        .iter()
        .map(|ch| {
            egui::WidgetText::from(rich(&ch.to_string())).into_galley(
                ui,
                Some(egui::TextWrapMode::Extend),
                f32::INFINITY,
                egui::TextStyle::Body,
            )
        })
        .collect();
    let width = galleys.iter().map(|g| g.size().x).sum::<f32>();
    let height = galleys.iter().map(|g| g.size().y).fold(0.0f32, f32::max);
    // Through layout allocation (not painted flush at the cursor): vertical centering matches Label / swatches.
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());

    let mut x = rect.min.x;
    let top = rect.min.y;
    let mut painter = ui.painter().clone();
    for (index, ch) in chars.iter().enumerate() {
        let galley = galleys[index].clone();
        let size = galley.size();
        let slot = egui::Rect::from_min_size(egui::pos2(x, top), egui::vec2(size.x, size.y));
        let old_ch = index
            .checked_sub(offset)
            .and_then(|i| from_chars.get(i))
            .copied();
        let rolls = rolling && t < 1.0 && ch.is_ascii_digit() && old_ch != Some(*ch);
        if !rolls {
            painter.galley(slot.min, galley, fallback);
        } else {
            let clip = painter.clip_rect();
            painter.set_clip_rect(slot);
            if let Some(old_ch) = old_ch {
                // The old digit: slides up out of the slot with the tween.
                let old_galley = egui::WidgetText::from(rich(&old_ch.to_string())).into_galley(
                    ui,
                    Some(egui::TextWrapMode::Extend),
                    f32::INFINITY,
                    egui::TextStyle::Body,
                );
                painter.galley(
                    slot.min - egui::vec2(0.0, size.y * eased),
                    old_galley,
                    fallback,
                );
            }
            // The new digit: rolls in from below the slot.
            painter.galley(
                slot.min + egui::vec2(0.0, size.y * (1.0 - eased)),
                galley,
                fallback,
            );
            painter.set_clip_rect(clip);
        }
        x += size.x;
    }
    rect
}

/// Collapsible body with height animation: returns `None` while collapsed and the animation has finished (not rendered this frame).
///
/// Mid-animation, the parent layout first reserves the tweened height, then the body paints into an
/// independent subtree at the same position, clipped to the excess.
///
/// Fully open and mid-animation share one global `id`; full height is measured per frame into temp
/// storage, starting from a 10px offset while unknown.
pub fn animated_collapse<R>(
    ui: &mut egui::Ui,
    id: egui::Id,
    open: bool,
    add_body: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    let ctx = ui.ctx().clone();
    let openness = collapse_openness(&ctx, id, open);
    let height_id = id.with("open_height");
    if openness <= 0.0 {
        return None;
    }
    if openness >= 1.0 {
        let inner = ui.scope_builder(egui::UiBuilder::new().id(id), add_body);
        ctx.data_mut(|data| data.insert_temp(height_id, inner.response.rect.height()));
        return Some(inner.inner);
    }
    let stored = ctx
        .data(|data| data.get_temp::<f32>(height_id))
        .filter(|height| *height > 0.0);
    let known = stored.unwrap_or(10.0);
    let band = known * openness;
    let (band_rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), band), egui::Sense::hover());
    // The subtree gets full height (a page-sized placeholder while unknown).
    let child_max = egui::Rect::from_min_size(
        band_rect.min,
        egui::vec2(band_rect.width(), stored.unwrap_or(4096.0).max(known)),
    );
    let mut child = ui.new_child(egui::UiBuilder::new().id(id).max_rect(child_max));
    child.set_clip_rect(child.clip_rect().intersect(band_rect));
    let ret = add_body(&mut child);
    let measured = child.min_rect().height();
    if measured > 0.0 {
        ctx.data_mut(|data| data.insert_temp(height_id, measured));
    }
    Some(ret)
}

#[cfg(test)]
mod tests {
    use super::{animated_collapse, lerp_color, rolling_number};
    use eframe::egui;

    /// Collect all text shapes' positions (including nested).
    fn texts_at(out: &egui::FullOutput) -> Vec<(String, f32, f32)> {
        let mut hits = Vec::new();
        fn collect(shape: &egui::Shape, hits: &mut Vec<(String, f32, f32)>) {
            match shape {
                egui::Shape::Text(t) => {
                    hits.push((t.galley.text().to_string(), t.pos.x, t.pos.y));
                }
                egui::Shape::Vec(items) => items.iter().for_each(|s| collect(s, hits)),
                _ => {}
            }
        }
        for cs in &out.shapes {
            collect(&cs.shape, &mut hits);
        }
        hits
    }

    /// Run one frame; returns the output and the readout rect.
    fn run_frame(ctx: &egui::Context, time: f64, text: &str) -> (egui::FullOutput, egui::Rect) {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320.0, 200.0));
        let mut rect = egui::Rect::NOTHING;
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(screen),
                time: Some(time),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    rect = rolling_number(ui, egui::Id::new("n"), text, |s| {
                        egui::RichText::new(s).monospace()
                    });
                });
            },
        );
        (out, rect)
    }

    #[test]
    fn lerp_color_hits_endpoints_and_midpoint() {
        let a = egui::Color32::from_rgb(0, 0, 0);
        let b = egui::Color32::from_rgb(200, 100, 50);
        assert_eq!(lerp_color(a, b, 0.0), a);
        assert_eq!(lerp_color(a, b, 1.0), b);
        let mid = lerp_color(a, b, 0.5);
        assert_eq!(mid, egui::Color32::from_rgb(100, 50, 25));
        // Out-of-range `t` is clamped.
        assert_eq!(lerp_color(a, b, -1.0), a);
        assert_eq!(lerp_color(a, b, 2.0), b);
    }

    #[test]
    fn snap_collapse_pins_openness_to_the_end() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("bulk_card");
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(320.0, 200.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input.clone(), |ctx| {
            super::snap_collapse(ctx, id, true);
            assert_eq!(super::collapse_openness(ctx, id, true), 1.0);
            super::snap_collapse(ctx, id, false);
            assert_eq!(super::collapse_openness(ctx, id, false), 0.0);
        });
    }

    #[test]
    fn hover_t_is_binary_when_everything_is_visible() {
        let ctx = egui::Context::default();
        ctx.memory_mut(|mem| mem.set_everything_is_visible(true));
        let id = egui::Id::new("hover_card");
        assert_eq!(super::hover_t(&ctx, id, false), 0.0);
        assert_eq!(super::hover_t(&ctx, id, true), 1.0);
    }

    /// The shared curve: exact endpoints, monotonic, and past halfway at the midpoint (that is the ease-out).
    #[test]
    fn ease_out_hits_endpoints_and_leads_at_the_midpoint() {
        assert_eq!(super::ease_out(0.0), 0.0);
        assert_eq!(super::ease_out(1.0), 1.0);
        // Out-of-range clamps instead of overshooting.
        assert_eq!(super::ease_out(-1.0), 0.0);
        assert_eq!(super::ease_out(2.0), 1.0);
        let mid = super::ease_out(0.5);
        assert!(mid > 0.5, "缓出曲线在中点必须过半，否则和线性没区别：{mid}");
        assert!(mid < 1.0, "中点不该已经走完");
        // Monotonic: sampling must never go backwards.
        let mut previous = 0.0;
        for step in 0..=20 {
            let value = super::ease_out(step as f32 / 20.0);
            assert!(
                value >= previous,
                "缓动曲线必须单调递增：{previous} -> {value}"
            );
            previous = value;
        }
    }

    /// The three boolean-driven animations are wired to the shared curve, not egui's linear default.
    ///
    /// `animate_bool_with_time` eases with `linear`, which is what every transition used to run on; sampling
    /// at half the duration catches a silent return to it (linear reads exactly 0.5 there).
    #[test]
    fn duration_driven_animations_use_the_shared_curve() {
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320.0, 200.0));
        let input_at = |time: f64| egui::RawInput {
            screen_rect: Some(screen),
            time: Some(time),
            ..Default::default()
        };
        // One monotonically advancing clock across all samples: egui's animation state advances by input time,
        // and rewinding it (starting a sample "before" the previous one) makes egui treat the tween as fresh —
        // real frames never go backwards.
        let mut clock = 1.0f64;
        let expected = super::ease_out(0.5);
        let mut sample =
            |name: &str, duration: f32, progress: &dyn Fn(&egui::Context, bool) -> f32| {
                // Frame at the origin: the "off" state, so the tween has a known origin.
                let _ = ctx.run(input_at(clock), |ctx| {
                    progress(ctx, false);
                });
                // A hair later: flip to "on"; this is when the tween is armed.
                let flip = clock + 0.001;
                let _ = ctx.run(input_at(flip), |ctx| {
                    progress(ctx, true);
                });
                // Half the duration after arming.
                let mut seen = 0.0;
                let _ = ctx.run(input_at(flip + f64::from(duration) * 0.5), |ctx| {
                    seen = progress(ctx, true);
                });
                assert!(
                    seen > 0.5,
                    "{name}：半程进度 {seen:.3} 未过半（线性曲线正是 0.5）"
                );
                assert!(seen < 1.0, "{name}：半程不该已经走完");
                assert!(
                    (seen - expected).abs() < 0.05,
                    "{name}：半程进度 {seen:.3} 不匹配统一缓动 {expected:.3}"
                );
                clock += 1.0;
            };
        sample("eased_hover", super::HOVER_TIME, &|ctx, on| {
            super::hover_t(ctx, egui::Id::new("eased_hover"), on)
        });
        sample("eased_collapse", super::COLLAPSE_TIME, &|ctx, open| {
            super::collapse_openness(ctx, egui::Id::new("eased_collapse"), open)
        });
        sample("eased_toggle", super::TOGGLE_TIME, &|ctx, on| {
            super::toggle_progress(ctx, egui::Id::new("eased_toggle"), on)
        });
    }

    /// The theme cross-fade starts at 0 and is skipped (jumps to 1) under `everything_is_visible`.
    #[test]
    fn fade_progress_starts_at_zero_and_skips_in_debug_mode() {
        let ctx = egui::Context::default();
        assert_eq!(
            super::fade_progress(&ctx, ctx.input(|input| input.time)),
            0.0,
            "刚切主题的那一帧进度必须是 0"
        );
        assert_eq!(super::fade_progress(&ctx, 1e9), 0.0, "起点在未来时进度为 0");
        ctx.memory_mut(|mem| mem.set_everything_is_visible(true));
        assert_eq!(
            super::fade_progress(&ctx, 1e9),
            1.0,
            "调试模式必须直接给到终值"
        );
    }
    /// While expanded: renders and measures the full height; once collapsed returns `None`, taking no layout space.
    ///
    /// body reserves with a fixed-size widget, not text.
    #[test]
    fn collapse_renders_when_open_and_nothing_when_closed() {
        let ctx = egui::Context::default();
        ctx.memory_mut(|mem| mem.set_everything_is_visible(true));
        let id = egui::Id::new("test_card");
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(320.0, 200.0),
            )),
            ..Default::default()
        };
        let body_size = egui::vec2(100.0, 40.0);
        for frame in 0..3 {
            let _ = ctx.run(input.clone(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let before = ui.next_widget_position().y;
                    let ret = animated_collapse(ui, id, true, |ui| {
                        ui.allocate_exact_size(body_size, egui::Sense::hover());
                        7u8
                    });
                    assert_eq!(ret, Some(7), "第 {frame} 帧展开时必须渲染");
                    let used = ui.next_widget_position().y - before;
                    assert!(
                        used + 0.5 >= body_size.y,
                        "第 {frame} 帧 body 至少应占 {0}px，实际 {used}",
                        body_size.y
                    );
                    let stored = ctx.data(|data| data.get_temp::<f32>(id.with("open_height")));
                    assert_eq!(stored, Some(body_size.y), "满高没有落进存储");
                });
            });
        }
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let before = ui.next_widget_position().y;
                let ret = animated_collapse(ui, id, false, |ui| {
                    ui.allocate_exact_size(body_size, egui::Sense::hover());
                    0u8
                });
                assert!(ret.is_none(), "收起后不应渲染");
                assert!(
                    (ui.next_widget_position().y - before).abs() < 0.5,
                    "收起后不应占用布局空间"
                );
            });
        });
    }

    /// Mid-animation frames: reserve the tweened height first, painting body into an independent subtree; total
    /// height never exceeds the recorded full height × openness.
    #[test]
    fn animating_frames_reserve_a_clamped_band() {
        let ctx = egui::Context::default();
        ctx.memory_mut(|mem| mem.set_everything_is_visible(true));
        let id = egui::Id::new("test_card");
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(320.0, 200.0),
            )),
            ..Default::default()
        };
        // Expand one frame first, measuring the full height.
        let _ = ctx.run(input.clone(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = animated_collapse(ui, id, true, |ui| ui.label("body"));
            });
        });
        let full = ctx.data(|data| data.get_temp::<f32>(id.with("open_height")));
        assert!(full.is_some_and(|h| h > 0.0), "展开一帧后必须量到满高");
        // What's locked here: the full-height record survives collapsing.
        assert_eq!(
            ctx.data(|data| data.get_temp::<f32>(id.with("open_height"))),
            full,
            "收起后满高记录不能被清掉"
        );
    }

    /// The first frame is static; on a value-change frame the old value holds its slot while the new waits below;
    /// after the roll only the new remains, and the reserved width during the roll matches the final value (layout doesn't jump).
    #[test]
    fn rolling_number_rolls_old_out_and_new_in_then_settles() {
        let ctx = egui::Context::default();

        let (out, first_rect) = run_frame(&ctx, 0.0, "12");
        let texts = texts_at(&out);
        let top = texts
            .iter()
            .find(|(t, _, _)| t == "1")
            .expect("首帧应画出旧串首位")
            .2;
        assert!(
            texts.iter().all(|(_, _, y)| (*y - top).abs() < 0.01),
            "首帧应只画一层，实际：{texts:?}"
        );

        // Value-change frame: tween 0 — the old value in place, the new below the slot (clipped out but shaped).
        let (out, mid_rect) = run_frame(&ctx, 1.0, "34");
        let texts = texts_at(&out);
        let old_y = texts
            .iter()
            .find(|(t, _, _)| t == "1")
            .expect("旧值应仍在")
            .2;
        let new_y = texts
            .iter()
            .find(|(t, _, _)| t == "3")
            .expect("新值应已入列")
            .2;
        assert!((old_y - top).abs() < 0.01, "补间 0 时旧值应还在原位");
        assert!(new_y > top + 1.0, "新值应在槽下待入，实际 {new_y} vs {top}");
        let row_h = new_y - top;

        // Halfway: the old slides up out of the slot, the new rolls in from below.
        let (out, _) = run_frame(&ctx, 1.0 + f64::from(super::DIGIT_ROLL_TIME) * 0.5, "34");
        let texts = texts_at(&out);
        let old_y = texts.iter().find(|(t, _, _)| t == "1").unwrap().2;
        let new_y = texts.iter().find(|(t, _, _)| t == "3").unwrap().2;
        assert!(old_y < top, "半程时旧值应已上移出槽");
        assert!(
            new_y > top && new_y < top + row_h,
            "半程时新值应在槽内，实际 {new_y}"
        );

        // Roll finished: only the new value remains, back in place.
        let (out, end_rect) = run_frame(&ctx, 1.0 + f64::from(super::DIGIT_ROLL_TIME) + 0.1, "34");
        let texts = texts_at(&out);
        assert!(
            texts
                .iter()
                .all(|(t, _, y)| t != "1" && (*y - top).abs() < 0.01),
            "滚完后应只剩新值且在原位，实际：{texts:?}"
        );
        assert!(
            (mid_rect.width() - end_rect.width()).abs() < 0.01,
            "滚动期间预留宽度应与终值一致"
        );
        assert!(
            (first_rect.width() - end_rect.width()).abs() < 0.01,
            "等宽字体下两值同位数，宽度应相同"
        );
    }

    /// In debug mode, value changes don't roll — the final value paints directly.
    #[test]
    fn rolling_number_snaps_under_everything_is_visible() {
        let ctx = egui::Context::default();
        ctx.memory_mut(|mem| mem.set_everything_is_visible(true));
        let (_, _) = run_frame(&ctx, 0.0, "12");
        let (out, _) = run_frame(&ctx, 0.01, "34");
        let texts = texts_at(&out);
        let ys: Vec<f32> = texts.iter().map(|(_, _, y)| *y).collect();
        assert!(
            ys.iter().all(|y| (y - ys[0]).abs() < 0.01),
            "调试模式下不应出现待入层，实际：{texts:?}"
        );
        assert!(
            texts.iter().any(|(t, _, _)| t == "3") && !texts.iter().any(|(t, _, _)| t == "1"),
            "调试模式下应直接画终值"
        );
    }

    /// Count stroked/filled rect and path shapes (the pulse ring paints as a rounded-rect stroke).
    fn path_count(out: &egui::FullOutput) -> usize {
        fn collect(shape: &egui::Shape, n: &mut usize) {
            match shape {
                egui::Shape::Path(_) | egui::Shape::Rect(_) => *n += 1,
                egui::Shape::Vec(items) => items.iter().for_each(|s| collect(s, n)),
                _ => {}
            }
        }
        let mut n = 0;
        for cs in &out.shapes {
            collect(&cs.shape, &mut n);
        }
        n
    }

    /// Run one frame; `press` / `release` inject a primary-button press / release at the widget's spot.
    fn pulse_frame(ctx: &egui::Context, time: f64, press: bool, release: bool) -> egui::FullOutput {
        let hit = egui::pos2(70.0, 60.0);
        let mut events = Vec::new();
        if press {
            events.push(egui::Event::PointerButton {
                pos: hit,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            });
        }
        if release {
            events.push(egui::Event::PointerButton {
                pos: hit,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            });
        }
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(320.0, 200.0),
            )),
            time: Some(time),
            events,
            ..Default::default()
        };
        ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                // An explicit rect: the hit position can't drift with the panel's margins.
                let response = ui.allocate_rect(
                    egui::Rect::from_min_size(egui::pos2(50.0, 50.0), egui::vec2(40.0, 20.0)),
                    egui::Sense::click(),
                );
                super::click_pulse(
                    ui,
                    &response,
                    egui::Color32::from_rgb(200, 60, 60),
                    egui::CornerRadius::same(6),
                );
            });
        })
    }

    /// A click paints the pulse ring; past PULSE_TIME it is gone.
    #[test]
    fn click_pulse_paints_then_expires() {
        let ctx = egui::Context::default();
        let baseline = path_count(&pulse_frame(&ctx, 1.0, false, false));
        // Press + release in two frames: the click registers on release.
        let _ = pulse_frame(&ctx, 1.02, true, false);
        let clicked = pulse_frame(&ctx, 1.04, false, true);
        assert!(
            path_count(&clicked) > baseline,
            "the click frame must paint the pulse ring"
        );
        let mid = pulse_frame(&ctx, 1.1, false, false);
        assert!(
            path_count(&mid) > baseline,
            "the pulse must persist past the click frame"
        );
        let expired = pulse_frame(
            &ctx,
            1.04 + f64::from(super::PULSE_TIME) + 0.05,
            false,
            false,
        );
        assert_eq!(
            path_count(&expired),
            baseline,
            "the ring must be gone once PULSE_TIME has passed"
        );
    }

    /// A second click inside an active pulse restarts it from the new click (no stacking, no early cutoff).
    #[test]
    fn click_pulse_restarts_on_rapid_reclick() {
        let ctx = egui::Context::default();
        let baseline = path_count(&pulse_frame(&ctx, 1.0, false, false));
        let _ = pulse_frame(&ctx, 1.02, true, false);
        let _ = pulse_frame(&ctx, 1.04, false, true); // click 1: pulse ends at 1.26
        let _ = pulse_frame(&ctx, 1.20, true, false);
        let _ = pulse_frame(&ctx, 1.21, false, true); // click 2 restarts: ends at 1.43
        let late = pulse_frame(&ctx, 1.35, false, false);
        assert!(
            path_count(&late) > baseline,
            "at 1.35 the first pulse (ending 1.26) would be gone — a ring here proves the restart"
        );
    }

    /// Debug / offscreen-test mode suppresses pulses entirely (stable output, no repaint churn).
    #[test]
    fn click_pulse_suppressed_under_everything_is_visible() {
        let ctx = egui::Context::default();
        ctx.memory_mut(|mem| mem.set_everything_is_visible(true));
        let baseline = path_count(&pulse_frame(&ctx, 1.0, false, false));
        let _ = pulse_frame(&ctx, 1.02, true, false);
        let clicked = pulse_frame(&ctx, 1.04, false, true);
        assert_eq!(
            path_count(&clicked),
            baseline,
            "debug mode must not paint pulses"
        );
    }
}
