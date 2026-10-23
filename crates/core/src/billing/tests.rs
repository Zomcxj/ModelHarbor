#![cfg(test)]

use super::*;

/// Real response: a Hajimi-style API station (quota is a placeholder 1e8, $216.00 used).
const HAJIMI_SUB: &str = r#"{"object":"billing_subscription","has_payment_method":true,
        "soft_limit_usd":100000000,"hard_limit_usd":100000000,"system_hard_limit_usd":100000000,
        "access_until":0}"#;
const HAJIMI_USAGE: &str = r#"{"object":"list","total_usage":21600}"#;
const HAJIMI_STATUS: &str =
    r#"{"success":true,"data":{"system_name":"哈基米API站","version":"v1.0.0-rc.19.3518"}}"#;

/// Real response: another shape, credit_summary (unit unlabeled).
const CREDIT_SUMMARY: &str = r#"{"object":"credit_summary","total_granted":-1300000,
        "total_used":0,"total_available":-1300000,"expires_at":0}"#;

#[test]
fn panel_name_combines_system_and_version() {
    assert_eq!(
        parse_panel(Some(HAJIMI_STATUS)),
        "哈基米API站 v1.0.0-rc.19.3518"
    );
    assert_eq!(parse_panel(None), "");
    assert_eq!(parse_panel(Some("not json")), "");
    assert_eq!(
        parse_panel(Some(r#"{"data":{"system_name":"SheApi"}}"#)),
        "SheApi"
    );
}

#[test]
fn placeholder_limit_shows_used_only() {
    // Placeholder quota 1e8: neither displayed nor kept.
    let billing = parse(HAJIMI_SUB, Some(HAJIMI_USAGE), Some(HAJIMI_STATUS));
    assert_eq!(billing.shape, Shape::Subscription);
    assert_eq!(billing.used_usd, Some(216.0));
    assert!(billing.placeholder_limit);
    assert_eq!(billing.limit_usd, None, "占位额度不算额度");
    assert_eq!(billing.balance_usd, None, "占位额度不显示余额");
    let inline = billing.inline();
    assert_eq!(inline, "已用 $216.00");
    assert!(!inline.contains("余额"), "别出现没意义的余额：{inline}");
    assert!(
        !inline.contains('~'),
        "不要用 `~`（易被读成负号）：{inline}"
    );
    assert_eq!(billing.inline_full(), "已用 $216.00");
    let detail = billing.detail();
    // Placeholder quotas show usage only.
    assert!(!detail.contains("占位值"), "不再解释占位：{detail}");
    assert!(!detail.contains("额度"), "占位额度下不该有额度行：{detail}");
    assert!(!detail.contains("余额"), "占位额度下不该给余额行：{detail}");
    assert!(
        detail.contains("哈基米API站"),
        "detail 要带面板名：{detail}"
    );
}

#[test]
fn real_limit_gives_plain_balance() {
    let sub = r#"{"object":"billing_subscription","hard_limit_usd":50.0}"#;
    let usage = r#"{"object":"list","total_usage":1234}"#;
    let billing = parse(sub, Some(usage), None);
    assert_eq!(billing.used_usd, Some(12.34), "total_usage 是美分");
    assert_eq!(billing.balance_usd, Some(37.66));
    assert!(!billing.placeholder_limit);
    assert_eq!(billing.inline(), "已用 $12.34 · 余额 $37.66");
    assert!(!billing.detail().contains("~"));
}

#[test]
fn credit_summary_is_shown_raw() {
    let billing = parse(CREDIT_SUMMARY, None, None);
    assert_eq!(billing.shape, Shape::CreditSummary);
    assert!(billing.balance_usd.is_none(), "单位未知时不要算余额");
    assert_eq!(billing.inline(), "额度单位未标注（悬停看原值）");
    let raw = billing.raw_credit.clone().unwrap_or_default();
    assert!(raw.contains("-1,300,000"), "原值要带千分位：{raw}");
    assert!(billing.detail().contains("单位未标注"));
}

#[test]
fn zero_hard_limit_is_treated_as_no_limit() {
    // Some stations use hard_limit_usd: 0 to mean no hard cap.
    let sub = r#"{"object":"billing_subscription","hard_limit_usd":0,"soft_limit_usd":0}"#;
    let usage = r#"{"object":"list","total_usage":500}"#;
    let billing = parse(sub, Some(usage), None);
    assert_eq!(billing.used_usd, Some(5.0));
    assert_eq!(billing.limit_usd, None, "额度 0 不算额度");
    assert_eq!(billing.balance_usd, None);
    assert_eq!(billing.inline(), "已用 $5.00");
}

#[test]
fn balance_never_shows_negative() {
    // Quota smaller than usage (inconsistent data / overspent): usage only.
    let sub = r#"{"object":"billing_subscription","hard_limit_usd":10.0}"#;
    let usage = r#"{"object":"list","total_usage":1500}"#;
    let billing = parse(sub, Some(usage), None);
    assert_eq!(billing.balance_usd, None);
    assert_eq!(billing.inline(), "已用 $15.00");
    assert!(!billing.detail().contains("-$"), "不要出现负金额");
}

/// Real observed shape: `/api/status` (conversion fields + panel name).
const STATUS_UNITS: &str = r#"{"success":true,"data":{"system_name":"哈基米API站",
        "version":"v1.0.0-rc.19.3518","quota_per_unit":500000,"quota_display_type":"CNY",
        "display_in_currency":true,"usd_exchange_rate":7.3}}"#;
