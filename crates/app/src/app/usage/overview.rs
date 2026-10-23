//! The usage view's "range + heatmap + overview card" trio.
//!
//! Division of labor with [`super::view`]: that side draws tables, this side draws the time-dimension
//! pieces. Split out because this module drives `Painter` directly (the heatmap can't be assembled
//! from `ui.label` calls); keeping it here stops `view.rs` from becoming a catch-all.
//!
//! Colors always come from the active theme (`Palette::accent` and `visuals`), nothing hard-coded —
//! all eight themes (dark / light / ocean / polar / moss / mint / lavender / rose) must read well.

use super::aggregate;
use super::range::{self, Range};
use crate::app::App;
use eframe::egui;

/// Minimum heatmap cell size: any narrower and shade levels blur together; beyond it, scroll sideways.
/// No upper bound: cells grow with the window to fill the card width (user feedback).
const CELL_MIN: f32 = 6.0;

/// How many weeks the heatmap shows: 53 ≈ one year (like GitHub's graph).
const HEATMAP_WEEKS: usize = 53;

/// Height of the top month-label row.
const MONTH_ROW_HEIGHT: f32 = 16.0;

/// Range-switch button width.
///
/// The status row reserves right-side space by group width, so the width is only exposed via
/// [`Self::usage_range_chips_width`] — callers must not recompute it from the tier count themselves
/// (two copies of the same number will eventually disagree).
const RANGE_BUTTON_WIDTH: f32 = 56.0;

/// Small label font size (for painter-drawn text).
const TEXT_CAPTION: f32 = crate::theme::TEXT_CAPTION;

impl App {
    /// The overview card + trend chart + heatmap (the range switch lives in [`super::view`]'s status line).
    pub(in crate::app) fn ui_usage_overview(&mut self, ui: &mut egui::Ui) {
        self.ui_usage_summary(ui);
        ui.add_space(crate::theme::SPACE_4);
        self.ui_usage_chart(ui);
        ui.add_space(crate::theme::SPACE_4);
        self.ui_usage_heatmap(ui);
    }

    /// The range-switch group: today / week / month / all, wrapped in a segmented control group.
    ///
    /// Painted at the status line's far right by [`super::view::ui_usage_status`] (user request): it switches
    /// the whole page's accounting, sharing the filter text's line — no need for its own vertical row.
    pub(in crate::app) fn ui_usage_range_chips(&mut self, ui: &mut egui::Ui) {
        super::component::chip_group(ui, |ui| {
            for range in Range::ALL {
                if super::chip(
                    ui,
                    self.usage_range == range,
                    range.label(),
                    RANGE_BUTTON_WIDTH,
                ) {
                    self.usage_range = range;
                }
            }
        });
    }

    /// How wide the range-switch group is: the status line reserves right-side space by it, giving the rest to the left text.
    pub(in crate::app) fn usage_range_chips_width() -> f32 {
        super::component::chip_group_width(Range::ALL.len(), RANGE_BUTTON_WIDTH)
    }

