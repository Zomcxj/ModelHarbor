//! Per-day usage buckets — the data foundation for the heatmap and "today / week / month / total".
//!
//! Session-level sums alone won't do: one session can span dozens of days, and bucketing by its
//! `last_seen_ms` would pile a whole month onto its last day. tokscale's `ParsedMessage.date` already
//! computes the local calendar day per message; buckets follow that granularity.
//!
//! Besides totals, buckets keep **per-agent / per-model** breakdowns: when time filtering and dimension
//! filtering stack ("this month × pi", "this week × claude-opus-5"), same-granularity data is required,
//! otherwise the overview card's numbers won't match the detail tables below.

use crate::format::ConfigFormat;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// A group of token counts (no session count: in per-day terms one session falls into several days and must not be summed).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DayTotals {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub reasoning: i64,
    /// The message count appearing in this bucket (each message of the same session on the same day counts once).
    pub messages: i64,
    /// Converted cost (USD): as reported upstream, or computed from the local pricing cache. Old entries have none — 0.
    pub cost: f64,
}

impl DayTotals {
    /// Total tokens. Same accounting as `SessionSnapshot::total`:
    /// `cache_write` is a subset and isn't added twice; likewise `cache_write_1h` stays out.
    pub fn total(&self) -> i64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
            .saturating_add(self.reasoning)
    }

    /// Whether any count exists (all-zero buckets aren't worth persisting).
    pub fn is_empty(&self) -> bool {
        self.total() == 0 && self.messages == 0
    }

    /// Absorb one message.
    pub fn add_message(&mut self, message: &tokscale_core::ParsedMessage) {
        self.input = self.input.saturating_add(message.input);
        self.output = self.output.saturating_add(message.output);
        self.cache_read = self.cache_read.saturating_add(message.cache_read);
        self.cache_write = self.cache_write.saturating_add(message.cache_write);
        self.reasoning = self.reasoning.saturating_add(message.reasoning);
        self.messages = self
            .messages
            .saturating_add(i64::from(message.message_count.max(1)));
        self.cost += message.cost;
    }

    /// Take the larger value per field. Used by the ledger merge: tokens only grow, and an occasional
    /// source-side rescan dip must not shrink history.
    pub fn merge_max(&mut self, other: &Self) {
        self.input = self.input.max(other.input);
        self.output = self.output.max(other.output);
        self.cache_read = self.cache_read.max(other.cache_read);
        self.cache_write = self.cache_write.max(other.cache_write);
        self.reasoning = self.reasoning.max(other.reasoning);
        self.messages = self.messages.max(other.messages);
        self.cost = self.cost.max(other.cost);
    }

    /// Accumulate (for cross-day sums).
    pub fn add_assign(&mut self, other: &Self) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
        self.cache_write = self.cache_write.saturating_add(other.cache_write);
        self.reasoning = self.reasoning.saturating_add(other.reasoning);
        self.messages = self.messages.saturating_add(other.messages);
        self.cost += other.cost;
    }

    fn to_json(self) -> Value {
        json!({
            "input": self.input,
            "output": self.output,
            "cache_read": self.cache_read,
            "cache_write": self.cache_write,
            "reasoning": self.reasoning,
            "messages": self.messages,
            "cost": self.cost,
        })
    }

    fn from_json(value: &Value) -> Self {
        let get = |key: &str| value.get(key).and_then(Value::as_i64).unwrap_or(0);
        Self {
            input: get("input"),
            output: get("output"),
            cache_read: get("cache_read"),
            cache_write: get("cache_write"),
            reasoning: get("reasoning"),
            messages: get("messages"),
            cost: value.get("cost").and_then(Value::as_f64).unwrap_or(0.0),
        }
    }
}

/// One day's usage: total + per-agent / model / session breakdowns.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DailyBucket {
    /// The local calendar day, `YYYY-MM-DD` (given by tokscale's `ParsedMessage.date`; upstream computes with
    /// `chrono::Local`, so it's already a local day — no timezone conversion needed).
    pub date: String,
    /// The day's total.
    pub totals: DayTotals,
    /// The day's per-agent breakdown (keys are [`ConfigFormat::label`]).
    pub by_client: BTreeMap<String, DayTotals>,
    /// The day's per-model breakdown (keys are model ids; empty names go to [`UNKNOWN_MODEL`]).
    pub by_model: BTreeMap<String, DayTotals>,
    /// The day's per-session breakdown (keys match [`crate::usage::SessionSnapshot::key`]: `client:session_id`).
    ///
    /// The session dimension must use this under time filtering: one session can span dozens of days, and
    /// using its lifetime total as "this month's usage" would overstate.
    pub by_session: BTreeMap<String, DayTotals>,
    /// The day's "agent × model" cross-breakdown (outer keys are agent labels, inner are model ids).
    ///
    /// The model dimension needs it under time filtering: `by_client` and `by_model` are two independent
    /// dimensions that can't cross into "which agent used this model". A nested map instead of assembled
    /// string keys avoids ambiguity when model names contain separators.
    pub by_client_model: BTreeMap<String, BTreeMap<String, DayTotals>>,
}

