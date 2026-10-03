//! `app` 模块的单元测试：序列化排版、预览查找/重建、语法高亮、
//! 模型获取与延迟探测（协议分类、SSE 首字、节流门控）。
//!
//! 按领域拆分到子文件（由 tests.rs 机械拆分而来，内容逐字不变）。

#[cfg(test)]
mod compact_tests;
/// 跨页写 agent：`model` 的 provider 前缀必须是**目标页网关**认的那个。
///
/// 三页（opencode / kilocode / mimocode）共用同一份 agents 数据，而 `model` 是
/// `provider/model`。把 opencode 页配好的 `opencode/…` 原样写进 kilo.json，kilo 网关
/// 不认这个前缀，agent 直接跑不起来。保存路径必须逐页归一。
#[cfg(test)]
mod cross_page_agent_model_tests;
/// 拖动光标：任一拖动源（含页签/后端图标）都必须点亮自定义抓取光标。
///
/// 背景：抓取态的判定此前漏了 `tab_drag_src`，于是拖卡片是抓取光标、拖后端图标
/// 却退回系统手型。自定义光标是整窗生效的，漏一个拖动源就少一处。
#[cfg(all(test, target_os = "windows"))]
mod drag_cursor_tests;
#[cfg(test)]
mod latency_tests;
#[cfg(test)]
mod layer_order_tests;
/// 「启用」开关只属于 WorkBuddy 一页。
///
/// 曾经的 bug：开关可见性取自 `page_has_model_field("disabled")`，而那个函数在
/// 「已加载的文件格式 ≠ 当前页」时**一律返回 true**（为了让切到的新页面能填所有
/// 字段）。用户加载的是 `opencode.json`，于是切到 pi / omp / DSH / ZCode 每一页都
/// 冒出了「启用」开关——只有 opencode 自己那页（格式相同）没中招，正是用户报的
/// 「除了 opencode 都加了」。
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
///
/// 这是用户报的场景：加载 `opencode.json` 后，除 opencode 外的每一页都冒出了开关。
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
/// 页签（后端图标）的状态配色：底色随状态走，图标色**不随状态走**。
///
/// 背景：高亮此前只改按钮填充，而 16px 图标几乎占满按钮，能看见的只剩一圈细边。
/// 后来改成「选中就把图标 tint 成强调色上的文字色」，深色主题下那正好是黑色，
/// 图标整个变黑（用户实测报障）。现在状态一律靠底色 + 描边表达，三态三色：
/// 选中 = **悬浮色 + 加粗描边**（悬浮的加重版，不是另起一套颜色）；
/// 拖动源 = 橙（与卡片拖动源同源 `DRAG_SOURCE_FILL`）；
/// 换位目标 = 绿（与卡片落点同源 `DROP_TARGET_COLOR`，拖动中补画在落点上）。
/// 图标 tint 只在白（已安装）/ 压淡（未安装）之间选。这里用离屏渲染把实际颜色钉住。
#[cfg(test)]
mod tab_highlight_tests;
/// 进 WorkBuddy 页时，同一 id 多条启用必须收敛成「只启用第一条」。
///
/// 症状来自跨方言共享数据：opencode 等格式没有 `disabled` 概念（`convert` 里一律
/// 读成启用），所以切到 WorkBuddy 页时每个模型都是勾选态——而 WorkBuddy 按裸 id
/// 全局去重，多开的根本不生效，界面显示成「全部启用」是在骗人。
#[cfg(test)]
mod workbuddy_enable_normalization_tests;
