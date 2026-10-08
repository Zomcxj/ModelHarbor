//! 厂商连通性与模型延迟探测，以及模型列表获取。
//!
//! 探测请求直连（不经系统代理），按协议伪装成白名单客户端；节流与串行由
//! [`ProbeGate`] 统一把关。

use eframe::egui;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

use super::App;
use crate::app::bars::{sanitize_network_error, short_err};

/// 模型探测（「探测模型」按钮）：后台线程 + 通道 + generation 取消。
mod discovery;
/// 单个 provider 的模型获取状态（后台线程 + 通道）。
mod net;
mod polling;
mod ui;
pub(crate) use discovery::*;
pub(crate) use net::*;
pub(crate) use ui::*;

pub(super) struct ModelFetchState {
    pub(super) rx: Option<std::sync::mpsc::Receiver<Result<Vec<String>, String>>>,
    pub(super) result: Option<Result<Vec<String>, String>>,
}

/// 单个后端的「内置网关免费模型」状态（列表 + 拉取通道 + 失败原因）。
///
/// 裸 id 存这里，界面显示时再拼 `provider_id/` 前缀（见 [`crate::opencode_models`]）。
#[derive(Default)]
pub(super) struct FreeModelsState {
    /// 免费模型 id（裸 id，按字典序）。
    pub(super) models: Vec<String>,
    /// 后台拉取通道（`Some` = 正在飞）。
    pub(super) rx: Option<std::sync::mpsc::Receiver<Result<Vec<String>, String>>>,
    /// 最近一次拉取失败的原因（成功后清空），在模型下拉旁提示。
    pub(super) error: Option<String>,
}

impl FreeModelsState {
    pub(super) fn fetching(&self) -> bool {
        self.rx.is_some()
    }
}

/// 新增 Provider 表单使用固定的内部 key 保存获取状态。
pub(super) const NEW_PROVIDER_FETCH_KEY: &str = "__new_provider__";

/// 传给延迟测试门控的守卫值：`Some(原因)` = 拦截，`None` = 放行。
///
/// 用户打开 `allow_model_test_with_proxy` 后，检测到的代理不再拦截模型探测。
/// 只吃两个字段的自由函数，便于在持有 `&mut self.providers[idx]` 时调用。
pub(super) fn net_guard_gate(net_guard: &Option<String>, allow: bool) -> Option<String> {
    if allow {
        return None;
    }
    net_guard.clone()
}

/// 单个 provider 的延迟测试状态（provider 级 + 模型级并发）。
#[derive(Default)]
pub(super) struct LatencyState {
    /// provider 级：模型列表接口往返耗时（毫秒）。
    pub(super) provider: Option<Result<u64, String>>,
    pub(super) provider_rx: Option<std::sync::mpsc::Receiver<Result<u64, String>>>,
    /// 模型级：模型 id → 往返耗时（毫秒）。
    pub(super) models: HashMap<String, Result<u64, String>>,
    pub(super) model_rx: Option<std::sync::mpsc::Receiver<(String, Result<u64, String>)>>,
    /// 模型级测试进度：已完成 / 总数。
    pub(super) done: usize,
    pub(super) total: usize,
    /// 本次测试中尚未返回结果的模型 id（用于显示测试中的乱码动画）。
    pub(super) pending: HashSet<String>,
}

/// 延迟测试的读取超时与「超时」判定阈值（毫秒）：单个读操作 / 首字的等待上限。
pub(super) const LATENCY_TIMEOUT_MS: u64 = 10_000;
/// 首字延迟着色阈值（毫秒）：低于此值为绿色。
pub(super) const LATENCY_GOOD_MS: u64 = 2_000;
/// 首字延迟超过此值算慢（红）。
pub(super) const LATENCY_SLOW_MS: u64 = 5_000;

/// 模型延迟探测的风控节流参数：**同一个 provider** 的任意两次探测（同模型、
/// 不同模型都算）间隔 ≥5 秒；**不同 provider 互不牵连**（各自独立计数、可并行）。
pub(super) const PROBE_PROVIDER_GAP_S: f64 = 5.0;