/// Placeholder for an empty model name, kept consistent with the app layer's display.
pub const UNKNOWN_MODEL: &str = "(未知模型)";

impl DailyBucket {
    pub fn new(date: impl Into<String>) -> Self {
        Self {
            date: date.into(),
            ..Default::default()
        }
    }

    /// Absorb one message (updating the total and all three breakdowns).
    pub fn add_message(&mut self, client: ConfigFormat, message: &tokscale_core::ParsedMessage) {
        self.totals.add_message(message);
        let label = client.label();
        self.by_client
            .entry(label.to_string())
            .or_default()
            .add_message(message);
        let model = if message.model_id.trim().is_empty() {
            UNKNOWN_MODEL.to_string()
        } else {
            message.model_id.clone()
        };
        self.by_model
            .entry(model.clone())
            .or_default()
            .add_message(message);
        self.by_session
            .entry(crate::usage::SessionSnapshot::key(
                client,
                &message.session_id,
            ))
            .or_default()
            .add_message(message);
        // The cross-breakdown is what lets the model dimension say "which agent used this model".
        self.by_client_model
            .entry(label.to_string())
            .or_default()
            .entry(model)
            .or_default()
            .add_message(message);
    }

    /// Take the larger value per field (for the ledger merge).
    ///
    /// Breakdown tables merge **by key** instead of wholesale replacement: one scan may only see part of the
    /// day (say one agent's ledger was unreadable at the time) — wholesale replacement would erase the
    /// breakdown already recorded.
    pub fn merge_max(&mut self, other: &Self) {
        self.totals.merge_max(&other.totals);
        merge_breakdown(&mut self.by_client, &other.by_client);
        merge_breakdown(&mut self.by_model, &other.by_model);
        merge_breakdown(&mut self.by_session, &other.by_session);
        for (client, models) in &other.by_client_model {
            let target = self.by_client_model.entry(client.clone()).or_default();
            merge_breakdown(target, models);
        }
    }

