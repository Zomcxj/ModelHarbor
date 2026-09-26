//! KimiCode 后端：`~/.kimi-code/config.toml`（TOML）。
//!
//! 本项目的**第一个 TOML 后端**。中间表示仍是 `serde_json::Value`，
//! 由 `toml` crate 做 `from_str::<Value>` / `to_string(&Value)`，与既有架构同构。
//!
//! ## 结构：两张**分开的**表，靠 `provider` 字段 join
//!
//! ```toml
//! default_model = "sensenova/sensenova-6.8-flash-lite"   # 顶层标量
//! default_permission_mode = "auto"
//!
//! [providers."managed:kimi-code"]        # 键 = provider 名（含 `:` 必须加引号）
//! base_url = "https://api.kimi.com/coding/v1"
//! type = "kimi"                          # 必填
//! api_key = ""                           # 空串 = 未设置（见下）
//!
//! [providers."managed:kimi-code".oauth]  # /login 注入，只读保留
//! storage = "file"
//! key = "oauth/kimi-code"
//!
//! [models."kimi-code/k3"]                # **顶层独立表**！键 = alias
//! provider = "managed:kimi-code"         # join key → [providers.*]，必填
//! model = "k3"                           # 上游 wire id，必填
//! max_context_size = 1048576             # 必填，≥1
//! capabilities = [ "thinking", "tool_use" ]
//! display_name = "K3"
//! support_efforts = [ "low", "high", "max" ]
//! default_effort = "high"
//!
//! [thinking]
//! enabled = true
//! ```
//!
//! 这是本项目**从未遇到过**的结构：模型不在 provider 下，而在顶层全局表里，
//! 靠 `provider = "<providers 键>"` 关联。所以 [`parse`] 要 join，[`serialize_root`]
//! 要拆回两张表。
//!
//! ## 四个决定建模方式的要点
//!
//! 1. **表键是 alias，`model` 是 wire id，两者可以不同。** 本机的
//!    `[models."kimi-code/k3"]` 里 `model = "k3"`。界面显示 wire id（[`ModelRow::id`]，
//!    它会被发给上游，改名即 model-not-found），alias 另存
//!    [`ModelRow::kimi_alias`]，写回时作表键。
//!
//! 2. **`api_key` / `api_key_env` / `oauth` 三者互斥**，同时写会让 Kimi Code
//!    **启动失败**（源码 `declaredProviderCredential`：任意两者同时非空即返回
//!    `kind: "conflict"`）。这是本后端最严重的一条约束，见 [`credential_conflict`]。
//!    注意判据是 **非空字符串**：`api_key = ""` 视同未设置，所以本机文件里
//!    `managed:kimi-code` 的 `api_key = ""` 与 `oauth` 并存是合法的。
//!
//! 3. **`managed:*` provider 来自 OAuth 登录，只读**。官方明确这类账号不出现在
//!    `/provider` 里，只能用 `/login`、`/logout` 管理。误改会破坏登录态
//!    （`credentials/` 里的凭据与这份声明是配对的），所以界面不给删除、不给改字段。
//!
//! 4. **`capabilities` 只增不减**（官方："only ever added, never removed"）。保存时
//!    不因为界面没勾就删掉已有标签——那会静默降级能力。见 [`merge_capabilities`]。
//!
//! ## 启用/停用（照搬 WorkBuddy 的两份配置）
//!
//! Kimi 的模型表里**没有** `disabled` 字段：停用的语义就是**整条不写进 `config.toml`**。
//! 界面要显示全部配置，这两件事不可能由同一个文件承担，于是拆成两份：
//!
//! - `~/.kimi-code/config.toml` —— Kimi Code 真正读的生效清单：`[models.*]` 只含启用条目。
//! - 同目录 `models.full.toml` —— 本工具维护的全量副本：所有条目 + 每条自己的勾选标记
//!   （`disabled`，**每条必写**，含 `false`）。为什么 `false` 也必须写，见
//!   `backends::workbuddy` 的模块说明——同一条教训。
//!
//! 副本名不能叫 `models.json`：Kimi 的伴生文件里有 `mcp.json`，且 `credentials/` 是敏感
//! 目录。`models.full.toml` 放在 `~/.kimi-code/` 根下与 `config.toml` 同级，Kimi Code
//! 只按精确文件名读自己的文件，不会误读它。
//!
//! ## 两条「认不出来就原样保留」的规则
//!
//! 界面只重建自己认得的条目，所以有两类内容必须显式带过去，否则一次保存就没了：
//!
//! - **`managed:*` provider 与其模型**：界面不显示（只读），只能整块原样保留。
//! - **孤儿模型**（`provider` 指向的键在 `[providers.*]` 里不存在）：无处归属，
//!   界面不显示，但**不能删**——那可能是用户先写了模型、后补 provider 的中间态。
//!
//! 反过来，**认得出来**的条目一律以界面状态为准——包括用户把卡片删掉的情况。所以
//! 「保留」只针对认不出来的内容，不会让删掉的条目复活。
//!
//! ## 已知代价
//!
//! TOML 全量序列化会**丢注释**（与 opencode 系「保存后丢 JSONC 注释」同性质，
//! 本项目的既有口径就是这样）。另外 Kimi Code 桌面端与本文件**共享**：本工具改它，
//! 会同时影响 CLI 与桌面端——这是功能，但页面上要说明。

use super::{Backend, BackendLoad};
use crate::format::ConfigFormat;
use crate::model::{AgentRow, ModelRow, ProviderRow};
use crate::util::{home_dir_string, read_config_content, wsl_home};
use serde_json::{Map, Value};

pub struct KimiCodeBackend;

pub static BACKEND: KimiCodeBackend = KimiCodeBackend;

/// 全量副本的文件名（与 `config.toml` 同目录）。
const FULL_STORE_NAME: &str = "models.full.toml";

/// OAuth 登录写入的 provider 前缀（源码 `KIMI_CODE_PROVIDER_NAME = "managed:kimi-code"`）。
const MANAGED_PREFIX: &str = "managed:";

/// 本工具认得的全部 capability 标签（源码 `UNKNOWN_CAPABILITY_MARKER` 的键集，
/// 外加 `always_thinking`——它由 `withAnthropicProfile` 加入，本机文件里就有）。
///
/// 只用于**读**时推导界面控件（思考 / 工具调用 / 输入模态）；写回时一律走
/// [`merge_capabilities`] 的「只增不减」，不按这份清单去删。
const CAP_THINKING: &str = "thinking";
const CAP_ALWAYS_THINKING: &str = "always_thinking";
const CAP_TOOL_USE: &str = "tool_use";
const CAP_IMAGE_IN: &str = "image_in";
const CAP_VIDEO_IN: &str = "video_in";
const CAP_AUDIO_IN: &str = "audio_in";

fn default_local_path() -> String {
    format!("{}\\.kimi-code\\config.toml", home_dir_string())
}

/// 全量副本的路径：与主配置同目录、固定文件名。
pub fn full_store_path(config_path: &str) -> String {
    crate::util::sibling_path(config_path, FULL_STORE_NAME)
}

/// 某个 provider 是否由 OAuth 登录维护（只读）。
///
/// 公开是因为 `app::strip_cross_format_containers` 也要用：跨格式保存时界面接管
/// `[providers.*]`，但 `managed:*` 不归界面管，必须原样留着。
pub fn is_managed_provider(name: &str) -> bool {
    name.starts_with(MANAGED_PREFIX)
}