/// 探测用的中性短问句库：跨领域的常识名词（地理 / 天文 / 生物 / 化学 / 物理 /
/// 文学 / 艺术 / 音乐 / 历史），都是「一句话能答」的定论型题目。
///
/// 不校验答案：判定只看 HTTP 是否成功与往返耗时。
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

/// FNV-1a：给 provider key 一个稳定的起始题号偏移。
pub(super) fn fnv1a(value: &str) -> usize {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash as usize
}

/// 取一条探测问句：provider 偏移 + 轮换游标，同一 provider 连续探测不重复同一题。
pub(super) fn probe_question(provider_key: &str, cursor: usize) -> &'static str {
    let index = fnv1a(provider_key).wrapping_add(cursor) % PROBE_QUESTIONS.len();
    PROBE_QUESTIONS[index]
}

/// 单次探测的门控结论。
#[derive(Clone, Debug, PartialEq)]
pub(super) enum ProbeGateState {
    /// 可以探测。
    Ready,
    /// 节流冷却中，剩余秒数。
    Cooling(f64),
    /// 全局串行：上一个探测还没结束。
    Busy,
    /// 网络守卫拦截（系统代理 / VPN）。
    NetBlocked(String),
}

/// 模型延迟探测的节流与串行状态（按 provider 各自一份）。
///
/// 纯内存、重启清零，不落盘。
#[derive(Default)]
pub(super) struct ProbeGate {
    /// provider key → 上次探测时刻（egui 秒）。
    last_provider: HashMap<String, f64>,
    /// 正在飞的探测（provider key）：同一 provider 一次只允许一个。
    in_flight: HashSet<String>,
    /// 每个 provider 的题目轮换游标。
    cursor: HashMap<String, usize>,
}

impl ProbeGate {
    /// 累加一个「还需等待」的约束。
    fn accumulate(wait: &mut Option<f64>, left: Option<f64>) {
        if let Some(left) = left.filter(|left| *left > 0.0) {
            *wait = Some(match *wait {
                Some(current) => current.max(left),
                None => left,
            });
        }
    }

    /// 当前是否允许探测；不允许时给出原因（按钮禁用 + 倒计时 + 悬停说明）。
    ///
    /// 只看本 provider 的状态，不看别的 provider（跨厂商不排队）。
    pub(super) fn state(
        &self,
        provider_key: &str,
        now: f64,
        net_guard: Option<&str>,
    ) -> ProbeGateState {
        if let Some(reason) = net_guard {
            return ProbeGateState::NetBlocked(reason.to_string());
        }
        // 同一 provider 一次只允许一个探测在飞。
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

    /// 记录一次探测并占用该 provider 的串行位。
    pub(super) fn start(&mut self, provider_key: &str, now: f64) {
        self.last_provider.insert(provider_key.to_string(), now);
        self.in_flight.insert(provider_key.to_string());
    }

    /// 探测结束（拿到结果 / 失败 / 超时）后释放该 provider 的串行位。
    pub(super) fn finish(&mut self, provider_key: &str) {
        self.in_flight.remove(provider_key);
    }

    /// 探测状态整体被丢弃时释放串行位（重载配置 / 关闭新增表单）。
    ///
    /// `provider_key` 为 `None` 表示全部释放。
    pub(super) fn release(&mut self, provider_key: Option<&str>) {
        match provider_key {
            Some(key) => {
                self.in_flight.remove(key);
            }
            None => self.in_flight.clear(),
        }
    }

    /// 取该 provider 的下一条探测问句并推进游标。
    pub(super) fn next_question(&mut self, provider_key: &str) -> &'static str {
        let cursor = self.cursor.entry(provider_key.to_string()).or_insert(0);
        let question = probe_question(provider_key, *cursor);
        *cursor = cursor.wrapping_add(1);
        question
    }
}