/// Real observed shape: `/api/usage/token/` (unlimited quota).
const USAGE_TOKEN_UNLIMITED: &str = r#"{"code":true,"message":"","data":{"name":"demo",
        "unlimited_quota":true,"total_granted":0,"total_used":137000000,
        "total_available":-137000000,"expires_at":0}}"#;
/// Real observed shape: `/api/log/token` (fields taken from live probing).
const TOKEN_LOGS: &str = r#"{"success":true,"message":"","data":[
        {"created_at":1789436810,"type":2,"model_name":"deepseek-v4-flash","quota":1000000},
        {"created_at":1789436900,"type":2,"model_name":"grok-4.6","quota":500000},
        {"created_at":1789350000,"type":2,"model_name":"deepseek-v4-flash","quota":2000000}]}"#;

#[test]
fn units_come_from_status_and_fall_back_when_missing() {
    let units = parse_units(Some(STATUS_UNITS));
    assert_eq!(units.quota_per_unit, 500_000.0);
    assert_eq!(units.currency, "CNY");
    assert!(!units.assumed);

    for payload in [None, Some("not json"), Some(r#"{"data":{}}"#)] {
        let units = parse_units(payload);
        assert_eq!(units.quota_per_unit, QUOTA_PER_USD);
        assert!(units.assumed, "站点没给换算比要标记出来");
    }
}

#[test]
fn token_usage_parses_and_rejects_empty() {
    let usage = parse_token_usage(USAGE_TOKEN_UNLIMITED).expect("应能解析");
    assert_eq!(usage.name, "demo");
    assert!(usage.unlimited);
    assert_eq!(usage.used, Some(137_000_000.0));

    for payload in ["", "not json", "{}", r#"{"data":[]}"#, r#"{"data":{}}"#] {
        assert!(parse_token_usage(payload).is_none(), "{payload}");
    }
}

#[test]
fn token_logs_summarize_by_window_and_model() {
    let logs = parse_token_logs(TOKEN_LOGS);
    assert_eq!(logs.len(), 3);

    // Today: the first two (the third falls in yesterday's window)
    let today = summarize_logs(&logs, Some(1_789_430_000));
    assert_eq!(today.count, 2);
    assert_eq!(today.quota, 1_500_000.0);
    assert_eq!(
        today.models[0],
        ("deepseek-v4-flash".to_string(), 1_000_000.0)
    );
    assert_eq!(today.models[1], ("grok-4.6".to_string(), 500_000.0));

    let all = summarize_logs(&logs, None);
    assert_eq!(all.count, 3);
    assert_eq!(all.quota, 3_500_000.0);
    assert_eq!(all.models[0].1, 3_000_000.0, "同模型要合并");

    // Empty logs / corrupt logs never panic
    assert_eq!(parse_token_logs("not json").len(), 0);
    assert_eq!(parse_token_logs(r#"{"success":false}"#).len(), 0);
    assert_eq!(summarize_logs(&[], Some(1)).count, 0);
}

#[test]
fn token_billing_describes_unlimited_station() {
    let units = parse_units(Some(STATUS_UNITS));
    let usage = parse_token_usage(USAGE_TOKEN_UNLIMITED).expect("应能解析");
    let logs = parse_token_logs(TOKEN_LOGS);
    let info = parse_token_billing(TokenInputs {
        usage: Some(&usage),
        logs: &logs,
        units: &units,
        status_json: Some(STATUS_UNITS),
        now: 1_789_437_000,
        today_from: 1_789_430_000,
        note: None,
    });
    assert_eq!(info.source, Source::Token);
    assert_eq!(info.shape, Shape::TokenUnlimited);
    assert!(info.unlimited);
    assert_eq!(info.used_usd, Some(274.0), "137,000,000 quota = $274");
    assert_eq!(info.balance_usd, None, "不限额度不该编出余额");
    assert_eq!(info.limit_usd, None);
    assert_eq!(info.today_usd, Some(3.0));
    assert_eq!(info.week_usd, Some(7.0), "近 7 天含今日");
    assert_eq!(info.today_calls, Some(2));
    assert_eq!(info.today_models.len(), 2);
    assert_eq!(info.inline(), "已用 $274.00 · 今日 $3.00");
    let detail = info.detail();
    assert!(detail.contains("不限"), "{detail}");
    assert!(detail.contains("deepseek-v4-flash"), "{detail}");
    assert!(detail.contains("今日已用：$3.00（2 次请求）"), "{detail}");
}

#[test]
fn token_billing_shows_balance_when_quota_is_finite() {
    let usage = parse_token_usage(
        r#"{"data":{"name":"paid","unlimited_quota":false,"total_granted":10000000,
                "total_used":2500000,"total_available":7500000}}"#,
    )
    .expect("应能解析");
    // The station gave no conversion ratio → assume the default and flag the assumption
    let units = parse_units(None);
    let info = parse_token_billing(TokenInputs {
        usage: Some(&usage),
        logs: &[],
        units: &units,
        status_json: None,
        now: 1,
        today_from: 0,
        note: None,
    });
    assert_eq!(info.shape, Shape::TokenQuota);
    assert!(!info.unlimited);
    assert_eq!(info.balance_usd, Some(15.0));
    assert_eq!(info.limit_usd, Some(20.0));
    assert_eq!(info.used_usd, Some(5.0));
    assert_eq!(info.today_usd, None, "没有日志就不编今日数字");
    assert!(info.unit_assumed);
    assert_eq!(info.inline(), "余额 $15.00 · 累计 $5.00");
    assert!(info.detail().contains("默认 1 美元 = 500,000 quota"));
}

#[test]
fn detail_keeps_numbers_and_drops_the_endpoint_footnotes() {
    // The hover's small window holds only numbers and the accounting that affects them (ratio, day rollover).
    let items: Vec<String> = (0..LOG_PAGE_LIMIT)
        .map(|i| {
            format!(
                r#"{{"created_at":{},"model_name":"m","quota":1000}}"#,
                1_789_430_000 + i as i64
            )
        })
        .collect();
    let payload = format!(r#"{{"success":true,"data":[{}]}}"#, items.join(","));
    let logs = parse_token_logs(&payload);
    assert_eq!(logs.len(), LOG_PAGE_LIMIT);
    let usage = parse_token_usage(USAGE_TOKEN_UNLIMITED).expect("应能解析");
    let units = parse_units(Some(STATUS_UNITS));
    let info = parse_token_billing(TokenInputs {
        usage: Some(&usage),
        logs: &logs,
        units: &units,
        status_json: None,
        now: 1_789_430_000,
        today_from: 1_789_430_000,
        note: None,
    });
    let detail = info.detail();
    for noise in ["来源", "/api/", "分页", "只读", "接口"] {
        assert!(
            !detail.contains(noise),
            "「{noise}」是实现细节，不该进小窗：{detail}"
        );
    }
    assert!(detail.contains("近 7 天"), "数字要留着：{detail}");
}

#[test]
fn a_degraded_result_shows_no_note_line() {
    // Degradation reasons (call logs unavailable, panel token invalid) stay in the result; the small window shows numbers only.
    let units = parse_units(Some(STATUS_UNITS));
    let usage = parse_token_usage(USAGE_TOKEN_UNLIMITED).expect("应能解析");
    let info = parse_token_billing(TokenInputs {
        usage: Some(&usage),
        logs: &[],
        units: &units,
        status_json: None,
        now: 1_789_430_000,
        today_from: 1_789_430_000,
        note: Some("调用日志不可用（HTTP 404 Not Found），今日用量取不到".to_string()),
    });
    assert!(info.note.is_some(), "原因本身要留在结果里，不当场丢掉");
    let detail = info.detail();
    assert!(!detail.contains("备注"), "{detail}");
    assert!(!detail.contains("不可用"), "{detail}");
}

#[test]
fn unsupported_site_is_unknown() {
    // Not supported: what arrives may be a 404 page or an empty object
    for payload in ["", "{}", "<html>404</html>", r#"{"success":false}"#] {
        let billing = parse(payload, None, None);
        assert_eq!(billing.shape, Shape::Unknown, "{payload}");
        assert_eq!(billing.inline(), "未返回额度信息");
        assert!(billing.detail().contains("没有返回可识别的账单信息"));
    }
}

#[test]
fn usage_card_visibility_hides_unknown_and_empty_results() {
    let unknown = Billing::default();
    assert!(!unknown.is_displayable());

    let empty_known = Billing {
        shape: Shape::Subscription,
        ..Default::default()
    };
    assert!(!empty_known.is_displayable());

    let visible = Billing {
        shape: Shape::Subscription,
        used_usd: Some(12.5),
        ..Default::default()
    };
    assert!(visible.is_displayable());
    assert_eq!(visible.inline_full(), "已用 $12.50");
    assert!(visible.detail().contains("已用：$12.50"));
}
#[test]
fn usage_without_limit_still_reports_used() {
    let billing = parse(
        r#"{"object":"billing_subscription"}"#,
        Some(HAJIMI_USAGE),
        None,
    );
    assert_eq!(billing.used_usd, Some(216.0));
    assert!(billing.balance_usd.is_none());
    assert_eq!(billing.inline(), "已用 $216.00");
}

#[test]
fn money_uses_thousands_separators() {
    assert_eq!(money(100_000_000.0), "$100,000,000.00");
    assert_eq!(money(216.0), "$216.00");
    assert_eq!(money(1234.5), "$1,234.50");
    assert_eq!(money(0.0), "$0.00");
    assert_eq!(money(-1300.0), "-$1,300.00");
    assert_eq!(money(999.999), "$1,000.00");
}

#[test]
fn token_log_url_requests_a_bounded_page() {
    let endpoints = endpoints("https://example.test/v1");
    assert_eq!(
        endpoints.token_logs(),
        format!("https://example.test/api/log/token?p=0&page_size={LOG_PAGE_LIMIT}")
    );
}
#[test]
fn endpoints_derive_origin_and_base() {
    let ep = endpoints("https://gemai.huchan.cn/v1");
    assert_eq!(ep.status, "https://gemai.huchan.cn/api/status");
    assert_eq!(
        ep.subscription,
        "https://gemai.huchan.cn/v1/dashboard/billing/subscription"
    );
    assert_eq!(
        ep.usage,
        "https://gemai.huchan.cn/v1/dashboard/billing/usage"
    );

    // anthropic-family baseUrls lack /v1; the trailing slash must go
    let ep = endpoints("https://kktoken.cc/");
    assert_eq!(ep.status, "https://kktoken.cc/api/status");
    assert_eq!(
        ep.subscription,
        "https://kktoken.cc/dashboard/billing/subscription"
    );

    // No scheme: the whole thing counts as the origin.
    let ep = endpoints("api.example.com/v1");
    assert_eq!(ep.status, "api.example.com/v1/api/status");
}

#[test]
fn endpoints_v1_only_touches_origin_only_bases() {
    // In the pi page, anthropic-family baseUrls carry only the origin → the alternate endpoint appends /v1
    let ep = endpoints_v1("https://ps.air-outer.com");
    assert_eq!(
        ep.subscription,
        "https://ps.air-outer.com/v1/dashboard/billing/subscription"
    );
    assert_eq!(
        ep.status, "https://ps.air-outer.com/api/status",
        "站点信息仍在 origin"
    );
    assert_eq!(
        endpoints_v1("https://kktoken.cc/").subscription,
        "https://kktoken.cc/v1/dashboard/billing/subscription"
    );

    // /v1 already present: returned as-is, never /v1/v1/...
    let same = endpoints("https://gemai.huchan.cn/v1");
    let alt = endpoints_v1("https://gemai.huchan.cn/v1");
    assert_eq!(alt.subscription, same.subscription);

    // Another path present: returned as-is
    let same = endpoints("https://host/api");
    let alt = endpoints_v1("https://host/api");
    assert_eq!(alt.subscription, same.subscription);
}

/// Real response shape: `/api/user/self` (requires a panel PAT).
/// Field names from new-api's `buildSelfUserData` (controller/user.go).
const ACCOUNT_SELF: &str = r#"{"success":true,"message":"","data":{
        "id":1,"username":"tester","display_name":"Tester","role":1,"status":1,
        "group":"default","quota":6150000,"used_quota":10250000,"request_count":321}}"#;

