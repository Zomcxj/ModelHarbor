//! Local usage statistics — scans 10 agents' local session ledgers, summing token usage across four
//! dimensions: agent / model / session / time.
//!
//! The scan engine is [`tokscale_core`] (upstream `junhoyeo/tokscale`, MIT): it natively parses 56 agents'
//! session formats. This module does four things —
//!
//! 1. [`agents`]: maps [`ConfigFormat`](crate::format::ConfigFormat) to tokscale's client ids
//!    (**6 of 10 spellings disagree** — the mapping must be explicit);
//! 2. [`scan`]: calls `parse_local_clients` for the flat message list, then aggregates per dimension;
//! 3. [`titles`]: backfills session titles (tokscale's message stream carries none);
//! 4. [`ledger`]: persists each scan's session snapshots, **keeping deleted sessions' usage forever**.
//!
//! Accounting (matches tokscale; don't adjust on your own):
//! - `input` is **fresh input**, already excluding `cache_read` — don't add it twice;
//! - `cache_write_1h` is a subset of `cache_write` and **excluded from totals**.

pub mod agents;
pub mod daily;
pub mod delete;
pub mod ledger;
pub mod titles;

// The pi family's storage layout: shared by deletion and title reading, not exported.
mod pi;

pub use agents::{config_format_for_client, tokscale_client_for};
pub use daily::{DailyBucket, DailyMap, DayTotals, UNKNOWN_MODEL};
pub use delete::{delete_session, DeleteError, Deleted};
pub use ledger::{Ledger, SessionSnapshot};

use std::collections::HashMap;

/// The scan engine's public entry: scans only the agents ModelHarbor supports.
///
/// Order follows [`crate::backends::BACKENDS`] (the authoritative registry), keeping UI sorting stable.
pub fn tokscale_clients() -> Vec<String> {
    crate::backends::BACKENDS
        .iter()
        .filter_map(|backend| tokscale_client_for(backend.id()).map(str::to_string))
        .collect()
}

/// One scan's raw result (before ledger merge).
pub struct ScanResult {
    /// Session snapshots merged by `client:session_id`.
    pub sessions: HashMap<String, SessionSnapshot>,
    /// Usage bucketed per local calendar day (the heatmap's and "today / month / total" foundation).
    ///
    /// Session-level sums can't do this: one session can span dozens of days, and bucketing by its
    /// `last_seen_ms` would pile a whole month onto its last day.
    pub daily: DailyMap,
    /// Message count (tokscale's `ParsedMessage` count).
    pub message_count: usize,
    /// Scan duration self-reported by the engine (milliseconds).
    pub processing_time_ms: u32,
}

/// Scan every known agent's local session ledger.
///
/// `ignored` holds keys of sessions the user deleted and permanently ignored (`client:session_id`): seen
/// in a scan, they stay out of the result — otherwise deleted sessions would revive on the next scan.
///
/// **This is a full scan; callers must run it on a background thread.** The UI thread decides when to
/// rescan via fingerprint changes + debounce — see `app::usage`.
pub fn scan(ignored: &std::collections::BTreeSet<String>) -> Result<ScanResult, String> {
    let options = tokscale_core::LocalParseOptions {
        home_dir: None,
        use_env_roots: false,
        clients: Some(tokscale_clients()),
        since: None,
        until: None,
        year: None,
        scanner_settings: tokscale_core::scanner::ScannerSettings::default(),
    };
    let mut parsed = tokscale_core::parse_local_clients(options)?;
    let message_count = parsed.messages.len();
    // Pricing: tokscale's parsing lane applies no pricing (the upstream source comments say so plainly:
    // "This lane applies no pricing"); only upstream-reported cost carries a value. Everything else
    // computes from the local pricing cache (%APPDATA%\tokscale\cache — synced, offline, no network
    // requests). With no cache, costs stay 0: the UI shows no amounts, and $0.00 must never impersonate
    // "cost nothing".
    if let Some(pricing) = tokscale_core::pricing::PricingService::load_cached_any_age() {
        for message in &mut parsed.messages {
            // Upstream-reported costs aren't recomputed; empty model names can't be priced and skip too.
            if message.cost != 0.0 || message.model_id.is_empty() {
                continue;
            }
            let breakdown = tokscale_core::TokenBreakdown {
                input: message.input,
                output: message.output,
                cache_read: message.cache_read,
                cache_write: message.cache_write,
                cache_write_1h: message.cache_write_1h,
                reasoning: message.reasoning,
            };
            message.cost = tokscale_core::calculate_cost_with_service_tier(
                &pricing,
                &message.model_id,
                if message.provider_id.is_empty() {
                    None
                } else {
                    Some(&message.provider_id)
                },
                &breakdown,
                message.service_tier.as_deref(),
            );
        }
    }
    let mut sessions: HashMap<String, SessionSnapshot> = HashMap::new();
    let mut daily = DailyMap::new();
    for message in &parsed.messages {
        let Some(client) = config_format_for_client(&message.client) else {
            // tokscale supports more agents than ModelHarbor (56): clients missing from the mapping are dropped
            // outright, so the UI never grows orphan entries that can't be paged to.
            continue;
        };
        let key = SessionSnapshot::key(client, &message.session_id);
        if ignored.contains(&key) {
            // User-deleted sessions: into neither the session aggregation nor the daily buckets.
            continue;
        }
        let entry = sessions.entry(key).or_insert_with(|| {
            SessionSnapshot::new(client, &message.session_id, &message.model_id)
        });
        entry.absorb(message);
        // The same message enters both the session aggregation and the daily buckets: the two serve different
        // views and can't be derived from each other.
        if !message.date.is_empty() {
            daily
                .entry(message.date.clone())
                .or_insert_with(|| DailyBucket::new(&message.date))
                .add_message(client, message);
        }
    }
    // Session titles: absent from tokscale's message stream — a separate pass reads them from the sources
    // (the opencode family reads the database's title; the pi family takes the first user message).
    // Unreadable stays empty.
    titles::collect(&mut sessions);
    Ok(ScanResult {
        sessions,
        daily,
        message_count,
        processing_time_ms: parsed.processing_time_ms,
    })
}
