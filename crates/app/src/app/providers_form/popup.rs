use super::*;

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

/// 模型获取结果的勾选面板：勾选后把其中未配置的模型追加到 `models`。
pub(in crate::app) fn model_fetch_popup<H: std::hash::Hash>(
    ui: &mut egui::Ui,
    state: Option<&ModelFetchState>,
    models: &mut Vec<ModelRow>,
    current_page: ConfigFormat,
    id_salt: H,
) {
    let Some(state) = state else {
        ui.label(egui::RichText::new("尚未获取，请先点击「获取模型」").weak());
        return;
    };
    if state.rx.is_some() {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(16.0));
            ui.label(egui::RichText::new("正在获取模型…").weak());
        });
        return;
    }
    let Some(result) = &state.result else {
        return;
    };
    match result {
        Ok(models_remote) if models_remote.is_empty() => {
            ui.label(egui::RichText::new("接口未返回任何模型").weak());
        }
        Ok(models_remote) => {
            let ids = models_remote.clone();
            ui.label(egui::RichText::new("勾选可新增未配置的模型：").weak());
            // 区域高度固定为 22 行，每列超出部分在区域内垂直滚动。
            // ScrollArea 内不能用 `ui.columns`（会把内容裁到当前可用高度，内容不进滚动区）。
            // 列数须在滚动区内部按可用宽度计算，以扣除始终可见的滚动条占用的宽度。
            let row_h = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
            egui::ScrollArea::vertical()
                .id_salt(id_salt)
                .max_height(row_h * 22.5)
                .auto_shrink([false, true])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| {
                    let (cols, col_w) = fetch_grid_columns(ids.len(), ui.available_width());
                    let per_col = ids.len().div_ceil(cols);
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = FETCH_GRID_GAP_X;
                        for ci in 0..cols {
                            ui.vertical(|ui| {
                                // 定宽列 + 截断：超长模型名悬停看全名，不把列撑宽。
                                ui.set_width(col_w);
                                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                                for id in ids.iter().skip(ci * per_col).take(per_col) {
                                    let mut checked = models.iter().any(|m| m.id.trim() == id);
                                    if ui
                                        .checkbox(&mut checked, id)
                                        .on_hover_text(id.as_str())
                                        .changed()
                                        && checked
                                    {
                                        let mut row = ModelRow::new();
                                        row.id = id.clone();
                                        row.name = id.clone();
                                        row.source_format = Some(current_page);
                                        models.push(row);
                                    }
                                }
                            });
                        }
                    });
                });
        }
        Err(err) => {
            ui.label(
                egui::RichText::new(format!("获取失败：{}", err))
                    .color(crate::theme::semantics(ui).err),
            );
        }
    }
}