/// Display when there is account data but no token data.
fn account_only_billing() -> Billing {
    let units = parse_units(Some(STATUS_UNITS));
    Billing {
        source: Source::Token,
        account: parse_account_self(ACCOUNT_SELF, &units),
        ..Default::default()
    }
}

#[test]
fn account_self_parses_quota_used_and_requests() {
    let units = parse_units(Some(STATUS_UNITS));
    let account = parse_account_self(ACCOUNT_SELF, &units).expect("应能解析");
    assert_eq!(account.balance_usd, Some(12.30), "6,150,000 quota = $12.30");
    assert_eq!(account.used_usd, Some(20.50), "10,250,000 quota = $20.50");
    assert_eq!(account.requests, Some(321));
    assert_eq!(account.group, "default");
}

#[test]
fn account_self_rejects_failures_and_useless_payloads() {
    let units = parse_units(Some(STATUS_UNITS));
    for payload in [
        "",
        "not json",
        "{}",
        "[]",
        // Invalid / missing PAT: new-api puts the error in a 200 response's success:false
        r#"{"success":false,"message":"Unauthorized, invalid access token"}"#,
        // Not a single usable field
        r#"{"success":true,"data":{}}"#,
        // Only the group, no numbers at all
        r#"{"success":true,"data":{"group":"default"}}"#,
        // Type mismatch
        r#"{"success":true,"data":{"quota":"nope"}}"#,
    ] {
        assert!(parse_account_self(payload, &units).is_none(), "{payload}");
    }
}

