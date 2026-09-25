//! 厂商连通性与模型延迟探测，以及模型列表获取。
//!
//! 探测请求一律直连（不经系统代理），并按协议伪装成白名单客户端；
//! 节流与串行由 [`ProbeGate`] 统一把关，避免触发中转站的测活风控。

use eframe::egui;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

use super::App;
use crate::app::bars::{sanitize_network_error, short_err};

/// 单个 provider 的模型获取状态（后台线程 + 通道）。
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
/// 用户打开 `allow_model_test_with_proxy` 后，检测到的代理不再拦截模型探测
/// （中转站的多 IP / 测活风控风险由用户自己承担，见 `crate::netguard` 说明）。
///
/// 写成只吃两个字段的自由函数而不是 `&self` 方法：调用处往往正持有
/// `&mut self.providers[idx]`（甚至在还需要独占 `*self` 的闭包里），
/// 整结构借用会直接编译不过；只借这两个字段则不受影响。
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
///（首字延迟量级远小于整段生成耗时，不能沿用同步请求的 5 秒口径。）
pub(super) const LATENCY_GOOD_MS: u64 = 2_000;
/// 首字延迟超过此值算慢（红）。
pub(super) const LATENCY_SLOW_MS: u64 = 5_000;

/// 模型延迟探测的风控节流参数（中转站的「多 IP 检测 / 测活封号」）：
/// **同一个 provider** 的任意两次探测（同模型、不同模型都算）间隔 ≥5 秒；
/// **不同 provider 互不牵连**（不同中转站是不同站点，各自独立计数、可并行）。
pub(super) const PROBE_PROVIDER_GAP_S: f64 = 5.0;

/// 探测用的中性短问句库：跨领域的常识名词（地理 / 天文 / 生物 / 化学 / 物理 /
/// 文学 / 艺术 / 音乐 / 历史），都是「一句话能答」的定论型题目。
///
/// 目的只是让请求看起来像普通对话而不是脚本测活（**不校验答案**：判定只看 HTTP
/// 是否成功与往返耗时），所以题目要求「短、无歧义、与政治 / 敏感话题无关」。
/// 题目太浅（如「1 加 1 等于几」）反而不像真人在问，所以选题偏向各领域的常识名词。
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

/// FNV-1a：给 provider key 一个稳定的起始题号偏移，避免所有 provider 都从第 1 题开始。
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
/// 纯内存、重启清零：重启后只能手动一个个点，人手点击的节奏本身不构成突发，
/// 因此无需落盘（写盘也不增加实际安全性）。
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
        // 同一 provider 一次只允许一个探测在飞：探测最长 10 秒，可能超过 5 秒间隔，
        // 否则会同时向同一个中转站发两个请求。
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
    /// 不释放的话，被丢弃的通道不会再有结果回传，该 provider 会永久卡在 Busy。
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

/// 延迟测试用的 HTTP 客户端（较短超时，避免卡住 UI 线程池）。
///
/// **不要给它设置 `Proxy`**：ureq 默认不使用系统代理，探测请求始终直连，开着 Clash /
/// VPN 时也不会从代理出口发出（中转站的「多 IP 检测」看的正是出口 IP）。
/// 一旦在这里引入 `Proxy::try_from_env()`，探测就会改走代理口，务必保持直连。
pub(super) fn latency_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(5))
        .timeout_read(std::time::Duration::from_millis(LATENCY_TIMEOUT_MS))
        .build()
}

pub(super) fn http_error(err: ureq::Error, elapsed: u64) -> String {
    match err {
        // 状态码后面补人话原因与处理建议（悬停提示看全文，卡片上只显示短摘要）。
        ureq::Error::Status(code, _) => {
            format!("{}（{} ms）", crate::http_status::detail(code), elapsed)
        }
        ureq::Error::Transport(t) => {
            format!("网络错误：{}", sanitize_network_error(&t.to_string()))
        }
    }
}

/// 延迟配色：<2s 绿色、2~5s 黄色、≥5s 红色（超时同样显示红色错误）。
///
/// 具体色值由 [`crate::theme::semantics`] 提供：全部主题共享一套语义色，
/// 并在主题的 panel / faint / extreme 三类底色上保持可读性。
pub(super) fn latency_color(ms: u64, colors: crate::theme::Semantics) -> egui::Color32 {
    if ms < LATENCY_GOOD_MS {
        colors.ok
    } else if ms < LATENCY_SLOW_MS {
        colors.warn
    } else {
        colors.err
    }
}