    /// The overview card: the current range's total + composition bar + aggregate facts.
    ///
    /// The range buttons aren't here: they sit at the status line's far right (user request), painted by
    /// [`super::view::ui_usage_status`].
    fn ui_usage_summary(&mut self, ui: &mut egui::Ui) {
        let daily = self.usage_daily_filtered();
        let today = self.usage_today();
        let range = self.usage_range;
        let stats = range::range_stats(&daily, range, today);
        let all = range::totals_in_range(&daily, Range::All, today);
        // The streak ignores the range: it describes the "current state".
        let streak = range::current_streak(&daily, today);
        let weak = ui.visuals().weak_text_color();

        super::component::card(ui, |ui| {
            // The headline: like the reference, the "big number + small label" sit together for an instant read of
            // magnitude. Value changes roll digit by digit (aligned with token-monitor v0.70.0's digit roll).
            ui.horizontal(|ui| {
                crate::motion::rolling_number(
                    ui,
                    ui.id().with("total"),
                    &aggregate::format_full(stats.totals.total()),
                    |s| {
                        egui::RichText::new(s)
                            .size(crate::theme::TEXT_HEADING)
                            .strong()
                            .monospace()
                    },
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

            // The ≈ line: an approximate magnitude reading, matching tokenmonitor (M, B past 1B).
            // The cost line is gone (user feedback: no spending display); pricing-computed costs stay in the core
            // layer, no longer shown. The headline is the full number; this line gives a glanceable magnitude.
            ui.horizontal(|ui| {
                crate::motion::rolling_number(
                    ui,
                    ui.id().with("approx"),
                    &format!("≈ {}", aggregate::format_tokens(stats.totals.total())),
                    |s| {
                        egui::RichText::new(s)
                            .size(crate::theme::TEXT_TITLE)
                            .strong()
                            .monospace()
                    },
                );
                ui.label(
                    egui::RichText::new(format!(
                        "tokens · 日均 {}",
                        aggregate::format_tokens(stats.average_per_active_day())
                    ))
                    .size(crate::theme::TEXT_BODY)
                    .color(weak),
                );
            });

            ui.add_space(crate::theme::SPACE_2);

            // The composition bar: four buckets' shares at a glance — cache reads often exceed 90%,
            // and without proportions you can't tell where the tokens went.
            composition_bar(ui, &stats.totals, self.theme.accent_color());

            ui.add_space(crate::theme::SPACE_3);

            // Four stat blocks: beyond the total, frequency and peaks — a total alone reveals no habits.
            // Aggregate facts: beyond the total, frequency, peaks, and preference. Listed like the reference
            // (tokscale's Stats page) — a total alone reveals no habits.
            //
            // "Long" context like dates and model usage goes on a line below the value rather than inside its
            // parentheses: cramming them together overflows the block and forces a wrap.
            let (best_value, best_detail) = match stats.best_day {
                Some((day, total)) => (
                    aggregate::format_tokens(total),
                    Some(aggregate::day_label(day, today)),
                ),
                None => ("—".to_string(), None),
            };
            let share = if range == Range::All {
                "—".to_string()
            } else {
                percent_of(stats.totals.total(), all.total())
            };
            let (top_model, top_model_detail) =
                match range::top_model_in_range(&daily, range, today) {
                    Some((model, total)) => (model, Some(aggregate::format_tokens(total))),
                    None => ("—".to_string(), None),
                };
            stat_grid(
                ui,
                &[
                    Stat {
                        label: "活跃天数",
                        value: format!("{} 天", stats.active_days),
                        detail: None,
                    },
                    Stat {
                        label: "连续天数",
                        value: format!("{} 天", streak),
                        detail: None,
                    },
                    Stat {
                        label: "日均",
                        value: aggregate::format_tokens(stats.average_per_active_day()),
                        detail: None,
                    },
                    Stat {
                        label: "最高一天",
                        value: best_value,
                        detail: best_detail,
                    },
                    Stat {
                        label: "最爱模型",
                        value: top_model,
                        detail: top_model_detail,
                    },
                    Stat {
                        label: "占全部",
                        value: share,
                        detail: None,
                    },
                ],
            );

            if stats.totals.total() == 0 {
                ui.add_space(crate::theme::SPACE_2);
                ui.label(
                    egui::RichText::new(format!("{}没有用量记录。", range::totals_label(range)))
                        .small()
                        .color(crate::theme::semantics(ui).warn),
                );
            }
        });
    }

    /// The heatmap: 53 weeks × 7 days, rectangles painted with `Painter`.
    fn ui_usage_heatmap(&mut self, ui: &mut egui::Ui) {
        let daily = self.usage_daily_filtered();
        let today = self.usage_today();
        let accent = self.theme.accent_color();
        let weak_text = ui.visuals().weak_text_color();

        super::component::card(ui, |ui| {
            ui.horizontal(|ui| {
                super::component::heading(ui, "热力图");
                ui.label(egui::RichText::new("最近 53 周").small().color(weak_text));
            });
            ui.add_space(crate::theme::SPACE_2);

            // Cells scale with the available width (user feedback: the heatmap must grow with the window):
            // 53 weeks over the available width, clamping only the floor — the wider the window, the bigger the
            // cells, filling the card's width. Computed **before** entering the horizontal ScrollArea — inside it,
            // available_width is infinite.
            let size = (ui.available_width() / HEATMAP_WEEKS as f32).max(CELL_MIN);
            // The scroll area: when cells hit the floor and still don't fit (window < 53 × 6px), scroll sideways.
            egui::ScrollArea::horizontal()
                .id_salt("usage_heatmap")
                .show(ui, |ui| {
                    let grid = range::heatmap_grid(&daily, today, HEATMAP_WEEKS);
                    let totals: Vec<i64> = grid.iter().map(|cell| cell.total).collect();
                    let thresholds = range::heat_thresholds(&totals);
                    let gap = (size * 2.0 / 13.0).min(2.0); // About 2px at fixed width, scaling with the cells
                    let width = HEATMAP_WEEKS as f32 * size;
                    let height = MONTH_ROW_HEIGHT + 7.0 * size;
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());

                    // The hovered cell: the whole heatmap is one response, so the tooltip must position itself via the
                    // computed `hit` — `response.on_hover_text` would pin the tip to the whole block's top-right corner,
                    // not the mouse.
                    let mut hit: Option<(&range::HeatCell, egui::Rect)> = None;

                    if ui.is_rect_visible(rect) {
                        let painter = ui.painter_at(rect);
                        let step = size - gap;
                        let grid_top = rect.min.y + MONTH_ROW_HEIGHT;
                        let hover_pos = ui.ctx().pointer_hover_pos();

                        for (index, cell) in grid.iter().enumerate() {
                            let week = index / 7;
                            let weekday = index % 7;
                            let min = egui::pos2(
                                rect.min.x + week as f32 * size,
                                grid_top + weekday as f32 * size,
                            );
                            let cell_rect = egui::Rect::from_min_size(min, egui::vec2(step, step));
                            let level = range::heat_level(cell.total, &thresholds);
                            painter.rect_filled(cell_rect, 2.0, heat_color(level, accent, ui));

                            if hover_pos.is_some_and(|pos| cell_rect.contains(pos)) {
                                hit = Some((cell, cell_rect));
                            }
                        }

                        // Month labels: marked when a column head first shows a "new month".
                        let mut last_month = 0i64;
                        for week in 0..HEATMAP_WEEKS {
                            let day = grid[week * 7].day;
                            let (_, month, _) = aggregate::civil_from_days(day);
                            if month != last_month {
                                last_month = month;
                                painter.text(
                                    egui::pos2(rect.min.x + week as f32 * size, rect.min.y),
                                    egui::Align2::LEFT_TOP,
                                    format!("{month} 月"),
                                    egui::FontId::proportional(TEXT_CAPTION),
                                    weak_text,
                                );
                            }
                        }
                    }

                    // The pointer hovers a cell: the tip follows the mouse, never the block's top-right.
                    if let Some((cell, _)) = hit {
                        egui::Tooltip::for_widget(&response)
                            .at_pointer()
                            .show(|ui| {
                                // Neither line wraps (Extend): a wrapped usage line would split the number.
                                for line in cell_hover_lines(cell) {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(line)
                                                .size(crate::theme::TEXT_CAPTION),
                                        )
                                        .wrap_mode(egui::TextWrapMode::Extend),
                                    );
                                }
                            });
                    }

                    ui.add_space(crate::theme::SPACE_2);
                    heatmap_legend(ui, accent, weak_text);
                });
        });
    }
}