#[test]
fn account_zero_quota_is_a_real_zero_balance() {
    // A genuine zero quota (used up) displays $0.00 — not treated as no data.
    let units = parse_units(None);
    let account = parse_account_self(
        r#"{"success":true,"data":{"quota":0,"used_quota":500000,"request_count":3}}"#,
        &units,
    )
    .expect("应能解析");
    assert_eq!(account.balance_usd, Some(0.0));
    assert_eq!(account.used_usd, Some(1.0));
    assert_eq!(account.requests, Some(3));
    assert!(account.group.is_empty(), "没给分组就是空串");
}

#[test]
fn account_only_result_still_shows_how_much_was_used() {
    // A probed station (ps.air-outer.com): the token side is entirely empty and only `/api/user/self` works;
    // its `used_quota` is that station's only usage number.
    let info = account_only_billing();
    let line = info.inline();
    assert!(line.contains("账号余额 $12.30"), "{line}");
    assert!(
        line.contains("已用 $20.50"),
        "用量要跟着余额一起显示：{line}"
    );
}

/// Account data and check-in status in one result.
fn account_with_checkin_billing() -> Billing {
    let units = parse_units(Some(STATUS_UNITS));
    Billing {
        source: Source::Token,
        account: parse_account_self(ACCOUNT_SELF, &units),
        checkin: parse_checkin_status(CHECKIN_STATUS, &units).ok(),
        ..Default::default()
    }
}

