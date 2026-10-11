//! 本机用量统计 —— 扫描 10 个 agent 的本地会话账本，按 agent / 模型 / 会话 / 时间
//! 四个维度汇总 token 使用量。
//!
//! 扫描内核用 [`tokscale_core`]（上游 `junhoyeo/tokscale`，MIT）：它原生解析 56 个
//! agent 的会话格式，本模块只做三件事 ——
//!
//! 1. [`agents`]：把 [`ConfigFormat`](crate::format::ConfigFormat) 映射到 tokscale 的
//!    client id（**6/10 拼写不一致**，必须显式映射）；
//! 2. [`scan`]：调 `parse_local_clients` 拿回扁平消息列表，再按维度汇总；
//! 3. [`ledger`]：把每次扫描到的会话快照落盘，**已删除的会话用量永久保留**。
//!
//! 口径（与 tokscale 一致，勿自行调整）：
//! - `input` 是**新鲜输入**，已排除 `cache_read`，不要重复相加；
//! - `cache_write_1h` 是 `cache_write` 的子集，**不参与合计**。

pub mod agents;
pub mod daily;
pub mod ledger;

pub use agents::{config_format_for_client, tokscale_client_for};
pub use daily::{DailyBucket, DailyMap, DayTotals, UNKNOWN_MODEL};
pub use ledger::{Ledger, SessionSnapshot};

use std::collections::HashMap;

/// 扫描内核的公开入口：只扫 ModelHarbor 支持的这些 agent。
///
/// 顺序取 [`crate::backends::BACKENDS`]（权威注册表），便于 UI 稳定排序。
pub fn tokscale_clients() -> Vec<String> {
    crate::backends::BACKENDS
        .iter()
        .filter_map(|backend| tokscale_client_for(backend.id()).map(str::to_string))
        .collect()
}

/// 一次扫描的原始结果（未经账本合并）。
pub struct ScanResult {
    /// 按 `client:session_id` 归并后的会话快照。
    pub sessions: HashMap<String, SessionSnapshot>,
    /// 按本地日历日分桶的用量（热力图与「今日 / 本月 / 总计」的基础）。
    ///
    /// 会话级汇总做不到这件事：一个会话可以横跨几十天（本机实测最长 31 天），
    /// 按它的 `last_seen_ms` 归日会把整月的量堆到最后一天。
    pub daily: DailyMap,
    /// 消息条数（tokscale 的 `ParsedMessage` 计数）。
    pub message_count: usize,
    /// 内核自报的扫描耗时（毫秒）。
    pub processing_time_ms: u32,
}

/// 扫描本机全部已知 agent 的本地会话账本。
///
/// `ignored` 是用户已删除并永久忽略的会话键（`client:session_id`）：扫到也不
/// 进结果，否则删掉的会话会在下一轮扫描里复活。
///
/// **这是全量扫描（实测 2.4-2.7s），调用方必须放在后台线程**；UI 线程按指纹
/// 变化 + debounce 决定何时重扫，见 `app::usage`。
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
    // 定价：tokscale 的解析通道不套定价（上游源码注释明说 "This lane applies
    // no pricing"），只有上游实报的 cost 有值；其余用本机定价缓存现算
    // （%APPDATA%\tokscale\cache，同步、离线，无网络请求）。缓存不存在时
    // 保持 0：界面不显示金额，不能拿 $0.00 冒充「没花钱」。
    if let Some(pricing) = tokscale_core::pricing::PricingService::load_cached_any_age() {
        for message in &mut parsed.messages {
            // 上游实报过的不再现算；空模型名没法定价，也跳过。
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
            // tokscale 支持的 agent 比 ModelHarbor 多（56 个）：不在映射表里的
            // 客户端直接丢弃，避免 UI 出现无法切页的孤儿条目。
            continue;
        };
        let key = SessionSnapshot::key(client, &message.session_id);
        if ignored.contains(&key) {
            // 用户删掉的会话：不进会话汇总，也不进按天分桶。
            continue;
        }
        let entry = sessions.entry(key).or_insert_with(|| {
            SessionSnapshot::new(client, &message.session_id, &message.model_id)
        });
        entry.absorb(message);
        // 同一条消息同时进会话汇总与按天分桶：两者服务于不同视图，不能互相推导。
        if !message.date.is_empty() {
            daily
                .entry(message.date.clone())
                .or_insert_with(|| DailyBucket::new(&message.date))
                .add_message(client, message);
        }
    }
    Ok(ScanResult {
        sessions,
        daily,
        message_count,
        processing_time_ms: parsed.processing_time_ms,
    })
}
