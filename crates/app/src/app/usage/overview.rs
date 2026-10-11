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

/// 热力图格子边长的下限：再窄分不清颜色深浅，超出后横向滚动。
/// 没有上限：窗口放大时格子同步变大，铺满卡片宽度（用户反馈）。
const CELL_MIN: f32 = 6.0;

/// 热力图展示多少周：53 周 ≈ 一年（照 GitHub 贡献图）。
const HEATMAP_WEEKS: usize = 53;

/// 顶部月份标签行高。
const MONTH_ROW_HEIGHT: f32 = 16.0;

/// 区间切换按钮宽度。
const RANGE_BUTTON_WIDTH: f32 = 56.0;

/// 小标签字号（用于 painter 画的文字）。
const TEXT_CAPTION: f32 = crate::theme::TEXT_CAPTION;

impl App {
    /// 总览卡 + 趋势图 + 热力图（区间按钮在 [`super::view`] 的双栏之上）。
    pub(in crate::app) fn ui_usage_overview(&mut self, ui: &mut egui::Ui) {
        self.ui_usage_summary(ui);
        ui.add_space(crate::theme::SPACE_4);
        self.ui_usage_chart(ui);
        ui.add_space(crate::theme::SPACE_4);
        self.ui_usage_heatmap(ui);
    }

