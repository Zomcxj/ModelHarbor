#![cfg(test)]

use super::*;

/// 真实响应：哈基米API站（公益站，额度是占位值 1e8，已用 $216.00）。
const HAJIMI_SUB: &str = r#"{"object":"billing_subscription","has_payment_method":true,
        "soft_limit_usd":100000000,"hard_limit_usd":100000000,"system_hard_limit_usd":100000000,
        "access_until":0}"#;
const HAJIMI_USAGE: &str = r#"{"object":"list","total_usage":21600}"#;
const HAJIMI_STATUS: &str =
    r#"{"success":true,"data":{"system_name":"哈基米API站","version":"v1.0.0-rc.19.3518"}}"#;

/// 真实响应：另一种形状 credit_summary（单位未标注）。
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
    // 公益站额度是占位值 1e8：算出来的「余额」没意义，所以既不显示也不保留。
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
    // 占位额度只显示已用：既没有额度数字，也没有余额。
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
    // 不少站用 hard_limit_usd: 0 表示「无硬上限」，直接算会得到负数余额。
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
    // 额度比已用小（数据不一致 / 已超支）：只说已用，不显示负数余额。
    let sub = r#"{"object":"billing_subscription","hard_limit_usd":10.0}"#;
    let usage = r#"{"object":"list","total_usage":1500}"#;
    let billing = parse(sub, Some(usage), None);
    assert_eq!(billing.balance_usd, None);
    assert_eq!(billing.inline(), "已用 $15.00");
    assert!(!billing.detail().contains("-$"), "不要出现负金额");
}

/// 真实际形状：`/api/status`（换算字段 + 面板名）。
const STATUS_UNITS: &str = r#"{"success":true,"data":{"system_name":"哈基米API站",
        "version":"v1.0.0-rc.19.3518","quota_per_unit":500000,"quota_display_type":"CNY",
        "display_in_currency":true,"usd_exchange_rate":7.3}}"#;
/// 真实际形状：`/api/usage/token/`（公益站：不限额度）。
const USAGE_TOKEN_UNLIMITED: &str = r#"{"code":true,"message":"","data":{"name":"demo",
        "unlimited_quota":true,"total_granted":0,"total_used":137000000,
        "total_available":-137000000,"expires_at":0}}"#;
/// 真实际形状：`/api/log/token`（字段取自实测；`content` / `ip` 等隐私字段不入测试）。
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

    // 今日：前两条（第三条在昨天的窗口里）
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

    // 空日志 / 坏日志都不要 panic
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
    // 站点没给换算比 → 按默认并标记假设
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
    // 悬停小窗只放「数字 + 会影响读数的口径」（换算比、跳日）。
    // 接口来源、分页上限、只读这类实现细节在卡片上只是噪音，一律不写。
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
    // 降级原因（调用日志不可用、面板令牌失效）留在结果里，小窗只显示数字。
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
    // 站点不支持：拿到的可能是 404 页面或空对象
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

    // anthropic 系 baseUrl 没有 /v1，末尾斜杠要去掉
    let ep = endpoints("https://kktoken.cc/");
    assert_eq!(ep.status, "https://kktoken.cc/api/status");
    assert_eq!(
        ep.subscription,
        "https://kktoken.cc/dashboard/billing/subscription"
    );

    // 无协议头：整体当 origin（这类地址已被 baseUrl 体检标为「缺少协议头」）
    let ep = endpoints("api.example.com/v1");
    assert_eq!(ep.status, "api.example.com/v1/api/status");
}

#[test]
fn endpoints_v1_only_touches_origin_only_bases() {
    // pi 页里 anthropic 系的 baseUrl 只有 origin → 备选端点补上 /v1
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

    // 已带 /v1：必须原样返回，否则会拼出 /v1/v1/...（实测 404）
    let same = endpoints("https://gemai.huchan.cn/v1");
    let alt = endpoints_v1("https://gemai.huchan.cn/v1");
    assert_eq!(alt.subscription, same.subscription);

    // 带了其他路径：不猜该插在哪里，原样返回
    let same = endpoints("https://host/api");
    let alt = endpoints_v1("https://host/api");
    assert_eq!(alt.subscription, same.subscription);
}

/// 真实响应形状：`/api/user/self`（需要面板 PAT，普通登录用户即可）。
/// 字段名取自 new-api `buildSelfUserData`（controller/user.go）。
const ACCOUNT_SELF: &str = r#"{"success":true,"message":"","data":{
        "id":1,"username":"tester","display_name":"Tester","role":1,"status":1,
        "group":"default","quota":6150000,"used_quota":10250000,"request_count":321}}"#;

/// 只有账号数据、没有令牌数据时的展示（站点未开 /api/usage/token/）。
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
        // PAT 无效 / 未提供：new-api 把错误放在 200 响应的 success:false 里
        r#"{"success":false,"message":"Unauthorized, invalid access token"}"#,
        // 一个可用字段都没有
        r#"{"success":true,"data":{}}"#,
        // 只有分组，没有任何数字
        r#"{"success":true,"data":{"group":"default"}}"#,
        // 类型不符
        r#"{"success":true,"data":{"quota":"nope"}}"#,
    ] {
        assert!(parse_account_self(payload, &units).is_none(), "{payload}");
    }
}

