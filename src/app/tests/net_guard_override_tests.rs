use crate::app::fetch::net_guard_gate;
use crate::app::App;

/// 守卫值换算：默认拦截，放行开关打开后不再拦截。
#[test]
fn override_releases_the_model_probe_gate() {
    let guard = Some("检测到系统代理".to_string());
    assert_eq!(
        net_guard_gate(&guard, false).as_deref(),
        Some("检测到系统代理"),
        "默认必须继续拦截"
    );
    assert_eq!(net_guard_gate(&guard, true), None, "放行后不该再拦");
    // 没检测到代理时，开关开或关都不产生拦截。
    assert_eq!(net_guard_gate(&None, false), None);
    assert_eq!(net_guard_gate(&None, true), None);
}

/// 开关确实贯通到 App：既影响门控值，也进 `current_prefs`（才能落盘）。
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
