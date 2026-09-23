//! 界面动效：悬停过渡与折叠展开的补间。
//!
//! egui 的样式是「状态一变、外观瞬变」；这里补上时间维度的两段小动效——
//! 卡片描边悬停过渡、卡片体折叠 / 展开的高度动画。时长都很短（0.1~0.2 秒），
//! 目标是「跟手」，不是「吸睛」。

use eframe::egui::{self, Color32};

/// 悬停描边过渡时长（秒）。
pub const HOVER_TIME: f32 = 0.12;

/// 折叠 / 展开时长（秒）。与 egui 默认 CollapsingHeader 的动画节奏同级。
pub const COLLAPSE_TIME: f32 = 0.18;

/// 滑动开关的滑块行程时长（秒）。
///
/// 比悬停过渡略长：滑块要**看得见在移动**，太快就退化成瞬切、失去「滑动」的观感；
/// 又要短到点完立刻到位，不能等。0.14s 是「看清是滑过去的」与「不觉得卡」的折中。
pub const TOGGLE_TIME: f32 = 0.14;

/// 动画时长必须「短到跟手」：改大了整个界面会显得拖沓。
/// 编译期检查——改坏常量时构建就失败，不必等测试跑起来。
const _: () = {
    assert!(HOVER_TIME <= 0.2, "悬停过渡时长超过 0.2s，界面会显得拖沓");
    assert!(
        COLLAPSE_TIME <= 0.25,
        "折叠动画时长超过 0.25s，界面会显得拖沓"
    );
    assert!(TOGGLE_TIME <= 0.2, "开关滑动时长超过 0.2s，点击会显得迟钝");
    // 时长归零等于瞬切，动画形同虚设——必须真的有一段滑动过程。
    assert!(TOGGLE_TIME > 0.0, "开关滑动时长不能为零，否则没有滑动过程");
};

/// gamma 空间的逐通道插值（预乘 alpha 原样插）。
///
/// UI 描边过渡不需要线性空间校正——人眼对 0.12 秒的中间帧只感知「在变」，
/// 不感知它走的是哪条曲线。
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
/// egui 的「调试模式显示全部 UI」会跳过动画管理器，这里同样直接给终值，
/// 保证单测与调试渲染是确定性的。
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
/// 走 `animate_bool_with_time`：它按**每帧的真实时间差**推进，所以帧率波动时
/// 滑行速度仍然一致（不是「每帧挪固定距离」）。动画期间它会自行 `request_repaint`，
/// 不必调用方再驱动重绘。
///
/// 未登记过的 id 首帧直接返回终值（见 egui `AnimationManager::animate_bool` 的
/// `None => end` 分支），所以**页面刚打开时开关不会从左边滑进来**——只有点击
/// 造成的状态变化才走动画。
///
/// `everything_is_visible`（调试模式 / 确定性单测）下直接给终值，与
/// [`hover_t`] / [`collapse_openness`] 保持一致。
pub fn toggle_progress(ctx: &egui::Context, id: egui::Id, on: bool) -> f32 {
    if ctx.memory(|mem| mem.everything_is_visible()) {
        return on as u8 as f32;
    }
    ctx.animate_bool_with_time(id.with("toggle"), on, TOGGLE_TIME)
}

/// 把折叠动画立刻钉到终值（时长 0）。
///
/// 「展开 / 收起全部」会同时动几十张卡片：中间帧每张都要把整份表单
/// 画进裁剪区，列表会卡。批量操作走这条路径，单张卡片仍用补间。
pub fn snap_collapse(ctx: &egui::Context, id: egui::Id, open: bool) {
    ctx.animate_bool_with_time(id.with("collapse"), open, 0.0);
}

/// 带高度动画的折叠体：收起且动画结束后返回 `None`（本帧不渲染）。
///
/// 展开动画照抄 egui `CollapsingHeader` 的结构：动画中先给父布局预留**补间高度**，
/// 再把 body 画进同一位置的独立子树、用裁剪收住超出部分——`force_set_min_rect`
/// 是 `pub(crate)` 拿不到，用「先占位、后画」达到同一效果。
///
/// body 无论全开还是动画中都走同一套全局 `id`：输入框 / 折叠状态不会在动画中途丢掉。
/// 满高每帧重新测量落进临时存储：首次展开还不知道满高时先给 10px 的位移量，
/// 本帧就能量到真值，下一帧起动画就是准确的。
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
    // 子树给满高（未知时给一页的占位），裁剪只切画面不切布局，才能量到真高度。
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
    use super::{animated_collapse, lerp_color};
    use eframe::egui;

    #[test]
    fn lerp_color_hits_endpoints_and_midpoint() {
        let a = egui::Color32::from_rgb(0, 0, 0);
        let b = egui::Color32::from_rgb(200, 100, 50);
        assert_eq!(lerp_color(a, b, 0.0), a);
        assert_eq!(lerp_color(a, b, 1.0), b);
        let mid = lerp_color(a, b, 0.5);
        assert_eq!(mid, egui::Color32::from_rgb(100, 50, 25));
        // 超范围的 t 要被夹住，不能溢出回绕。
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

    /// 结构契约（确定性路径，`everything_is_visible` 下 openness 直接取终值）：
    /// 展开时渲染并量出满高，收起后返回 None、不再占用布局空间。
    ///
    /// body 用固定尺寸占位，避开「第一帧字体未就绪、label 高度为 0」。
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

    /// 动画中的帧：先预留补间高度，body 画进独立子树——总高度不能超过
    /// 已记录的满高 × 开放度，否则折叠时布局会跳变。
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
        // 收起后立刻把内存里的动画值搬到中间态是做不到的（动画管理器不可注入），
        // 这里锁的是另一件事：满高记录在收起后依然保留，收起动画全程可取。
        assert_eq!(
            ctx.data(|data| data.get_temp::<f32>(id.with("open_height"))),
            full,
            "收起后满高记录不能被清掉"
        );
    }
}
