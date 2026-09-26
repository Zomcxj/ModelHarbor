//! Agent 配置后端注册表 —— 每种配置格式一个后端模块。
//!
//! 新增 agent 集成：新建 `src/backends/<id>.rs` 实现 [`Backend`]，
//! 在 [`BACKENDS`] 注册即可；加载 / 保存 / 判别 / 目标解析全部走本注册表，
//! app 层不出现按格式的硬编码分支。
//!
//! 设计约束（Phase 1）：
//! - 序列化风格（pretty / compact JSON）由 app 层的序列化器处理，
//!   后端只产出 `serde_json::Value` 形式的 root；
//! - 跨格式保存的合并语义内置于各后端的 `serialize_root`。

pub mod deepseek_harness;
pub mod oh_my_pi;
pub mod opencode;
pub mod pi;
pub mod qwen_code;
pub mod workbuddy;
pub mod zcode;

use crate::format::ConfigFormat;
use crate::model::{AgentRow, ProviderRow};
use crate::util::{is_wsl_path, WslPathProbe};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

/// 一个后端加载结果的完整快照。
#[derive(Clone)]
pub struct BackendLoad {
    /// 完整原始 root（当前文件保存时的基底）。
    pub root: Value,
    /// opencode 格式的 agents（pi 系为空）。
    pub agents: Vec<AgentRow>,
    /// providers（两种格式都有）。
    pub providers: Vec<ProviderRow>,
    /// 本后端顶层未知字段：opencode 为整个 root（agent/provider 会被整体替换），
    /// pi 系为 `providers` 之外的顶层字段；DSH 为完整 root。
    pub extras: Value,
}

/// 配置后端：一种 agent 配置格式的加载 / 保存 / 判别 / 路径知识。
pub trait Backend: Sync {
    /// 唯一标识（对应 ConfigFormat 枚举）。
    fn id(&self) -> ConfigFormat;

    /// 默认本地路径（Windows 风格）。
    fn default_local_path(&self) -> String;

    /// 默认 WSL 路径（None = 不支持 WSL）。
    fn default_wsl_path(&self) -> Option<String>;

    /// 某个配置路径的**等价候选**，按优先级排列（首个是主名）。
    ///
    /// 覆写它的是那些「主配置可能有多个等价文件名」的后端：opencode 系首次运行
    /// 生成的可能是 `.jsonc` 变体（`kilo.jsonc`），而默认路径写的是 `.json`。
    /// 若只认默认名，页面会判成「未安装」，保存还会**另建**一个 `.json`，
    /// 把用户真正的配置晾在一边。
    fn path_candidates(&self, path: &str) -> Vec<String> {
        vec![path.to_string()]
    }

    /// 实际要读写的本地文件路径：候选里第一个真实存在的；都不存在则用主名
    /// （新建场景要往主名写）。
    fn resolve_local_path(&self, local_path: &str) -> String {
        self.path_candidates(local_path)
            .into_iter()
            .find(|c| Path::new(c.as_str()).exists())
            .unwrap_or_else(|| local_path.to_string())
    }

    /// 本地目标是否可用。默认判定：文件或**父目录**存在（父目录存在 = 可新建）。
    /// opencode 系覆写：候选文件名可能不同，只认文件本体（见 `path_candidates`）。
    fn local_available(&self, local_path: &str) -> bool {
        let path = Path::new(local_path);
        path.exists() || path.parent().is_some_and(|dir| dir.exists())
    }

    /// WSL 目标是否可用（已安装判定：配置文件或其目录存在）。
    /// 旗标由注册表批量探测（单次 `wsl` 调用）后传入，后端自身不拉起进程。
    /// opencode 系覆写为只认文件本体：宽松判定会把并不存在的候选 `.json` 判成已安装。
    fn wsl_available(&self, probe: WslPathProbe) -> bool {
        probe.path_exists || probe.parent_dir_exists
    }

    /// 内容判别：该内容是否属于本格式。`path` 提供扩展名上下文（可为空）。
    fn detect(&self, content: &str, path: &str) -> bool;

    /// 内容 → 结构化数据；读取/解析失败返回 Err。
    fn parse(&self, content: &str) -> Result<BackendLoad, String>;

    /// 带源路径解析。默认与 `parse` 相同；需要同级 sidecar 文件的后端覆写。
    fn parse_at(&self, content: &str, _path: &str) -> Result<BackendLoad, String> {
        self.parse(content)
    }

