use crate::app::preview::{diff_recompute_due, diff_signature, diff_step, preview_should_rebuild};

#[test]
fn rebuild_gate_ignores_focus_but_keeps_user_text() {
    // Not hand-edited in the preview: rebuild from component state
    assert!(preview_should_rebuild(false, None, 100.0));
    // Just typed in the preview (within 2 seconds): keep the user's text
    assert!(!preview_should_rebuild(false, Some(99.5), 100.0));
    assert!(!preview_should_rebuild(false, Some(100.0), 100.0));
    // Input idle over 2 seconds: back to component state
    assert!(preview_should_rebuild(false, Some(97.9), 100.0));
    // The last parse failed: keep the user's text
    assert!(!preview_should_rebuild(true, None, 100.0));
    assert!(!preview_should_rebuild(true, Some(1.0), 100.0));
}

/// The signature separates "the draft changed" from "a write happened".
#[test]
fn diff_signature_changes_with_draft_and_with_save_count() {
    let base = diff_signature("/tmp/a.json", "{\"a\":1}", 0);
    assert_eq!(
        base,
        diff_signature("/tmp/a.json", "{\"a\":1}", 0),
        "同一输入必须得到同一签名（否则每帧都重算）"
    );
    assert_ne!(
        base,
        diff_signature("/tmp/a.json", "{\"a\":2}", 0),
        "草稿变化要换签名"
    );
    assert_ne!(
        base,
        diff_signature("/tmp/a.json", "{\"a\":1}", 1),
        "写盘次数变化要换签名（磁盘内容已变）"
    );
    assert_ne!(
        base,
        diff_signature("/tmp/b.json", "{\"a\":1}", 0),
        "目标路径变化要换签名"
    );
}

#[test]
fn diff_recompute_waits_for_the_same_signature_to_settle() {
    let sig = 42;
    // First sighting of the signature: no timing yet — not due.
    assert!(!diff_recompute_due(None, sig, 100.0));
    assert!(!diff_recompute_due(Some((sig, 100.0)), sig, 100.0));
    // The debounce duration hasn't elapsed.
    assert!(!diff_recompute_due(Some((sig, 100.0)), sig, 100.4));
    // Elapsed means due.
    assert!(diff_recompute_due(Some((sig, 100.0)), sig, 100.5));
    assert!(diff_recompute_due(Some((sig, 100.0)), sig, 101.0));
}

#[test]
fn a_new_signature_restarts_the_debounce_clock() {
    // Last frame was timing sig; this frame the draft changed (new signature): the old moment can't carry over.
    assert!(!diff_recompute_due(Some((1, 100.0)), 2, 100.9));
    // The new signature, once it has waited enough, comes due all the same.
    assert!(diff_recompute_due(Some((2, 100.0)), 2, 100.5));
}

/// A debounce clock reset every frame never comes due.
///
/// Drives `diff_step` over several frames, asserting the moment stays frozen after being **first** recorded.
#[test]
fn the_debounce_clock_is_recorded_once_and_not_reset_every_frame() {
    let sig = 7;
    // First frame: no cached result → compute immediately, no timing starts.
    let (due, pending) = diff_step(None, None, sig, 100.0);
    assert!(due, "没有结果可显示时应立刻算");
    assert_eq!(pending, None);

    // Afterwards the cache holds the old signature (the result computed, but the draft changed again):
    // advancing frame by frame, the clock stays at the first moment.
    let mut pending = None;
    for frame in 0..5 {
        let now = 100.1 + frame as f64 * 0.1;
        let (due, next) = diff_step(Some(999), pending, sig, now);
        assert!(!due, "第 {frame} 帧不该到点（才过了 {} 秒）", now - 100.1);
        pending = next;
    }
    assert_eq!(
        pending,
        Some((sig, 100.1)),
        "计时起点必须是第一次见到该签名的那一刻，不能被后续帧刷新"
    );
    // Finally waited enough: due, and the timing state clears.
    let (due, pending) = diff_step(Some(999), pending, sig, 100.6);
    assert!(due, "等够 0.5 秒后必须到点");
    assert_eq!(pending, None);
}

/// With the cache already at the newest signature, no recompute — and no timing state left behind.
#[test]
fn an_up_to_date_cache_never_recomputes() {
    let sig = 11;
    let (due, pending) = diff_step(Some(sig), Some((sig, 100.0)), sig, 999.0);
    assert!(!due);
    assert_eq!(pending, None, "已是最新就该清掉计时状态");
}
