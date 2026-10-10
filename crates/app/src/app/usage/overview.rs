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

/// 顶部月份标签行高。
const MONTH_ROW_HEIGHT: f32 = 16.0;

/// 区间切换按钮宽度。
const RANGE_BUTTON_WIDTH: f32 = 56.0;

/// 小标签字号（用于 painter 画的文字）。
const TEXT_CAPTION: f32 = crate::theme::TEXT_CAPTION;

impl App {
    /// 区间切换 + 总览卡 + 热力图。
    pub(in crate::app) fn ui_usage_overview(&mut self, ui: &mut egui::Ui) {
        self.ui_usage_controls(ui);
        ui.add_space(crate::theme::SPACE_3);
        self.ui_usage_summary(ui);
        ui.add_space(crate::theme::SPACE_4);
        self.ui_usage_heatmap(ui);
    }

    /// 一行控件：区间（左）+ 维度（右）。
    ///
    /// 合成一行而不是两行：两者都是「看哪部分数据」的选择，同一行更好扫，
    /// 也给下面的表格省出一行高度。
    fn ui_usage_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            self.range_buttons(ui);
            // 维度推到右侧：与区间区分开（区间管时间，维度管分组）。
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.dimension_buttons(ui);
            });
        });
    }

    /// 区间切换：今日 / 本周 / 本月 / 全部。
    fn range_buttons(&mut self, ui: &mut egui::Ui) {
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
                        .min_size(egui::vec2(RANGE_BUTTON_WIDTH, 22.0)),
                )
                .clicked()
            {
                self.usage_range = range;
            }
        }
    }

    /// 总览卡：当前区间的合计 + 输入 / 输出 / 缓存分解。
    fn ui_usage_summary(&mut self, ui: &mut egui::Ui) {
        let daily = self.usage_daily_filtered();
        let today = self.usage_today();
        let range = self.usage_range;
        let stats = range::range_stats(&daily, range, today);
        let all = range::totals_in_range(&daily, Range::All, today);
        let weak = ui.visuals().weak_text_color();

        egui::Frame::NONE
            .inner_margin(egui::Margin::same(crate::theme::SPACE_3 as i8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());

                // 主数字：照参考实现把「大数 + 小标签」放在一起，一眼看到量级。
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(aggregate::format_tokens(stats.totals.total()))
                            .size(crate::theme::TEXT_HEADING)
                            .strong()
                            .monospace(),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "tokens · {}",
                            range::totals_label(range).trim_end_matches("用量")
                        ))
                        .size(crate::theme::TEXT_BODY)
                        .color(weak),
                    );
                });

                ui.add_space(crate::theme::SPACE_3);

                // 四个统计块：总量之外补上频率与峰值，单看总量看不出使用习惯。
                ui.horizontal_wrapped(|ui| {
                    let best = stats
                        .best_day
                        .map(|(day, total)| {
                            format!(
                                "{}（{}）",
                                aggregate::format_tokens(total),
                                aggregate::day_label(day, today)
                            )
                        })
                        .unwrap_or_else(|| "—".to_string());
                    let share = if range == Range::All {
                        "—".to_string()
                    } else {
                        percent_of(stats.totals.total(), all.total())
                    };
                    for (label, value) in [
                        ("活跃天数", format!("{} 天", stats.active_days)),
                        (
                            "日均",
                            aggregate::format_tokens(stats.average_per_active_day()),
                        ),
                        ("最高一天", best),
                        ("占全部", share),
                    ] {
                        stat_block(ui, label, &value);
                    }
                });

                ui.add_space(crate::theme::SPACE_3);

                // 分解：照参考实现的明细行，四个桶都列。
                ui.label(
                    egui::RichText::new(usage_breakdown_line(&stats.totals))
                        .size(crate::theme::TEXT_SMALL)
                        .color(weak),
                );

                if stats.totals.total() == 0 {
                    ui.add_space(crate::theme::SPACE_2);
                    ui.label(
                        egui::RichText::new(format!(
                            "{}没有用量记录。",
                            range::totals_label(range)
                        ))
                        .small()
                        .color(crate::theme::semantics(ui).warn),
                    );
                }
            });
    }

    /// 热力图：53 周 × 7 天，用 `Painter` 画矩形。
    fn ui_usage_heatmap(&mut self, ui: &mut egui::Ui) {
        let daily = self.usage_daily_filtered();
        let today = self.usage_today();
        let accent = self.theme.accent_color();
        let weak_text = ui.visuals().weak_text_color();

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("每日用量").strong());
            ui.label(egui::RichText::new("最近 53 周").small().color(weak_text));
        });
        ui.add_space(crate::theme::SPACE_2);

        // 滚动区域：53 周在窄窗口里放不下，横向可滚。
        egui::ScrollArea::horizontal()
            .id_salt("usage_heatmap")
            .show(ui, |ui| {
                let grid = range::heatmap_grid(&daily, today, HEATMAP_WEEKS);
                let totals: Vec<i64> = grid.iter().map(|cell| cell.total).collect();
                let thresholds = range::heat_thresholds(&totals);

                // 顶部月份标签占一行，下面是 7 行格子。
                let width = WEEKDAY_LABEL_WIDTH + HEATMAP_WEEKS as f32 * CELL;
                let height = MONTH_ROW_HEIGHT + 7.0 * CELL;
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());

                // 命中的格子：整个热力图只有一个 response，tooltip 必须靠
                // 自己算出的 `hit` 定位，不能用 `response.on_hover_text`
                // （那会把提示钉在整块的右上角，而不是鼠标处）。
                let mut hit: Option<(&range::HeatCell, egui::Rect)> = None;

                if ui.is_rect_visible(rect) {
                    let painter = ui.painter_at(rect);
                    let step = CELL - CELL_GAP;
                    let grid_top = rect.min.y + MONTH_ROW_HEIGHT;
                    let hover_pos = ui.ctx().pointer_hover_pos();

                    for (index, cell) in grid.iter().enumerate() {
                        let week = index / 7;
                        let weekday = index % 7;
                        let min = egui::pos2(
                            rect.min.x + WEEKDAY_LABEL_WIDTH + week as f32 * CELL,
                            grid_top + weekday as f32 * CELL,
                        );
                        let cell_rect = egui::Rect::from_min_size(min, egui::vec2(step, step));
                        let level = range::heat_level(cell.total, &thresholds);
                        painter.rect_filled(cell_rect, 2.0, heat_color(level, accent, ui));

                        if hover_pos.is_some_and(|pos| cell_rect.contains(pos)) {
                            hit = Some((cell, cell_rect));
                        }
                    }

                    // 星期标签（只标隔行，否则 7 行太挤）。
                    for (weekday, label) in [(0usize, "一"), (2, "三"), (4, "五")] {
                        painter.text(
                            egui::pos2(rect.min.x, grid_top + weekday as f32 * CELL + step / 2.0),
                            egui::Align2::LEFT_CENTER,
                            label,
                            egui::FontId::proportional(TEXT_CAPTION),
                            weak_text,
                        );
                    }

                    // 月份标签：每列头一次出现「新的月份」时标一下。
                    let mut last_month = 0i64;
                    for week in 0..HEATMAP_WEEKS {
                        let day = grid[week * 7].day;
                        let (_, month, _) = aggregate::civil_from_days(day);
                        if month != last_month {
                            last_month = month;
                            painter.text(
                                egui::pos2(
                                    rect.min.x + WEEKDAY_LABEL_WIDTH + week as f32 * CELL,
                                    rect.min.y,
                                ),
                                egui::Align2::LEFT_TOP,
                                format!("{month} 月"),
                                egui::FontId::proportional(TEXT_CAPTION),
                                weak_text,
                            );
                        }
                    }
                }

                // 鼠标悬停在某个格子上：提示跟在鼠标旁，不在整块右上角。
                if let Some((cell, _)) = hit {
                    egui::Tooltip::for_widget(&response)
                        .at_pointer()
                        .show(|ui| {
                            ui.label(cell_hover_text(cell, today));
                        });
                }

                ui.add_space(crate::theme::SPACE_2);
                heatmap_legend(ui, accent, weak_text);
            });
    }
}

