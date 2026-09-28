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
//! Kimi 的模型 schema（源码 `ModelAliasBaseSchema`）里**没有** `disabled`/`enabled`
//! 字段，模型表又是按别名一一索引的 `record`——每条别名都是独立生效的一行，
//! 既不去重、也没有开关。曾经照 WorkBuddy/Qwen 的样子给这里加过停用开关与全量副本
//! `models.full.toml`，按用户指正移除：那是本工具发明的状态，Kimi 自己的三条写入路径
//! （`managedModelKey`、`applyOpenPlatformConfig`、自定义注册表 provider）全都不产生
//! 这样的概念。删卡片就是真删（`.bak` 备份仍然兜底）。
//!
//! ## 别名的缺省值
//!
//! Kimi 自己写模型条目时，表键一律是 **`<provider>/<model>`**：managed 走
//! `managedModelKey` = `` `${KIMI_CODE_PLATFORM_ID}/${modelId}` ``（平台名 `kimi-code`，
//! 即去掉 provider 键的 `managed:` 前缀），open 平台与自定义注册表走
//! `` `${providerKey}/${model.id}` ``。所以界面新增（或从别的格式复制来的）模型
//! 缺省别名也按这个约定生成，而不是裸的 wire id。
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
use crate::model::{AgentRow, ProviderRow};
use crate::util::{home_dir_string, set_str, split_csv, wsl_home};
use serde_json::{Map, Value};

mod conflict;
mod convert;
mod merge;
mod toml_io;
pub(crate) use conflict::*;
pub(crate) use convert::*;
pub(crate) use merge::*;
pub(crate) use toml_io::*;

pub struct KimiCodeBackend;

pub static BACKEND: KimiCodeBackend = KimiCodeBackend;

/// OAuth 登录写入的 provider 前缀（源码 `KIMI_CODE_PROVIDER_NAME = "managed:kimi-code"`）。
pub(crate) const MANAGED_PREFIX: &str = "managed:";

fn default_local_path() -> String {
    format!("{}\\.kimi-code\\config.toml", home_dir_string())
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

// ---------- 结构访问 ----------

// ---------- 解析：两张表 join ----------

// ---------- 序列化：拆回两张表 ----------

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
        let mut load = build_load(join(&root));
        load.root = root.clone();
        load.extras = root;
        Ok(load)
    }

    // `parse_at` 用 trait 缺省实现（直接 `parse`）：没有全量副本可读，
    // 主配置就是全部状态。

    fn serialize_root(
        &self,
        _agents: &[AgentRow],
        providers: &[ProviderRow],
        extras: &Value,
        target_root: Option<&Value>,
    ) -> Value {
        // 产物就是 **Kimi Code 的全部生效状态**：没有停用概念，`config.toml` 一份
        // 文件承担所有条目。
        let base = target_root.unwrap_or(extras);
        let mut root = base.as_object().cloned().unwrap_or_default();
        set_or_drop_table(&mut root, "providers", all_providers(providers, base));
        let mut models = model_table(providers, base);
        // 认不出来的条目原样补回（孤儿模型 / managed 名下的模型）。
        merge_unmanaged_models(&mut models, base, providers);
        set_or_drop_table(&mut root, "models", models);
        Value::Object(root)
    }

    /// 没有全量副本可写（见模块说明「没有『停用』这回事」），但仍挂在保存序列里，
    /// 作为**写盘前的总闸**：
    /// - 凭据 XOR（同时写 `api_key` 与 `api_key_env` 会让 Kimi Code **启动失败**，
    ///   见模块说明第 2 点，本后端最严重的一条约束）；
    /// - `managed:` 保留前缀与别名撞名（[`first_structural_conflict`]）——写出去
    ///   界面就再也读不回来。
    fn save_sidecars(&self, _path: &str, providers: &[ProviderRow]) -> Result<(), String> {
        if let Some(conflict) = first_credential_conflict(providers) {
            return Err(format!("凭据冲突，已取消保存: {conflict}"));
        }
        if let Some(conflict) = first_structural_conflict(providers) {
            return Err(format!("配置冲突，已取消保存: {conflict}"));
        }
        Ok(())
    }

    fn load_target_root(&self, path: &str) -> Result<Value, String> {
        super::load_target_root_with(path, parse_toml, || Value::Object(Map::new()))
    }

    fn icon_rgba(&self) -> Option<(&'static [u8], u32, u32)> {
        Some((
            include_bytes!("../../../assets/agents/kimi-code_32.bin"),
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
mod tests;