    /// 构造保存用 root。
    /// - `target_root = None`：当前文件保存，以 `extras` 为基底整体写入 UI 状态；
    /// - `target_root = Some`：目标文件已存在，与目标现有内容合并（upsert）。
    ///   调用方在跨格式转换（来源格式 ≠ 目标格式）前会先剔除目标里由界面接管的
    ///   provider / agent 容器（见 `app::strip_cross_format_containers`），
    ///   因此这种情况下的合并基底只剩目标文件的其他顶层字段。
    fn serialize_root(
        &self,
        agents: &[AgentRow],
        providers: &[ProviderRow],
        extras: &Value,
        target_root: Option<&Value>,
    ) -> Value;

    /// 保存目标（非当前文件）的现有 root，作为合并基底。
    ///
    /// 文件不存在返回空 root（新建场景）；**文件存在但读不出 / 解析不了返回 Err**，
    /// 由调用方取消保存——静默回落空对象会让「先读后合并」变成「对空合并后整体覆写」，
    /// 目标文件里 provider / agent 以外的顶层设置会被清掉。
    fn load_target_root(&self, path: &str) -> Result<Value, String>;

    /// 保存与主配置同级的 sidecar 文件（默认无 sidecar）。
    /// 主配置写入前调用，sidecar 失败会取消本次保存。
    fn save_sidecars(&self, _path: &str, _providers: &[ProviderRow]) -> Result<(), String> {
        Ok(())
    }

    /// 官方图标：32×32 未预乘 RGBA 原始字节与尺寸；None = 无图标。
    fn icon_rgba(&self) -> Option<(&'static [u8], u32, u32)> {
        None
    }

    /// root → 文件内容。JSON 后端（opencode 系 / pi / zcode）共用默认实现；
    /// YAML 后端（omp / DSH）与数组根的 WorkBuddy 覆写（compact 对它们另有一套口径）。
    fn render(&self, root: &Value, compact: bool) -> Result<String, String> {
        Ok(if compact {
            crate::app::compact_json(root)
        } else {
            crate::app::pretty_json(root)
        })
    }
}

/// 全部后端。**顺序即语义**：第 0 个是判别回落项，其余按“更具体优先”排列
/// （omp 在 pi 之前：.yml 扩展名优先归 omp，无扩展名时 JSON 语法内容让位给 pi；
/// workbuddy / zcode 有各自的唯一顶层标记——数组根 / `config.providerOrder`——
/// 放在 pi 系之前，避免被 `providers` 判定抢走）。
///
/// opencode 系（opencode / kilocode / mimocode）三者内容形状一致，靠**路径**互相区分
/// （见 [`opencode::path_matches`]），所以它们的相对顺序不影响判别结果；本家 opencode
/// 仍是第 0 个回落项。
pub static BACKENDS: &[&dyn Backend] = &[
    &opencode::BACKEND,
    &opencode::KILOCODE_BACKEND,
    &opencode::MIMOCODE_BACKEND,
    &workbuddy::BACKEND,
    &zcode::BACKEND,
    &qwen_code::BACKEND,
    &deepseek_harness::BACKEND,
    &oh_my_pi::BACKEND,
    &pi::BACKEND,
];

/// 按标识查找后端。
pub fn backend(id: ConfigFormat) -> &'static dyn Backend {
    BACKENDS
        .iter()
        .copied()
        .find(|b| b.id() == id)
        .expect("BACKENDS 必须覆盖所有 ConfigFormat 变体")
}

/// 内容判别：遍历非回落后端找首个命中，否则回落第 0 个。
pub fn detect_format(content: &str, path: &str) -> ConfigFormat {
    for b in BACKENDS.iter().skip(1) {
        if b.detect(content, path) {
            return b.id();
        }
    }
    BACKENDS[0].id()
}

/// 通用加载：读文件（本地或 WSL）+ 按后端解析。
pub fn load_backend(id: ConfigFormat, path: &str) -> Result<BackendLoad, String> {
    let content = crate::util::read_config_content(path)?;
    backend(id).parse_at(&content, path)
}

/// WSL 目标路径（存在则返回）。
///
/// 探测结果进程级缓存（单次批量 `wsl` 调用 + 缓存的 `$HOME`）：
/// 运行期间在 WSL 侧新装 agent 不会被感知，需重启应用。
pub fn wsl_target(id: ConfigFormat) -> Option<String> {
    static CACHE: OnceLock<HashMap<ConfigFormat, Option<String>>> = OnceLock::new();
    CACHE
        .get_or_init(probe_wsl_targets)
        .get(&id)
        .cloned()
        .flatten()
}

