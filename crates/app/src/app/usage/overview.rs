//! 用量视图的「区间 + 热力图 + 总览卡」三块。
//!
//! 与 [`super::view`] 的分工：那边画表格，这边画时间维度相关的部分。
//! 拆开是因为这里要直接操作 `Painter`（热力图不能用 `ui.label` 拼），
//! 混在一起会让 `view.rs` 变成什么都塞的大文件。
//!
//! 配色一律取自当前主题（`Palette::accent` 与 `visuals`），不硬编码颜色 ——
//! 八个主题（深色 / 亮色 / 海洋 / 极地 / 苔藓 / 薄荷 / 薰衣草 / 玫瑰）都要能看。

use super::aggregate;
use super::range::{self, Range};
use crate::app::App;
use eframe::egui;

/// 热力图格子边长（含间隙）。
const CELL: f32 = 13.0;

/// 格子间隙：留 2px 让格子之间看得出分隔。
const CELL_GAP: f32 = 2.0;

/// 热力图展示多少周：53 周 ≈ 一年（照 GitHub 贡献图）。
const HEATMAP_WEEKS: usize = 53;

/// 星期标签列宽。
const WEEKDAY_LABEL_WIDTH: f32 = 22.0;

/// 区间切换按钮宽度。
const RANGE_BUTTON_WIDTH: f32 = 56.0;

impl App {
    /// 区间切换 + 总览卡 + 热力图。
    pub(in crate::app) fn ui_usage_overview(&mut self, ui: &mut egui::Ui) {
        self.ui_usage_ranges(ui);
        ui.add_space(crate::theme::SPACE_3);
        self.ui_usage_summary(ui);
        ui.add_space(crate::theme::SPACE_3);
        self.ui_usage_heatmap(ui);
    }

    /// 区间切换：今日 / 本周 / 本月 / 全部。
    fn ui_usage_ranges(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("区间").small().weak());
            for range in Range::ALL {
                let selected = self.usage_range == range;
                let visuals = ui.visuals();
                let (fill, stroke) = if selected {
                    (
                        visuals.widgets.hovered.bg_fill,
                        egui::Stroke::new(2.0f32, visuals.widgets.hovered.bg_stroke.color),
                    )
                } else {
                    (egui::Color32::TRANSPARENT, egui::Stroke::NONE)
                };
                let text = egui::RichText::new(range.label());
                let text = if selected { text.strong() } else { text };
                if ui
                    .add(
                        egui::Button::new(text)
                            .fill(fill)
                            .stroke(stroke)
                            .min_size(egui::vec2(RANGE_BUTTON_WIDTH, 24.0)),
                    )
                    .clicked()
                {
                    self.usage_range = range;
                }
            }
        });
    }

    /// 总览卡：当前区间的合计 + 输入 / 输出 / 缓存分解。
    fn ui_usage_summary(&mut self, ui: &mut egui::Ui) {
        let Some(daily) = self.usage_daily().cloned() else {
            return;
        };
        let today = self.usage_today();
        let range = self.usage_range;
        let totals = range::totals_in_range(&daily, range, today);
        let all = range::totals_in_range(&daily, Range::All, today);

        let semantics = crate::theme::semantics(ui);
        egui::Frame::NONE
            .inner_margin(egui::Margin::same(crate::theme::SPACE_3 as i8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        egui::RichText::new(range::totals_label(range))
                            .small()
                            .weak(),
                    );
                    ui.label(
                        egui::RichText::new(aggregate::format_tokens(totals.total()))
                            .size(crate::theme::TEXT_TITLE)
                            .strong()
                            .monospace(),
                    );
                    if range != Range::All {
                        // 与总计的对比：让「本月」有个量级参照，不然只有绝对数看不出多寡。
                        ui.label(
                            egui::RichText::new(format!(
                                "占全部 {}",
                                percent_of(totals.total(), all.total())
                            ))
                            .small()
                            .weak(),
                        );
                    }
                });
                ui.add_space(crate::theme::SPACE_2);
                ui.horizontal_wrapped(|ui| {
                    for (label, value, color) in [
                        ("输入", totals.input, semantics.info),
                        ("输出", totals.output, semantics.ok),
                        ("缓存读", totals.cache_read, semantics.warn),
                        ("缓存写", totals.cache_write, semantics.warn),
                    ] {
                        ui.label(egui::RichText::new(label).small().weak());
                        ui.label(
                            egui::RichText::new(aggregate::format_tokens(value))
                                .small()
                                .monospace()
                                .color(color),
                        );
                        ui.label(egui::RichText::new("·").weak());
                    }
                    ui.label(
                        egui::RichText::new(format!("{} 条消息", totals.messages))
                            .small()
                            .weak(),
                    );
                });
                if totals.total() == 0 {
                    ui.add_space(crate::theme::SPACE_1);
                    ui.label(
                        egui::RichText::new(format!(
                            "{}没有用量记录。",
                            range::totals_label(range)
                        ))
                        .small()
                        .color(semantics.warn),
                    );
                }
            });
    }

    /// 热力图：53 周 × 7 天，用 `Painter` 画矩形。
    fn ui_usage_heatmap(&mut self, ui: &mut egui::Ui) {
        let Some(daily) = self.usage_daily().cloned() else {
            return;
        };
        let today = self.usage_today();
        let accent = self.theme.accent_color();
        let weak_text = ui.visuals().weak_text_color();

        ui.label(egui::RichText::new("每日用量").small().weak());
        ui.add_space(crate::theme::SPACE_1);

        // 滚动区域：53 周在窄窗口里放不下，横向可滚。
        egui::ScrollArea::horizontal()
            .id_salt("usage_heatmap")
            .show(ui, |ui| {
                let grid = range::heatmap_grid(&daily, today, HEATMAP_WEEKS);
                let totals: Vec<i64> = grid.iter().map(|cell| cell.total).collect();
                let thresholds = range::heat_thresholds(&totals);

                let width = WEEKDAY_LABEL_WIDTH + HEATMAP_WEEKS as f32 * CELL;
                let height = 7.0 * CELL;
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());

                if ui.is_rect_visible(rect) {
                    let painter = ui.painter_at(rect);
                    let step = CELL - CELL_GAP;

                    for (index, cell) in grid.iter().enumerate() {
                        let week = index / 7;
                        let weekday = index % 7;
                        let min = egui::pos2(
                            rect.min.x + WEEKDAY_LABEL_WIDTH + week as f32 * CELL,
                            rect.min.y + weekday as f32 * CELL,
                        );
                        let cell_rect = egui::Rect::from_min_size(min, egui::vec2(step, step));
                        let level = range::heat_level(cell.total, &thresholds);
                        painter.rect_filled(cell_rect, 2.0, heat_color(level, accent, ui));

                        // 悬停提示：只有真的划过某个格子才查数据，不预先构造 371 条字符串。
                        if response.hovered() {
                            if let Some(pos) = ui.ctx().pointer_hover_pos() {
                                if cell_rect.contains(pos) {
                                    response.clone().on_hover_text(cell_hover_text(cell, today));
                                }
                            }
                        }
                    }
                }
                ui.add_space(crate::theme::SPACE_2);
                heatmap_legend(ui, &thresholds, accent, weak_text);
            });
    }
}

