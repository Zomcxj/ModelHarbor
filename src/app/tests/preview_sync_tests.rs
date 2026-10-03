use crate::app::preview::{diff_recompute_due, diff_signature, diff_step, preview_should_rebuild};

#[test]
fn rebuild_gate_ignores_focus_but_keeps_user_text() {
    // 没在预览里手改：按组件状态重建
    assert!(preview_should_rebuild(false, None, 100.0));
    // 刚在预览里输入（2 秒内）：保留用户文本
    assert!(!preview_should_rebuild(false, Some(99.5), 100.0));
    assert!(!preview_should_rebuild(false, Some(100.0), 100.0));
    // 停止输入超过 2 秒：回到组件状态
    assert!(preview_should_rebuild(false, Some(97.9), 100.0));
    // 上次解析失败：保留用户文本
    assert!(!preview_should_rebuild(true, None, 100.0));
    assert!(!preview_should_rebuild(true, Some(1.0), 100.0));
}

/// 签名区分「草稿变了」与「写盘了」两件事。
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
    // 第一次见到该签名：还没计时，不算到点。
    assert!(!diff_recompute_due(None, sig, 100.0));
    assert!(!diff_recompute_due(Some((sig, 100.0)), sig, 100.0));
    // 未等够防抖时长。
    assert!(!diff_recompute_due(Some((sig, 100.0)), sig, 100.4));
    // 等够即到点。
    assert!(diff_recompute_due(Some((sig, 100.0)), sig, 100.5));
    assert!(diff_recompute_due(Some((sig, 100.0)), sig, 101.0));
}

#[test]
fn a_new_signature_restarts_the_debounce_clock() {
    // 上一帧在给 sig 计时，这一帧草稿变了（新签名）：不能沿用旧时刻。
    assert!(!diff_recompute_due(Some((1, 100.0)), 2, 100.9));
    // 新签名自己等够后照样到点。
    assert!(diff_recompute_due(Some((2, 100.0)), 2, 100.5));
}

/// 防抖时钟被每帧重置时永远不到点。
///
/// 驱动 `diff_step` 走多帧，断言时刻**第一次**记下后就不再变。
#[test]
fn the_debounce_clock_is_recorded_once_and_not_reset_every_frame() {
    let sig = 7;
    // 第一帧：没有缓存结果 → 立刻算，不进入计时。
    let (due, pending) = diff_step(None, None, sig, 100.0);
    assert!(due, "没有结果可显示时应立刻算");
    assert_eq!(pending, None);

    // 之后缓存的是旧签名（结果算出来了，但草稿又变了）：
    // 逐帧推进，时钟停在第一次那一刻。
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
    // 终于等够：到点，并清空计时状态。
    let (due, pending) = diff_step(Some(999), pending, sig, 100.6);
    assert!(due, "等够 0.5 秒后必须到点");
    assert_eq!(pending, None);
}

/// 缓存已经是最新签名时不该重算，也不该留下计时状态。
#[test]
fn an_up_to_date_cache_never_recomputes() {
    let sig = 11;
    let (due, pending) = diff_step(Some(sig), Some((sig, 100.0)), sig, 999.0);
    assert!(!due);
    assert_eq!(pending, None, "已是最新就该清掉计时状态");
}
