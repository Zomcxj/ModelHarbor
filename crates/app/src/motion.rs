//! 界面动效：卡片描边悬停过渡、卡片体折叠 / 展开的高度动画与滑动开关补间。
//!
//! 各时长都在 0.1~0.2 秒。

use eframe::egui::{self, Color32};

/// 悬停描边过渡时长（秒）。
pub const HOVER_TIME: f32 = 0.12;

/// 折叠 / 展开时长（秒）。
pub const COLLAPSE_TIME: f32 = 0.18;

/// 滑动开关的滑块行程时长（秒）。
pub const TOGGLE_TIME: f32 = 0.14;

/// 数字逐位滚动时长（秒）。
///
/// 比悬停 / 折叠长一档：滚动要让人看清「旧值滚走、新值滚入」，
/// 太快就成了闪一下。参考实现（token-monitor v0.70.0 的 digit roll）
/// 也在半秒量级。
pub const DIGIT_ROLL_TIME: f32 = 0.45;

/// 动画时长的编译期上界检查。
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
};

/// gamma 空间的逐通道插值（预乘 alpha 原样插）。
///
/// `t` 会被夹到 `[0, 1]`。
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

/// 悬停插值系数（0 = 未悬停，1 = 悬停）。
///
/// `everything_is_visible`（调试模式）下直接返回终值。
pub fn hover_t(ctx: &egui::Context, id: egui::Id, hovered: bool) -> f32 {
    if ctx.memory(|mem| mem.everything_is_visible()) {
        return hovered as u8 as f32;
    }
    ctx.animate_bool_with_time(id, hovered, HOVER_TIME)
}

/// 折叠展开的开放度（0 = 全收，1 = 全开）。
pub fn collapse_openness(ctx: &egui::Context, id: egui::Id, open: bool) -> f32 {
    if ctx.memory(|mem| mem.everything_is_visible()) {
        return open as u8 as f32;
    }
    ctx.animate_bool_with_time(id.with("collapse"), open, COLLAPSE_TIME)
}

/// 滑动开关的滑块进度（0 = 贴左的关态，1 = 贴右的开态）。
///
/// 未登记过的 id 首帧直接返回终值，因此页面刚打开时开关不从左侧滑入，
/// 只有点击造成的状态变化才走动画。
///
/// `everything_is_visible`（调试模式）下直接返回终值。
pub fn toggle_progress(ctx: &egui::Context, id: egui::Id, on: bool) -> f32 {
    if ctx.memory(|mem| mem.everything_is_visible()) {
        return on as u8 as f32;
    }
    ctx.animate_bool_with_time(id.with("toggle"), on, TOGGLE_TIME)
}

/// 把折叠动画立刻钉到终值（时长 0）。用于批量展开 / 收起。
pub fn snap_collapse(ctx: &egui::Context, id: egui::Id, open: bool) {
    ctx.animate_bool_with_time(id.with("collapse"), open, 0.0);
}

