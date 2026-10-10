//! 按天的用量桶 —— 热力图与「今日 / 本周 / 本月 / 总计」的数据基础。
//!
//! 为什么不能只靠会话级汇总：一个会话可以横跨几十天（本机实测最长 31 天），
//! 用它的 `last_seen_ms` 归日会把整月的量堆到最后一天。tokscale 的
//! `ParsedMessage.date` 已经按消息算好了本地日历日，这里按那个粒度分桶。
//!
//! 桶里除了合计还留了**按 agent / 按模型**的分解：时间筛选与维度筛选叠加时
//! （「本月 × pi」「本周 × claude-opus-5」）必须用同粒度的数据，否则总览卡片的
//! 数字会和下面的明细表对不上。

use crate::format::ConfigFormat;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// 一组 token 计数（不含会话数：按天分解时一个会话会落进多天，不能相加）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DayTotals {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub reasoning: i64,
    /// 该桶里出现的消息条数（同一天同一会话的多条消息各计一次）。
    pub messages: i64,
}

impl DayTotals {
    /// 合计 token。口径与 `SessionSnapshot::total` 一致：
    /// `cache_write` 是子集不重复加，`cache_write_1h` 同理不参与。
    pub fn total(&self) -> i64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
            .saturating_add(self.reasoning)
    }

    /// 是否有任何计数（全 0 的桶不值得落盘）。
    pub fn is_empty(&self) -> bool {
        self.total() == 0 && self.messages == 0
    }

    /// 并入一条消息。
    pub fn add_message(&mut self, message: &tokscale_core::ParsedMessage) {
        self.input = self.input.saturating_add(message.input);
        self.output = self.output.saturating_add(message.output);
        self.cache_read = self.cache_read.saturating_add(message.cache_read);
        self.cache_write = self.cache_write.saturating_add(message.cache_write);
        self.reasoning = self.reasoning.saturating_add(message.reasoning);
        self.messages = self
            .messages
            .saturating_add(i64::from(message.message_count.max(1)));
    }

    /// 逐字段取较大值。账本合并用：token 只增不减，源里偶发的重扫回退不该让
    /// 历史缩水。
    pub fn merge_max(&mut self, other: &Self) {
        self.input = self.input.max(other.input);
        self.output = self.output.max(other.output);
        self.cache_read = self.cache_read.max(other.cache_read);
        self.cache_write = self.cache_write.max(other.cache_write);
        self.reasoning = self.reasoning.max(other.reasoning);
        self.messages = self.messages.max(other.messages);
    }

    /// 累加（跨天求和用）。
    pub fn add_assign(&mut self, other: &Self) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
        self.cache_write = self.cache_write.saturating_add(other.cache_write);
        self.reasoning = self.reasoning.saturating_add(other.reasoning);
        self.messages = self.messages.saturating_add(other.messages);
    }

    fn to_json(self) -> Value {
        json!({
            "input": self.input,
            "output": self.output,
            "cache_read": self.cache_read,
            "cache_write": self.cache_write,
            "reasoning": self.reasoning,
            "messages": self.messages,
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
        }
    }
}

/// 一天的用量：合计 + 按 agent / 模型 / 会话的分解。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DailyBucket {
    /// 本地日历日，`YYYY-MM-DD`（由 tokscale 的 `ParsedMessage.date` 给出；
    /// 上游用 `chrono::Local` 算，所以已经是本地日，不需再换算时区）。
    pub date: String,
    /// 当天合计。
    pub totals: DayTotals,
    /// 当天按 agent 分解（键是 [`ConfigFormat::label`]）。
    pub by_client: BTreeMap<String, DayTotals>,
    /// 当天按模型分解（键是模型 id，空名归入 [`UNKNOWN_MODEL`]）。
    pub by_model: BTreeMap<String, DayTotals>,
    /// 当天按会话分解（键同 [`crate::usage::SessionSnapshot::key`]：`client:session_id`）。
    ///
    /// 会话维度在时间筛选下必须用这个：一个会话可以横跨几十天，拿它的总量
    /// 当成「本月用量」会虚高。
    pub by_session: BTreeMap<String, DayTotals>,
}

/// 模型名为空时的占位，与 app 层的展示保持一致。
pub const UNKNOWN_MODEL: &str = "(未知模型)";

impl DailyBucket {
    pub fn new(date: impl Into<String>) -> Self {
        Self {
            date: date.into(),
            ..Default::default()
        }
    }

