//! Model discovery (A-2)'s pure logic and scheduling tests: auth choice, candidate wording, check seeding,
//! fill-only-add computation, status wording, HH:MM, and cache hits.
//!
//! All offline: only an empty base_url (`probe` judges `BadBaseUrl` locally), locally malformed URLs (failing
//! at parse), and cache-hit paths — no network requests at all.

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
    // With a key: only Bearer is sent, per the OpenAI-compatible gateway default shape.
    assert_eq!(probe_auth_for_secret("sk-xxx"), ProbeAuth::Bearer);
}

#[test]
fn candidates_label_lists_fallback_endpoints() {
    // An ordinary domain: /v1/models first, falling back to /models.
    let label = probe_candidates_label("https://api.example.com");
    assert!(
        label.contains("https://api.example.com/v1/models"),
        "{label}"
    );
    assert!(label.contains("https://api.example.com/models"), "{label}");
    // A base already carrying /v1 has one candidate and no fallback arrow.
    let label = probe_candidates_label("https://api.example.com/v1");
    assert!(label.contains("/v1/models"), "{label}");
    assert!(!label.contains(" → "), "{label}");
    // No address filled: prompt for one rather than listing empty candidates.
    assert!(probe_candidates_label("  ").contains("Base URL"));
}

#[test]
fn seed_checked_defaults_to_unchecked() {
    let existing = vec!["GPT-4o".to_string()];
    let discovered = vec![
        "gpt-4o".to_string(), // Existing (case-insensitive) → unchecked (disabled in the panel)
        "o4-mini".to_string(), // New → unchecked by default; the user picks
        "claude-x".to_string(), // New → unchecked by default; the user picks
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
    // Whitespace-only ids don't count as existing (the merge side drops them).
    assert!(!is_existing_model(&existing, "  "));
}

#[test]
fn planned_additions_keep_order_and_skip_existing_unchecked_blank() {
    let existing = vec!["GPT-4o".to_string()];
    let discovered = vec![
        "gpt-4o".to_string(),   // Existing (case-insensitive) → excluded
        "o4-mini".to_string(),  // Checked → added
        "claude-x".to_string(), // Unchecked → excluded
        "  ".to_string(),       // Whitespace → dropped
        "GPT-4O".to_string(),   // Case-duplicate of an existing one → excluded
        "o3".to_string(),       // Checked → added
    ];
    let checked = vec![true, true, false, true, true, true];
    assert_eq!(
        planned_additions(&existing, &discovered, &checked),
        vec!["o4-mini".to_string(), "o3".to_string()]
    );
}

#[test]
fn planned_additions_equals_merge_missing_tail() {
    // What the "add" button actually appends — count and content — must match running merge_missing's tail
    // exactly (so the button's N never lies).
    let existing = vec!["a".to_string()];
    let discovered = vec!["b".to_string(), "a".to_string(), "c".to_string()];
    let checked = vec![true, true, true];
    let planned = planned_additions(&existing, &discovered, &checked);
    let merged = merge_missing(&existing, discovered.clone());
    assert_eq!(&merged[existing.len()..], &planned[..]);
    assert_eq!(planned, ["b".to_string(), "c".to_string()]);
    // Fewer check bits than discovered entries count as unchecked, no panic.
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
    // 2023-11-14 22:13:20 UTC.
    assert_eq!(hhmm_from_unix_utc(1_700_000_000), "22:13");
    // Negative values (clock rollback etc.) don't panic either — read as the day's remainder.
    assert_eq!(hhmm_from_unix_utc(-1), "23:59");
    assert_eq!(format_hhmm(7, 5), "07:05");
    // Out-of-range values clamp back into range (defending against external clock oddities).
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
    // Leading/trailing whitespace doesn't affect the cache key.
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
    // Defaults all unchecked: new finds are picked manually by the user (m-2 exists — disabled in the panel).
    assert_eq!(found.checked, [false, false]);
    // Panel open = the user wants a refresh: no cache check, straight to a background thread
    // (a locally malformed URL fails at parse — no network IO).
    let msg = start_discovery(&mut states, &mut cache, "k", "not-a-url", "", &existing);
    assert_eq!(msg, None);
    assert!(states.get("k").unwrap().in_flight());
}

#[test]
fn start_and_cancel_bump_generation_to_invalidate_replies() {
    let mut states: HashMap<String, _> = HashMap::new();
    let mut cache = DiscoveryCache::default();
    // Empty base_url: probe fails locally (BadBaseUrl), no network request at all.
    assert_eq!(
        start_discovery(&mut states, &mut cache, "k", "", "", &[]),
        None
    );
    {
        let state = states.get("k").unwrap();
        assert!(state.in_flight());
        assert_eq!(state.generation, 1);
    }
    // Cancel: gen bumps + the channel drops; a later gen=1 reply is discarded wholesale by polling.
    assert_eq!(
        cancel_discovery(&mut states, "k").as_deref(),
        Some("已取消模型获取")
    );
    {
        let state = states.get("k").unwrap();
        assert!(!state.in_flight());
        assert_eq!(state.generation, 2);
    }
    // Cancel with nothing in flight: the status bar is untouched.
    assert_eq!(cancel_discovery(&mut states, "k"), None);
    // Probe again: gen keeps advancing; a cancelled old reply can never pass as a new result.
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
    // The channel still exists (whether or not the thread replied): the button is disabled — defensively ignore duplicate clicks.
    assert_eq!(
        start_discovery(&mut states, &mut cache, "k", "", "", &[]),
        None
    );
    assert_eq!(states.get("k").unwrap().generation, 1);
}

#[test]
fn cache_hit_seeds_checked_for_new_provider_form_key() {
    // The add form's fixed key (__new_provider__) shares the state table with ordinary provider keys.
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