/// 延迟测试进行中的「乱码」动画字符集（半角片假名 + 数字，参考 MemoPaws 密钥页）。
pub(super) const MATRIX_CHARS: &str = "ｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝ0123456789";
/// 乱码动画每帧时长（毫秒）与每行字符数。
pub(super) const MATRIX_FRAME_MS: u64 = 70;
pub(super) const MATRIX_LEN: usize = 8;

/// 生成一帧乱码：同一帧号 + 同一 salt 结果稳定（不保存随机状态）。
pub(super) fn matrix_glyphs(frame: u64, salt: &str, len: usize) -> String {
    let mut state = 0xcbf2_9ce4_8422_2325u64 ^ frame.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    for byte in salt.as_bytes() {
        state ^= u64::from(*byte);
        state = state.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let chars: Vec<char> = MATRIX_CHARS.chars().collect();
    let mut out = String::with_capacity(len);
    for _ in 0..len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.push(chars[(state % chars.len() as u64) as usize]);
    }
    out
}

/// 测试进行中的乱码标签：每帧换一组字符，并请求下一次重绘。
pub(super) fn matrix_label(ui: &mut egui::Ui, salt: &str) {
    let frame = (ui.ctx().input(|i| i.time) * 1000.0 / MATRIX_FRAME_MS as f64) as u64;
    ui.label(
        egui::RichText::new(matrix_glyphs(frame, salt, MATRIX_LEN))
            .monospace()
            .color(crate::theme::semantics(ui).ok),
    )
    .on_hover_text("延迟测试进行中");
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(MATRIX_FRAME_MS));
}

/// 模型行延迟显示：测试中显示乱码动画，完成后按阈值着色。
pub(super) fn model_latency_label(ui: &mut egui::Ui, state: Option<&LatencyState>, model_id: &str) {
    let Some(state) = state else {
        return;
    };
    match state.models.get(model_id) {
        Some(Ok(ms)) => {
            // 固定宽度右对齐：数字位数变化时按钮不会左右跳动。
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(72.0, ui.spacing().interact_size.y),
                egui::Sense::hover(),
            );
            ui.painter().text(
                rect.left_center(),
                egui::Align2::LEFT_CENTER,
                format!("{}ms", ms),
                egui::FontId::proportional(13.0),
                latency_color(*ms, crate::theme::semantics(ui)),
            );
            response.on_hover_text(format!(
                "最近一次首字延迟 {} ms（流式：请求发出 → 第一个字）",
                ms
            ));
        }
        Some(Err(err)) => {
            ui.label(egui::RichText::new(short_err(err)).color(crate::theme::semantics(ui).err))
                .on_hover_text(err);
        }
        None if state.pending.contains(model_id) => matrix_label(ui, model_id),
        None => {}
    }
}

/// 模型行上的单模型延迟测试按钮（位于拖动按钮右侧，结果标签就在它右侧）。
///
/// 返回 `true` 表示用户点了测试；调用方负责在 UI 循环外真正发起探测
/// （节流/串行的权威判定也在那里再做一次）。
pub(super) fn model_probe_button(
    ui: &mut egui::Ui,
    gate: &ProbeGate,
    provider_key: &str,
    now: f64,
    net_guard: Option<&str>,
) -> bool {
    let state = gate.state(provider_key, now, net_guard);
    // 状态一律写在按钮文案里（`测试中` / `测试(5s)`），不再挂悬停提示：
    // 「为什么点不了」的原因由别处可见文本承担（节流看倒计时、网络封锁在页头
    // 已经有一条红字说明），按钮自己不必再复述一遍。
    let label = match &state {
        ProbeGateState::Ready => "测试".to_string(),
        ProbeGateState::Busy => "测试中".to_string(),
        ProbeGateState::Cooling(left) => format!("测试({}s)", left.ceil() as u64),
        ProbeGateState::NetBlocked(_) => "测试".to_string(),
    };
    let enabled = matches!(state, ProbeGateState::Ready);
    ui.add_enabled(enabled, egui::Button::new(label)).clicked()
}

/// 线上协议（api）的调用形状：端点、鉴权与最小请求体各不相同。
/// 未列出的值按 OpenAI Chat Completions 兼容层处理，与 [`crate::convert::npm_to_api`] 的口径一致。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ApiWire {
    /// `openai-completions` / `mistral-conversations` / 未知值。
    ChatCompletions,
    /// `openai-responses` / `openai-codex-responses`。
    Responses,
    /// `azure-openai-responses`（鉴权头为 `api-key`）。
    AzureResponses,
    /// `anthropic-messages`。
    AnthropicMessages,
    /// `google-generative-ai`（密钥走 `?key=` 查询参数）。
    GoogleGenerativeAi,
    /// `google-vertex`（Bearer + `publishers/google` 路径）。
    GoogleVertex,
    /// `pi-messages`。
    PiMessages,
    /// 需要专有签名或私有网关，无法用最小请求测延迟。
    Unsupported,
}