#[test]
fn checkin_state_joins_the_summary_line() {
    let line = account_with_checkin_billing().inline();
    assert!(line.contains("账号余额 $12.30"), "{line}");
    assert!(
        line.contains("签到 今日已签 $0.30"),
        "签到要和余额同一条报出来：{line}"
    );
}

#[test]
fn checkin_state_gets_its_own_detail_line() {
    let text = account_with_checkin_billing().detail();
    assert!(
        text.contains("签到：今日已签 $0.30（本月 14 次，累计获得 $13.00）"),
        "{text}"
    );
}

#[test]
fn checkin_alone_counts_as_displayable_data() {
    // Even when the account endpoint is down and only check-in reads, something must display.
    let units = parse_units(Some(STATUS_UNITS));
    let info = Billing {
        source: Source::Token,
        checkin: parse_checkin_status(CHECKIN_STATUS, &units).ok(),
        ..Default::default()
    };
    assert!(info.is_displayable(), "只有签到数据也要显示");
    let line = info.inline();
    assert!(line.contains("签到 今日已签 $0.30"), "{line}");
    assert!(
        !line.contains("未返回额度信息"),
        "有签到就不该说没数据：{line}"
    );
    // Check-in is account-level info; it stays out of the "this token" section.
    let text = info.detail();
    assert!(text.contains("签到：今日已签"), "{text}");
    assert!(!text.contains("本令牌"), "{text}");
}

#[test]
fn no_checkin_data_means_no_checkin_word() {
    // The station has no check-in, or the user set no panel token: the words "check-in" never appear on the card.
    let info = account_only_billing();
    assert!(!info.inline().contains("签到"), "{}", info.inline());
    assert!(!info.detail().contains("签到"), "{}", info.detail());
}

#[test]
fn account_balance_leads_the_summary_line() {
    let units = parse_units(Some(STATUS_UNITS));
    let usage = parse_token_usage(USAGE_TOKEN_UNLIMITED).expect("应能解析");
    let logs = parse_token_logs(TOKEN_LOGS);
    let mut info = parse_token_billing(TokenInputs {
        usage: Some(&usage),
        logs: &logs,
        units: &units,
        status_json: Some(STATUS_UNITS),
        now: 1_789_437_000,
        today_from: 1_789_430_000,
        note: None,
    });
    info.account = parse_account_self(ACCOUNT_SELF, &units);

    let line = info.inline_full();
    assert!(line.starts_with("账号余额 $12.30"), "账号余额优先：{line}");
    assert!(
        line.contains("已用 $20.50"),
        "账号侧的用量要跟着余额：{line}"
    );
    assert!(line.contains("今日 $3.00"), "令牌侧凭据仍保留：{line}");
    assert!(
        !line.contains("已用 $274.00"),
        "令牌级累计不该挤在主行（降到详情）：{line}"
    );

    let detail = info.detail();
    // The hover's small window prizes compactness: numbers merge into lines, but the "shared per station" nuance keeps a short marker.
    assert!(
        detail.contains("账号级额度（面板访问令牌，同站点共用）"),
        "{detail}"
    );
    assert!(detail.contains("余额 $12.30 · 已用 $20.50"), "{detail}");
    assert!(detail.contains("请求 321 次 · 分组 default"), "{detail}");
    assert!(detail.contains("本令牌"), "两套数据要分区：{detail}");
}

#[test]
fn account_only_result_is_still_displayable() {
    let plain = Billing {
        source: Source::Token,
        ..Default::default()
    };
    assert!(!plain.is_displayable(), "没数据就不显示");

    let info = account_only_billing();
    assert!(info.is_displayable(), "有账号数据就必须显示");
    assert_eq!(info.inline(), "账号余额 $12.30 · 已用 $20.50");
    assert_eq!(info.inline_full(), "账号余额 $12.30 · 已用 $20.50");
    assert!(info.detail().contains("账号级额度（面板访问令牌"));
}

#[test]
fn the_card_has_one_headline_number_and_a_quieter_rest() {
    // The main line keeps one "first-glance" number: balance wins; the rest demote a tier.
    let info = account_only_billing();
    assert_eq!(info.headline().as_deref(), Some("账号余额 $12.30"));
    assert_eq!(info.inline_rest(), "已用 $20.50");
    // The two parts concatenated equal the full summary.
    assert_eq!(
        format!("{} · {}", info.headline().unwrap(), info.inline_rest()),
        info.inline_full()
    );
}

