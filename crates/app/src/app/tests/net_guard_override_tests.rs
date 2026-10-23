use crate::app::fetch::net_guard_gate;
use crate::app::App;

/// Guard-value conversion: blocking by default; with the allow toggle on, no more blocking.
#[test]
fn override_releases_the_model_probe_gate() {
    let guard = Some("检测到系统代理".to_string());
    assert_eq!(
        net_guard_gate(&guard, false).as_deref(),
        Some("检测到系统代理"),
        "默认必须继续拦截"
    );
    assert_eq!(net_guard_gate(&guard, true), None, "放行后不该再拦");
    // With no proxy detected, the toggle on or off produces no blocking.
    assert_eq!(net_guard_gate(&None, false), None);
    assert_eq!(net_guard_gate(&None, true), None);
}

/// The toggle reaches into App: it affects the gate value and enters `current_prefs` (so it can persist).
#[test]
fn app_gate_and_prefs_follow_the_switch() {
    let blocked = App {
        net_guard: Some("检测到 VPN".to_string()),
        allow_model_test_with_proxy: false,
        ..App::default()
    };
    assert!(
        net_guard_gate(&blocked.net_guard, blocked.allow_model_test_with_proxy).is_some(),
        "默认拦截"
    );
    assert!(
        !blocked.current_prefs().allow_model_test_with_proxy,
        "默认值要按拦截写出"
    );

    let allowed = App {
        allow_model_test_with_proxy: true,
        ..blocked
    };
    assert_eq!(
        net_guard_gate(&allowed.net_guard, allowed.allow_model_test_with_proxy),
        None,
        "放行后不该再拦"
    );
    assert!(
        allowed.current_prefs().allow_model_test_with_proxy,
        "放行要能写进 settings.json"
    );
}
