use crate::app::fetch::{
    latency_color, matrix_glyphs, LATENCY_GOOD_MS, LATENCY_SLOW_MS, MATRIX_CHARS, MATRIX_LEN,
};
use crate::theme::SEMANTICS;

#[test]
fn latency_color_thresholds() {
    let colors = SEMANTICS;
    assert_eq!(latency_color(0, colors), colors.ok);
    assert_eq!(latency_color(LATENCY_GOOD_MS - 1, colors), colors.ok);
    assert_eq!(latency_color(LATENCY_GOOD_MS, colors), colors.warn);
    assert_eq!(latency_color(LATENCY_SLOW_MS - 1, colors), colors.warn);
    assert_eq!(latency_color(LATENCY_SLOW_MS, colors), colors.err);
}

#[test]
fn matrix_glyphs_shape_and_variation() {
    let frame = matrix_glyphs(7, "provider/model", MATRIX_LEN);
    assert_eq!(frame.chars().count(), MATRIX_LEN);
    assert!(frame.chars().all(|c| MATRIX_CHARS.contains(c)));
    // 同一帧 + 同一 salt 稳定（不依赖保存的随机状态）
    assert_eq!(frame, matrix_glyphs(7, "provider/model", MATRIX_LEN));
    // 换行（salt 不同）或换帧都会刷新字符
    assert_ne!(frame, matrix_glyphs(7, "provider/other", MATRIX_LEN));
    assert!((8..16).any(|f| frame != matrix_glyphs(f, "provider/model", MATRIX_LEN)));
}