/// Composition bar: four buckets laid out proportionally in one horizontal bar + a legend row.
///
/// Beats a small-print line like "input x · output y · cache read z · cache write w": cache reads
/// often exceed 90%, and without proportions you can't tell where the tokens actually went.
///
/// Widths are proportional to share; the whole bar is skipped when total usage is 0 (the overview
/// card already shows an empty state).
fn composition_bar(ui: &mut egui::Ui, totals: &crate::usage::DayTotals, accent: egui::Color32) {
    let total = totals.total();
    if total <= 0 {
        return;
    }
    let dark = ui.visuals().dark_mode;
    let buckets = [
        ("输入", totals.input),
        ("输出", totals.output),
        ("缓存读", totals.cache_read),
        ("缓存写", totals.cache_write),
    ];

    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), COMPOSITION_BAR_HEIGHT),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    let radius = 3.0;
    let mut x = rect.left();
    for (index, (_, value)) in buckets.iter().enumerate() {
        if *value <= 0 {
            continue;
        }
        let width = rect.width() * (*value as f32 / total as f32);
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(x, rect.top()),
                egui::vec2(width.max(1.0), rect.height()),
            ),
            radius,
            super::series_color(index, accent, dark),
        );
        x += width;
    }

    ui.add_space(crate::theme::SPACE_2);
    let weak = ui.visuals().weak_text_color();
    ui.horizontal_wrapped(|ui| {
        for (index, (label, value)) in buckets.iter().enumerate() {
            let (swatch, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
            ui.painter()
                .rect_filled(swatch, 2.0, super::series_color(index, accent, dark));
            // The whole legend (bucket name + share + value) goes through one digit-roll: non-digits already paint
            // statically, all characters share one slot top, so "the number sits half a line above the text" can't
            // happen (label vertical-centering versus top-painted rolling numbers would differ by half a line).
            crate::motion::rolling_number(
                ui,
                ui.id().with(("bucket", index)),
                &format!(
                    "{label} {} · {}",
                    percent_of(*value, total),
                    aggregate::format_tokens(*value)
                ),
                |s| {
                    egui::RichText::new(s)
                        .size(crate::theme::TEXT_CAPTION)
                        .color(weak)
                },
            );
        }
    });
}