/// 请求鉴权方式。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum AuthKind {
    /// `Authorization: Bearer <key>`。
    Bearer,
    /// `x-api-key` + `anthropic-version`。
    AnthropicKey,
    /// Azure OpenAI 的 `api-key`。
    AzureKey,
    /// 密钥已放进 URL 查询参数（Google 系），不再加鉴权头。
    QueryKey,
}

pub(super) fn api_wire(api: &str) -> ApiWire {
    match api.trim() {
        "anthropic-messages" => ApiWire::AnthropicMessages,
        "openai-responses" | "openai-codex-responses" => ApiWire::Responses,
        "azure-openai-responses" => ApiWire::AzureResponses,
        "google-generative-ai" => ApiWire::GoogleGenerativeAi,
        "google-vertex" => ApiWire::GoogleVertex,
        "pi-messages" => ApiWire::PiMessages,
        // 这两个需要专有签名 / 私有网关（SigV4、CloudCode），最小请求测不出真实可用性。
        "bedrock-converse-stream" | "google-gemini-cli" => ApiWire::Unsupported,
        _ => ApiWire::ChatCompletions,
    }
}

pub(super) fn auth_kind(wire: ApiWire) -> AuthKind {
    match wire {
        ApiWire::AnthropicMessages => AuthKind::AnthropicKey,
        ApiWire::AzureResponses => AuthKind::AzureKey,
        ApiWire::GoogleGenerativeAi | ApiWire::GoogleVertex => AuthKind::QueryKey,
        _ => AuthKind::Bearer,
    }
}

/// 该协议是否支持用最小请求测延迟 / 拉模型列表；不支持时给出原因。
pub(super) fn unsupported_reason(api: &str) -> Option<String> {
    match api_wire(api) {
        ApiWire::Unsupported => Some(format!(
            "协议 {} 需要专有鉴权（签名 / 私有网关），暂不支持自动测试",
            api.trim()
        )),
        _ => None,
    }
}

/// 按鉴权方式给请求加鉴权头；`QueryKey` 的密钥已在 URL 里。
pub(super) fn apply_auth(request: ureq::Request, auth: AuthKind, secret: &str) -> ureq::Request {
    match auth {
        AuthKind::Bearer => request.set("Authorization", &format!("Bearer {}", secret)),
        AuthKind::AnthropicKey => request
            .set("x-api-key", secret)
            .set("anthropic-version", "2023-06-01"),
        AuthKind::AzureKey => request.set("api-key", secret),
        AuthKind::QueryKey => request,
    }
}

/// Google 系把密钥放查询参数；其余协议原样返回。
pub(super) fn with_query_key(url: &str, auth: AuthKind, secret: &str) -> String {
    if auth == AuthKind::QueryKey && !secret.is_empty() {
        // URL 里可能已经带了查询参数（Google 流式端点的 `?alt=sse`），要用 `&` 接着拼
        let separator = if url.contains('?') { '&' } else { '?' };
        format!("{}{}key={}", url, separator, secret)
    } else {
        url.to_string()
    }
}

/// 探测请求的 User-Agent：按协议伪装成主流客户端。
///
/// 中转站普遍只放行白名单客户端：实测同一站点同一 key，`ureq/2.12.1`、不传 UA、
/// `pi/0.1.0` 一律返回 `401 unauthorized client detected`（有的站点直接卡住到超时），
/// 而 `claude-cli/*` 与 `opencode/*` 正常返回 200。版本号不被校验（`claude-cli/9.9.9`
/// 同样放行），但写成真实存在的版本更自然（版本号取自本机安装的客户端）。
pub(super) fn probe_user_agent(wire: ApiWire) -> &'static str {
    match wire {
        // Anthropic 系中转站本来就是给 Claude Code 用的
        ApiWire::AnthropicMessages | ApiWire::PiMessages => "claude-cli/1.18.30 (external, cli)",
        // OpenAI 兼容 / Responses / Google 系用 opencode 的身份
        _ => "opencode/1.18.30",
    }
}

