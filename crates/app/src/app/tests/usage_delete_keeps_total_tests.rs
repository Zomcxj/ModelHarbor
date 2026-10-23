//! Probe: does "source deleted, details really removed, totals kept" hold end to end when deleting a session?
//!
//! The requirement has two faces:
//! - User-visible: the deleted session leaves the list, but its already-counted tokens stay in the totals /
//!   heatmap / time table and the per-agent and per-model dimensions;
//! - Invisible: session details (snapshots and per-day `by_session`) really delete, so the ledger never
//!   grows without bound with session count.
//!
//! Goes through `apply_session_removal` (the deletion's second half) rather than `forget_usage_session`: the
//! latter calls `usage::delete_session` and would really touch this machine's agents' session stores — tests
//! must not touch user data. Source-side results are constructed here directly, covering both "deleted" and
//! "blocked" paths.

use crate::app::usage::range::{totals_in_range, Range};
use crate::app::App;
use crate::format::ConfigFormat;
use crate::usage::{DailyBucket, DailyMap, DayTotals, DeleteError, Deleted, SessionSnapshot};

/// A successful source-deletion result (test use: path and row count take no part in the second half's logic).
fn source_deleted() -> Result<Deleted, DeleteError> {
    Ok(Deleted {
        path: std::path::PathBuf::from("C:/fake/opencode.db"),
        rows: 13,
    })
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
        cache_read: 0,
        cache_write: 0,
        reasoning: 0,
        message_count: 3,
        cost: 0.0,
        first_seen_ms: 1_700_000_000_000,
        last_seen_ms: 1_700_000_000_000,
        archived: false,
    }
}

/// Build one day's bucket (one entry per of the two sessions).
fn day_bucket(date: &str, a_input: i64, b_input: i64) -> DailyBucket {
    let a = DayTotals {
        input: a_input,
        output: a_input / 10,
        messages: 1,
        ..DayTotals::default()
    };
    let b = DayTotals {
        input: b_input,
        output: b_input / 10,
        messages: 1,
        ..DayTotals::default()
    };
    let mut bucket = DailyBucket::new(date);
    bucket.totals = DayTotals {
        input: a_input + b_input,
        output: (a_input + b_input) / 10,
        messages: 2,
        ..DayTotals::default()
    };
    bucket.by_client.insert("pi".into(), a);
    bucket.by_client.insert("opencode".into(), b);
    bucket
        .by_model
        .insert("claude-opus-5".into(), bucket.totals);
    bucket
        .by_session
        .insert(SessionSnapshot::key(ConfigFormat::Pi, "pi-keep-me"), a);
    bucket.by_session.insert(
        SessionSnapshot::key(ConfigFormat::Opencode, "oc-delete-me"),
        b,
    );
    bucket.by_client_model.insert(
        "pi".into(),
        [("claude-opus-5".to_string(), a)].into_iter().collect(),
    );
    bucket.by_client_model.insert(
        "opencode".into(),
        [("claude-opus-5".to_string(), b)].into_iter().collect(),
    );
    bucket
}

/// Construct an App with a scan result injected.
fn app_with_data() -> App {
    let mut app = App {
        main_view: crate::app::MainView::Usage,
        guide_dismissed: true,
        ..App::default()
    };
    let sessions = vec![
        session(ConfigFormat::Pi, "pi-keep-me", 1_000),
        session(ConfigFormat::Opencode, "oc-delete-me", 5_000),
    ];
    let daily: DailyMap = [(
        "2026-01-01".to_string(),
        day_bucket("2026-01-01", 1_000, 5_000),
    )]
    .into_iter()
    .collect();
    app.set_usage_result_for_test(sessions, daily);
    app
}

/// The in-range total (the all range).
fn total_of(app: &App) -> i64 {
    let daily = app.usage_daily_filtered();
    let today = app.usage_today();
    totals_in_range(&daily, Range::All, today).total()
}

/// The deleted session's ledger key.
fn deleted_key() -> String {
    SessionSnapshot::key(ConfigFormat::Opencode, "oc-delete-me")
}

/// Delete the session: one fewer list row, totals unchanged.
#[test]
fn deleting_a_session_hides_the_row_but_keeps_the_total() {
    let mut app = app_with_data();
    let before_rows = app.usage_sessions().len();
    let before_total = total_of(&app);
    assert_eq!(before_rows, 2, "注入两条会话");
    assert_eq!(before_total, 6_000 + 600, "1_000 + 100 + 5_000 + 500");

    let key = deleted_key();
    app.apply_session_removal(&key, source_deleted(), 0.0);

    assert_eq!(
        app.usage_sessions().len(),
        before_rows - 1,
        "被删的会话要从列表里消失"
    );
    assert!(
        !app.usage_sessions()
            .iter()
            .any(|s| s.session_id == "oc-delete-me"),
        "被删的会话不许出现在列表里"
    );
    assert_eq!(total_of(&app), before_total, "总量不因删除缩水");
    assert!(app.usage_source_error.is_none(), "源删成了就不该报错");
}