/// Composition bar height.
const COMPOSITION_BAR_HEIGHT: f32 = 8.0;

/// One stat item in the overview card.
struct Stat {
    /// The small label (above the value).
    label: &'static str,
    /// The value (large, monospace).
    value: String,
    /// The supplement (date, usage), on a line below the value; absent when there is none.
    detail: Option<String>,
}

/// Stat grid: [`STAT_COLUMNS`] per row, block widths split evenly across the available width.
///
/// Even split instead of fixed widths: fixed widths overflow narrow windows, forcing wraps, and any
/// wrap breaks row alignment. Even split plus in-block truncation keeps "equal widths per row,
/// rows aligned" at every window width.
fn stat_grid(ui: &mut egui::Ui, stats: &[Stat]) {
    let spacing = ui.spacing().item_spacing.x;
    let columns = STAT_COLUMNS as f32;
    let width = ((ui.available_width() - spacing * (columns - 1.0)) / columns).max(1.0);
    for (row_index, row) in stats.chunks(STAT_COLUMNS).enumerate() {
        if row_index > 0 {
            // The two stat rows sit closer (user feedback: too much space between active-days and peak-day rows).
            ui.add_space(crate::theme::SPACE_1);
        }
        // `horizontal_top` instead of `horizontal`: the latter centers vertically, so with unequal block heights
        // (some blocks lack supplements) labels in the same row stagger.
        ui.horizontal_top(|ui| {
            for stat in row {
                stat_block(ui, width, stat);
            }
        });
    }
}