/// 测量 provider 模型列表接口的往返延迟（毫秒）。
pub(super) fn measure_provider_latency(url: &str, secret: &str, api: &str) -> Result<u64, String> {
    if let Some(reason) = unsupported_reason(api) {
        return Err(reason);
    }
    if url.is_empty() {
        return Err("缺少 baseURL".to_string());
    }
    if secret.is_empty() {
        return Err("缺少 API Key".to_string());
    }
    let wire = api_wire(api);
    let auth = auth_kind(wire);
    let target = with_query_key(url, auth, secret);
    let agent = latency_agent();
    let request = apply_auth(
        agent
            .get(&target)
            .set("User-Agent", probe_user_agent(wire))
            .set("Accept", "application/json"),
        auth,
        secret,
    );
    let started = std::time::Instant::now();
    let result = request.call();
    let elapsed = started.elapsed().as_millis() as u64;
    if elapsed >= LATENCY_TIMEOUT_MS {
        return Err(format!("超时（{} ms）", elapsed));
    }
    match result {
        Ok(_) => Ok(elapsed),
        Err(err) => Err(http_error(err, elapsed)),
    }
}

/// 最小对话请求的地址：按协议决定路径（Google 系需要模型名参与路径）。
/// Google 系的推理动作：非流式 `:generateContent`，流式 `:streamGenerateContent?alt=sse`。
///
/// Google 的流式靠**换端点**区分，而不是请求体里的 `stream` 字段。
pub(super) fn google_action(stream: bool) -> &'static str {
    if stream {
        ":streamGenerateContent?alt=sse"
    } else {
        ":generateContent"
    }
}

pub(super) fn chat_url(base_url: &str, api: &str, model: &str, stream: bool) -> String {
    let base = base_url.trim().trim_end_matches('/');
    let model = model.trim();
    match api_wire(api) {
        ApiWire::AnthropicMessages => {
            if base.ends_with("/v1") {
                format!("{}/messages", base)
            } else {
                format!("{}/v1/messages", base)
            }
        }
        ApiWire::Responses | ApiWire::AzureResponses => format!("{}/responses", base),
        ApiWire::GoogleGenerativeAi => {
            format!("{}/models/{}{}", base, model, google_action(stream))
        }
        ApiWire::GoogleVertex => format!(
            "{}/publishers/google/models/{}{}",
            base,
            model,
            google_action(stream)
        ),
        ApiWire::PiMessages => format!("{}/messages", base),
        ApiWire::ChatCompletions | ApiWire::Unsupported => format!("{}/chat/completions", base),
    }
}

/// 单次探测的请求体：内容是题库里的中性短问句。
///
/// - **不校验答案**：目的只是让请求看起来像正常对话（规避中转站测活特征），
///   判定只看 HTTP 是否成功与往返耗时。
/// - token 上限给到 16：太小会让推理模型返回空内容甚至直接报错。
/// - 不设 `temperature`：部分推理模型只接受默认值，设了反而报错。
pub(super) fn minimal_body(wire: ApiWire, model: &str, question: &str) -> Value {
    match wire {
        ApiWire::Responses | ApiWire::AzureResponses => serde_json::json!({
            "model": model,
            "max_output_tokens": 16,
            "input": question,
            "stream": true
        }),
        ApiWire::GoogleGenerativeAi | ApiWire::GoogleVertex => serde_json::json!({
            "contents": [{ "role": "user", "parts": [{ "text": question }] }],
            "generationConfig": { "maxOutputTokens": 16 }
        }),
        ApiWire::AnthropicMessages | ApiWire::PiMessages => serde_json::json!({
            "model": model,
            "max_tokens": 16,
            "stream": true,
            "messages": [{ "role": "user", "content": question }]
        }),
        ApiWire::ChatCompletions | ApiWire::Unsupported => serde_json::json!({
            "model": model,
            "max_tokens": 16,
            "stream": true,
            "messages": [{ "role": "user", "content": question }]
        }),
    }
}