/// After deletion, every dimension's breakdown is preserved too (per-agent filtering mustn't shrink),
/// while the session's **details** really delete (the snapshot leaves the list, and per-day `by_session` keeps nothing).
#[test]
fn deleting_a_session_keeps_every_breakdown() {
    let mut app = app_with_data();
    let before_total = total_of(&app);

    let key = deleted_key();
    app.apply_session_removal(&key, source_deleted(), 0.0);

    let daily = app.usage_daily_filtered();
    let bucket = daily.get("2026-01-01").expect("分桶还在");
    assert_eq!(bucket.totals.input, 6_000, "合计保留");
    assert_eq!(bucket.by_client["opencode"].input, 5_000, "按 agent 保留");
    assert_eq!(bucket.by_model["claude-opus-5"].input, 6_000, "按模型保留");
    assert_eq!(
        bucket.by_client_model["opencode"]["claude-opus-5"].input, 5_000,
        "交叉分解保留"
    );
    assert!(
        !bucket.by_session.contains_key(&key),
        "会话明细真删：留着会随会话数无上限增长"
    );

    // Filter to the deleted session's agent: the numbers hold (going through the UI's real filtering path).
    // Save the unfiltered total first: once the filter is set, `total_of` reads the filtered number.
    let full_total = total_of(&app);
    app.usage_filter = Some(ConfigFormat::Opencode);
    let scoped = app.usage_daily_filtered();
    let scoped_bucket = scoped.get("2026-01-01").expect("筛选后分桶还在");
    assert_eq!(
        scoped_bucket.totals.input, 5_000,
        "筛到该 agent 时合计换成该 agent 的分解"
    );
    assert_eq!(full_total, before_total, "全量总量不变");
}

/// Deleting the same session twice: idempotent — no double subtraction, no error.
#[test]
fn deleting_the_same_session_twice_is_idempotent() {
    let mut app = app_with_data();
    let before_total = total_of(&app);
    let key = deleted_key();

    app.apply_session_removal(&key, source_deleted(), 0.0);
    let after_first = total_of(&app);
    app.apply_session_removal(&key, source_deleted(), 0.0);

    assert_eq!(after_first, before_total, "第一次删除不缩水");
    assert_eq!(total_of(&app), before_total, "第二次删除同样不缩水");
    assert_eq!(app.usage_sessions().len(), 1, "行数稳定");
}

/// Source undeletable but user-fixable (database locked / no permission): **neither half proceeds** — once the
/// row leaves the list the user can never click it again, and the source session would be stuck undeletable.
#[test]
fn a_blocked_source_delete_keeps_the_row_and_reports_why() {
    let mut app = app_with_data();
    let before_total = total_of(&app);
    let key = deleted_key();

    app.apply_session_removal(
        &key,
        Err(DeleteError::Failed {
            detail: "database is locked".to_string(),
        }),
        0.0,
    );

    assert_eq!(
        app.usage_sessions().len(),
        2,
        "源没删成，行要留在列表里让用户重试"
    );
    assert_eq!(total_of(&app), before_total, "总量也没动");
    // Per-day details persist too: the next retry walks the same data again.
    assert!(app
        .usage_daily_filtered()
        .get("2026-01-01")
        .expect("分桶还在")
        .by_session
        .contains_key(&key));
    let message = app.usage_source_error.clone().expect("要给出失败原因");
    assert!(
        message.contains("database is locked") && message.contains("源未改动"),
        "提示要说清失败原因与「源没被动过」：{message}"
    );
}

/// Source undeletable and user-**unfixable** (the backend can't delete sources): statistics proceed as usual —
/// keeping it would only leave a permanently stuck undeletable session in the list.
#[test]
fn an_unfixable_source_delete_still_clears_the_stats() {
    let mut app = app_with_data();
    let before_total = total_of(&app);
    let key = deleted_key();

    app.apply_session_removal(
        &key,
        Err(DeleteError::Unsupported {
            client: "workbuddy",
        }),
        0.0,
    );

    assert_eq!(app.usage_sessions().len(), 1, "统计里要收走");
    assert_eq!(total_of(&app), before_total, "已计入的用量仍然保留");
    let message = app.usage_source_error.clone().expect("要说清源没删");
    assert!(
        message.contains("workbuddy"),
        "提示要点名哪个后端不支持：{message}"
    );
}
