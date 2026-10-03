//! `app` 模块的单元测试：序列化排版、预览查找/重建、语法高亮、
//! 模型获取与延迟探测。按领域拆分到子文件。

#[cfg(test)]
mod compact_tests;
/// 跨页写 agent：`model` 的 provider 前缀必须是目标页网关认的那个。
///
/// 三页（opencode / kilocode / mimocode）共用同一份 agents 数据，而 `model` 是
/// `provider/model`，保存路径逐页归一。
#[cfg(test)]
mod cross_page_agent_model_tests;
/// 拖动光标：任一拖动源（含页签/后端图标）都必须点亮自定义抓取光标。
#[cfg(all(test, target_os = "windows"))]
mod drag_cursor_tests;
#[cfg(test)]
mod latency_tests;
#[cfg(test)]
mod layer_order_tests;
/// 「启用」开关只属于 WorkBuddy 一页。
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
/// 用真实配置文件确认「启用」开关只在 WorkBuddy 页出现。
#[cfg(test)]
mod real_config_enable_visibility;
#[cfg(test)]
mod real_file_grouping;
#[cfg(test)]
mod save_all_tests;
#[cfg(test)]
mod station_token_tests;
#[cfg(test)]
mod syntax_highlight_tests;
/// 页签（后端图标）的状态配色：底色随状态走，图标色不随状态走。
///
/// 三态三色：选中 = 悬浮色 + 加粗描边；拖动源 = 橙（`DRAG_SOURCE_FILL`）；
/// 换位目标 = 绿（`DROP_TARGET_COLOR`）。图标 tint 只在白（已安装）/ 压淡（未安装）
/// 之间选。这里用离屏渲染把实际颜色钉住。
#[cfg(test)]
mod tab_highlight_tests;
/// 进 WorkBuddy 页时，同一 id 多条启用收敛成「只启用第一条」。
#[cfg(test)]
mod workbuddy_enable_normalization_tests;