/// 判断一个 SSE 载荷是不是「第一个字」（即真的开始出内容了）。
///
/// 只看内容类字段，不看 role / usage / 各种元事件：中转站常常在模型真正开始生成前
/// 先推一个 role 块或心跳块，把它当首字，测出来的就不是用户体感的「首字延迟」。
pub(super) fn chunk_has_content(wire: ApiWire, chunk: &Value) -> bool {
    match wire {
        ApiWire::ChatCompletions | ApiWire::Unsupported => {
            let delta = &chunk["choices"][0]["delta"];
            // 推理模型先出 reasoning_content（思维链）也算已经出字
            non_empty_text(&delta["content"]) || non_empty_text(&delta["reasoning_content"])
        }
        ApiWire::Responses | ApiWire::AzureResponses => {
            // 形如 {"type":"response.output_text.delta","delta":"你"}
            chunk["type"]
                .as_str()
                .is_some_and(|kind| kind.ends_with(".delta"))
                && non_empty_text(&chunk["delta"])
        }
        ApiWire::AnthropicMessages | ApiWire::PiMessages => {
            // 形如 {"type":"content_block_delta","delta":{"text":"你"}}
            let delta = &chunk["delta"];
            non_empty_text(&delta["text"]) || non_empty_text(&delta["thinking"])
        }
        ApiWire::GoogleGenerativeAi | ApiWire::GoogleVertex => chunk["candidates"][0]["content"]
            ["parts"]
            .as_array()
            .is_some_and(|parts| parts.iter().any(|part| non_empty_text(&part["text"]))),
    }
}

/// 字段是否含有非空文本（兼容「字符串 / 内容块数组 / 嵌套对象」三种形态）。
pub(super) fn non_empty_text(value: &Value) -> bool {
    match value {
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => items.iter().any(non_empty_text),
        Value::Object(fields) => fields.values().any(non_empty_text),
        _ => false,
    }
}

/// 流结束标记：OpenAI 兼容的 `[DONE]`、Anthropic 的 `message_stop`、Responses 的 `response.completed`。
///
/// Anthropic 会先发一行 `event: message_stop` 再发 `data: {"type":"message_stop"}`，
/// 两种形态都要认（裸标记行没有引号，所以按子串匹配）。
/// 站点发完标记后未必立刻关连接，靠它提前收尾，避免一直读到读超时才结束。
pub(super) fn is_stream_end(line: &str) -> bool {
    line.contains("[DONE]")
        || line.contains("message_stop")
        || line.contains("response.completed")
        || line.contains("response.incomplete")
        || line.contains("response.failed")
}

/// 读完流式响应，返回**首字延迟**（毫秒，从请求发出算起）。
///
/// - 边读边解析 SSE 的 `data:` 载荷，碰到第一个带内容的块立刻记下耗时；
/// - 记下之后**继续把流读完**（`[DONE]` / `message_stop` / EOF / 64 KB 上限）再关闭连接：
///   真实客户端不会拿到流就断，匆匆断开在中转站日志里反而像探测流量；
/// - 全程没有数据返回时返回 `None`（调用方按超时 / 协议不支持流式处理）；
/// - 有数据但认不出内容块（形态罕见）时退回「第一个 `data:` 包到达的时刻」。
pub(super) fn read_stream_ttft(
    reader: impl std::io::Read,
    wire: ApiWire,
    started: std::time::Instant,
) -> Option<u64> {
    use std::io::BufRead;
    /// 读取上限：足够装下 16 token 的流式响应，又能顶住发完不关连接的站点。
    const MAX_STREAM_BYTES: usize = 64 * 1024;
    let mut buffer = std::io::BufReader::new(reader);
    let mut line = String::new();
    let mut read_bytes = 0usize;
    let mut first_data: Option<u64> = None;
    let mut ttft: Option<u64> = None;
    loop {
        line.clear();
        match buffer.read_line(&mut line) {
            Ok(0) => break,
            Ok(read) => read_bytes += read,
            // 读超时 / 连接中断：保留已经测到的首字
            Err(_) => break,
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if is_stream_end(trimmed) {
            break;
        }
        if let Some(payload) = trimmed.strip_prefix("data:") {
            if let Ok(chunk) = serde_json::from_str::<Value>(payload.trim()) {
                if first_data.is_none() {
                    first_data = Some(started.elapsed().as_millis() as u64);
                }
                if ttft.is_none() && chunk_has_content(wire, &chunk) {
                    ttft = Some(started.elapsed().as_millis() as u64);
                }
            }
        }
        if read_bytes >= MAX_STREAM_BYTES {
            break;
        }
    }
    ttft.or(first_data)
}

/// 对单个模型发一个**流式**探测请求，测量**首字延迟**（毫秒）。
///
/// 走流式是为了不像脚本测活：主流客户端（Claude Code / opencode / pi 等）默认全部流式，
/// 同步请求在中转站日志里会显示成「类型：同步」，反而是少数派特征。
/// 解响应体只是为了找第一个内容块（测首字），**不校验答案**。
/// 端点、鉴权与请求体都按所选协议构造；失败仍会报出耗时，便于判断服务是否可达。
pub(super) fn measure_model_latency(
    base_url: &str,
    secret: &str,
    api: &str,
    model: &str,
    question: &str,
) -> Result<u64, String> {
    if let Some(reason) = unsupported_reason(api) {
        return Err(reason);
    }
    if base_url.trim().is_empty() {
        return Err("缺少 baseURL".to_string());
    }
    if secret.is_empty() {
        return Err("缺少 API Key".to_string());
    }
    let wire = api_wire(api);
    let auth = auth_kind(wire);
    let url = with_query_key(&chat_url(base_url, api, model, true), auth, secret);
    let body = minimal_body(wire, model, question).to_string();
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(5))
        .timeout_read(std::time::Duration::from_millis(LATENCY_TIMEOUT_MS))
        .build();
    let started = std::time::Instant::now();
    // Accept 与主流 SDK 的流式口径一致；UA 伪装成白名单客户端（见 probe_user_agent）。
    let result = apply_auth(
        agent
            .post(&url)
            .set("User-Agent", probe_user_agent(wire))
            .set("Accept", "text/event-stream")
            .set("Content-Type", "application/json"),
        auth,
        secret,
    )
    .send_string(&body);
    let response = match result {
        Ok(response) => response,
        Err(err) => return Err(http_error(err, started.elapsed().as_millis() as u64)),
    };
    match read_stream_ttft(response.into_reader(), wire, started) {
        Some(ms) if ms < LATENCY_TIMEOUT_MS => Ok(ms),
        Some(ms) => Err(format!("超时（{} ms）", ms)),
        None => {
            let waited = started.elapsed().as_millis() as u64;
            Err(if waited >= LATENCY_TIMEOUT_MS {
                format!("超时（{} ms）", waited)
            } else {
                "流式响应没有数据".to_string()
            })
        }
    }
}