/// One stat block: small label on top, value in the middle, supplementary line below.
///
/// Values use the monospace face: side by side, monospace keeps number columns visually aligned
/// instead of jittering with glyph widths. No wrapping inside a block (over-wide truncates to
/// ellipses, full text on hover) — wrapping would make block heights in the same row uneven and the
/// whole group look misaligned.
///
/// Block height is always computed as three lines even when the supplement is absent: if any block in
/// one row is shorter, the next row's origin shifts and alignment breaks.
fn stat_block(ui: &mut egui::Ui, width: f32, stat: &Stat) {
    let weak = ui.visuals().weak_text_color();
    let line = |ui: &mut egui::Ui, size: f32| {
        ui.fonts_mut(|fonts| fonts.row_height(&egui::FontId::proportional(size)))
    };
    let spacing = ui.spacing().item_spacing.y;
    let height = line(ui, crate::theme::TEXT_CAPTION)
        + spacing
        + line(ui, crate::theme::TEXT_TITLE)
        + spacing
        + line(ui, crate::theme::TEXT_CAPTION);
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        egui::Layout::top_down(egui::Align::LEFT),
        |ui| {
            ui.set_width(width);
            ui.label(
                egui::RichText::new(stat.label)
                    .size(crate::theme::TEXT_CAPTION)
                    .color(weak),
            );
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&stat.value)
                        .size(crate::theme::TEXT_TITLE)
                        .strong()
                        .monospace(),
                )
                .truncate(),
            )
            .on_hover_text(&stat.value);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(stat.detail.as_deref().unwrap_or_default())
                        .size(crate::theme::TEXT_CAPTION)
                        .color(weak),
                )
                .truncate(),
            )
            .on_hover_text(stat.detail.as_deref().unwrap_or_default());
        },
    );
}

/// Stats per row: six stat items in two rows of three.
const STAT_COLUMNS: usize = 3;

/// Heatmap cell colors: level 0 is an empty cell (weak color); levels 1-4 raise accent opacity.
///
/// "Background + accent alpha blend" instead of four fixed colors: the eight themes have different
/// accents, and fixed colors would clash with some backgrounds.
fn heat_color(level: u8, accent: egui::Color32, ui: &egui::Ui) -> egui::Color32 {
    if level == 0 {
        // No data: a neutral slightly deeper than the background — never pure transparent (the grid would vanish).
        return ui.visuals().widgets.noninteractive.bg_fill;
    }
    // Levels 1-4 → alpha 40 / 90 / 150 / 235.
    let alpha = [40u8, 90, 150, 235][(level.clamp(1, 4) - 1) as usize];
    egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), alpha)
}

/// Legend: `less ■■■■ more`.
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

/// Cell hover text: two lines — date on the first, usage on the second (user-requested format).
/// Hover tip layout: one line for the date, one for usage.
fn cell_hover_lines(cell: &range::HeatCell) -> [String; 2] {
    let (year, month, day) = aggregate::civil_from_days(cell.day);
    let date = format!("{year:04}-{month:02}-{day:02}");
    let amount = if cell.has_data {
        format!("{} tokens", aggregate::format_m(cell.total))
    } else {
        "无记录".to_string()
    };
    [date, amount]
}

/// Share text: `12.3%`; renders `—` when the denominator is 0.
pub(in crate::app) fn percent_of(part: i64, whole: i64) -> String {
    if whole <= 0 {
        return "—".to_string();
    }
    let pct = (part as f64 / whole as f64) * 100.0;
    if pct < 0.1 && part > 0 {
        // Non-zero but under 0.1%: "<0.1%" is more honest than rounding to 0.0%.
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
        // Has usage but shouldn't display as 0.0%.
        assert_eq!(percent_of(1, 1_000_000), "<0.1%");
    }

    #[test]
    fn hover_text_distinguishes_missing_from_zero() {
        let cell = range::HeatCell {
            day: 0,
            total: 0,
            has_data: false,
        };
        let lines = cell_hover_lines(&cell);
        assert_eq!(lines[1], "无记录");

        let cell = range::HeatCell {
            day: 0,
            total: 0,
            has_data: true,
        };
        // Two-line format: the date on the first line, the M-unit usage on the second.
        let lines = cell_hover_lines(&cell);
        assert_eq!(lines[0], "1970-01-01", "第一行是年月日");
        assert_eq!(lines[1], "0.0M tokens", "第二行是 M 单位");
    }
}
