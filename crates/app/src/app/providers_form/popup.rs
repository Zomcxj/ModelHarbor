pub(in crate::app) const FETCH_GRID_GAP_X: f32 = 28.0;

pub(in crate::app) const FETCH_GRID_MIN_COL_W: f32 = 150.0;

pub(in crate::app) const FETCH_GRID_MAX_COLS: usize = 5;

/// 计算模型勾选面板的横向分栏：列数由可用宽度推出，列宽再把总宽均分，
/// 使 `列数 * 列宽 + 间距 * (列数 - 1)` 恒等于可用宽度。
/// 返回 `(列数, 每列宽度)`。
pub(in crate::app) fn fetch_grid_columns(id_count: usize, avail_width: f32) -> (usize, f32) {
    if id_count == 0 {
        return (1, avail_width.max(1.0));
    }
    let max_cols = FETCH_GRID_MAX_COLS.min(id_count);
    // 浮点转整数为饱和转换（NaN -> 0），不会 panic；clamp 只是兜底。
    let cols = (((avail_width + FETCH_GRID_GAP_X) / (FETCH_GRID_MIN_COL_W + FETCH_GRID_GAP_X))
        .floor() as usize)
        .clamp(1, max_cols);
    let col_w = ((avail_width - FETCH_GRID_GAP_X * (cols - 1) as f32) / cols as f32).max(1.0);
    (cols, col_w)
}