/// 解析 TOML 文本为 root（供保存流程做「会不会变小」的判断）。
///
/// 单独暴露是因为那条判断在 `app::save` 里，而 TOML 不能用 `util::parse_config_content`
/// （那是 JSONC 解析器）。解析失败返回 `None`，调用方按「会收缩」处理（保守）。
pub fn parse_root_for_shrink(content: &str) -> Option<Value> {
    parse_toml(content).ok()
}

/// 这次保存是否会让**模型表变小**（停用条目不再写出）。
///
/// 用途与 `backends::workbuddy` / `qwen_code` 的同名函数一样：一次普通保存可能删掉
/// 比跨格式转换还多的内容，所以保存前必须先滚动备份。判据是模型表条目数变少——
/// 孤儿与 `managed:` 模型都会被原样保留，所以它们不会造成误判。
pub fn shrinks_on_save(before: &Value, after: &Value) -> bool {
    let count = |root: &Value| models_map(root).map(Map::len).unwrap_or(0);
    count(before) > count(after)
}

// ---------- TOML 读写 ----------

/// TOML 文本 → Value。空文本视为空对象。
///
/// 不用 `util::parse_config_content`：那是 JSONC 解析器，TOML 进不去。
fn parse_toml(content: &str) -> Result<Value, String> {
    if content.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    toml::from_str(content).map_err(|e| format!("解析失败: {}", e))
}

/// Value → TOML 文本。
///
/// **必须先剔除 `null`**：TOML 没有 null 类型，`toml` 遇到 `Value::Null` 会直接报
/// `unsupported unit type`，整次保存失败。JSONC 后端里 `null` 是合法值（serde_json
/// 也照写），跨格式从那些页面转过来时完全可能出现，所以在写之前统一清掉——
/// 「键不存在」与「键为 null」对 Kimi 是同一件事，删掉没有语义损失。
fn to_toml_string(root: &Value) -> Result<String, String> {
    let cleaned = strip_nulls(root);
    toml::to_string(&cleaned).map_err(|e| format!("序列化失败: {}", e))
}

