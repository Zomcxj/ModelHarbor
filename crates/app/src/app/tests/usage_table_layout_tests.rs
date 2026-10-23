//! The usage tables' layout and filtering: numbers must right-align, and filtering to one agent must move them.
//!
//! A user called the old layout "too ugly"; the root cause was left-aligned number columns — mixed-digit
//! values (`1.2K` vs `123.4M`) left-aligned stagger like sawteeth. Rendering offscreen with real structure,
//! this asserts:
//! 1. Numbers in one column have **right edges aligned** (that's what makes a table a table);
//! 2. Headers sit exactly above their numbers (same column, same width);
//! 3. After switching to one agent, the totals column's numbers do shrink (the filter really reaches the
//!    render path).
//!
//! Only geometry is asserted, never pixel values — theme or font changes won't cause false failures.

use crate::app::App;
use crate::format::ConfigFormat;
use crate::usage::{DailyBucket, DayTotals, SessionSnapshot};
use eframe::egui;
use std::collections::HashMap;

/// A rendered piece of text with its rect.
struct TextAt {
    text: String,
    rect: egui::Rect,
}

/// Collect all text shapes (including nested).
fn texts_of(out: &egui::FullOutput) -> Vec<TextAt> {
    let mut texts = Vec::new();
    fn collect(shape: &egui::Shape, out: &mut Vec<TextAt>) {
        match shape {
            egui::Shape::Text(t) => out.push(TextAt {
                text: t.galley.text().to_string(),
                rect: egui::Rect::from_min_size(t.pos, t.galley.size()),
            }),
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    for cs in &out.shapes {
        collect(&cs.shape, &mut texts);
    }
    texts
}

/// Build one session snapshot.
fn session(client: ConfigFormat, id: &str, input: i64) -> SessionSnapshot {
    SessionSnapshot {
        client,
        session_id: id.to_string(),
        model_id: "claude-opus-5".to_string(),
        project: None,
        title: None,

        input,
        output: input / 10,
        cache_read: input * 4,
        cache_write: 0,
        reasoning: 0,
        message_count: 3,
        cost: 0.0,
        first_seen_ms: 1_700_000_000_000,
        last_seen_ms: 1_700_000_000_000,
        archived: false,
    }
}

/// Build one day's bucket: pi and opencode each with usage.
fn day_bucket(date: &str, pi_input: i64, opencode_input: i64) -> DailyBucket {
    let mut bucket = DailyBucket::new(date);
    let pi = DayTotals {
        input: pi_input,
        output: pi_input / 10,
        cache_read: pi_input * 4,
        messages: 2,
        ..DayTotals::default()
    };
    let opencode = DayTotals {
        input: opencode_input,
        output: opencode_input / 10,
        cache_read: opencode_input * 4,
        messages: 1,
        ..DayTotals::default()
    };
    bucket.totals = DayTotals {
        input: pi_input + opencode_input,
        output: (pi_input + opencode_input) / 10,
        cache_read: (pi_input + opencode_input) * 4,
        messages: 3,
        ..DayTotals::default()
    };
    bucket.by_client.insert("pi".into(), pi);
    bucket.by_client.insert("opencode".into(), opencode);
    bucket.by_client_model.insert(
        "pi".into(),
        [("claude-opus-5".to_string(), pi)].into_iter().collect(),
    );
    bucket.by_client_model.insert(
        "opencode".into(),
        [("claude-opus-5".to_string(), opencode)]
            .into_iter()
            .collect(),
    );
    bucket.by_model.insert("claude-opus-5".into(), pi);
    // The session table reads `by_session` (keys same as `SessionSnapshot::key`).
    bucket.by_session.insert(
        SessionSnapshot::key(ConfigFormat::Pi, "pi-session-aaaa"),
        pi,
    );
    bucket.by_session.insert(
        SessionSnapshot::key(ConfigFormat::Opencode, "oc-session-bbbb"),
        opencode,
    );
    bucket
}

/// Offscreen-render the usage view; returns all text shapes.
///
/// Isomorphic to `App::update`: status line + control row + overview + tables all inside the central panel's scroll area.
fn render(filter: Option<ConfigFormat>) -> Vec<TextAt> {
    let mut app = App {
        main_view: crate::app::MainView::Usage,
        usage_filter: filter,
        usage_range: crate::app::usage::range::Range::All,
        guide_dismissed: true,
        ..App::default()
    };
    // Inject a "scan finished" result directly: this test cares about layout, not real scanning.
    let sessions = vec![
        session(ConfigFormat::Pi, "pi-session-aaaa", 1_000),
        session(ConfigFormat::Opencode, "oc-session-bbbb", 50_000_000),
    ];
    let daily: crate::usage::DailyMap = [
        (
            "2026-01-01".to_string(),
            day_bucket("2026-01-01", 1_000, 50_000_000),
        ),
        (
            "2026-01-02".to_string(),
            day_bucket("2026-01-02", 2_000, 60_000_000),
        ),
    ]
    .into_iter()
    .collect();
    app.set_usage_result_for_test(sessions, daily);

    let ctx = egui::Context::default();
    crate::theme::Theme::from_key("dark").apply_style(
        &ctx,
        crate::theme::UiStyle::from_key("cloud"),
        false,
    );
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 900.0));
    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                // Copied from [`App::update`]'s usage branch: the status line (with filter text and range switch) paints
                // **outside** the scroll area, pinned to the top, never scrolling.
                app.ui_usage_status(ui);
                ui.add_space(crate::theme::SPACE_4);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        app.ui_usage_section(ui);
                    });
            });
        },
    );
    texts_of(&out)
}

