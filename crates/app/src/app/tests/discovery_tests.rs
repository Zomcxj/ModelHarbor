//! 模型探测（A-2）的纯逻辑与调度测试：鉴权选择、候选文案、勾选种子、
//! 只填空新增计算、状态栏文案、HH:MM 与缓存命中。
//!
//! 全部不联网：只用空 base_url（`probe` 在本地即判 `BadBaseUrl`）、本地非法
//! URL（解析阶段即失败）与缓存命中路径，不起任何网络请求。

use crate::app::fetch::{
    cancel_discovery, format_hhmm, hhmm_from_unix_utc, is_existing_model, planned_additions,
    probe_auth_for_secret, probe_candidates_label, probe_status_text, seed_checked,
    should_use_cache, start_discovery, ProbeSummary,
};
use model_harbor_core::discovery::{merge_missing, DiscoveryCache, ProbeAuth};
use std::collections::HashMap;

#[test]
fn empty_secret_probes_without_auth_headers() {
    assert_eq!(probe_auth_for_secret(""), ProbeAuth::None);
    assert_eq!(probe_auth_for_secret("   "), ProbeAuth::None);
    // 填了 key：按 OpenAI 兼容网关的默认形态只发 Bearer。
    assert_eq!(probe_auth_for_secret("sk-xxx"), ProbeAuth::Bearer);
}

#[test]
fn candidates_label_lists_fallback_endpoints() {
    // 普通域名：先 /v1/models，失败退 /models。
    let label = probe_candidates_label("https://api.example.com");
    assert!(
        label.contains("https://api.example.com/v1/models"),
        "{label}"
    );
    assert!(label.contains("https://api.example.com/models"), "{label}");
    // 已带 /v1 的 base 只有一个候选，没有回退箭头。
    let label = probe_candidates_label("https://api.example.com/v1");
    assert!(label.contains("/v1/models"), "{label}");
    assert!(!label.contains(" → "), "{label}");
    // 未填地址：提示先填，而不是列空候选。
    assert!(probe_candidates_label("  ").contains("Base URL"));
}

#[test]
fn seed_checked_defaults_to_unchecked() {
    let existing = vec!["GPT-4o".to_string()];
    let discovered = vec![
        "gpt-4o".to_string(),   // 已有（大小写不敏感）→ 不勾（面板里禁用）
        "o4-mini".to_string(),  // 新增 → 默认不勾，用户手动选
        "claude-x".to_string(), // 新增 → 默认不勾，用户手动选
    ];
    assert_eq!(
        seed_checked(&discovered, &existing),
        vec![false, false, false]
    );
}

#[test]
fn existing_model_check_is_case_insensitive_and_trimmed() {
    let existing = vec![" GPT-4o ".to_string()];
    assert!(is_existing_model(&existing, "gpt-4o"));
    assert!(is_existing_model(&existing, "GPT-4O"));
    assert!(!is_existing_model(&existing, "gpt-4o-mini"));
    // 空白 id 不算已有（merge 侧会丢弃它）。
    assert!(!is_existing_model(&existing, "  "));
}

#[test]
fn planned_additions_keep_order_and_skip_existing_unchecked_blank() {
    let existing = vec!["GPT-4o".to_string()];
    let discovered = vec![
        "gpt-4o".to_string(),   // 已有（大小写不敏感）→ 排除
        "o4-mini".to_string(),  // 勾选 → 新增
        "claude-x".to_string(), // 未勾选 → 排除
        "  ".to_string(),       // 空白 → 丢弃
        "GPT-4O".to_string(),   // 与已有项大小写重复 → 排除
        "o3".to_string(),       // 勾选 → 新增
    ];
    let checked = vec![true, true, false, true, true, true];
    assert_eq!(
        planned_additions(&existing, &discovered, &checked),
        vec!["o4-mini".to_string(), "o3".to_string()]
    );
}

#[test]
fn planned_additions_equals_merge_missing_tail() {
    // 「添加」按钮点下去实际追加的条数与内容，必须与直接跑 merge_missing 的
    // 尾部完全一致（按钮上的 N 才不会说谎）。
    let existing = vec!["a".to_string()];
    let discovered = vec!["b".to_string(), "a".to_string(), "c".to_string()];
    let checked = vec![true, true, true];
    let planned = planned_additions(&existing, &discovered, &checked);
    let merged = merge_missing(&existing, discovered.clone());
    assert_eq!(&merged[existing.len()..], &planned[..]);
    assert_eq!(planned, ["b".to_string(), "c".to_string()]);
    // 勾选位数少于发现列表时按未勾选处理，不 panic。
    let short = planned_additions(&existing, &discovered, &[true]);
    assert_eq!(short, ["b".to_string()]);
}

#[test]
fn status_text_reports_success_count_or_failure_reason() {
    assert_eq!(
        probe_status_text("14:23", &ProbeSummary::Success(5)),
        "最近获取：14:23（成功 5 个）"
    );
    assert_eq!(
        probe_status_text(
            "09:05",
            &ProbeSummary::Failure("未配置 API Key".to_string())
        ),
        "最近获取：09:05（失败：未配置 API Key）"
    );
}