/// 热力图格子颜色：0 级是空格子（用弱色），1-4 级按强调色递增加不透明度。
///
/// 用「底色 + 强调色 alpha 混合」而不是四档固定色：八个主题的强调色各不相同，
/// 固定色会在某些主题下与背景撞色。
fn heat_color(level: u8, accent: egui::Color32, ui: &egui::Ui) -> egui::Color32 {
    if level == 0 {
        // 没有数据：用比背景略深一点的中性色，不能是纯透明（那样看不出网格）。
        return ui.visuals().widgets.noninteractive.bg_fill;
    }
    // 1-4 级 → alpha 40 / 90 / 150 / 235。
    let alpha = [40u8, 90, 150, 235][(level.clamp(1, 4) - 1) as usize];
    egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), alpha)
}

/// 图例：`少 ■■■■ 多`。
fn heatmap_legend(
    ui: &mut egui::Ui,
    _thresholds: &[i64; 3],
    accent: egui::Color32,
    weak: egui::Color32,
) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("少").small().color(weak));
        for level in 1..=4u8 {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().rect_filled(
                rect,
                2.0,
                egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), {
                    [40u8, 90, 150, 235][(level - 1) as usize]
                }),
            );
        }
        ui.label(egui::RichText::new("多").small().color(weak));
        ui.label(
            egui::RichText::new("（按分位数分级，非固定阈值）")
                .small()
                .color(weak),
        );
    });
}

/// 格子的悬停文案。
fn cell_hover_text(cell: &range::HeatCell, today: i64) -> String {
    let label = aggregate::day_label(cell.day, today);
    if !cell.has_data {
        return format!("{label}：无记录");
    }
    if cell.total == 0 {
        return format!("{label}：0");
    }
    format!("{label}：{}", aggregate::format_tokens(cell.total))
}

/// 占比文案：`12.3%`；分母为 0 时给 `—`。
fn percent_of(part: i64, whole: i64) -> String {
    if whole <= 0 {
        return "—".to_string();
    }
    let pct = (part as f64 / whole as f64) * 100.0;
    if pct < 0.1 && part > 0 {
        // 有量但不足 0.1%：给个「<0.1%」比四舍五入成 0.0% 诚实。
        "<0.1%".to_string()
    } else {
        format!("{pct:.1}%")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_of_handles_zero_denominator() {
        assert_eq!(percent_of(10, 0), "—");
        assert_eq!(percent_of(0, 0), "—");
    }

    #[test]
    fn percent_of_formats_normally() {
        assert_eq!(percent_of(50, 100), "50.0%");
        assert_eq!(percent_of(1, 3), "33.3%");
    }

    #[test]
    fn percent_of_marks_tiny_nonzero_share() {
        // 有量但不该显示成 0.0%。
        assert_eq!(percent_of(1, 1_000_000), "<0.1%");
    }

    #[test]
    fn hover_text_distinguishes_missing_from_zero() {
        let cell = range::HeatCell {
            day: 0,
            total: 0,
            has_data: false,
        };
        assert!(cell_hover_text(&cell, 0).contains("无记录"));

        let cell = range::HeatCell {
            day: 0,
            total: 0,
            has_data: true,
        };
        assert!(cell_hover_text(&cell, 0).contains("：0"));
    }
}