#[test]
fn headline_falls_back_to_used_when_there_is_no_balance() {
    // On unlimited stations / when no balance is available, usage IS the headline number.
    let billing = Billing {
        source: Source::Token,
        used_usd: Some(3.0),
        today_usd: Some(0.5),
        ..Default::default()
    };
    assert_eq!(billing.headline().as_deref(), Some("已用 $3.00"));
    assert_eq!(billing.inline_rest(), "今日 $0.50");
}

#[test]
fn headline_is_none_when_there_is_nothing_to_show() {
    // When nothing can be queried, nothing is highlighted.
    let empty = Billing::default();
    assert_eq!(empty.headline(), None);
    assert_eq!(empty.inline_rest(), "");
}

#[test]
fn account_detail_is_disclosed_as_account_level() {
    // The small window omits endpoint paths, but the two disclosures — "account-level" and "panel access token" — must stay.
    let info = account_only_billing();
    let detail = info.detail();
    assert!(
        detail.contains("面板访问令牌"),
        "要说明靠面板令牌拿到：{detail}"
    );
    assert!(detail.contains("账号级"), "口径要说清是账号级：{detail}");
}

#[test]
fn logs_only_result_still_reports_today_and_week() {
    // Some stations lack the /api/usage/token/ panel route, yet /api/log/token still works.
    let units = parse_units(Some(STATUS_UNITS));
    let logs = parse_token_logs(TOKEN_LOGS);
    let info = parse_token_billing(TokenInputs {
        usage: None,
        logs: &logs,
        units: &units,
        status_json: Some(STATUS_UNITS),
        now: 1_789_437_000,
        today_from: 1_789_430_000,
        note: None,
    });
    assert_eq!(info.shape, Shape::TokenLogsOnly);
    assert_eq!(info.today_usd, Some(3.0), "今日仍要算出来");
    assert_eq!(info.today_calls, Some(2));
    assert_eq!(info.week_usd, Some(7.0), "近 7 天仍要算出来");
    assert_eq!(info.today_models.len(), 2);
    assert_eq!(info.used_usd, None, "没有额度接口就不编累计");
    assert_eq!(info.balance_usd, None, "更不能编余额");
    assert_eq!(info.limit_usd, None);
    assert!(info.is_displayable(), "只有日志数据也要能显示");

    let inline = info.inline();
    assert!(inline.contains("今日 $3.00"), "{inline}");
    assert!(!inline.contains("余额"), "没有额度就不该出现余额：{inline}");

    let detail = info.detail();
    assert!(detail.contains("今日已用：$3.00（2 次请求）"), "{detail}");
    // With quota missing, no quota line is left behind.
    assert!(!detail.contains("额度"), "没有额度就不该有额度行：{detail}");
    assert!(!detail.contains("接口"), "{detail}");
    assert!(!detail.contains("/api/"), "{detail}");
}

#[test]
fn logs_only_result_without_logs_is_not_displayable() {
    // Neither side yielded anything: stay at "no data".
    let units = parse_units(None);
    let info = parse_token_billing(TokenInputs {
        usage: None,
        logs: &[],
        units: &units,
        status_json: None,
        now: 1,
        today_from: 0,
        note: None,
    });
    assert_eq!(info.shape, Shape::TokenLogsOnly);
    assert!(!info.is_displayable(), "没有任何数据就不该显示");
}

/// Real observed shape: `GET /api/user/checkin` (fields taken from live probing).
const CHECKIN_STATUS: &str = r#"{"success":true,"data":{"enabled":true,
        "max_quota":250000,"min_quota":50000,"stats":{"checked_in_today":true,
        "checkin_count":14,"total_checkins":45,"total_quota":6500000,
        "records":[{"checkin_date":"2026-09-17","quota_awarded":150000},
                   {"checkin_date":"2026-09-16","quota_awarded":100000}]}}}"#;
#[test]
fn checkin_status_reads_the_real_payload() {
    let units = parse_units(Some(STATUS_UNITS));
    let status = parse_checkin_status(CHECKIN_STATUS, &units).expect("应能解析");
    assert!(status.enabled);
    assert!(status.today, "今天已签到");
    assert_eq!(status.month_count, 14);
    assert_eq!(status.total_count, 45);
    assert_eq!(status.total_usd, Some(13.0), "6_500_000 / 500_000");
    assert_eq!(status.min_usd, Some(0.1), "50_000 / 500_000");
    assert_eq!(status.max_usd, Some(0.5), "250_000 / 500_000");
}

