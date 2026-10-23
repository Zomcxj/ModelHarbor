//! The startup page's choice: follows the top bar's "installed" segment drag order, not a fixed backend order.

use super::super::startup_page_with;
use crate::format::{ConfigFormat, ConfigPaths};

/// The drag order rules: the earliest page in the installed set is the startup page, even when opencode sits
/// earlier in the fixed order (a user report: "stop always opening opencode").
#[test]
fn startup_page_follows_the_dragged_tab_order() {
    let paths = ConfigPaths::default();
    let installed = |id: ConfigFormat| {
        matches!(
            id,
            ConfigFormat::Opencode | ConfigFormat::Pi | ConfigFormat::ZCode
        )
    };
    // The user dragged pi to the front.
    let order: Vec<String> = ["pi", "zcode", "opencode"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let (format, _) = startup_page_with(&paths, &order, installed).expect("有已安装页面");
    assert_eq!(format, ConfigFormat::Pi);
}

/// Never dragged (empty order): the installed group sorts by name initial, taking its first;
/// uninstalled pages don't participate.
#[test]
fn startup_page_without_saved_order_takes_the_first_installed() {
    let paths = ConfigPaths::default();
    let installed = |id: ConfigFormat| matches!(id, ConfigFormat::ZCode | ConfigFormat::WorkBuddy);
    let (format, _) = startup_page_with(&paths, &[], installed).expect("有已安装页面");
    // Alphabetical: workbuddy < zcode.
    assert_eq!(format, ConfigFormat::WorkBuddy);
}

/// Not a single page installed: return `None`, and the caller falls back to the old content sniffing.
#[test]
fn startup_page_is_none_when_nothing_is_installed() {
    let paths = ConfigPaths::default();
    assert!(startup_page_with(&paths, &[], |_| false).is_none());
}
