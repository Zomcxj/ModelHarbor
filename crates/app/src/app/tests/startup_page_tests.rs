//! 启动页选择：跟随顶栏「已安装」段的拖动顺序，而不是固定后端序。

use super::super::startup_page_with;
use crate::format::{ConfigFormat, ConfigPaths};

/// 拖动顺序说了算：已安装集合里排最前的页面即启动页，哪怕固定序里
/// opencode 更靠前（用户报障「不要老是 opencode」）。
#[test]
fn startup_page_follows_the_dragged_tab_order() {
    let paths = ConfigPaths::default();
    let installed = |id: ConfigFormat| {
        matches!(
            id,
            ConfigFormat::Opencode | ConfigFormat::Pi | ConfigFormat::ZCode
        )
    };
    // 用户把 pi 拖到了最前。
    let order: Vec<String> = ["pi", "zcode", "opencode"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let (format, _) = startup_page_with(&paths, &order, installed).expect("有已安装页面");
    assert_eq!(format, ConfigFormat::Pi);
}

/// 没拖动过（顺序为空）：已安装组按名字首字母排，取其中第一个；
/// 未安装的页面不参与。
#[test]
fn startup_page_without_saved_order_takes_the_first_installed() {
    let paths = ConfigPaths::default();
    let installed = |id: ConfigFormat| matches!(id, ConfigFormat::ZCode | ConfigFormat::WorkBuddy);
    let (format, _) = startup_page_with(&paths, &[], installed).expect("有已安装页面");
    // 首字母序：workbuddy < zcode。
    assert_eq!(format, ConfigFormat::WorkBuddy);
}

/// 一个页面都没安装：返回 `None`，调用方退回旧的内容探测。
#[test]
fn startup_page_is_none_when_nothing_is_installed() {
    let paths = ConfigPaths::default();
    assert!(startup_page_with(&paths, &[], |_| false).is_none());
}
