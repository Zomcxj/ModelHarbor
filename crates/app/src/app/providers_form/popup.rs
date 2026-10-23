pub(in crate::app) const FETCH_GRID_GAP_X: f32 = 28.0;

pub(in crate::app) const FETCH_GRID_MIN_COL_W: f32 = 150.0;

pub(in crate::app) const FETCH_GRID_MAX_COLS: usize = 5;

/// Compute the model check panel's column layout: column count derives from the available width, then column
/// width splits the total evenly, so `columns * column width + gaps * (columns - 1)` always equals the
/// available width. Returns `(column count, per-column width)`.
pub(in crate::app) fn fetch_grid_columns(id_count: usize, avail_width: f32) -> (usize, f32) {
    if id_count == 0 {
        return (1, avail_width.max(1.0));
    }
    let max_cols = FETCH_GRID_MAX_COLS.min(id_count);
    // Float-to-int is a saturating conversion (NaN -> 0), never panicking; clamp is just a backstop.
    let cols = (((avail_width + FETCH_GRID_GAP_X) / (FETCH_GRID_MIN_COL_W + FETCH_GRID_GAP_X))
        .floor() as usize)
        .clamp(1, max_cols);
    let col_w = ((avail_width - FETCH_GRID_GAP_X * (cols - 1) as f32) / cols as f32).max(1.0);
    (cols, col_w)
}
