use crate::app::App;
use crate::model::ProviderRow;

/// Build an App holding only the given providers, with an empty token table.
fn app_with_providers(stations: &[(&str, &str)]) -> App {
    let providers: Vec<ProviderRow> = stations
        .iter()
        .map(|(key, base_url)| {
            let mut provider = ProviderRow::new();
            provider.key = (*key).to_string();
            provider.base_url = (*base_url).to_string();
            provider
        })
        .collect();
    App {
        providers,
        tokens: crate::tokens::StationTokens::default(),
        ..App::default()
    }
}

#[test]
fn station_token_is_shared_by_every_provider_on_the_same_site() {
    let mut app = app_with_providers(&[
        ("oc_gemai", "https://gemai.huchan.cn/v1"),
        ("pi_gemai", "https://Gemai.Huchan.CN/v1/"),
        ("other", "https://other.example.com/v1"),
    ]);
    // Station-level: one entry per station origin.
    app.tokens.set("https://gemai.huchan.cn", "pat-shared");

    assert_eq!(app.station_pat("https://gemai.huchan.cn/v1"), "pat-shared");
    assert_eq!(
        app.station_pat("https://gemai.huchan.cn"),
        "pat-shared",
        "同一站点的不同写法必须命中同一份令牌"
    );
    assert_eq!(
        app.station_pat("https://other.example.com/v1"),
        "",
        "别的站点不能被牵连"
    );
}

#[test]
fn station_token_lookup_is_empty_when_never_configured() {
    let app = app_with_providers(&[("a", "https://a.example.com/v1")]);
    assert_eq!(app.station_pat("https://a.example.com/v1"), "");
    // Providers without a baseUrl hit no tokens.
    assert_eq!(app.station_pat(""), "");
}

#[test]
fn removing_a_station_token_clears_it_for_all_its_providers() {
    let mut app = app_with_providers(&[
        ("a", "https://a.example.com/v1"),
        ("b", "https://a.example.com/v2"),
        ("keep", "https://keep.example.com/v1"),
    ]);
    app.tokens.set("https://a.example.com", "pat-a");
    app.tokens.set("https://keep.example.com", "pat-keep");

    app.tokens.remove("https://a.example.com");

    assert_eq!(app.station_pat("https://a.example.com/v1"), "");
    assert_eq!(app.station_pat("https://a.example.com/v2"), "");
    assert_eq!(
        app.station_pat("https://keep.example.com/v1"),
        "pat-keep",
        "其他站点的令牌不受影响"
    );
}

#[test]
fn station_user_id_is_station_scoped_and_optional() {
    let mut app = app_with_providers(&[
        ("oc_gemai", "https://gemai.huchan.cn/v1"),
        ("pi_gemai", "https://Gemai.Huchan.CN/v1/"),
        ("other", "https://other.example.com/v1"),
    ]);
    // Unset by default: an empty string returns.
    assert_eq!(app.station_user_id("https://gemai.huchan.cn/v1"), "");
    app.tokens.set("https://gemai.huchan.cn", "pat-shared");
    app.tokens.set_user_id("https://gemai.huchan.cn", "12345");
    // Different spellings of one station yield the same ID (merged by origin, like tokens).
    assert_eq!(app.station_user_id("https://gemai.huchan.cn/v1"), "12345");
    assert_eq!(app.station_user_id("https://Gemai.Huchan.CN/v1/"), "12345");
    assert_eq!(app.station_user_id("https://other.example.com/v1"), "");
}

#[test]
fn user_id_hint_follows_the_last_query_result() {
    use crate::app::balance::BalanceState;
    let mut app = app_with_providers(&[("a", "https://a.example.com/v1")]);
    let keys = vec!["a".to_string()];
    assert!(!app.station_needs_user_id(&keys), "还没查过就不该提示");

    // The token side failed too: the error sits in Err.
    app.balance.insert(
        "a".to_string(),
        BalanceState {
            result: Some(Err(
                "HTTP 401（New-Api-User header not provided）".to_string()
            )),
            ..Default::default()
        },
    );
    assert!(app.station_needs_user_id(&keys), "Err 里的缺头信息要认出来");

    // The token side succeeded, only the account part failed: the reason sits in the success result's note.
    app.balance.insert(
        "a".to_string(),
        BalanceState {
            result: Some(Ok(crate::billing::Billing {
                note: Some("账号令牌查询失败：该站点要求 New-Api-User".to_string()),
                ..Default::default()
            })),
            ..Default::default()
        },
    );
    assert!(
        app.station_needs_user_id(&keys),
        "note 里的缺头信息也要认出来"
    );

    // Unrelated failures don't trigger the hint.
    app.balance.insert(
        "a".to_string(),
        BalanceState {
            result: Some(Err("网络错误：连接被重置".to_string())),
            ..Default::default()
        },
    );
    assert!(!app.station_needs_user_id(&keys));
    assert!(
        !app.station_needs_user_id(&["missing".to_string()]),
        "没有记录的 provider 不该 panic 也不该提示"
    );
}