    /// Withdraw a session's **details** for the day (removing the key from `by_session` only).
    ///
    /// The total and the agent / model / cross breakdowns **stay untouched**: deletion withdraws session
    /// details, not usage. This matches cc-switch's `usage_daily_rollups` (no session dimension at all) and
    /// token-monitor's `periods.allTime` (pruning drops only the today / month windows) — both keep
    /// aggregates and drop session details.
    ///
    /// The session table takes rows from `by_session` and templates from ledger snapshots; the deleted
    /// session's snapshot already left via [`crate::usage::Ledger::ignore_session`], so withdrawing details
    /// costs the UI no row — keeping them would only bloat the ledger without bound as sessions accumulate.
    ///
    /// Returns whether anything was actually withdrawn (`false` when the session has no record that day).
    pub fn remove_session_detail(&mut self, key: &str) -> bool {
        self.by_session.remove(key).is_some()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "totals": self.totals.to_json(),
            "by_client": breakdown_to_json(&self.by_client),
            "by_model": breakdown_to_json(&self.by_model),
            "by_session": breakdown_to_json(&self.by_session),
            "by_client_model": self
                .by_client_model
                .iter()
                .map(|(client, models)| (client.clone(), Value::Object(breakdown_to_json(models))))
                .collect::<serde_json::Map<_, _>>(),
        })
    }

    pub fn from_json(date: &str, value: &Value) -> Self {
        Self {
            date: date.to_string(),
            totals: value
                .get("totals")
                .map(DayTotals::from_json)
                .unwrap_or_default(),
            by_client: breakdown_from_json(value, "by_client"),
            by_model: breakdown_from_json(value, "by_model"),
            by_session: breakdown_from_json(value, "by_session"),
            by_client_model: value
                .get("by_client_model")
                .and_then(Value::as_object)
                .map(|obj| {
                    obj.iter()
                        .map(|(client, models)| (client.clone(), breakdown_from_value(models)))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

/// Merge a breakdown table by key (see [`DailyBucket::merge_max`]).
fn merge_breakdown(target: &mut BTreeMap<String, DayTotals>, other: &BTreeMap<String, DayTotals>) {
    for (key, totals) in other {
        target.entry(key.clone()).or_default().merge_max(totals);
    }
}

fn breakdown_to_json(map: &BTreeMap<String, DayTotals>) -> serde_json::Map<String, Value> {
    map.iter().map(|(k, v)| (k.clone(), v.to_json())).collect()
}

fn breakdown_from_json(value: &Value, key: &str) -> BTreeMap<String, DayTotals> {
    value.get(key).map(breakdown_from_value).unwrap_or_default()
}

fn breakdown_from_value(value: &Value) -> BTreeMap<String, DayTotals> {
    value
        .as_object()
        .map(|obj| {
            obj.iter()
                .map(|(k, v)| (k.clone(), DayTotals::from_json(v)))
                .collect()
        })
        .unwrap_or_default()
}

/// Collection of daily buckets, keyed by `YYYY-MM-DD` (lexicographic = chronological).
pub type DailyMap = BTreeMap<String, DailyBucket>;

#[cfg(test)]
mod tests {
    use super::*;
    use tokscale_core::ParsedMessage;

    fn message(input: i64, output: i64, date: &str, model: &str) -> ParsedMessage {
        ParsedMessage {
            client: "pi".to_string(),
            model_id: model.to_string(),
            provider_id: String::new(),
            session_id: "s".to_string(),
            workspace_key: None,
            workspace_label: None,
            timestamp: 0,
            date: date.to_string(),
            input,
            output,
            cache_read: 0,
            cache_write: 0,
            cache_write_1h: 0,
            reasoning: 0,
            duration_ms: None,
            message_count: 1,
            agent: None,
            cost: 0.0,
            cost_source: Default::default(),
            service_tier: None,
        }
    }

    /// Cost accumulates with add_message and survives the JSON round trip; old entries (no cost key) read back as 0.
    #[test]
    fn cost_accumulates_and_survives_json() {
        let mut m = message(1, 2, "2026-01-01", "m");
        m.cost = 1.25;
        let mut t = DayTotals::default();
        t.add_message(&m);
        assert!((t.cost - 1.25).abs() < 1e-9);

        let back = DayTotals::from_json(&t.to_json());
        assert_eq!(back, t);

        let legacy = DayTotals::from_json(&json!({ "input": 5 }));
        assert_eq!(legacy.cost, 0.0);
    }

    /// Total accounting: five buckets summed, subsets never double-counted.
    #[test]
    fn totals_sum_every_bucket() {
        let mut t = DayTotals::default();
        t.add_message(&message(1, 2, "2026-01-01", "m"));
        t.cache_read = 4;
        t.cache_write = 8;
        t.reasoning = 16;
        assert_eq!(t.total(), 31);
        assert_eq!(t.messages, 1);
    }

    /// One message enters three places at once: the total, per-agent, and per-model.
    #[test]
    fn one_message_updates_totals_and_both_breakdowns() {
        let mut bucket = DailyBucket::new("2026-01-01");
        bucket.add_message(ConfigFormat::Pi, &message(100, 10, "2026-01-01", "glm-5"));
        assert_eq!(bucket.totals.input, 100);
        assert_eq!(bucket.by_client["pi"].input, 100);
        assert_eq!(bucket.by_model["glm-5"].input, 100);
        assert_eq!(
            bucket.by_session["pi:s"].input, 100,
            "会话分解用 client:id 键"
        );
    }

    /// Session breakdown: multiple messages of one session on one day accumulate onto the same key.
    #[test]
    fn session_breakdown_accumulates_within_a_day() {
        let mut bucket = DailyBucket::new("2026-01-01");
        bucket.add_message(ConfigFormat::Pi, &message(100, 10, "2026-01-01", "m"));
        bucket.add_message(ConfigFormat::Pi, &message(50, 5, "2026-01-01", "m"));
        assert_eq!(bucket.by_session.len(), 1, "同一会话只占一个键");
        assert_eq!(bucket.by_session["pi:s"].input, 150);
        assert_eq!(bucket.totals.input, 150);
    }

    /// The session breakdown must distinguish same-named session ids under different agents.
    #[test]
    fn session_breakdown_separates_clients() {
        let mut bucket = DailyBucket::new("2026-01-01");
        bucket.add_message(ConfigFormat::Pi, &message(100, 0, "2026-01-01", "m"));
        bucket.add_message(ConfigFormat::QwenCode, &message(7, 0, "2026-01-01", "m"));
        assert_eq!(bucket.by_session.len(), 2);
        assert_eq!(bucket.by_session["pi:s"].input, 100);
        assert_eq!(bucket.by_session["qwen-code:s"].input, 7);
    }

    /// Empty model names go to the placeholder; no empty-string keys.
    #[test]
    fn blank_model_falls_back_to_placeholder() {
        let mut bucket = DailyBucket::new("2026-01-01");
        bucket.add_message(ConfigFormat::Pi, &message(1, 1, "2026-01-01", "   "));
        assert!(bucket.by_model.contains_key(UNKNOWN_MODEL));
        assert!(!bucket.by_model.contains_key(""));
    }

    /// Breakdown tables merge by key: a scan seeing only some agents must not erase other agents' records.
    #[test]
    fn merge_keeps_breakdown_keys_from_both_sides() {
        let mut left = DailyBucket::new("2026-01-01");
        left.add_message(ConfigFormat::Pi, &message(100, 0, "2026-01-01", "m1"));
        let mut right = DailyBucket::new("2026-01-01");
        right.add_message(ConfigFormat::Opencode, &message(50, 0, "2026-01-01", "m2"));

        left.merge_max(&right);
        assert_eq!(left.by_client["pi"].input, 100, "原有 agent 保留");
        assert_eq!(left.by_client["opencode"].input, 50, "新 agent 并入");
        assert_eq!(left.by_model.len(), 2, "两个模型都在");
    }

    /// Merges take the larger value; a dipping rescan doesn't shrink history.
    #[test]
    fn merge_max_never_shrinks() {
        let mut left = DailyBucket::new("2026-01-01");
        left.add_message(ConfigFormat::Pi, &message(1000, 0, "2026-01-01", "m"));
        let mut small = DailyBucket::new("2026-01-01");
        small.add_message(ConfigFormat::Pi, &message(10, 0, "2026-01-01", "m"));
        left.merge_max(&small);
        assert_eq!(left.totals.input, 1000);
    }

    /// Cross-breakdown: two agents using different models the same day never cross-contaminate.
    #[test]
    fn cross_breakdown_separates_models_per_client() {
        let mut bucket = DailyBucket::new("2026-01-01");
        bucket.add_message(ConfigFormat::Pi, &message(100, 0, "2026-01-01", "glm-5"));
        bucket.add_message(
            ConfigFormat::QwenCode,
            &message(7, 0, "2026-01-01", "qwen3"),
        );

        assert_eq!(bucket.by_client_model["pi"]["glm-5"].input, 100);
        assert!(
            !bucket.by_client_model["pi"].contains_key("qwen3"),
            "qwen3 不是 pi 用的"
        );
        assert_eq!(bucket.by_client_model["qwen-code"]["qwen3"].input, 7);
        assert!(!bucket.by_client_model["qwen-code"].contains_key("glm-5"));
    }

    /// The cross-breakdown's merge: every agent's keys are kept.
    #[test]
    fn cross_breakdown_merges_per_client() {
        let mut left = DailyBucket::new("2026-01-01");
        left.add_message(ConfigFormat::Pi, &message(100, 0, "2026-01-01", "m1"));
        let mut right = DailyBucket::new("2026-01-01");
        right.add_message(ConfigFormat::Opencode, &message(50, 0, "2026-01-01", "m2"));

        left.merge_max(&right);
        assert_eq!(left.by_client_model["pi"]["m1"].input, 100);
        assert_eq!(left.by_client_model["opencode"]["m2"].input, 50);
    }

    /// The JSON round trip is consistent.
    #[test]
    fn json_round_trip() {
        let mut bucket = DailyBucket::new("2026-01-01");
        bucket.add_message(ConfigFormat::Pi, &message(100, 10, "2026-01-01", "glm-5"));
        bucket.add_message(ConfigFormat::QwenCode, &message(7, 3, "2026-01-01", "q"));

        let restored = DailyBucket::from_json("2026-01-01", &bucket.to_json());
        assert_eq!(restored, bucket);
    }

    /// All-zero buckets are recognized (filtered out at flush time).
    #[test]
    fn empty_bucket_is_detected() {
        assert!(DayTotals::default().is_empty());
        let mut t = DayTotals::default();
        t.add_message(&message(0, 1, "2026-01-01", "m"));
        assert!(!t.is_empty());
    }
}