/// 递归删掉所有 `null` 值（对象里删键，数组里删元素）。
fn strip_nulls(value: &Value) -> Value {
    match value {
        Value::Null => Value::Null,
        Value::Array(items) => Value::Array(
            items
                .iter()
                .filter(|v| !v.is_null())
                .map(strip_nulls)
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k.clone(), strip_nulls(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

// ---------- 结构访问 ----------

fn providers_map(root: &Value) -> Option<&Map<String, Value>> {
    root.get("providers").and_then(Value::as_object)
}

fn models_map(root: &Value) -> Option<&Map<String, Value>> {
    root.get("models").and_then(Value::as_object)
}

/// provider 条目的 `type`（Kimi 必填字段）。
fn provider_type(entry: &Value) -> &str {
    entry
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
}

/// 非空字符串：`Some` 表示这个键**确实被设置了**。
///
/// 与 Kimi 源码的 `nonEmptyString` 同义：空串视同未设置。这是 XOR 判据的核心——
/// `api_key = ""` 与 `oauth` 并存是合法的（本机文件就是这样）。
fn non_empty_str(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// 三选一的凭据冲突描述；`None` = 无冲突。
///
/// 判据与源码 `declaredProviderCredential` 逐条对齐（顺序也一致，报错信息才会一样）。
/// 返回的是**人话**，直接进状态栏 / 保存错误。
fn credential_conflict(name: &str, entry: &Value) -> Option<String> {
    let inline = non_empty_str(entry.get("api_key")).is_some();
    let env = non_empty_str(entry.get("api_key_env")).is_some();
    let oauth = entry.get("oauth").is_some();
    let pair = match (inline, env, oauth) {
        (true, true, _) => Some(("api_key", "api_key_env")),
        (_, true, true) => Some(("api_key_env", "oauth")),
        (true, _, true) => Some(("api_key", "oauth")),
        _ => None,
    };
    pair.map(|(first, second)| {
        format!(
            "Provider \"{name}\" has both {first} and {second} set in config.toml - \
             they are mutually exclusive. Remove one."
        )
    })
}

/// 全部 provider 里的凭据冲突（保存前挡下来）。
///
/// 为什么要挡：同时写 `api_key` 与 `api_key_env` 会让 Kimi Code **启动失败**
/// （源码把这种情况判成 `kind: "conflict"` 并拒绝）。那比「配置不生效」严重得多——
/// 用户会以为是自己把 Kimi Code 弄坏了。宁可不让保存，也不能写出一个启动不了的文件。
fn first_credential_conflict(providers: &[ProviderRow]) -> Option<String> {
    providers
        .iter()
        .filter(|p| !p.key.trim().is_empty())
        .find_map(|p| credential_conflict(p.key.trim(), &provider_entry_view(p)))
}

/// [`ProviderRow`] → 它将要写出的 provider 条目形状（仅用于冲突体检）。
///
/// 不能直接用 `p.raw`：那可能是从别的后端转过来的形状，键名完全不同。这里只按
/// **Kimi 的语义**还原出三个凭据键的存在性。
fn provider_entry_view(p: &ProviderRow) -> Value {
    let mut obj = Map::new();
    if !p.api_key.trim().is_empty() {
        obj.insert("api_key".into(), Value::String(p.api_key.clone()));
    }
    if !p.api_key_env.trim().is_empty() {
        obj.insert("api_key_env".into(), Value::String(p.api_key_env.clone()));
    }
    // `oauth` 不由界面建模（只读保留），它只可能来自 raw。
    if p.raw.get("oauth").is_some() {
        obj.insert("oauth".into(), p.raw["oauth"].clone());
    }
    Value::Object(obj)
}

/// 模型条目的 capability 标签集。
fn capabilities_of(entry: &Value) -> Vec<String> {
    entry
        .get("capabilities")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// capability 集合 → 输入模态串（界面口径）。
///
/// **未声明时返回空串**（不返回 `"text"`）：那会把「未声明」变成「只支持文本」，
/// 跨格式写出时给别的后端凭空补字段。与 WorkBuddy / QwenCode 同一口径。
fn modalities_from_caps(caps: &[String]) -> String {
    let has = |name: &str| caps.iter().any(|c| c.eq_ignore_ascii_case(name));
    if caps.is_empty() {
        return String::new();
    }
    let image = has(CAP_IMAGE_IN) || has(CAP_VIDEO_IN) || has(CAP_AUDIO_IN);
    crate::convert::supports_to_modalities([("text", true), ("image", image)])
}

/// 界面模态串 → 要**增加**的 capability 标签。
///
/// 只产出「该加什么」，不产出「该删什么」——capabilities 只增不减（见
/// [`merge_capabilities`]）。
fn caps_for_modalities(text: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    if text
        .split(',')
        .any(|s| s.trim().eq_ignore_ascii_case("image"))
    {
        out.push(CAP_IMAGE_IN);
    }
    out
}

/// 已有 capabilities + 界面状态 → 新的 capabilities（**只增不减**）。
///
/// 官方明确 capabilities "only ever added, never removed"，所以这里只在界面**勾上**时
/// 追加标签，绝不因为没勾就删。删标签会让 Kimi 静默降级能力（例如丢掉 `tool_use`
/// 后模型不再能调工具），而多一个标签是无害的——官方自己的迁移逻辑也只做并集。
///
/// 唯一的例外是 `thinking`：它与 `always_thinking` 是界面同一个「支持思考」开关的
/// 两种表达，取消勾选时一并移除，否则开关关不掉。但 `always_thinking` 若来自
/// anthropic profile 的自动注入，移除后 Kimi 下次读取会按模型名重新注入——
/// 那是它自己的行为，不是本工具在猜。
fn merge_capabilities(old: &[String], m: &ModelRow) -> Vec<String> {
    let mut out: Vec<String> = old.to_vec();
    let mut push = |name: &str| {
        if !out.iter().any(|c| c.eq_ignore_ascii_case(name)) {
            out.push(name.to_string());
        }
    };
    if m.reasoning {
        push(CAP_THINKING);
    }
    if m.tool_call {
        push(CAP_TOOL_USE);
    }
    for name in caps_for_modalities(&m.modalities_input) {
        push(name);
    }
    if !m.reasoning {
        out.retain(|c| {
            !c.eq_ignore_ascii_case(CAP_THINKING) && !c.eq_ignore_ascii_case(CAP_ALWAYS_THINKING)
        });
    }
    out
}

/// 逗号串 → 列表（去空白、去空项）。
fn split_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

// ---------- 解析：两张表 join ----------

/// 一个 provider 条目 + 挂在它名下的模型条目（含 alias）。
struct Joined {
    name: String,
    provider: Value,
    models: Vec<(String, Value)>,
}

/// 按 `provider` 字段把顶层 `[models.*]` 挂到 `[providers.*]` 上。
///
/// 孤儿模型（`provider` 缺失、指向不存在的键、或指向 `managed:*`）单独返回，由调用方
/// 决定是保留还是显示。**不 panic、不丢弃**——丢掉就是静默删用户配置。
fn join(root: &Value) -> (Vec<Joined>, Vec<(String, Value)>) {
    let mut joined: Vec<Joined> = Vec::new();
    let empty = Map::new();
    let providers = providers_map(root).unwrap_or(&empty);
    for (name, provider) in providers {
        joined.push(Joined {
            name: name.clone(),
            provider: provider.clone(),
            models: Vec::new(),
        });
    }
    let mut orphans: Vec<(String, Value)> = Vec::new();
    for (alias, model) in models_map(root).unwrap_or(&empty) {
        let owner = model.get("provider").and_then(Value::as_str).unwrap_or("");
        match joined.iter_mut().find(|j| j.name == owner) {
            Some(entry) => entry.models.push((alias.clone(), model.clone())),
            // `managed:` 的模型属于只读 provider，整块保留；其余孤儿也保留。
            None => orphans.push((alias.clone(), model.clone())),
        }
    }
    (joined, orphans)
}

/// provider 条目 → [`ProviderRow`]（一个 provider 一张卡片，模型挂其下）。
fn provider_from_entry(name: &str, provider: &Value, models: &[(String, Value)]) -> ProviderRow {
    let mut row = ProviderRow::new();
    row.key = name.to_string();
    row.base_url = provider
        .get("base_url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    row.api_key = provider
        .get("api_key")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    row.api_key_env = provider
        .get("api_key_env")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    row.original_api_key_env = row.api_key_env.clone();
    // `type` 逐字进 pi_api：它既是界面下拉的当前值，也是写回时的原值。
    // 空 type 留给 `effective_api()` 回落，不在这里编造一个。
    row.pi_api = provider_type(provider).to_string();
    row.models = models
        .iter()
        .map(|(alias, m)| model_from_entry(alias, m))
        .collect();
    row.source_format = Some(ConfigFormat::KimiCode);
    row.raw = provider.clone();
    row
}

/// 模型条目 → [`ModelRow`]。`alias` 是表键，`model` 是 wire id。
fn model_from_entry(alias: &str, entry: &Value) -> ModelRow {
    let mut m = ModelRow::new();
    let wire = entry
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    // 界面显示 wire id（发给上游的那个）；表键单独存。
    m.id = wire.clone();
    m.kimi_alias = alias.to_string();
    m.name = entry
        .get("display_name")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&wire)
        .to_string();
    // 逐个覆盖 `ModelRow::new()` 的默认值：条目没声明的字段一律留空，不能沿用
    // 「新建模型」的预填值——那会把默认的上下文/输出/档位当成用户配置写进文件。
    m.context = entry
        .get("max_context_size")
        .map(crate::util::number_text_public)
        .unwrap_or_default();
    m.output = entry
        .get("max_output_size")
        .map(crate::util::number_text_public)
        .unwrap_or_default();
    let caps = capabilities_of(entry);
    m.modalities_input = modalities_from_caps(&caps);
    m.reasoning = caps.iter().any(|c| {
        c.eq_ignore_ascii_case(CAP_THINKING) || c.eq_ignore_ascii_case(CAP_ALWAYS_THINKING)
    });
    m.tool_call = caps.iter().any(|c| c.eq_ignore_ascii_case(CAP_TOOL_USE));
    m.variants = entry
        .get("support_efforts")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    m.original_variants = m.variants.clone();
    m.disabled = entry
        .get("disabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    m.source_format = Some(ConfigFormat::KimiCode);
    m.raw = entry.clone();
    m
}

// ---------- 序列化：拆回两张表 ----------

/// 界面模型 → 模型条目里**由界面接管**的字段。
///
/// 以**旧条目**为基底，逐个覆盖界面接管的键；界面没接管的键（`reasoning_key` /
/// `beta_api` / `protocol` / `overrides` / 模型级 `base_url` 等）原样留着。
///
/// 界面清空的字段要**删掉**对应键而不是写空值：`display_name = ""` 是个真的空标题，
/// `max_context_size` 写成 0 更是非法（schema 要求 ≥1）。
fn entry_from_model(m: &ModelRow, alias: &str, old: Option<&Value>) -> Value {
    let mut obj = old.and_then(Value::as_object).cloned().unwrap_or_default();
    obj.insert("model".into(), Value::String(m.id.trim().to_string()));
    // alias 就是表键，不写进条目里（写了 Kimi 也不认，schema 无此字段）。
    let _ = alias;

    // `display_name` **逐字写**：等于 wire id 也写。
    //
    // 曾经按「等于 `model` 就省掉」处理，理由是「冗余」——那是错的：官方写法（Kimi 自己的
    // `/provider` 流程与文档示例）每条都带 `display_name`，本机 config.toml 的 7 条也全部
    // 带着，包括与 `model` 同名的那些。冗余与否是 Kimi 的判断，不是本工具的；一次保存就
    // 悄悄删掉用户文件里的一个字段，是实打实的数据丢失。
    // 只有界面把名称清空时才删键（Kimi 会回落用 `model` 显示，空标题反而是它不认的写法）。
    set_or_remove(&mut obj, "display_name", m.name.trim());

    // `max_context_size` 必填且 ≥1：解析不出正整数就**不写**这个键，让 Kimi 自己
    // 报错说缺少必填字段，而不是由本工具写一个 0 进去（那是非法值）。
    set_num_min1(&mut obj, "max_context_size", m.context.trim());
    set_num_min1(&mut obj, "max_output_size", m.output.trim());

    let old_caps = capabilities_of(old.unwrap_or(&Value::Null));
    let caps = merge_capabilities(&old_caps, m);
    if caps.is_empty() {
        obj.shift_remove("capabilities");
    } else {
        obj.insert(
            "capabilities".into(),
            Value::Array(caps.into_iter().map(Value::String).collect()),
        );
    }

    // 档位：`support_efforts` 是数组；`default_effort` 必须落在其中，否则 Kimi 读时会
    // 静默丢弃（源码 `effectiveModelAlias`：defaultEffort 不在 supportEfforts 里就删）。
    let efforts = split_list(&m.variants);
    if efforts.is_empty() {
        obj.shift_remove("support_efforts");
        obj.shift_remove("default_effort");
    } else {
        let existing_default = obj
            .get("default_effort")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        obj.insert(
            "support_efforts".into(),
            Value::Array(efforts.iter().cloned().map(Value::String).collect()),
        );
        // 原有 default_effort 仍在清单里就留着（不擅自改用户选的默认档）；
        // 不在了就删掉这个键——留一个不在清单里的值是无效配置。
        if efforts.iter().any(|e| e == &existing_default) {
            obj.insert("default_effort".into(), Value::String(existing_default));
        } else {
            obj.shift_remove("default_effort");
        }
    }
    Value::Object(obj)
}

/// 非空则写字符串，空白则删键。
fn set_or_remove(obj: &mut Map<String, Value>, key: &str, value: &str) {
    if value.is_empty() {
        obj.shift_remove(key);
    } else {
        obj.insert(key.to_string(), Value::String(value.to_string()));
    }
}

/// 能解析成**≥1 的整数**才写入；否则删键（`max_context_size` 要求 ≥1）。
fn set_num_min1(obj: &mut Map<String, Value>, key: &str, text: &str) {
    match text.parse::<i64>() {
        Ok(n) if n >= 1 => {
            obj.insert(key.to_string(), Value::Number(n.into()));
        }
        _ => {
            obj.shift_remove(key);
        }
    }
}

/// provider 条目里**由界面接管**的字段（以旧条目为基底）。
fn provider_entry_from_row(p: &ProviderRow, old: Option<&Value>) -> Value {
    let mut obj = old.and_then(Value::as_object).cloned().unwrap_or_default();
    set_or_remove(&mut obj, "base_url", p.base_url.trim());
    // `type` 必填：界面选空时保留旧值，旧值也没有才写一个安全的默认。
    let api = p.effective_api();
    let ty = if api.trim().is_empty() {
        provider_type(old.unwrap_or(&Value::Null)).to_string()
    } else {
        api
    };
    obj.insert(
        "type".into(),
        Value::String(if ty.is_empty() {
            "openai".to_string()
        } else {
            ty
        }),
    );
    // 凭据三选一：界面只有 `api_key` / `api_key_env` 两个框，`oauth` 不建模。
    //
    // 用户填了密钥就**清掉** `api_key_env`，反之亦然——这正是 XOR 的落地点。
    // 若旧条目带 `oauth`（只读 provider），它不在界面里，这里也不动它：
    // 那种 provider 的凭据由 Kimi Code 自己维护。
    let key = p.api_key.trim();
    let env = p.api_key_env.trim();
    if !key.is_empty() {
        obj.insert("api_key".into(), Value::String(key.to_string()));
        obj.shift_remove("api_key_env");
    } else if !env.is_empty() {
        obj.insert("api_key_env".into(), Value::String(env.to_string()));
        obj.shift_remove("api_key");
    } else {
        // 两个都空：保持原状（可能本就没有，也可能是 oauth provider）。
        obj.shift_remove("api_key");
        obj.shift_remove("api_key_env");
    }
    Value::Object(obj)
}

/// 界面状态 → 全部模型条目（含停用条目、含 `disabled` 标记）。
///
/// 主配置与全量副本共用这一步。`disabled` 每条必写（含 `false`）的理由与 WorkBuddy
/// 完全相同：省掉 `false` 会让「用户全勾上」与「这份副本从没记录过勾选」变成同一状态。
fn all_models(providers: &[ProviderRow], base: &Value) -> Vec<(String, String, Value)> {
    // 旧条目按 alias 建索引：界面没接管的键要从这里继承。
    let old: Map<String, Value> = models_map(base).cloned().unwrap_or_default();
    let mut out: Vec<(String, String, Value)> = Vec::new();
    for p in providers.iter().filter(|p| !p.key.trim().is_empty()) {
        let provider = p.key.trim().to_string();
        // 没有模型的 provider 也要留一条？——不。Kimi 的模型必须挂在 provider 下，
        // 而 provider 本身没有模型是合法的（本机 `sensenova` 就有模型，但一个 provider
        // 完全没模型也不会让文件非法）。写一条空模型反而会产出缺 `model` 的非法条目。
        for m in &p.models {
            let wire = m.id.trim();
            if wire.is_empty() {
                // 没有 wire id 的模型行写出去就是非法条目（`model` 必填）。
                // 界面允许这种中间态（刚点「新增模型」），保存时跳过它。
                continue;
            }
            // alias 缺省 = wire id（与 Kimi 自己的 /provider 行为一致）。
            let alias = if m.kimi_alias.trim().is_empty() {
                wire.to_string()
            } else {
                m.kimi_alias.trim().to_string()
            };
            let mut entry = entry_from_model(m, &alias, old.get(&alias));
            if let Some(obj) = entry.as_object_mut() {
                obj.insert("provider".into(), Value::String(provider.clone()));
                obj.insert("disabled".into(), Value::Bool(m.disabled));
            }
            out.push((provider.clone(), alias, entry));
        }
    }
    out
}

/// 全量副本的 `[models.*]`：所有条目 + 每条自己的 `disabled`。
fn full_models(providers: &[ProviderRow], base: &Value) -> Map<String, Value> {
    let mut out: Map<String, Value> = Map::new();
    for (_, alias, entry) in all_models(providers, base) {
        out.insert(alias, entry);
    }
    out
}

/// 生效清单的 `[models.*]`：只含启用条目，且**不带** `disabled` 键
/// （Kimi 的 schema 里没有这个字段，写了是未知键）。
fn effective_models(providers: &[ProviderRow], base: &Value) -> Map<String, Value> {
    let mut out: Map<String, Value> = Map::new();
    for (_, alias, entry) in all_models(providers, base) {
        if entry.get("disabled").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let mut entry = entry;
        if let Some(obj) = entry.as_object_mut() {
            obj.shift_remove("disabled");
        }
        out.insert(alias, entry);
    }
    out
}

/// 把**认不出来**的模型条目并进一张模型表（原样，不加 `disabled`）。
fn merge_unmanaged_models(
    models: &mut Map<String, Value>,
    base: &Value,
    providers: &[ProviderRow],
) {
    for (alias, model) in unmanaged_models(base, providers) {
        models.entry(alias).or_insert(model);
    }
}

/// 空表不写出来。
///
/// Kimi 自己的 `setRecordSection` 就是「转换后为空则删键」，照做才能让它读到的文件
/// 与自己写出的形态一致；而且 `[providers]` 这种空表在 TOML 里是**合法但多余**的，
/// 凭空多出来只会让文件看着像被改坏了。
fn set_or_drop_table(root: &mut Map<String, Value>, key: &str, table: Map<String, Value>) {
    if table.is_empty() {
        root.shift_remove(key);
    } else {
        root.insert(key.to_string(), Value::Object(table));
    }
}

/// `[providers.*]`：界面 provider + 只读 provider（原样）+ 认不出来的形状。
fn all_providers(providers: &[ProviderRow], base: &Value) -> Map<String, Value> {
    let mut out: Map<String, Value> = Map::new();
    let old = providers_map(base).cloned().unwrap_or_default();
    for p in providers.iter().filter(|p| !p.key.trim().is_empty()) {
        let name = p.key.trim();
        // 只读 provider 不归界面管，原样保留（见 [`unmanaged_providers`]）。
        if is_managed_provider(name) {
            continue;
        }
        out.insert(name.to_string(), provider_entry_from_row(p, old.get(name)));
    }
    for (name, value) in unmanaged_providers(base) {
        out.insert(name, value);
    }
    out
}

/// 基座里**不归界面管**的 provider：`managed:*`（OAuth 登录态）与「界面卡片没覆盖到、
/// 但确实存在于基座里的」条目。
///
/// `managed:*` 必须原样带过去：它的 `oauth` 子表与 `credentials/` 里的凭据配对，
/// 改写会破坏登录态。其余基座里有、界面没有的 provider 也要保留——界面只重建自己
/// 认得的卡片，删掉一个卡片是「删了」，而基座里多出来的 provider 说明它从未进过界面
/// （例如用户手编的、或 `type` 非法被跳过的），静默删掉就是丢配置。
fn unmanaged_providers(base: &Value) -> Vec<(String, Value)> {
    providers_map(base)
        .map(|m| {
            m.iter()
                .filter(|(name, _)| is_managed_provider(name))
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// 基座里**认不出来**的模型条目：孤儿（`provider` 指向不存在的键）与 `managed:*` 名下的。
///
/// 返回 `alias → 条目`。它们不进界面，但必须原样写回（见模块说明末节）。
fn unmanaged_models(base: &Value, managed: &[ProviderRow]) -> Map<String, Value> {
    let providers = providers_map(base).cloned().unwrap_or_default();
    let mut out: Map<String, Value> = Map::new();
    for (alias, model) in models_map(base).unwrap_or(&Map::new()) {
        let owner = model.get("provider").and_then(Value::as_str).unwrap_or("");
        // 界面认得的：provider 存在、不是 managed、且该 provider 有卡片。
        let known = !owner.is_empty()
            && !is_managed_provider(owner)
            && providers.contains_key(owner)
            && managed.iter().any(|p| p.key.trim() == owner);
        if !known {
            out.insert(alias.clone(), model.clone());
        }
    }
    out
}

/// 按 alias 逐条判断勾选状态。
///
/// `trusted` = 这批条目的 `disabled` 键可以信任（读的是全量副本时为 `true`）。
/// 主配置里没有这个键（Kimi 不认），所以从主配置读时一律当启用。
fn build_load(joined: Vec<Joined>, orphans: Vec<(String, Value)>, trusted: bool) -> BackendLoad {
    let mut providers: Vec<ProviderRow> = Vec::new();
    for j in &joined {
        // `managed:*` 不进界面：它由 OAuth 维护，界面无从编辑，显示出来只会让人
        // 以为能改。模型也一样（它们是登录时自动写入的）。
        if is_managed_provider(&j.name) {
            continue;
        }
        let mut row = provider_from_entry(&j.name, &j.provider, &j.models);
        if !trusted {
            for m in &mut row.models {
                m.disabled = false;
            }
        }
        providers.push(row);
    }
    let _ = orphans;
    BackendLoad {
        root: Value::Object(Map::new()),
        agents: Vec::new(),
        providers,
        extras: Value::Object(Map::new()),
    }
}

/// 全量副本的完整 root；读不出 / 没有 `providers` 表都返回 `None`（调用方退回主配置）。
fn load_full_store_root(config_path: &str) -> Option<Value> {
    if config_path.trim().is_empty() {
        return None;
    }
    let text = read_config_content(&full_store_path(config_path)).ok()?;
    if text.trim().is_empty() {
        return None;
    }
    let root = parse_toml(&text).ok()?;
    models_map(&root)?;
    Some(root)
}

/// 把主配置里「副本还没有」的模型条目并进来（按 alias 判重）。
///
/// 用户可能手改了 `config.toml`（Kimi Code 自己也会写它——`/login`、`/model` 都会），
/// 手加/自动加的条目不在副本里；不补的话它在界面上根本看不见。
fn merge_full_and_effective(
    full: &Map<String, Value>,
    effective: &Map<String, Value>,
) -> Map<String, Value> {
    let mut out = full.clone();
    for (alias, model) in effective {
        out.entry(alias.clone()).or_insert_with(|| model.clone());
    }
    out
}

impl Backend for KimiCodeBackend {
    fn id(&self) -> ConfigFormat {
        ConfigFormat::KimiCode
    }

    fn default_local_path(&self) -> String {
        default_local_path()
    }

    fn default_wsl_path(&self) -> Option<String> {
        Some(format!("{}/.kimi-code/config.toml", wsl_home()?))
    }

    fn detect(&self, content: &str, path: &str) -> bool {
        // 路径强命中：Kimi Code 的用户级配置就在这个位置。这也是与 Codex 区分的
        // **主要**手段——两者都是 `config.toml` + TOML，只能靠路径与键名分辨。
        let normalized = path.replace('\\', "/");
        if normalized.contains("/.kimi-code/") && normalized.ends_with("config.toml") {
            return true;
        }
        let Ok(root) = parse_toml(content) else {
            return false;
        };
        // `model_providers` 是 Codex 的顶层键（带下划线），Kimi 是 `providers`。
        // 有 Codex 特征就直接让位，别把别人的配置收进来。
        if root.get("model_providers").is_some() {
            return false;
        }
        // 极强特征：`managed:kimi-code` + `type = "kimi"`。
        if providers_map(&root).is_some_and(|m| {
            m.iter()
                .any(|(name, v)| name.starts_with(MANAGED_PREFIX) && provider_type(v) == "kimi")
        }) {
            return true;
        }
        // 一般特征：`[providers.` 与 `[models.` 两张表同时出现，且模型带
        // `max_context_size`（Kimi 特有且必填）。只验键名不够——同名不同形的东西
        // 多得是，`providers` 这个词别的格式也用。
        let has_providers = providers_map(&root).is_some_and(|m| !m.is_empty());
        let has_models = models_map(&root).is_some_and(|m| {
            m.values()
                .any(|v| v.get("provider").is_some() && v.get("max_context_size").is_some())
        });
        if has_providers && has_models {
            return true;
        }
        // 顶层独有标量：`default_permission_mode` 是 Kimi 的权限模式键。
        root.get("default_permission_mode").is_some()
    }

    fn parse(&self, content: &str) -> Result<BackendLoad, String> {
        let root = parse_toml(content)?;
        let (joined, orphans) = join(&root);
        let mut load = build_load(joined, orphans, false);
        load.root = root.clone();
        load.extras = root;
        Ok(load)
    }

    /// 带路径解析：优先读全量副本（含停用条目与逐条的勾选标记）。
    ///
    /// 副本里也有完整的 provider 表（含 `managed:*`），所以 extras 仍取**主配置**——
    /// 那才是 Kimi Code 实际生效的那份，`[thinking]` 之类的顶层设置以它为准。
    fn parse_at(&self, content: &str, path: &str) -> Result<BackendLoad, String> {
        let root = parse_toml(content)?;
        let mut trusted = false;
        let models = match load_full_store_root(path) {
            Some(full) => {
                trusted = true;
                merge_full_and_effective(
                    &models_map(&full).cloned().unwrap_or_default(),
                    &models_map(&root).cloned().unwrap_or_default(),
                )
            }
            None => models_map(&root).cloned().unwrap_or_default(),
        };
        // join 用的是「模型表」+「provider 表」，两侧合并后的模型表要重新挂一次。
        let mut merged = root.clone();
        if let Some(obj) = merged.as_object_mut() {
            obj.insert("models".into(), Value::Object(models));
        }
        let (joined, orphans) = join(&merged);
        let mut load = build_load(joined, orphans, trusted);
        load.root = root.clone();
        load.extras = root;
        Ok(load)
    }

    fn serialize_root(
        &self,
        _agents: &[AgentRow],
        providers: &[ProviderRow],
        extras: &Value,
        target_root: Option<&Value>,
    ) -> Value {
        // 产物是 **Kimi Code 的生效清单**：只含启用的模型条目。全量副本由
        // `save_sidecars` 另写一份，两者共用 `all_models` 构造，字段口径必然一致。
        let base = target_root.unwrap_or(extras);
        let mut root = base.as_object().cloned().unwrap_or_default();
        set_or_drop_table(&mut root, "providers", all_providers(providers, base));
        let mut models = effective_models(providers, base);
        // 认不出来的条目原样补回（孤儿模型 / managed 名下的模型）。
        merge_unmanaged_models(&mut models, base, providers);
        set_or_drop_table(&mut root, "models", models);
        Value::Object(root)
    }

    /// 全量副本（`models.full.toml`）：所有模型条目 + 每条自己的 `disabled` + provider 表。
    ///
    /// 继承未知字段的基底取**副本本身**（上次的完整状态，字段最全），没有才退回主配置。
    /// 只用主配置当基底会让停用条目的未知字段在每次保存时被抹掉——那正是这份副本要
    /// 解决的问题。副本读不出、主配置也读不出就取消保存。
    fn save_sidecars(&self, path: &str, providers: &[ProviderRow]) -> Result<(), String> {
        // 凭据 XOR 在写盘**之前**挡住：同时写 api_key 与 api_key_env 会让 Kimi Code
        // 启动失败（见模块说明第 2 点）。这是本后端最严重的一条约束。
        if let Some(conflict) = first_credential_conflict(providers) {
            return Err(format!("凭据冲突，已取消保存: {conflict}"));
        }
        let base = match load_full_store_root(path) {
            Some(root) => root,
            None => self
                .load_target_root(path)
                .map_err(|e| format!("无法读取全量副本与主配置，已取消保存: {e}"))?,
        };
        let mut root = Map::new();
        set_or_drop_table(&mut root, "providers", all_providers(providers, &base));
        let mut models = full_models(providers, &base);
        // 副本是「全部配置」的落盘形态：认不出来的条目也要在里面，否则它们只剩主配置
        // 一处记录——而主配置会被筛成「只剩启用条目」，下一轮就没有全量基底可继承了。
        merge_unmanaged_models(&mut models, &base, providers);
        set_or_drop_table(&mut root, "models", models);
        super::write_config(
            &full_store_path(path),
            &to_toml_string(&Value::Object(root))?,
        )
    }

    fn load_target_root(&self, path: &str) -> Result<Value, String> {
        super::load_target_root_with(path, parse_toml, || Value::Object(Map::new()))
    }

    fn icon_rgba(&self) -> Option<(&'static [u8], u32, u32)> {
        Some((
            include_bytes!("../../assets/agents/kimi-code_32.bin"),
            32,
            32,
        ))
    }

    fn render(&self, root: &Value, _compact: bool) -> Result<String, String> {
        // TOML 没有「紧凑 / 美化」两种口径：`toml` 的输出就是规范形态。
        // `compact` 参数对本后端无意义（忽略），与 YAML 后端同性质。
        to_toml_string(root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root_of(text: &str) -> Value {
        parse_toml(text).unwrap()
    }

    const SAMPLE: &str = r#"
default_model = "kimi-code/k3"
default_permission_mode = "auto"

[providers."managed:kimi-code"]
base_url = "https://api.kimi.com/coding/v1"
type = "kimi"
api_key = ""

[providers."managed:kimi-code".oauth]
storage = "file"
key = "oauth/kimi-code"

[providers.sensenova]
base_url = "https://token.sensenova.cn/v1"
type = "openai"
api_key = "sk-test"

[models."kimi-code/k3"]
provider = "managed:kimi-code"
model = "k3"
max_context_size = 1048576
capabilities = [ "thinking", "tool_use" ]
display_name = "K3"

[models."sensenova/deepseek-v4-flash"]
provider = "sensenova"
model = "deepseek-v4-flash"
max_context_size = 262000
capabilities = [ "tool_use" ]

[thinking]
enabled = true
"#;

    /// 双表 join：模型按 `provider` 挂到对应 provider 下。
    #[test]
    fn models_join_to_their_provider() {
        let root = root_of(SAMPLE);
        let (joined, orphans) = join(&root);
        let sensenova = joined.iter().find(|j| j.name == "sensenova").unwrap();
        assert_eq!(sensenova.models.len(), 1);
        assert_eq!(sensenova.models[0].0, "sensenova/deepseek-v4-flash");
        assert!(orphans.is_empty());
    }

    /// 孤儿模型（provider 不存在）不 panic、不被丢弃。
    #[test]
    fn orphan_models_are_kept() {
        let root = root_of(
            r#"
[providers.p]
type = "openai"

[models."gone/x"]
provider = "nonexistent"
model = "x"
max_context_size = 1
"#,
        );
        let (joined, orphans) = join(&root);
        assert_eq!(joined.len(), 1);
        assert_eq!(orphans.len(), 1);
        // 保存后仍在（原样带过去）
        let out = KimiCodeBackend.serialize_root(&[], &[], &root, None);
        assert_eq!(out["models"]["gone/x"]["model"], "x");
    }

    /// `managed:*` 的 provider 与模型都不进界面。
    #[test]
    fn managed_providers_are_not_listed() {
        let root = root_of(SAMPLE);
        let (joined, orphans) = join(&root);
        let load = build_load(joined, orphans, false);
        assert_eq!(load.providers.len(), 1, "只列出 sensenova");
        assert_eq!(load.providers[0].key, "sensenova");
    }

    /// `managed:*` 的 provider 与模型保存后原样还在（含 oauth 子表）。
    #[test]
    fn managed_entries_survive_a_save() {
        let root = root_of(SAMPLE);
        let (joined, orphans) = join(&root);
        let load = build_load(joined, orphans, false);
        let out = KimiCodeBackend.serialize_root(&[], &load.providers, &root, None);
        let managed = &out["providers"]["managed:kimi-code"];
        assert_eq!(managed["type"], "kimi");
        assert_eq!(managed["oauth"]["key"], "oauth/kimi-code", "oauth 子表保留");
        assert_eq!(
            out["models"]["kimi-code/k3"]["model"], "k3",
            "managed 模型保留"
        );
    }

    /// 界面没有 provider 时，基座里的 provider 不被删。
    #[test]
    fn providers_are_not_dropped_when_ui_is_empty() {
        let root = root_of(SAMPLE);
        let out = KimiCodeBackend.serialize_root(&[], &[], &root, None);
        assert!(out["providers"].get("managed:kimi-code").is_some());
        assert!(out["models"].get("kimi-code/k3").is_some());
    }

    /// alias ≠ model 时往返保持 alias 作表键、model 作 wire id。
    #[test]
    fn alias_is_preserved_when_it_differs_from_model() {
        let root = root_of(
            r#"
[providers.p]
type = "openai"
api_key = "k"

[models."my-alias"]
provider = "p"
model = "real-wire-id"
max_context_size = 1000
"#,
        );
        let (joined, orphans) = join(&root);
        let load = build_load(joined, orphans, false);
        assert_eq!(
            load.providers[0].models[0].id, "real-wire-id",
            "id 是 wire id"
        );
        assert_eq!(load.providers[0].models[0].kimi_alias, "my-alias");
        let out = KimiCodeBackend.serialize_root(&[], &load.providers, &root, None);
        assert!(out["models"].get("my-alias").is_some(), "alias 仍是表键");
        assert_eq!(out["models"]["my-alias"]["model"], "real-wire-id");
    }

    /// 含 `.` / `:` 的键序列化后仍是合法 TOML 且键名不变。
    #[test]
    fn dotted_and_colon_keys_round_trip() {
        let root = root_of(SAMPLE);
        let text = to_toml_string(&root).unwrap();
        let back = parse_toml(&text).unwrap();
        assert_eq!(root, back, "往返语义不变");
        assert!(
            text.contains("[providers.\"managed:kimi-code\"]"),
            "含冒号的键必须加引号"
        );
    }

    /// capabilities 只增不减：界面没勾的已有标签保存后仍在。
    #[test]
    fn capabilities_are_only_added() {
        let entry: Value = toml::from_str(
            "model = \"m\"\nmax_context_size = 1\ncapabilities = [\"thinking\", \"dynamically_loaded_tools\"]\n",
        )
        .unwrap();
        let mut m = model_from_entry("a", &entry);
        m.tool_call = true; // 界面勾上工具调用
        m.reasoning = true;
        let out = entry_from_model(&m, "a", Some(&entry));
        let caps: Vec<&str> = out["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(
            caps.contains(&"dynamically_loaded_tools"),
            "原有标签不得删除"
        );
        assert!(caps.contains(&"tool_use"), "新勾的标签要加上");
        assert!(caps.contains(&"thinking"));
    }

    /// 取消「支持思考」要能真的关掉（否则开关形同虚设）。
    #[test]
    fn unchecking_reasoning_removes_thinking_caps() {
        let entry: Value = toml::from_str(
            "model = \"m\"\nmax_context_size = 1\ncapabilities = [\"thinking\", \"always_thinking\", \"tool_use\"]\n",
        )
        .unwrap();
        let mut m = model_from_entry("a", &entry);
        m.reasoning = false;
        let out = entry_from_model(&m, "a", Some(&entry));
        let caps: Vec<&str> = out["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(!caps.contains(&"thinking"));
        assert!(!caps.contains(&"always_thinking"));
        assert!(caps.contains(&"tool_use"), "只动思考相关标签");
    }

    /// 停用条目：不进生效清单，但进全量副本且带 `disabled`。
    #[test]
    fn disabled_entries_leave_the_effective_file() {
        let root = root_of(SAMPLE);
        let (joined, orphans) = join(&root);
        let mut load = build_load(joined, orphans, false);
        load.providers[0].models[0].disabled = true;
        let effective = KimiCodeBackend.serialize_root(&[], &load.providers, &root, None);
        assert!(
            effective["models"]
                .get("sensenova/deepseek-v4-flash")
                .is_none(),
            "停用条目不得进生效清单"
        );
        let full = full_models(&load.providers, &root);
        let entry = &full["sensenova/deepseek-v4-flash"];
        assert_eq!(entry["disabled"], true, "全量副本必须记录停用");
    }

    /// 全量副本里 `disabled` 每条必写，含 `false`。
    #[test]
    fn full_store_records_every_flag_including_false() {
        let root = root_of(SAMPLE);
        let (joined, orphans) = join(&root);
        let load = build_load(joined, orphans, false);
        let full = full_models(&load.providers, &root);
        for (alias, entry) in &full {
            assert!(
                entry.get("disabled").and_then(Value::as_bool).is_some(),
                "{alias} 缺 disabled 标记"
            );
        }
    }

    /// 生效清单里**不写** `disabled`（Kimi 的 schema 没这个键）。
    #[test]
    fn effective_file_has_no_disabled_key() {
        let root = root_of(SAMPLE);
        let (joined, orphans) = join(&root);
        let load = build_load(joined, orphans, false);
        let effective = KimiCodeBackend.serialize_root(&[], &load.providers, &root, None);
        for (alias, entry) in effective["models"].as_object().unwrap() {
            assert!(entry.get("disabled").is_none(), "{alias} 不该带 disabled");
        }
    }

    /// XOR 体检：同时填 api_key 与 api_key_env 要被判成冲突。
    #[test]
    fn credential_conflict_is_detected() {
        let mut p = ProviderRow::new();
        p.key = "x".into();
        p.api_key = "sk-1".into();
        p.api_key_env = "MY_KEY".into();
        let err = first_credential_conflict(&[p]).unwrap();
        assert!(err.contains("api_key_env"), "{err}");
        assert!(err.contains("mutually exclusive"), "{err}");
    }

    /// `api_key = ""` 与 `oauth` 并存**不是**冲突（本机真实文件就是这样）。
    #[test]
    fn empty_api_key_does_not_conflict_with_oauth() {
        let root = root_of(SAMPLE);
        let managed = &root["providers"]["managed:kimi-code"];
        assert_eq!(credential_conflict("managed:kimi-code", managed), None);
    }

    /// 非空 api_key 与 oauth 并存才是冲突。
    #[test]
    fn non_empty_api_key_conflicts_with_oauth() {
        let entry: Value = toml::from_str(
            "type = \"kimi\"\napi_key = \"sk-x\"\n[oauth]\nstorage = \"file\"\nkey = \"k\"\n",
        )
        .unwrap();
        let msg = credential_conflict("p", &entry).unwrap();
        assert!(msg.contains("api_key") && msg.contains("oauth"), "{msg}");
    }

    /// 界面填了密钥就清掉 env 变量名（XOR 的落地点）。
    #[test]
    fn writing_a_key_clears_the_env_name() {
        let old: Value = toml::from_str("type = \"openai\"\napi_key_env = \"OLD_ENV\"\n").unwrap();
        let mut p = ProviderRow::new();
        p.key = "p".into();
        p.pi_api = "openai".into();
        p.api_key = "sk-new".into();
        p.api_key_env = "OLD_ENV".into();
        let out = provider_entry_from_row(&p, Some(&old));
        assert_eq!(out["api_key"], "sk-new");
        assert!(out.get("api_key_env").is_none(), "必须清掉 env 名");
    }

    /// 界面填了 env 名就清掉密钥。
    #[test]
    fn writing_an_env_name_clears_the_key() {
        let old: Value = toml::from_str("type = \"openai\"\napi_key = \"sk-old\"\n").unwrap();
        let mut p = ProviderRow::new();
        p.key = "p".into();
        p.pi_api = "openai".into();
        p.api_key_env = "MY_KEY".into();
        let out = provider_entry_from_row(&p, Some(&old));
        assert_eq!(out["api_key_env"], "MY_KEY");
        assert!(out.get("api_key").is_none(), "必须清掉密钥");
    }

    /// `max_context_size` 必填且 ≥1：填不出正数就不写这个键（不写 0）。
    #[test]
    fn invalid_context_size_is_omitted_not_zeroed() {
        let mut obj = Map::new();
        set_num_min1(&mut obj, "max_context_size", "0");
        assert!(obj.get("max_context_size").is_none());
        set_num_min1(&mut obj, "max_context_size", "");
        assert!(obj.get("max_context_size").is_none());
        set_num_min1(&mut obj, "max_context_size", "abc");
        assert!(obj.get("max_context_size").is_none());
        set_num_min1(&mut obj, "max_context_size", "1024");
        assert_eq!(obj["max_context_size"], 1024);
    }

    /// `default_effort` 不在 `support_efforts` 里时删掉（否则是无效配置）。
    #[test]
    fn default_effort_must_stay_within_support_efforts() {
        let entry: Value = toml::from_str(
            "model = \"m\"\nmax_context_size = 1\nsupport_efforts = [\"low\", \"high\"]\ndefault_effort = \"high\"\n",
        )
        .unwrap();
        let mut m = model_from_entry("a", &entry);
        m.variants = "low, max".into(); // 用户改了档位，high 不在了
        let out = entry_from_model(&m, "a", Some(&entry));
        assert!(
            out.get("default_effort").is_none(),
            "不在清单里的 default_effort 必须删掉"
        );
        // 仍在清单里时保留用户选的默认档
        let mut m2 = model_from_entry("a", &entry);
        m2.variants = "low, high, max".into();
        let out2 = entry_from_model(&m2, "a", Some(&entry));
        assert_eq!(out2["default_effort"], "high");
    }

    /// 顶层 extras（`[thinking]`、`default_*`）往返不丢。
    #[test]
    fn top_level_extras_survive() {
        let root = root_of(SAMPLE);
        let out = KimiCodeBackend.serialize_root(&[], &[], &root, None);
        assert_eq!(out["default_model"], "kimi-code/k3");
        assert_eq!(out["default_permission_mode"], "auto");
        assert_eq!(out["thinking"]["enabled"], true);
    }

    /// `null` 不会让 TOML 序列化失败（跨格式转来的 null 要先清掉）。
    #[test]
    fn nulls_are_stripped_before_serializing() {
        let v: Value = serde_json::json!({"a": null, "b": {"c": null, "d": 1}, "e": [1, null]});
        let text = to_toml_string(&v).expect("null 不得让序列化失败");
        assert!(!text.contains("null"), "{text}");
        let back = parse_toml(&text).unwrap();
        assert_eq!(back["b"]["d"], 1);
    }

    /// 判别：Codex 的 `model_providers` 不归本后端。
    #[test]
    fn detect_rejects_codex_shape() {
        let codex = r#"
model = "gpt-5"
model_provider = "openai"

[model_providers.openai]
name = "OpenAI"
base_url = "https://api.openai.com/v1"
"#;
        assert!(!KimiCodeBackend.detect(codex, ""));
    }

    /// 判别：路径命中与内容特征都认。
    #[test]
    fn detect_accepts_kimi_shapes() {
        assert!(KimiCodeBackend.detect("", r"C:\Users\x\.kimi-code\config.toml"));
        assert!(KimiCodeBackend.detect(SAMPLE, ""));
        assert!(KimiCodeBackend.detect("default_permission_mode = \"auto\"\n", ""));
        // 空内容 / 无关 TOML 不认
        assert!(!KimiCodeBackend.detect("", ""));
        assert!(!KimiCodeBackend.detect("[package]\nname = \"x\"\n", ""));
    }

    /// 解析失败要有可读报错（非法 TOML）。
    #[test]
    fn invalid_toml_reports_an_error() {
        assert!(KimiCodeBackend.parse("this is not = = toml").is_err());
    }

    /// 空文件解析成空 load，不 panic。
    #[test]
    fn empty_file_parses_to_nothing() {
        let load = KimiCodeBackend.parse("").unwrap();
        assert!(load.providers.is_empty());
    }

    /// 全量副本路径与主配置同级。
    #[test]
    fn full_store_path_sits_beside_the_config() {
        assert_eq!(
            full_store_path(r"C:\Users\x\.kimi-code\config.toml"),
            r"C:\Users\x\.kimi-code\models.full.toml"
        );
    }

    /// 全量副本优先：停用条目仍可见，且勾选状态被信任。
    #[test]
    fn full_store_keeps_disabled_entries_visible() {
        let root = root_of(SAMPLE);
        let (joined, orphans) = join(&root);
        let mut load = build_load(joined, orphans, false);
        load.providers[0].models[0].disabled = true;
        let full = full_models(&load.providers, &root);
        let mut merged = root.clone();
        merged
            .as_object_mut()
            .unwrap()
            .insert("models".into(), Value::Object(full));
        let (j2, o2) = join(&merged);
        let reloaded = build_load(j2, o2, true);
        assert!(
            reloaded.providers[0].models[0].disabled,
            "副本里的停用状态要还原"
        );
    }

    /// 手加进主配置的条目（副本里没有）要能看见。
    #[test]
    fn entries_added_by_hand_are_merged_in() {
        let full: Map<String, Value> = toml::from_str(
            "[a]\nprovider = \"p\"\nmodel = \"a\"\nmax_context_size = 1\ndisabled = false\n",
        )
        .unwrap();
        let effective: Map<String, Value> = toml::from_str(
            "[a]\nprovider = \"p\"\nmodel = \"a\"\nmax_context_size = 1\n\n[b]\nprovider = \"p\"\nmodel = \"b\"\nmax_context_size = 1\n",
        )
        .unwrap();
        let merged = merge_full_and_effective(&full, &effective);
        assert_eq!(merged.len(), 2);
        assert!(merged.contains_key("b"));
    }

    /// 同一 alias 不重复写（副本与主配置合并后仍是一条）。
    #[test]
    fn merged_models_do_not_duplicate_aliases() {
        let root = root_of(SAMPLE);
        let (joined, orphans) = join(&root);
        let load = build_load(joined, orphans, false);
        let full = full_models(&load.providers, &root);
        let merged = merge_full_and_effective(&full, &full);
        assert_eq!(merged.len(), full.len());
    }

    /// 没有 wire id 的模型行（界面中间态）不写进文件。
    #[test]
    fn models_without_a_wire_id_are_skipped() {
        let root = root_of(SAMPLE);
        let (joined, orphans) = join(&root);
        let mut load = build_load(joined, orphans, false);
        load.providers[0].models.push(ModelRow::new()); // id 为空
        let out = KimiCodeBackend.serialize_root(&[], &load.providers, &root, None);
        for (_, entry) in out["models"].as_object().unwrap() {
            assert!(!entry["model"].as_str().unwrap_or("").is_empty());
        }
    }

    /// 认不出来的键（`overrides` / `reasoning_key` / 模型级 `base_url`）原样保留。
    #[test]
    fn unmodelled_keys_are_inherited() {
        let entry: Value = toml::from_str(
            r#"
model = "m"
max_context_size = 1000
reasoning_key = "reasoning_content"
beta_api = true
base_url = "https://override/v1"

[overrides]
max_output_size = 500
"#,
        )
        .unwrap();
        let m = model_from_entry("a", &entry);
        let out = entry_from_model(&m, "a", Some(&entry));
        assert_eq!(out["reasoning_key"], "reasoning_content");
        assert_eq!(out["beta_api"], true);
        assert_eq!(out["base_url"], "https://override/v1");
        assert_eq!(out["overrides"]["max_output_size"], 500);
    }
}