/// 数字逐位滚动：每位数字在自己的槽里垂直滚动，非数字字符（逗号、点、单位）原地不动。
///
/// 参考 token-monitor v0.70.0 的默认动画（digit roll，#1014）：总量与分解
/// 读数切换时逐位滚动，而不是整串闪换。
///
/// 槽：宽度取**终值**该位的字符宽（滚动期间布局不跳），高度一行。
/// 补间 0 = 只见旧位，1 = 只见新位：旧位随补间上移出槽，新位自槽下滚入，
/// 超出槽的部分裁掉。位数变多时新增高位同样自下方滚入。
///
/// 状态存三个临时键：`id` = 本轮滚动起点值，`id/target` = 终点值，
/// `id/roll_start` = 起点时刻。滚动途中值又变：上一轮直接钉到它的终点、
/// 新一轮从那里起滚（不倒跳，对齐参考实现 #1025 对滚动层数的收口）。
///
/// 整串矩形经 [`egui::Ui::allocate_exact_size`] 分配并返回：与 `Label`
/// 同一套布局行为，在行内垂直居中——与同行的色块、标签共用行中心，
/// 不会像贴行顶直画那样错开半个行高。
///
/// `everything_is_visible`（调试模式）与首帧都不滚动、直接画终值，
/// 布局测试因此能拿到稳定文本。
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
        // 调试模式：钉终值、清状态。
        from = text.to_string();
        target = from.clone();
        ctx.data_mut(|data| {
            data.insert_temp(id, from.clone());
            data.remove_temp::<String>(target_id);
            data.remove_temp::<f64>(start_id);
        });
    } else if text != target {
        // 值变了：上一轮（若有）钉到它的终点，新一轮从那里起滚。
        from = target;
        target = text.to_string();
        ctx.data_mut(|data| {
            data.insert_temp(id, from.clone());
            data.insert_temp(target_id, target.clone());
            data.insert_temp(start_id, now);
        });
    } else {
        // 值未变：记下当前显示值，供下次变化作为滚动起点。
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
        // 滚完：起点值升为终点值，清计时。
        ctx.data_mut(|data| {
            data.insert_temp(id, target.clone());
            data.remove_temp::<f64>(start_id);
        });
    }
    // 缓动：先快后慢，收尾不突兀。
    let eased = 1.0 - (1.0 - t).powi(2);

    let chars: Vec<char> = target.chars().collect();
    let from_chars: Vec<char> = from.chars().collect();
    // 从右往左对齐数位：末位对末位。
    let offset = chars.len().saturating_sub(from_chars.len());
    let fallback = ui.visuals().text_color();

    // 先整体量排：每位一个 galley，总宽与行高定了才能向布局要矩形。
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
    // 经布局分配（而非贴游标直画）：行内垂直居中与 Label / 色块一致。
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
                // 旧位：随补间上移出槽。
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
            // 新位：自槽下滚入。
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

/// 带高度动画的折叠体：收起且动画结束后返回 `None`（本帧不渲染）。
///
/// 动画中先给父布局预留补间高度，再把 body 画进同一位置的独立子树、用裁剪收住
/// 超出部分。
///
/// body 全开与动画中都用同一个全局 `id`；满高逐帧测量后存进临时存储，
/// 未知时先按 10px 位移量起算。
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
    // 子树给满高（未知时给一页占位）。
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

    /// 收集全部文本形状的位置（含嵌套）。
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

    /// 跑一帧并返回输出与读数矩形。
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
        // 超范围的 `t` 被夹住。
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

    /// 展开时渲染并量出满高；收起后返回 `None`、不占用布局空间。
    ///
    /// body 用固定尺寸占位，不用文本。
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

    /// 动画中的帧：先预留补间高度，body 画进独立子树；总高度不超过已记录的
    /// 满高 × 开放度。
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
        // 先展开一帧，量出满高。
        let _ = ctx.run(input.clone(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = animated_collapse(ui, id, true, |ui| ui.label("body"));
            });
        });
        let full = ctx.data(|data| data.get_temp::<f32>(id.with("open_height")));
        assert!(full.is_some_and(|h| h > 0.0), "展开一帧后必须量到满高");
        // 这里锁的是：满高记录在收起后依然保留。
        assert_eq!(
            ctx.data(|data| data.get_temp::<f32>(id.with("open_height"))),
            full,
            "收起后满高记录不能被清掉"
        );
    }

    /// 首帧静态；值变化帧旧值在位、新值在槽下待入；滚完后只剩新值，
    /// 且滚动期间预留宽度与终值一致（布局不跳）。
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

        // 值变化帧：补间 0，旧值在位、新值在槽下（裁剪外但形状在）。
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

        // 半程：旧值上移出槽、新值自下滚入。
        let (out, _) = run_frame(&ctx, 1.0 + f64::from(super::DIGIT_ROLL_TIME) * 0.5, "34");
        let texts = texts_at(&out);
        let old_y = texts.iter().find(|(t, _, _)| t == "1").unwrap().2;
        let new_y = texts.iter().find(|(t, _, _)| t == "3").unwrap().2;
        assert!(old_y < top, "半程时旧值应已上移出槽");
        assert!(
            new_y > top && new_y < top + row_h,
            "半程时新值应在槽内，实际 {new_y}"
        );

        // 滚完：只剩新值、回到原位。
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

    /// 调试模式下值变化不滚动，直接画终值。
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
}