#[test]
fn account_zero_quota_is_a_real_zero_balance() {
    // 额度真为 0（用完了）必须显示 $0.00，而不是当成「没拿到数据」隐掉。
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
    // 实测站点（ps.air-outer.com）：`/api/usage/token/` 是 404、
    // `/api/log/token` 回 200 但 `data:[]`，所以令牌侧整条都是空的，
    // 只有 `/api/user/self` 可用——它的 `used_quota` 是这一站**唯一**的
    // 用量数字。主行只报余额的话，用户会以为连使用量都读不到了。
    let info = account_only_billing();
    let line = info.inline();
    assert!(line.contains("账号余额 $12.30"), "{line}");
    assert!(
        line.contains("已用 $20.50"),
        "用量要跟着余额一起显示：{line}"
    );
}

/// 账号数据 + 签到状态同一条结果（「查询用户数据」的正常形态）。
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
    // 账号接口挂了（要 New-Api-User 的站点很常见）、只有签到读得到：
    // 「有就输出」——这一项也要能显示出来。
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
    // 签到是账号级信息（同一个面板令牌读的），不能被塞进「本令牌」分节。
    let text = info.detail();
    assert!(text.contains("签到：今日已签"), "{text}");
    assert!(!text.contains("本令牌"), "{text}");
}

#[test]
fn no_checkin_data_means_no_checkin_word() {
    // 站点没开签到、或用户没填面板令牌：卡片上不该出现「签到」这两个字。
    // 「有就输出，没有就不输出」——不写死占位。
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
    // 悬停小窗求紧凑：解释长句去掉、数字合并成行，但「同站点共用」这层
    // 含义要留一个短标记（否则读者会以为这是某个 sk- 令牌的余额）。
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
fn account_detail_is_disclosed_as_account_level() {
    // 小窗不写接口路径，但「账号级」与「面板访问令牌」两项口径披露必须留住：
    // 否则会被读成某个 sk- 令牌的余额。
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
    // 部分站点（如把 baseUrl 指向中转域名）没有 /api/usage/token/ 这个面板路由，
    // 但 /api/log/token 仍可用：今日 / 近 7 天用量必须单独拿出来，
    // 不能因为额度接口缺失就把日志统计一起丢掉。
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
    // 额度缺失时不留额度行。
    assert!(!detail.contains("额度"), "没有额度就不该有额度行：{detail}");
    assert!(!detail.contains("接口"), "{detail}");
    assert!(!detail.contains("/api/"), "{detail}");
}

#[test]
fn logs_only_result_without_logs_is_not_displayable() {
    // 两边都没拿到：保持“无数据”，不要凭空造出一个空结果。
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

/// 真实际形状：`GET /api/user/checkin`（字段取自实测；records 只留两条）。
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
    // 站点没开签到：new-api 用 200 + success:false + 这句中文报错。
    let units = parse_units(Some(STATUS_UNITS));
    let err = parse_checkin_status(r#"{"message":"签到功能未启用","success":false}"#, &units)
        .expect_err("未启用应报错");
    assert!(err.contains("签到功能未启用"), "{err}");
    // 不是 JSON 也不能当成「已签到」。
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
    // 短描述给卡片那一行：只说状态与今日金额。
    let short = checkin_short(&status);
    assert!(short.contains("今日已签"), "{short}");
    assert!(short.contains("$0.30"), "今日金额要带上：{short}");
    assert!(!short.contains("累计"), "短描述不放累计：{short}");
    // 长描述给悬停说明：短描述的内容 + 本月 / 累计。
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

    // 今天签了但站点没给金额：不能编一个 $0.00。
    let no_amount = CheckinStatus {
        enabled: true,
        today: true,
        ..Default::default()
    };
    assert_eq!(checkin_short(&no_amount), "今日已签");
    assert!(!checkin_long(&no_amount).contains('$'), "没给就不编金额");

    // 站点没开签到：明确说未启用，不显示成「未签」（那是两件事）。
    let disabled = CheckinStatus::default();
    assert_eq!(checkin_short(&disabled), "未启用");
    assert_eq!(checkin_long(&disabled), "该站点未启用签到");
}

#[test]
fn checkin_status_reports_todays_amount_from_the_records() {
    let units = parse_units(Some(STATUS_UNITS));
    let status = parse_checkin_status(CHECKIN_STATUS, &units).expect("应能解析");
    // fixture 的最新一条是 2026-09-17 / 150000 → $0.30。
    assert_eq!(status.today_usd, Some(0.3), "今日签到的金额");
    let text = checkin_short(&status);
    assert!(text.contains("$0.30"), "要能说清今天签了多少：{text}");
}

#[test]
fn checkin_status_has_no_today_amount_when_not_signed_today() {
    let units = parse_units(Some(STATUS_UNITS));
    // 今天没签：即使有历史记录，也不能把昨天/上一条的金额当成今天。
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

    // 今天签了但记录为空（站点只给标记不给明细）：同样不编金额。
    let no_records = parse_checkin_status(
            r#"{"success":true,"data":{"enabled":true,"stats":{"checked_in_today":true,"checkin_count":1}}}"#,
            &units,
        )
        .expect("应能解析");
    assert_eq!(no_records.today_usd, None);
    assert_eq!(checkin_short(&no_records), "今日已签", "只有标记、没金额");

    // 记录顺序被打乱时按最大日期取，不依赖站点给的先后。
    let shuffled = parse_checkin_status(
        r#"{"success":true,"data":{"enabled":true,"stats":{"checked_in_today":true,
                "checkin_count":2,"records":[{"checkin_date":"2026-09-10","quota_awarded":50000},
                {"checkin_date":"2026-09-17","quota_awarded":250000}]}}}"#,
        &units,
    )
    .expect("应能解析");
    assert_eq!(shuffled.today_usd, Some(0.5), "取最大日期那条");
}