#[test]
fn checkin_status_keeps_the_site_own_refusal_message() {
    // The station has no check-in: new-api answers 200 + success:false + this Chinese message.
    let units = parse_units(Some(STATUS_UNITS));
    let err = parse_checkin_status(r#"{"message":"签到功能未启用","success":false}"#, &units)
        .expect_err("未启用应报错");
    assert!(err.contains("签到功能未启用"), "{err}");
    // Non-JSON must never count as "already checked in".
    assert!(parse_checkin_status("not json", &units).is_err());
}

#[test]
fn checkin_parsers_survive_missing_fields() {
    let units = parse_units(None);
    let status = parse_checkin_status(r#"{"success":true,"data":{"enabled":true}}"#, &units)
        .expect("缺 stats 也要能解析");
    assert!(status.enabled);
    assert!(!status.today);
    assert_eq!(status.month_count, 0);
    assert_eq!(status.total_usd, None, "没给累计就不编数字");
    assert_eq!(status.min_usd, None, "额度区间缺省不编");
    assert!(
        parse_checkin_status(r#"{"success":true}"#, &units).is_err(),
        "没有 data 不算签到状态"
    );
}

#[test]
fn checkin_short_and_long_describe_the_same_state() {
    let units = parse_units(Some(STATUS_UNITS));
    let status = parse_checkin_status(CHECKIN_STATUS, &units).expect("应能解析");
    // The short description serves the card's line: status and today's amount only.
    let short = checkin_short(&status);
    assert!(short.contains("今日已签"), "{short}");
    assert!(short.contains("$0.30"), "今日金额要带上：{short}");
    assert!(!short.contains("累计"), "短描述不放累计：{short}");
    // The long description serves the hover: the short one's content + this month / lifetime.
    let long = checkin_long(&status);
    assert!(long.contains("今日已签"), "{long}");
    assert!(long.contains("本月 14 次"), "{long}");
    assert!(long.contains("累计获得 $13.00"), "{long}");
}

#[test]
fn checkin_descriptions_omit_numbers_the_site_did_not_give() {
    let bare = CheckinStatus {
        enabled: true,
        ..Default::default()
    };
    assert_eq!(checkin_short(&bare), "未签");
    let text = checkin_long(&bare);
    assert_eq!(text, "未签", "没给本月 / 累计就不提：{text}");

    // Checked in today but the station gave no amount: never fabricate $0.00.
    let no_amount = CheckinStatus {
        enabled: true,
        today: true,
        ..Default::default()
    };
    assert_eq!(checkin_short(&no_amount), "今日已签");
    assert!(!checkin_long(&no_amount).contains('$'), "没给就不编金额");

    // The station has no check-in: say plainly it's not enabled.
    let disabled = CheckinStatus::default();
    assert_eq!(checkin_short(&disabled), "未启用");
    assert_eq!(checkin_long(&disabled), "该站点未启用签到");
}

#[test]
fn checkin_status_reports_todays_amount_from_the_records() {
    let units = parse_units(Some(STATUS_UNITS));
    let status = parse_checkin_status(CHECKIN_STATUS, &units).expect("应能解析");
    // The fixture's latest record is 2026-09-17 / 150000 → $0.30.
    assert_eq!(status.today_usd, Some(0.3), "今日签到的金额");
    let text = checkin_short(&status);
    assert!(text.contains("$0.30"), "要能说清今天签了多少：{text}");
}

#[test]
fn checkin_status_has_no_today_amount_when_not_signed_today() {
    let units = parse_units(Some(STATUS_UNITS));
    // Not checked in today: even with history, yesterday's / the previous record's amount is never today's.
    let not_today = parse_checkin_status(
            r#"{"success":true,"data":{"enabled":true,"stats":{"checked_in_today":false,
                "checkin_count":3,"records":[{"checkin_date":"2026-09-16","quota_awarded":150000}]}}}"#,
            &units,
        )
        .expect("应能解析");
    assert_eq!(not_today.today_usd, None);
    let text = checkin_short(&not_today);
    assert_eq!(text, "未签", "今天没签就直说未签");
    assert!(!text.contains('$'), "没签就不该提金额：{text}");

    // Checked in today but the record is empty (the station gives a flag without details): likewise no fabricated amount.
    let no_records = parse_checkin_status(
            r#"{"success":true,"data":{"enabled":true,"stats":{"checked_in_today":true,"checkin_count":1}}}"#,
            &units,
        )
        .expect("应能解析");
    assert_eq!(no_records.today_usd, None);
    assert_eq!(checkin_short(&no_records), "今日已签", "只有标记、没金额");

    // Shuffled record order takes the greatest date.
    let shuffled = parse_checkin_status(
        r#"{"success":true,"data":{"enabled":true,"stats":{"checked_in_today":true,
                "checkin_count":2,"records":[{"checkin_date":"2026-09-10","quota_awarded":50000},
                {"checkin_date":"2026-09-17","quota_awarded":250000}]}}}"#,
        &units,
    )
    .expect("应能解析");
    assert_eq!(shuffled.today_usd, Some(0.5), "取最大日期那条");
}

#[test]
fn sub2api_usage_url_normalizes_base() {
    // Normal: the root path directly completes to v1/usage.
    assert_eq!(
        crate::billing::sub2api_usage_url("https://sub.example.com"),
        "https://sub.example.com/v1/usage"
    );
    // /v1 already present: only append usage.
    assert_eq!(
        crate::billing::sub2api_usage_url("https://sub.example.com/v1"),
        "https://sub.example.com/v1/usage"
    );
    // A full path: no repeated appending.
    assert_eq!(
        crate::billing::sub2api_usage_url("https://sub.example.com/v1/usage"),
        "https://sub.example.com/v1/usage"
    );
    // Trailing slashes and edge whitespace normalize first.
    assert_eq!(
        crate::billing::sub2api_usage_url("  https://sub.example.com/v1/  "),
        "https://sub.example.com/v1/usage"
    );
}