/// 分解行：`输入 x · 输出 y · 缓存读 z · 缓存写 w`。
///
/// 照参考实现（tokscale 的明细行）把四个桶都列出来，而不是只给合计 ——
/// 缓存读常常占九成以上，不给分解看不出量到底花在哪。
fn usage_breakdown_line(totals: &crate::usage::DayTotals) -> String {
    format!(
        "输入 {} · 输出 {} · 缓存读 {} · 缓存写 {}",
        aggregate::format_tokens(totals.input),
        aggregate::format_tokens(totals.output),
        aggregate::format_tokens(totals.cache_read),
        aggregate::format_tokens(totals.cache_write),
    )
}

/// 一个统计块：小标签在上、数字在下（照参考实现的 `Total / Tokens / Best day`）。
///
/// 数字用等宽体：四个块并排时，等宽能让数字列在视觉上对齐，
/// 不会因为字形宽度不同而跳。
fn stat_block(ui: &mut egui::Ui, label: &str, value: &str) {
    let weak = ui.visuals().weak_text_color();
    ui.allocate_ui_with_layout(
        egui::vec2(STAT_BLOCK_WIDTH, STAT_BLOCK_HEIGHT),
        egui::Layout::top_down(egui::Align::LEFT),
        |ui| {
            ui.label(
                egui::RichText::new(label)
                    .size(crate::theme::TEXT_CAPTION)
                    .color(weak),
            );
            ui.label(
                egui::RichText::new(value)
                    .size(crate::theme::TEXT_TITLE)
                    .strong()
                    .monospace(),
            );
        },
    );
}

/// 统计块宽高：固定宽让四个块等距排列。
const STAT_BLOCK_WIDTH: f32 = 132.0;
const STAT_BLOCK_HEIGHT: f32 = 38.0;

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
fn heatmap_legend(ui: &mut egui::Ui, accent: egui::Color32, weak: egui::Color32) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("少").size(TEXT_CAPTION).color(weak));
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
        ui.label(egui::RichText::new("多").size(TEXT_CAPTION).color(weak));
        ui.label(
            egui::RichText::new("按分位数分级")
                .size(TEXT_CAPTION)
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