/// 调用 OpenAI 兼容 /models 接口获取模型 id 列表（后台线程内执行）。
pub(super) fn fetch_models_remote(
    url: &str,
    secret: &str,
    api: &str,
) -> Result<Vec<String>, String> {
    if let Some(reason) = unsupported_reason(api) {
        return Err(reason);
    }
    if url.is_empty() {
        return Err("缺少 baseURL，无法获取模型".to_string());
    }
    if secret.is_empty() {
        return Err("缺少 API Key，无法获取模型".to_string());
    }
    let wire = api_wire(api);
    let auth = auth_kind(wire);
    let target = with_query_key(url, auth, secret);
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout_read(std::time::Duration::from_secs(30))
        .build();
    let request = apply_auth(
        agent
            .get(&target)
            .set("User-Agent", probe_user_agent(wire))
            .set("Accept", "application/json"),
        auth,
        secret,
    );
    let response = request.call().map_err(|err| match err {
        ureq::Error::Status(code, resp) => {
            // 「HTTP 404 接口或模型不存在：Not Found」+ 换行给出处理建议。
            let mut msg = crate::http_status::label(code);
            let text = resp.status_text().trim();
            if !text.is_empty() {
                msg.push('：');
                msg.push_str(text);
            }
            if let Some(hint) = crate::http_status::hint(code) {
                msg.push('\n');
                msg.push_str(hint);
            }
            msg
        }
        ureq::Error::Transport(transport) => format!(
            "网络错误：{}",
            sanitize_network_error(&transport.to_string())
        ),
    })?;
    let text = response.into_string().map_err(|err| err.to_string())?;
    parse_models_response(&text)
}

/// 解析 /models 响应中的模型 id（兼容 OpenAI/Anthropic/Gemini 等格式）。
pub(super) fn parse_models_response(text: &str) -> Result<Vec<String>, String> {
    let root: Value = serde_json::from_str(text).map_err(|err| {
        let snippet = text.chars().take(160).collect::<String>();
        format!("响应不是合法 JSON（{}）：{}", err, snippet)
    })?;
    if let Some(error) = root.get("error") {
        let msg = error
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| error.as_str())
            .unwrap_or("未知错误");
        return Err(msg.to_string());
    }
    fn push_ids(item: &Value, seen: &mut HashSet<String>, ids: &mut Vec<String>) {
        let raw = item
            .get("id")
            .and_then(Value::as_str)
            .or_else(|| item.get("name").and_then(Value::as_str))
            .unwrap_or("");
        let id = raw.trim().trim_start_matches("models/").to_string();
        if !id.is_empty() && seen.insert(id.clone()) {
            ids.push(id);
        }
    }
    let mut ids: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    if let Some(arr) = root.as_array() {
        for item in arr {
            push_ids(item, &mut seen, &mut ids);
        }
    }
    for key in ["data", "models"] {
        if let Some(arr) = root.get(key).and_then(Value::as_array) {
            for item in arr {
                push_ids(item, &mut seen, &mut ids);
            }
        }
    }
    Ok(ids)
}