    /// 控件行：区间切换（今日 / 本周 / 本月 / 全部）。
    ///
    /// 由 [`super::view`] 画在双栏之上：横跨整个窗口宽度保证单一行，
    /// 右列的 Agent 卡也和左列第一张卡顶部平齐（用户反馈）。
    pub(in crate::app) fn ui_usage_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            self.range_buttons(ui);
        });
    }

    /// 区间切换：今日 / 本周 / 本月 / 全部。
    fn range_buttons(&mut self, ui: &mut egui::Ui) {
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
    }

    /// 总览卡：当前区间的合计 + 构成条 + 汇总事实。
    ///
    /// 区间切换按钮不在这里：它要横跨整个窗口宽度单独一行（用户反馈：
    /// 今日 / 本周 / 本月 / 全部单一行），由 [`super::view`] 画在双栏之上 ——
    /// 这样右列的 Agent 卡也和左列第一张卡顶部平齐。
    fn ui_usage_summary(&mut self, ui: &mut egui::Ui) {
        let daily = self.usage_daily_filtered();
        let today = self.usage_today();
        let range = self.usage_range;
        let stats = range::range_stats(&daily, range, today);
        let all = range::totals_in_range(&daily, Range::All, today);
        // 连续天数不看区间：它描述的是「当前状态」。
        let streak = range::current_streak(&daily, today);
        let weak = ui.visuals().weak_text_color();

        super::component::card(ui, |ui| {
            // 主数字：照参考实现把「大数 + 小标签」放在一起，一眼看到量级。
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(aggregate::format_full(stats.totals.total()))
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

            // ≈ 行：量级的近似读法，与 tokenmonitor 一致（M，超 1B 用 B）。
            // 金额行已删（用户反馈：不要花费）；定价算出的成本仍留在核心层，
            // 界面不再展示。主数字是全数字，这行给一眼可读的量级。
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "≈ {}",
                        aggregate::format_tokens(stats.totals.total())
                    ))
                    .size(crate::theme::TEXT_TITLE)
                    .strong()
                    .monospace(),
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

            // 构成条：四个桶的占比一眼可见 —— 缓存读常常占九成以上，
            // 不给比例看不出量到底花在哪。
            composition_bar(ui, &stats.totals, self.theme.accent_color());

            ui.add_space(crate::theme::SPACE_3);

            // 四个统计块：总量之外补上频率与峰值，单看总量看不出使用习惯。
            // 汇总事实：总量之外补上频率、峰值与偏好。照参考实现
            // （tokscale 的 Stats 页）列出这类事实 —— 单看总量看不出使用习惯。
            //
            // 日期、模型用量这类「长的上下文」放到值下面一行，
            // 而不是塞进值的括号里：塞在一起会顶到块外，只能折行。
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

    /// 热力图：53 周 × 7 天，用 `Painter` 画矩形。
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

            // 格子随可用宽度同步缩放（用户反馈：窗口放大时热力图也要同步扩大）：
            // 53 周除以可用宽度，只夹下限 —— 窗口越宽格子越大，铺满卡片宽度。
            // **必须**在进入横向 ScrollArea 之前算 —— 里面 available_width 是无穷大。
            let size = (ui.available_width() / HEATMAP_WEEKS as f32).max(CELL_MIN);
            // 滚动区域：格子压到下限还放不下时（窗口 < 53 × 6px）横向可滚。
            egui::ScrollArea::horizontal()
                .id_salt("usage_heatmap")
                .show(ui, |ui| {
                    let grid = range::heatmap_grid(&daily, today, HEATMAP_WEEKS);
                    let totals: Vec<i64> = grid.iter().map(|cell| cell.total).collect();
                    let thresholds = range::heat_thresholds(&totals);
                    let gap = (size * 2.0 / 13.0).min(2.0); // 定宽时约 2px，随格子缩放
                    let width = HEATMAP_WEEKS as f32 * size;
                    let height = MONTH_ROW_HEIGHT + 7.0 * size;
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());

                    // 命中的格子：整个热力图只有一个 response，tooltip 必须靠
                    // 自己算出的 `hit` 定位，不能用 `response.on_hover_text`
                    // （那会把提示钉在整块的右上角，而不是鼠标处）。
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

                        // 月份标签：每列头一次出现「新的月份」时标一下。
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

                    // 鼠标悬停在某个格子上：提示跟在鼠标旁，不在整块右上角。
                    if let Some((cell, _)) = hit {
                        egui::Tooltip::for_widget(&response)
                            .at_pointer()
                            .show(|ui| {
                                // 两行都不换行（Extend）：用量行折行会把数字切开。
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

/// 构成条：四个桶按占比排成一条横条 + 一行图例。
///
/// 比一行「输入 x · 输出 y · 缓存读 z · 缓存写 w」的小字强：缓存读常常占
/// 九成以上，不给比例看不出量到底花在哪。
///
/// 宽度按占比分配；总用量为 0 时整条不画（总览卡已经给了空状态提示）。
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
            ui.label(
                egui::RichText::new(format!(
                    "{label} {} · {}",
                    percent_of(*value, total),
                    aggregate::format_tokens(*value)
                ))
                .size(crate::theme::TEXT_CAPTION)
                .color(weak),
            );
        }
    });
}

/// 构成条高度。
const COMPOSITION_BAR_HEIGHT: f32 = 8.0;

/// 总览卡里的一个统计项。
struct Stat {
    /// 小标签（在值上面）。
    label: &'static str,
    /// 值（大字、等宽）。
    value: String,
    /// 补充说明（日期、用量），排在值下面一行；没有就少一行。
    detail: Option<String>,
}

/// 统计项网格：每排 [`STAT_COLUMNS`] 个，块宽按可用宽度均分。
///
/// 按可用宽度均分而不是用固定宽度：固定宽度在窄窗口里放不下，
/// 排不下就换行，一换行上下两排就对不齐。均分配合块内截断，
/// 任何窗口宽度下都是「每排等宽、上下对齐」。
fn stat_grid(ui: &mut egui::Ui, stats: &[Stat]) {
    let spacing = ui.spacing().item_spacing.x;
    let columns = STAT_COLUMNS as f32;
    let width = ((ui.available_width() - spacing * (columns - 1.0)) / columns).max(1.0);
    for (row_index, row) in stats.chunks(STAT_COLUMNS).enumerate() {
        if row_index > 0 {
            // 两排统计贴紧些（用户反馈：活跃天数与最高一天两排间距太大）。
            ui.add_space(crate::theme::SPACE_1);
        }
        // 用 `horizontal_top` 而不是 `horizontal`：后者按垂直居中排列，
        // 块高不等时（有的块没有补充说明）同一排的标签会上下错开。
        ui.horizontal_top(|ui| {
            for stat in row {
                stat_block(ui, width, stat);
            }
        });
    }
}

/// 一个统计块：小标签在上、值在中、补充说明在下。
///
/// 值用等宽体：并排时等宽能让数字列在视觉上对齐，不会因为字形宽度不同而跳。
/// 块内一律不换行（超宽就省略号截断，全文走悬停）—— 换行会让同一排的
/// 块高参差，整块看起来像没对齐。
///
/// 块高恒按三行算，即使这一项没有补充说明：两排里只要有块高不同，
/// 下一排的起始位置就会不一样。
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

/// 统计项每排几个：六个统计项排成两排，每排三个。
const STAT_COLUMNS: usize = 3;

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

/// 格子的悬停文案：两行 —— 第一行年月日，第二行用量（用户要求的格式）。
/// 悬停提示的两行文案：日期一行、用量一行。
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

/// 占比文案：`12.3%`；分母为 0 时给 `—`。
pub(in crate::app) fn percent_of(part: i64, whole: i64) -> String {
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
        let lines = cell_hover_lines(&cell);
        assert_eq!(lines[1], "无记录");

        let cell = range::HeatCell {
            day: 0,
            total: 0,
            has_data: true,
        };
        // 两行格式：第一行年月日，第二行 M 单位用量。
        let lines = cell_hover_lines(&cell);
        assert_eq!(lines[0], "1970-01-01", "第一行是年月日");
        assert_eq!(lines[1], "0.0M tokens", "第二行是 M 单位");
    }
}