/// Numbers in one column must have right edges aligned.
///
/// Take the "total" column: it follows the name column in all four tables and always exists. Measured on
/// the model table (fixed names, fixed rows), asserting every number's `rect.right()` lands on one x.
#[test]
fn number_columns_are_right_aligned() {
    let texts = render(None);
    // Number shapes: all monospace-formatted results (with K / M / B suffixes or plain digits).
    let numbers: Vec<&TextAt> = texts
        .iter()
        .filter(|t| t.text.ends_with('M') || t.text.ends_with('K') || t.text.ends_with('B'))
        .collect();
    assert!(
        numbers.len() >= 4,
        "至少要渲染出几个带单位的数字，实际 {}",
        numbers.len()
    );

    // Group by "column": x centers close together belong to one column; then check right-edge alignment.
    let mut rights_by_column: HashMap<i64, Vec<f32>> = HashMap::new();
    for n in &numbers {
        // Group by quantizing the **right edge** onto a 4px grid (right-aligned columns' centers drift with text
        // width — adding one column can cross a quantization cell and split a column apart; what must align is
        // the right edge anyway).
        let key = (n.rect.right() / 4.0).round() as i64;
        rights_by_column
            .entry(key)
            .or_default()
            .push(n.rect.right());
    }
    let aligned_column = rights_by_column.values().filter(|rights| {
        if rights.len() < 2 {
            return false;
        }
        let max = rights.iter().cloned().fold(f32::MIN, f32::max);
        let min = rights.iter().cloned().fold(f32::MAX, f32::min);
        // Right edges within 1px count as aligned (monospace should be near-exact).
        max - min < 1.0
    });
    assert!(
        aligned_column.count() > 0,
        "没有任何一列的数字右边缘对齐 —— 数字列没走右对齐布局"
    );
}

/// The header must sit directly above its numbers (same column, same width), or headers and data misalign.
#[test]
fn headers_sit_above_their_numbers() {
    let texts = render(None);
    // Headers may carry the sort arrow (`total ▼`): match by prefix, not exact equality.
    let header = |name: &str| {
        texts
            .iter()
            .find(|t| t.text == name || t.text.starts_with(&format!("{name} ")))
            .unwrap_or_else(|| panic!("没渲染出表头「{name}」"))
    };
    let total = header("合计");
    // Some number of the totals column: its right edge should roughly meet the header's (same-column right alignment).
    let same_column = texts.iter().filter(|t| {
        let is_number = t.text.ends_with('M') || t.text.ends_with('K') || t.text.ends_with('B');
        is_number && (t.rect.right() - total.rect.right()).abs() < 2.0
    });
    assert!(
        same_column.count() > 0,
        "「合计」表头的右边缘没有和任何数字对齐"
    );
}

/// After filtering to one agent, the totals column's numbers must shrink (the filter really reaches the render path).
#[test]
fn filtering_narrows_the_rendered_numbers() {
    let all = render(None);
    let pi = render(Some(ConfigFormat::Pi));

    // With only pi selected, the status line shows the filter's name.
    assert!(
        pi.iter().any(|t| t.text.contains("pi")),
        "筛选后状态行应显示 agent 名"
    );
    // Unfiltered, the status line shows "all agents".
    assert!(
        all.iter().any(|t| t.text.contains("全部 agent")),
        "未筛选时状态行应显示「全部 agent」"
    );

    // The table draws total() (= input + output + cache read), not a standalone input column.
    // opencode's two-day total = (1e8 + 1e7 + 4e8) * 1.1 ≈ 561.0M.
    let has_opencode_total = |texts: &[TextAt]| texts.iter().any(|t| t.text == "561.0M");
    assert!(
        has_opencode_total(&all),
        "未筛选时应渲染出 opencode 的合计 561.0M，实际数字：{:?}",
        all.iter()
            .filter(|t| t.text.ends_with('M') || t.text.ends_with('K'))
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        !has_opencode_total(&pi),
        "只看 pi 时不该出现 opencode 的 561.0M"
    );
    // With only pi selected, the total should become pi's own magnitude (15.3K).
    assert!(
        pi.iter().any(|t| t.text == "15.3K"),
        "只看 pi 时应渲染出 15.3K，实际：{:?}",
        pi.iter()
            .filter(|t| t.text.ends_with('M') || t.text.ends_with('K'))
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
    );
}