/// 一次 `wsl` 调用探测全部后端的默认 WSL 路径，返回“已安装”后端的路径。
///
/// 每个后端可能有多条**等价候选**路径（opencode 系的 `.json` / `.jsonc`），
/// 全部一起探。选择顺序：先取**确实是文件**的候选——否则在「父目录存在即算安装」的
/// 宽松判定下，会挑中那个并不存在的 `.json`，写盘时就凭空多出一个文件、
/// 用户真正的 `.jsonc` 反而没人动。都不存在文件时才退回落到的宽松条件。
fn probe_wsl_targets() -> HashMap<ConfigFormat, Option<String>> {
    let mut out = HashMap::new();
    // (后端, 候选路径) —— 候选按优先级排（主名在前）。
    let entries: Vec<(ConfigFormat, Vec<String>)> = BACKENDS
        .iter()
        .filter_map(|b| {
            let base = b.default_wsl_path()?;
            Some((b.id(), b.path_candidates(&base)))
        })
        .collect();
    if entries.is_empty() {
        return out;
    }
    // 扁平化后一次性探测，再按后端切回来。
    let mut flat: Vec<String> = Vec::new();
    for (_, candidates) in &entries {
        flat.extend(candidates.iter().cloned());
    }
    let probes = crate::util::wsl_batch_probe(&flat);
    let mut cursor = 0;
    for (id, candidates) in entries {
        let slice = &probes[cursor..cursor + candidates.len()];
        cursor += candidates.len();
        let backend = backend(id);
        // 优先：候选里真实存在的文件。
        let by_file = candidates
            .iter()
            .zip(slice)
            .find(|(_, probe)| probe.file_exists)
            .map(|(path, _)| path.clone());
        // 其次：后端自己的宽松判定（父目录存在即算已安装）。
        let by_rule = candidates
            .iter()
            .zip(slice)
            .find(|(_, probe)| backend.wsl_available(**probe))
            .map(|(path, _)| path.clone());
        if let Some(path) = by_file.or(by_rule) {
            out.insert(id, Some(path));
        }
    }
    out
}

/// 目标是否可用（本地或 WSL）。
pub fn target_available(id: ConfigFormat, local_path: &str) -> bool {
    backend(id).local_available(local_path) || wsl_target(id).is_some()
}

/// 本地实际要读写的路径：后端可在默认名之外回退等价文件名（opencode 系的 `.jsonc`）。
pub fn resolve_local_path(id: ConfigFormat, local_path: &str) -> String {
    backend(id).resolve_local_path(local_path)
}

/// 实际写入路径：本地优先，本地不可用回落 WSL，最后回退本地默认（新建场景）。
pub fn target_path(id: ConfigFormat, local_path: &str) -> String {
    let local = backend(id).resolve_local_path(local_path);
    if Path::new(&local).exists() {
        return local;
    }
    if let Some(wsl) = wsl_target(id) {
        return wsl;
    }
    local
}

/// 统一写入：WSL 路径走 wsl 命令，本地路径**原子写**（先写临时文件再替换）。
///
/// 不能用裸 `fs::write`：写一半时崩溃（release 是 `panic = abort`）或断电会把
/// 文件截断——这些配置里是明文 API Key，损坏后重建只能靠用户记忆。
/// prefs 与 tokens 一直用同一套原子写，主配置没有理由例外。
pub fn write_config(path: &str, content: &str) -> Result<(), String> {
    if is_wsl_path(path) {
        crate::util::write_wsl_file(path, content)
    } else {
        crate::util::atomic_write_text(std::path::Path::new(path), content)
    }
}

/// [`Backend::load_target_root`] 的共享实现：各家只提供解析函数与「文件不存在」时的空 root。
///
/// 文件存在但读不出 / 解析不了返回 Err，且错误带路径——io::Error 的 Display
/// 不含文件名，不带路径用户无从得知是哪个文件坏了。
pub(crate) fn load_target_root_with(
    path: &str,
    parse: impl Fn(&str) -> Result<Value, String>,
    empty: impl FnOnce() -> Value,
) -> Result<Value, String> {
    let content =
        crate::util::read_config_content(path).map_err(|e| format!("读取失败（{path}）: {e}"))?;
    if content.trim().is_empty() {
        return Ok(empty());
    }
    parse(&content).map_err(|e| format!("解析失败（{path}）: {e}"))
}