impl App {
    /// 按 provider 的 api 类型构造模型列表接口地址。
    pub(super) fn models_url(base_url: &str, api: &str) -> String {
        let base = base_url.trim().trim_end_matches('/');
        if base.is_empty() {
            return String::new();
        }
        match api_wire(api) {
            // Anthropic 的模型接口固定为 /v1/models。
            ApiWire::AnthropicMessages => {
                if base.ends_with("/v1") {
                    format!("{}/models", base)
                } else {
                    format!("{}/v1/models", base)
                }
            }
            // Vertex 的模型列表挂在 publishers/google 下。
            ApiWire::GoogleVertex => format!("{}/publishers/google/models", base),
            _ => format!("{}/models", base),
        }
    }

    /// 启动后台线程获取 provider 模型列表。
    pub(super) fn start_model_fetch(&mut self, key: &str, base_url: &str, secret: &str, api: &str) {
        let url = Self::models_url(base_url, api);
        let secret = secret.trim().to_string();
        let api = api.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = fetch_models_remote(&url, &secret, &api);
            let _ = tx.send(result);
        });
        self.model_fetch.insert(
            key.to_string(),
            ModelFetchState {
                rx: Some(rx),
                result: None,
            },
        );
    }

    /// 启动 provider 级延迟测试（后台线程，结果经通道回传）。
    /// 接收 `&mut HashMap` 而非 `&mut self`，以便与 `providers[idx]` 借用共存。
    pub(super) fn start_provider_latency(
        latency: &mut HashMap<String, LatencyState>,
        key: &str,
        base_url: &str,
        secret: &str,
        api: &str,
    ) {
        let url = Self::models_url(base_url, api);
        let secret = secret.trim().to_string();
        let api = api.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(measure_provider_latency(&url, &secret, &api));
        });
        let state = latency.entry(key.to_string()).or_default();
        state.provider = None;
        state.provider_rx = Some(rx);
    }

    /// 单模型探测的统一入口：门控 → 选问句 → 启动后台线程 → 返回状态栏消息。
    ///
    /// 写成关联函数（不借 `&mut self`）是为了能在 provider 卡片内部调用：
    /// 那里 `providers` 字段已经被可变借用，只能按字段拆分借用。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn run_model_probe(
        probe: &mut ProbeGate,
        latency: &mut HashMap<String, LatencyState>,
        net_guard: Option<&str>,
        provider_key: &str,
        model_id: &str,
        now: f64,
        base_url: &str,
        secret: &str,
        api: &str,
    ) -> String {
        match probe.state(provider_key, now, net_guard) {
            ProbeGateState::Ready => {
                let question = probe.next_question(provider_key);
                probe.start(provider_key, now);
                Self::start_model_latency(
                    latency,
                    provider_key,
                    base_url,
                    secret,
                    api,
                    model_id,
                    question,
                );
                format!("正在测试 {} 的 {} 延迟…", provider_key, model_id)
            }
            ProbeGateState::Cooling(left) => {
                format!("节流中：{} 秒后可再测", left.ceil() as u64)
            }
            ProbeGateState::Busy => "上一个延迟测试尚未结束（一次只测一个模型）".to_string(),
            ProbeGateState::NetBlocked(reason) => {
                format!("{}：{}", crate::netguard::BLOCK_PREFIX, reason)
            }
        }
    }

    /// 启动单个模型的延迟探测（后台线程，结果经通道回传）。
    ///
    /// **一次只测一个**：中转站的测活风控对「批量扫模型」最敏感，因此不再提供批量
    /// 入口，节流与串行统一由 [`ProbeGate`] 把关。
    pub(super) fn start_model_latency(
        latency: &mut HashMap<String, LatencyState>,
        key: &str,
        base_url: &str,
        secret: &str,
        api: &str,
        model: &str,
        question: &str,
    ) {
        let base = base_url.to_string();
        let secret = secret.trim().to_string();
        let api = api.to_string();
        let model = model.trim().to_string();
        let question = question.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker_model = model.clone();
        std::thread::spawn(move || {
            let result = measure_model_latency(&base, &secret, &api, &worker_model, &question);
            let _ = tx.send((worker_model, result));
        });
        let state = latency.entry(key.to_string()).or_default();
        state.done = 0;
        state.total = 1;
        state.pending.clear();
        state.pending.insert(model);
        state.model_rx = Some(rx);
    }

    /// 每帧轮询延迟测试结果，并更新状态栏。
    pub(super) fn poll_latency(&mut self) {
        let mut notices: Vec<String> = Vec::new();
        for (key, state) in self.latency.iter_mut() {
            if let Some(rx) = &state.provider_rx {
                match rx.try_recv() {
                    Ok(result) => {
                        // 失败原因已就地显示在厂商行（红字 + 悬停详情），
                        // 不再重复推送到页面底部状态栏。
                        if let Ok(ms) = &result {
                            notices.push(format!("provider 延迟测试完成：{} ms", ms));
                        }
                        state.provider = Some(result);
                        state.provider_rx = None;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        // 测量线程异常退出（panic 等）时收不到结果：
                        // 终止等待，避免 Spinner/进度永久卡死；不在底部报错。
                        state.provider_rx = None;
                    }
                }
            }
            if let Some(rx) = &state.model_rx {
                loop {
                    match rx.try_recv() {
                        Ok((id, result)) => {
                            state.pending.remove(&id);
                            state.models.insert(id, result);
                            state.done += 1;
                            // 单模型探测：拿到结果即释放该 provider 的串行位。
                            self.probe.finish(key);
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            state.model_rx = None;
                            state.total = state.done;
                            // 单模型探测：total 恒为 1，只区分「已回传结果」与「线程异常退出」。
                            let unfinished = !state.pending.is_empty();
                            state.pending.clear();
                            notices.push(if unfinished {
                                "模型延迟测试中断（线程异常退出）".to_string()
                            } else {
                                "模型延迟测试完成".to_string()
                            });
                            break;
                        }
                    }
                }
            }
        }
        for msg in notices {
            self.status = msg;
        }
    }

    /// 每帧轮询后台线程的模型获取结果，并更新状态栏。
    pub(super) fn poll_model_fetch(&mut self) {
        let mut finished: Vec<String> = Vec::new();
        for (key, state) in self.model_fetch.iter_mut() {
            if let Some(rx) = &state.rx {
                if let Ok(result) = rx.try_recv() {
                    state.result = Some(result);
                    state.rx = None;
                    finished.push(key.clone());
                }
            }
        }
        for key in finished {
            let msg = match &self.model_fetch[&key].result {
                Some(Ok(models)) if models.is_empty() => {
                    format!("接口未返回任何模型（{}）", key)
                }
                Some(Ok(models)) => format!("已获取 {} 个模型（{}）", models.len(), key),
                Some(Err(err)) => format!("获取模型失败（{}）: {}", key, err),
                _ => continue,
            };
            self.status = msg;
        }
    }

    /// 后台拉取某个后端的内置网关免费模型列表（已有请求在飞时不重复发起）。
    ///
    /// 与 provider 的「获取模型」共用同一套「后台线程 + 通道」形状，但请求的是
    /// 公共模型库而非某个用户 provider，因此不需要 baseURL / API Key。
    /// 没有免费层的后端（mimocode）直接忽略。
    pub(super) fn start_free_models_fetch(&mut self, format: crate::format::ConfigFormat) {
        if crate::opencode_models::source_for(format).is_none() {
            return;
        }
        let state = self.free_models.entry(format).or_default();
        if state.fetching() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::opencode_models::fetch_remote(format));
        });
        state.rx = Some(rx);
    }

    /// 每帧轮询各后端的免费模型拉取结果：成功则刷新列表并落盘缓存。
    ///
    /// 拉取失败**不清空**已有列表：宁可继续用旧缓存，也不要因为一次网络抖动
    /// 让下拉变空（用户会以为配置坏了）。失败原因只在 Agents 区块旁提示。
    pub(super) fn poll_free_models(&mut self) {
        for (format, state) in self.free_models.iter_mut() {
            let Some(rx) = &state.rx else {
                continue;
            };
            match rx.try_recv() {
                Ok(Ok(models)) => {
                    state.rx = None;
                    state.error = None;
                    if !models.is_empty() {
                        // 缓存写失败不影响本次使用（下次启动重取即可）。
                        let _ = crate::opencode_models::save_cache(*format, &models);
                        state.models = models;
                    }
                }
                Ok(Err(err)) => {
                    state.rx = None;
                    state.error = Some(err);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                // 线程异常退出：终止等待，避免按钮永远显示「刷新中」。
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    state.rx = None;
                }
            }
        }
    }
}
