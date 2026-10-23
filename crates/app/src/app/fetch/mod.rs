//! Vendor connectivity and model latency probes, plus model-list fetching.
//!
//! Probe requests connect directly (never via a system proxy), masquerading as whitelisted clients per
//! protocol; throttling and serialization are unified in [`ProbeGate`].

use eframe::egui;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

use super::App;
use crate::app::bars::{sanitize_network_error, short_err};

/// Model discovery (the "discover models" button): background thread + channel + generation cancellation.
mod discovery;
/// Per-provider model-fetch state (background thread + channel).
mod net;
mod polling;
mod ui;
pub(crate) use discovery::*;
pub(crate) use net::*;
pub(crate) use ui::*;

/// One backend's "built-in gateway free models" state (list + fetch channel + failure reason).
///
/// Bare ids are stored here; the `provider_id/` prefix is added at display time (see [`crate::opencode_models`]).
#[derive(Default)]
pub(super) struct FreeModelsState {
    /// Free-model ids (bare ids, lexicographic).
    pub(super) models: Vec<String>,
    /// The background fetch channel (`Some` = in flight).
    pub(super) rx: Option<std::sync::mpsc::Receiver<Result<Vec<String>, String>>>,
    /// The last fetch failure's reason (cleared on success), noted beside the model dropdown.
    pub(super) error: Option<String>,
}

impl FreeModelsState {
    pub(super) fn fetching(&self) -> bool {
        self.rx.is_some()
    }
}

/// The add-provider form uses a fixed internal key to store fetch state.
pub(super) const NEW_PROVIDER_FETCH_KEY: &str = "__new_provider__";

/// Guard value passed to the latency-test gate: `Some(reason)` = block, `None` = allow.
///
/// After the user enables `allow_model_test_with_proxy`, detected proxies no longer block model probing.
/// A free function over two fields only, callable while holding `&mut self.providers[idx]`.
pub(super) fn net_guard_gate(net_guard: &Option<String>, allow: bool) -> Option<String> {
    if allow {
        return None;
    }
    net_guard.clone()
}

/// One provider's latency-test state (provider-level + model-level concurrency).
#[derive(Default)]
pub(super) struct LatencyState {
    /// Provider level: the model-list endpoint's round-trip time (milliseconds).
    pub(super) provider: Option<Result<u64, String>>,
    pub(super) provider_rx: Option<std::sync::mpsc::Receiver<Result<u64, String>>>,
    /// Model level: model id → round-trip time (milliseconds).
    pub(super) models: HashMap<String, Result<u64, String>>,
    pub(super) model_rx: Option<std::sync::mpsc::Receiver<(String, Result<u64, String>)>>,
    /// Model-level test progress: completed / total.
    pub(super) done: usize,
    pub(super) total: usize,
    /// Model ids still awaiting results in this test run (for the scrambling placeholder animation).
    pub(super) pending: HashSet<String>,
}

/// The latency test's read timeout and "timed out" threshold (milliseconds): the cap for a single read / first byte.
pub(super) const LATENCY_TIMEOUT_MS: u64 = 10_000;
/// First-byte latency coloring threshold (milliseconds): below this is green.
pub(super) const LATENCY_GOOD_MS: u64 = 2_000;
/// First-byte latency above this counts as slow (red).
pub(super) const LATENCY_SLOW_MS: u64 = 5_000;

/// Risk-control throttle for model latency probes: any two probes of the **same provider** (same model or
/// different models) are spaced ≥5 seconds; **different providers don't affect each other** (independent
/// counters, parallelizable).
pub(super) const PROBE_PROVIDER_GAP_S: f64 = 5.0;

/// Library of neutral short questions for probes: cross-domain common-sense nouns (geography / astronomy /
/// biology / chemistry / physics / literature / art / music / history) — all settled, one-sentence-answer facts.
///
/// Answers are never validated: the verdict only looks at HTTP success and round-trip time.
pub(super) const PROBE_QUESTIONS: [&str; 24] = [
    "世界上最长的河流是哪条？只回答河名",
    "世界上面积最大的国家是哪个？只回答国名",
    "世界上最高的山峰叫什么？只回答山名",
    "澳大利亚的首都是哪座城市？只回答城市名",
    "尼罗河位于哪个大洲？只回答大洲名",
    "地中海位于欧洲和哪个大洲之间？只回答大洲名",
    "太阳系中体积最大的行星是哪颗？只回答行星名",
    "太阳系中离太阳最近的行星是哪颗？只回答行星名",
    "地球的天然卫星叫什么？只回答名称",
    "北斗七星属于哪个星座？只回答星座名",
    "人体面积最大的器官是什么？只回答名称",
    "一个健康的成年人全身大约有多少块骨头？只回答数字",
    "血液中负责运输氧气的是哪种细胞？只回答名称",
    "植物通过光合作用吸收哪种气体？只回答化学式",
    "水的化学式是什么？只回答化学式",
    "食盐的主要成分是什么？只回答化学式",
    "空气中含量最多的气体是什么？只回答化学式",
    "常温常压下唯一呈液态的金属是哪种？只回答名称",
    "《红楼梦》的作者是谁？只回答姓名",
    "《论语》主要记录了哪位思想家及其弟子的言行？只回答姓名",
    "《蒙娜丽莎》的作者是谁？只回答姓名",
    "《命运交响曲》的作曲家是谁？只回答姓名",
    "中国古代四大发明中用于辨别方向的是哪一项？只回答名称",
    "泰姬陵位于哪个国家？只回答国名",
];

