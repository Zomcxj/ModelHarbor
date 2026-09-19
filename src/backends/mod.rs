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
pub mod workbuddy;
pub mod zcode;

use crate::format::ConfigFormat;
use crate::model::{AgentRow, ProviderRow};
use crate::util::{ensure_parent_dir, is_wsl_path, WslPathProbe};
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

    /// 本地目标是否可用（一般：文件存在；宽松后端：文件或父目录存在）。
    fn local_available(&self, local_path: &str) -> bool;

    /// WSL 目标是否可用（已安装判定：配置文件或其目录存在）。
    /// 旗标由注册表批量探测（单次 `wsl` 调用）后传入，后端自身不拉起进程。
    fn wsl_available(&self, probe: WslPathProbe) -> bool;

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

    /// 跨格式目标保存时读取目标现有 root（容错：读不到返回空对象）。
    fn load_target_root(&self, path: &str) -> Value;

    /// 保存与主配置同级的 sidecar 文件（默认无 sidecar）。
    /// 主配置写入前调用，sidecar 失败会取消本次保存。
    fn save_sidecars(&self, _path: &str, _providers: &[ProviderRow]) -> Result<(), String> {
        Ok(())
    }

    /// 官方图标：32×32 未预乘 RGBA 原始字节与尺寸；None = 无图标。
    fn icon_rgba(&self) -> Option<(&'static [u8], u32, u32)> {
        None
    }

    /// root → 文件内容（opencode/pi 为 JSON 两种风格，omp 为 YAML；compact 对 YAML 无意义）。
    fn render(&self, root: &Value, compact: bool) -> Result<String, String>;
}

/// 全部后端。**顺序即语义**：第 0 个是判别回落项，其余按“更具体优先”排列
/// （omp 在 pi 之前：.yml 扩展名优先归 omp，无扩展名时 JSON 语法内容让位给 pi；
/// workbuddy / zcode 有各自的唯一顶层标记——数组根 / `config.providerOrder`——
/// 放在 pi 系之前，避免被 `providers` 判定抢走）。
pub static BACKENDS: &[&dyn Backend] = &[
    &opencode::BACKEND,
    &workbuddy::BACKEND,
    &zcode::BACKEND,
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
fn probe_wsl_targets() -> HashMap<ConfigFormat, Option<String>> {
    let mut out = HashMap::new();
    let entries: Vec<(ConfigFormat, String)> = BACKENDS
        .iter()
        .filter_map(|b| b.default_wsl_path().map(|p| (b.id(), p)))
        .collect();
    if entries.is_empty() {
        return out;
    }
    let paths: Vec<String> = entries.iter().map(|(_, p)| p.clone()).collect();
    let probes = crate::util::wsl_batch_probe(&paths);
    for ((id, path), probe) in entries.into_iter().zip(probes) {
        if backend(id).wsl_available(probe) {
            out.insert(id, Some(path));
        }
    }
    out
}

/// 目标是否可用（本地或 WSL）。
pub fn target_available(id: ConfigFormat, local_path: &str) -> bool {
    backend(id).local_available(local_path) || wsl_target(id).is_some()
}

/// 实际写入路径：本地优先，本地不可用回落 WSL，最后回退本地默认（新建场景）。
pub fn target_path(id: ConfigFormat, local_path: &str) -> String {
    if Path::new(local_path).exists() {
        return local_path.to_string();
    }
    if let Some(wsl) = wsl_target(id) {
        return wsl;
    }
    local_path.to_string()
}

/// 统一写入：WSL 路径走 wsl 命令，本地路径自动创建父目录。
pub fn write_config(path: &str, content: &str) -> Result<(), String> {
    if is_wsl_path(path) {
        crate::util::write_wsl_file(path, content)
    } else {
        ensure_parent_dir(path)?;
        std::fs::write(path, content).map_err(|e| e.to_string())
    }
}