#[test]
fn sub2api_usage_parses_quota_remaining_and_unit_variants() {
    // Complete readings: quota.limit/used + remaining + unit.
    let full = crate::billing::parse_sub2api_usage(
        r#"{"quota":{"limit":100.0,"used":37.5},"remaining":62.5,"unit":"USD"}"#,
    )
    .expect("完整读数应解析成功");
    assert_eq!(full.total, Some(100.0));
    assert_eq!(full.used, Some(37.5));
    assert_eq!(full.remaining, Some(62.5));
    assert_eq!(full.unit, "USD");

    // balance only: remaining taken from it directly; the unit defaults to USD.
    let only_balance = crate::billing::parse_sub2api_usage(r#"{"balance": 12.0}"#).unwrap();
    assert_eq!(only_balance.remaining, Some(12.0));
    assert_eq!(only_balance.total, None);
    assert_eq!(only_balance.unit, "USD");

    // quota only: remaining falls back to total - used.
    let only_quota =
        crate::billing::parse_sub2api_usage(r#"{"quota":{"limit":50,"used":8}}"#).unwrap();
    assert_eq!(only_quota.remaining, Some(42.0));
    assert_eq!(only_quota.total, Some(50.0));

    // Empty readings / invalid JSON / other structures: None.
    for junk in [
        "{}",
        r#"{"unit":"USD"}"#,
        "not json",
        "[]",
        r#"{"quota":{}}"#,
    ] {
        assert!(
            crate::billing::parse_sub2api_usage(junk).is_none(),
            "{junk}"
        );
    }
}

#[test]
fn parse_data_self_aggregates_days_and_models() {
    let units = crate::billing::parse_units(None);
    let payload = r#"{"success":true,"data":[
        {"model_name":"gpt-5","count":3,"quota":500000,"token_used":100,"created_at":1767225600},
        {"model_name":"claude","count":1,"quota":250000,"token_used":50,"created_at":1767225600},
        {"model_name":"gpt-5","count":2,"quota":100000,"token_used":20,"created_at":1767139200}
    ]}"#;
    let days = crate::billing::parse_data_self(payload, &units).unwrap();
    assert_eq!(days.len(), 2, "按天聚合");
    let late = &days[1];
    assert_eq!(late.day_ts, 1767225600);
    let expect_usd = 750_000.0 / units.quota_per_unit;
    assert!((late.usd - expect_usd).abs() < 1e-9);
    assert_eq!(late.calls, 4);
    assert_eq!(late.tokens, 150);
    assert_eq!(late.top_model, "gpt-5", "取消耗最大的模型");
    assert!((days[0].usd - 100_000.0 / units.quota_per_unit).abs() < 1e-9);

    // Empty array / success:false / invalid JSON / missing data: None.
    for junk in [
        r#"{"success":true,"data":[]}"#,
        r#"{"success":false,"data":[]}"#,
        "not json",
        r#"{"success":true}"#,
    ] {
        assert!(
            crate::billing::parse_data_self(junk, &units).is_none(),
            "{junk}"
        );
    }
}

#[test]
fn data_self_url_carries_range() {
    let e = crate::billing::endpoints("https://api.x.com");
    assert_eq!(
        e.data_self(100, 200),
        "https://api.x.com/api/data/self?start_timestamp=100&end_timestamp=200"
    );
}

#[test]
fn token_usage_expiry_flows_into_billing() {
    let units = parse_units(Some(STATUS_UNITS));
    let usage = parse_token_usage(
        r#"{"code":true,"data":{"object":"token_usage","name":"t","unlimited_quota":false,"total_granted":1000000,"total_used":100000,"total_available":900000,"expires_at":1789546200}}"#,
    )
    .expect("应能解析");
    assert_eq!(usage.expires_at, Some(1_789_546_200.0));
    let logs = parse_token_logs(TOKEN_LOGS);
    let mut info = parse_token_billing(TokenInputs {
        usage: Some(&usage),
        logs: &logs,
        units: &units,
        status_json: Some(STATUS_UNITS),
        now: 1_789_437_000,
        today_from: 1_789_430_000,
        note: None,
    });
    // Before the app layer fills labels: only ts exists, so the hover omits the expiry line.
    {
        let expiry = info.expires.as_ref().expect("到期要透传");
        assert_eq!(expiry.ts, 1_789_546_200);
        assert!(expiry.label.is_empty(), "标签由应用层填");
    }
    {
        let expiry = info.expires.as_mut().expect("到期要透传");
        expiry.label = "2026-09-17".to_string();
        expiry.days_left = Some(3);
    }
    assert!(
        info.detail().contains("到期：2026-09-17"),
        "{}",
        info.detail()
    );
    assert!(info.inline().contains("3 天后到期"), "{}", info.inline());
}