    /// 并入一条消息（同时更新合计与三个分解）。
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
        self.by_model.entry(model).or_default().add_message(message);
        self.by_session
            .entry(crate::usage::SessionSnapshot::key(
                client,
                &message.session_id,
            ))
            .or_default()
            .add_message(message);
    }

    /// 逐字段取较大值（账本合并用）。
    ///
    /// 分解表按**键**合并而不是整体替换：一次扫描可能只看到当天的一部分
    /// （例如某个 agent 的账本当时读不到），整体替换会把已经记下的分解抹掉。
    pub fn merge_max(&mut self, other: &Self) {
        self.totals.merge_max(&other.totals);
        merge_breakdown(&mut self.by_client, &other.by_client);
        merge_breakdown(&mut self.by_model, &other.by_model);
        merge_breakdown(&mut self.by_session, &other.by_session);
    }

    pub fn to_json(&self) -> Value {
        json!({
            "totals": self.totals.to_json(),
            "by_client": breakdown_to_json(&self.by_client),
            "by_model": breakdown_to_json(&self.by_model),
            "by_session": breakdown_to_json(&self.by_session),
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
        }
    }
}

/// 按键合并一张分解表（见 [`DailyBucket::merge_max`]）。
fn merge_breakdown(target: &mut BTreeMap<String, DayTotals>, other: &BTreeMap<String, DayTotals>) {
    for (key, totals) in other {
        target.entry(key.clone()).or_default().merge_max(totals);
    }
}

fn breakdown_to_json(map: &BTreeMap<String, DayTotals>) -> serde_json::Map<String, Value> {
    map.iter().map(|(k, v)| (k.clone(), v.to_json())).collect()
}

fn breakdown_from_json(value: &Value, key: &str) -> BTreeMap<String, DayTotals> {
    value
        .get(key)
        .and_then(Value::as_object)
        .map(|obj| {
            obj.iter()
                .map(|(k, v)| (k.clone(), DayTotals::from_json(v)))
                .collect()
        })
        .unwrap_or_default()
}

/// 按天分桶的集合，键是 `YYYY-MM-DD`（字典序即时间序）。
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

    /// 合计口径：五个桶相加，不重复计子集。
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

    /// 一条消息同时进合计、按 agent、按模型三个地方。
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

    /// 会话分解：同一天同一会话的多条消息累加到同一键。
    #[test]
    fn session_breakdown_accumulates_within_a_day() {
        let mut bucket = DailyBucket::new("2026-01-01");
        bucket.add_message(ConfigFormat::Pi, &message(100, 10, "2026-01-01", "m"));
        bucket.add_message(ConfigFormat::Pi, &message(50, 5, "2026-01-01", "m"));
        assert_eq!(bucket.by_session.len(), 1, "同一会话只占一个键");
        assert_eq!(bucket.by_session["pi:s"].input, 150);
        assert_eq!(bucket.totals.input, 150);
    }

    /// 会话分解要能区分不同 agent 下的同名会话 id。
    #[test]
    fn session_breakdown_separates_clients() {
        let mut bucket = DailyBucket::new("2026-01-01");
        bucket.add_message(ConfigFormat::Pi, &message(100, 0, "2026-01-01", "m"));
        bucket.add_message(ConfigFormat::QwenCode, &message(7, 0, "2026-01-01", "m"));
        assert_eq!(bucket.by_session.len(), 2);
        assert_eq!(bucket.by_session["pi:s"].input, 100);
        assert_eq!(bucket.by_session["qwen-code:s"].input, 7);
    }

    /// 空模型名归入占位，不产生空字符串键。
    #[test]
    fn blank_model_falls_back_to_placeholder() {
        let mut bucket = DailyBucket::new("2026-01-01");
        bucket.add_message(ConfigFormat::Pi, &message(1, 1, "2026-01-01", "   "));
        assert!(bucket.by_model.contains_key(UNKNOWN_MODEL));
        assert!(!bucket.by_model.contains_key(""));
    }

    /// 分解表按键合并：一次只看到部分 agent 的扫描不该抹掉其他 agent 的记录。
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

    /// 合并取较大值，回退的扫描不让历史缩水。
    #[test]
    fn merge_max_never_shrinks() {
        let mut left = DailyBucket::new("2026-01-01");
        left.add_message(ConfigFormat::Pi, &message(1000, 0, "2026-01-01", "m"));
        let mut small = DailyBucket::new("2026-01-01");
        small.add_message(ConfigFormat::Pi, &message(10, 0, "2026-01-01", "m"));
        left.merge_max(&small);
        assert_eq!(left.totals.input, 1000);
    }

    /// JSON 往返一致。
    #[test]
    fn json_round_trip() {
        let mut bucket = DailyBucket::new("2026-01-01");
        bucket.add_message(ConfigFormat::Pi, &message(100, 10, "2026-01-01", "glm-5"));
        bucket.add_message(ConfigFormat::QwenCode, &message(7, 3, "2026-01-01", "q"));

        let restored = DailyBucket::from_json("2026-01-01", &bucket.to_json());
        assert_eq!(restored, bucket);
    }

    /// 全 0 的桶应被识别（落盘时过滤掉）。
    #[test]
    fn empty_bucket_is_detected() {
        assert!(DayTotals::default().is_empty());
        let mut t = DayTotals::default();
        t.add_message(&message(0, 1, "2026-01-01", "m"));
        assert!(!t.is_empty());
    }
}
