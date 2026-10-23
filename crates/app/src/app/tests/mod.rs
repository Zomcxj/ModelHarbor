//! The `app` module's unit tests: serialization layout, preview find/rebuild, syntax highlighting,
//! model fetching and latency probes. Split into subfiles by domain.

#[cfg(test)]
mod compact_tests;
/// Cross-page agent writes: `model`'s provider prefix must be one the target page's gateway accepts.
///
/// The three pages (opencode / kilocode / mimocode) share one agents dataset, while `model` is
/// `provider/model` — the save path normalizes per page.
#[cfg(test)]
mod cross_page_agent_model_tests;
/// Model discovery (A-2): candidate wording, check seeding, fill-only adds, status wording, and cache scheduling.
#[cfg(test)]
mod discovery_tests;
/// Drag cursors: every drag source (tabs / backend icons included) must light the custom grab cursor.
#[cfg(all(test, target_os = "windows"))]
mod drag_cursor_tests;
#[cfg(test)]
mod latency_tests;
#[cfg(test)]
mod layer_order_tests;
/// The "enable" switch belongs to the WorkBuddy page alone.
#[cfg(test)]
mod model_enable_visibility_tests;
#[cfg(test)]
mod model_fetch_tests;
mod net_guard_override_tests;
#[cfg(test)]
mod path_reload_tests;
#[cfg(test)]
mod prefs_snapshot_tests;
#[cfg(test)]
mod preview_collapse_tests;
#[cfg(test)]
mod preview_diff_tests;
#[cfg(test)]
mod preview_find_boundary_tests;
#[cfg(test)]
mod preview_sync_tests;
#[cfg(test)]
mod provider_header_layout_tests;
/// Use real config files to confirm the "enable" switch appears only on the WorkBuddy page.
#[cfg(test)]
mod real_config_enable_visibility;
#[cfg(test)]
mod real_file_grouping;
#[cfg(test)]
mod save_all_tests;
/// The startup page's choice: follows the top bar's "installed" segment drag order, not a fixed backend order.
#[cfg(test)]
mod startup_page_tests;
#[cfg(test)]
mod station_token_tests;
#[cfg(test)]
mod syntax_highlight_tests;
/// Tabs' (backend icons') state coloring: the fill follows state; the icon color doesn't.
///
/// Three states, three colors: selected = hover color + thicker border; drag source = orange
/// (`DRAG_SOURCE_FILL`); reorder target = green (`DROP_TARGET_COLOR`). Icon tints choose only between
/// white (installed) and dimmed (not). Offscreen rendering pins the actual colors.
#[cfg(test)]
mod tab_highlight_tests;
/// The usage view's chart and stat-block layout: the legend mustn't cover bars, stat blocks mustn't wrap misaligned.
#[cfg(test)]
mod usage_chart_layout_tests;
/// The end-to-end contract that deleting a session hides the row, really removes details, and keeps totals.
#[cfg(test)]
mod usage_delete_keeps_total_tests;
/// The usage view's two-column layout and top-bar button positions.
#[cfg(test)]
mod usage_layout_tests;
/// The usage tables' layout (right-aligned numbers, header alignment) and that agent filtering really reaches the render path.
#[cfg(test)]
mod usage_table_layout_tests;
/// The opencode v2 page's field visibility and labels follow v2 key names.
#[cfg(test)]
mod v2_field_visibility_tests;
/// Entering the WorkBuddy page converges multiple enabled per id to "only the first enabled".
#[cfg(test)]
mod workbuddy_enable_normalization_tests;