/// FNV-1a: gives each provider key a stable starting question offset.
pub(super) fn fnv1a(value: &str) -> usize {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash as usize
}

/// Take one probe question: provider offset + rotation cursor, so consecutive probes of one provider never repeat the same question.
pub(super) fn probe_question(provider_key: &str, cursor: usize) -> &'static str {
    let index = fnv1a(provider_key).wrapping_add(cursor) % PROBE_QUESTIONS.len();
    PROBE_QUESTIONS[index]
}

/// One probe's gate verdict.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum ProbeGateState {
    /// Probing is allowed.
    Ready,
    /// Throttle cooldown, seconds remaining.
    Cooling(f64),
    /// Global serialization: the previous probe hasn't finished.
    Busy,
    /// Blocked by the network guard (system proxy / VPN).
    NetBlocked(String),
}

/// Throttle and serialization state for model latency probes (one per provider).
///
/// Purely in-memory, reset on restart, never written to disk.
#[derive(Default)]
pub(super) struct ProbeGate {
    /// provider key → the last probe's moment (egui seconds).
    last_provider: HashMap<String, f64>,
    /// Probes in flight (provider key): one provider runs one at a time.
    in_flight: HashSet<String>,
    /// Each provider's question-rotation cursor.
    cursor: HashMap<String, usize>,
}

impl ProbeGate {
    /// Accumulate one "must still wait" constraint.
    fn accumulate(wait: &mut Option<f64>, left: Option<f64>) {
        if let Some(left) = left.filter(|left| *left > 0.0) {
            *wait = Some(match *wait {
                Some(current) => current.max(left),
                None => left,
            });
        }
    }

    /// Whether probing is allowed now; when not, gives the reason (button disabled + countdown + hover note).
    ///
    /// Looks only at this provider's state, never others' (no cross-vendor queueing).
    pub(super) fn state(
        &self,
        provider_key: &str,
        now: f64,
        net_guard: Option<&str>,
    ) -> ProbeGateState {
        if let Some(reason) = net_guard {
            return ProbeGateState::NetBlocked(reason.to_string());
        }
        // One provider allows only one probe in flight.
        if self.in_flight.contains(provider_key) {
            return ProbeGateState::Busy;
        }
        let mut wait: Option<f64> = None;
        Self::accumulate(
            &mut wait,
            self.last_provider
                .get(provider_key)
                .map(|at| PROBE_PROVIDER_GAP_S - (now - at)),
        );
        match wait {
            Some(left) => ProbeGateState::Cooling(left),
            None => ProbeGateState::Ready,
        }
    }

    /// Record a probe and take the provider's serialization slot.
    pub(super) fn start(&mut self, provider_key: &str, now: f64) {
        self.last_provider.insert(provider_key.to_string(), now);
        self.in_flight.insert(provider_key.to_string());
    }

    /// Release the provider's serialization slot once the probe ends (result / failure / timeout).
    pub(super) fn finish(&mut self, provider_key: &str) {
        self.in_flight.remove(provider_key);
    }

    /// Release the serialization slots when a provider's whole state is dropped (config reload / add form closed).
    ///
    /// A `None` provider_key releases everything.
    pub(super) fn release(&mut self, provider_key: Option<&str>) {
        match provider_key {
            Some(key) => {
                self.in_flight.remove(key);
            }
            None => self.in_flight.clear(),
        }
    }

    /// Take this provider's next probe question and advance the cursor.
    pub(super) fn next_question(&mut self, provider_key: &str) -> &'static str {
        let cursor = self.cursor.entry(provider_key.to_string()).or_insert(0);
        let question = probe_question(provider_key, *cursor);
        *cursor = cursor.wrapping_add(1);
        question
    }
}