#[test]
fn hhmm_formats_utc_wall_clock() {
    assert_eq!(hhmm_from_unix_utc(0), "00:00");
    assert_eq!(hhmm_from_unix_utc(86_399), "23:59");
    // 2023-11-14 22:13:20 UTC。
    assert_eq!(hhmm_from_unix_utc(1_700_000_000), "22:13");
    // 时钟回拨之类的负值也不 panic，按当日余数理解。
    assert_eq!(hhmm_from_unix_utc(-1), "23:59");
    assert_eq!(format_hhmm(7, 5), "07:05");
    // 超出范围的值夹回合法区间（防御外部时钟异常）。
    assert_eq!(format_hhmm(25, 61), "23:59");
    assert_eq!(format_hhmm(-1, -1), "00:00");
}

#[test]
fn cache_only_used_when_idle_and_panel_closed() {
    assert!(should_use_cache(false, false));
    assert!(!should_use_cache(true, false));
    assert!(!should_use_cache(false, true));
    assert!(!should_use_cache(true, true));
}

#[test]
fn cache_hit_opens_panel_without_network() {
    let mut states: HashMap<String, _> = HashMap::new();
    let mut cache = DiscoveryCache::default();
    cache.insert(
        "https://api.example.com",
        None,
        vec!["m-1".to_string(), "m-2".to_string()],
    );
    let existing = vec!["m-2".to_string()];
    // 首尾空白不影响缓存键。
    let msg = start_discovery(
        &mut states,
        &mut cache,
        "k",
        " https://api.example.com ",
        "",
        &existing,
    );
    let msg = msg.unwrap();
    assert!(msg.contains("缓存"), "{msg}");
    assert!(msg.contains("2 个模型"), "{msg}");
    let state = states.get("k").unwrap();
    assert!(!state.in_flight());
    let found = state.found.as_ref().unwrap();
    assert_eq!(found.models, ["m-1".to_string(), "m-2".to_string()]);
    // 默认全部不勾：新增项由用户手动挑选（m-2 已有，面板里禁用）。
    assert_eq!(found.checked, [false, false]);
    // 面板开着 = 用户想刷新：不再查缓存，直接起后台线程
    // （本地非法 URL 在解析阶段即失败，无网络 IO）。
    let msg = start_discovery(&mut states, &mut cache, "k", "not-a-url", "", &existing);
    assert_eq!(msg, None);
    assert!(states.get("k").unwrap().in_flight());
}

#[test]
fn start_and_cancel_bump_generation_to_invalidate_replies() {
    let mut states: HashMap<String, _> = HashMap::new();
    let mut cache = DiscoveryCache::default();
    // 空 base_url：probe 在本地即失败（BadBaseUrl），不起任何网络请求。
    assert_eq!(
        start_discovery(&mut states, &mut cache, "k", "", "", &[]),
        None
    );
    {
        let state = states.get("k").unwrap();
        assert!(state.in_flight());
        assert_eq!(state.generation, 1);
    }
    // 取消：gen 自增 + 丢弃通道；之后回传的 gen=1 会被轮询整体丢弃。
    assert_eq!(
        cancel_discovery(&mut states, "k").as_deref(),
        Some("已取消模型获取")
    );
    {
        let state = states.get("k").unwrap();
        assert!(!state.in_flight());
        assert_eq!(state.generation, 2);
    }
    // 没在飞时点取消：不打扰状态栏。
    assert_eq!(cancel_discovery(&mut states, "k"), None);
    // 再次探测：gen 继续前进；取消过的旧回包永远不会被当成新结果。
    assert_eq!(
        start_discovery(&mut states, &mut cache, "k", "", "", &[]),
        None
    );
    assert_eq!(states.get("k").unwrap().generation, 3);
}

#[test]
fn start_ignores_clicks_while_probe_in_flight() {
    let mut states: HashMap<String, _> = HashMap::new();
    let mut cache = DiscoveryCache::default();
    assert_eq!(
        start_discovery(&mut states, &mut cache, "k", "", "", &[]),
        None
    );
    // 通道还在（无论线程是否已回包）：按钮已禁用，防御性忽略重复点击。
    assert_eq!(
        start_discovery(&mut states, &mut cache, "k", "", "", &[]),
        None
    );
    assert_eq!(states.get("k").unwrap().generation, 1);
}

#[test]
fn cache_hit_seeds_checked_for_new_provider_form_key() {
    // 新增表单的固定 key（__new_provider__）与普通 provider key 走同一套状态表。
    let mut states: HashMap<String, _> = HashMap::new();
    let mut cache = DiscoveryCache::default();
    cache.insert("https://api.example.com", None, vec!["m-1".to_string()]);
    let msg = start_discovery(
        &mut states,
        &mut cache,
        crate::app::fetch::NEW_PROVIDER_FETCH_KEY,
        "https://api.example.com",
        "",
        &[],
    );
    assert!(msg.is_some());
    assert!(states.contains_key(crate::app::fetch::NEW_PROVIDER_FETCH_KEY));
}
