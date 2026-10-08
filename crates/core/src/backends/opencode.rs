//! opencode 系后端：`opencode` / `kilocode` / `mimocode` 三个同源格式共用一套实现。
//! 顶层 `agent`（subagent 定义 map）、`provider`（map）与其他顶层字段（如 `mcp`）原样保留。
//! 三者的差别只有配置目录名、主配置文件名、`$schema` 与图标，由 [`Flavor`] 区分。

use super::{Backend, BackendLoad};
use crate::convert;
use crate::format::ConfigFormat;
use crate::model::{AgentRow, ProviderRow};
use crate::util::{home_dir_string, parse_config_content, wsl_home, WslPathProbe};
use serde_json::{Map, Value};
use std::path::Path;

/// opencode 系的一个具体成员：只有目录名、文件名、`$schema` 与图标不同。
pub struct Flavor {
    pub id: ConfigFormat,
    /// 配置目录名（`~/.config/<dir>/`）。
    pub dir: &'static str,
    /// 主配置文件名；Kilo 与 MiMo 的 `.jsonc` 变体也被接受。
    pub file: &'static str,
    /// 官方 `$schema` 地址；新建 / 合并后缺失时写入，已有值一律保留。
    pub schema: &'static str,
    /// 32×32 未预乘 RGBA 原始字节。
    pub icon: &'static [u8],
}

pub struct OpenCodeFamilyBackend(pub &'static Flavor);

/// opencode 本家。
pub static OPENCODE: Flavor = Flavor {
    id: ConfigFormat::Opencode,
    dir: "opencode",
    file: "opencode.json",
    schema: "https://opencode.ai/config.json",
    icon: include_bytes!("../../assets/agents/opencode_32.bin"),
};

/// Kilo Code（`@kilocode/cli`）：配置目录 `~/.config/kilo/`，主配置 `kilo.json`；
/// 同目录下遗留的 `opencode.json` 也会被读取，但不回退 `.opencode` 目录。
pub static KILOCODE: Flavor = Flavor {
    id: ConfigFormat::Kilocode,
    dir: "kilo",
    file: "kilo.json",
    schema: "https://app.kilo.ai/config.json",
    icon: include_bytes!("../../assets/agents/kilocode_32.bin"),
};

/// MiMo Code（`@mimo-ai/cli`，小米）：配置目录 `~/.config/mimocode/`
/// （Windows 为 `%LOCALAPPDATA%\mimocode\`），主配置 `mimocode.json`；不读 `opencode.json`。
/// 图标为小米官方 logo（橙底白 `mi`）。
pub static MIMOCODE: Flavor = Flavor {
    id: ConfigFormat::Mimocode,
    dir: "mimocode",
    file: "mimocode.json",
    schema: "https://mimo.xiaomi.com/mimocode/config.json",
    icon: include_bytes!("../../assets/agents/mimocode_32.bin"),
};

/// 本家 opencode 的后端实例。
pub static BACKEND: OpenCodeFamilyBackend = OpenCodeFamilyBackend(&OPENCODE);

/// Kilo Code 后端实例。
pub static KILOCODE_BACKEND: OpenCodeFamilyBackend = OpenCodeFamilyBackend(&KILOCODE);

/// MiMo Code 后端实例。
pub static MIMOCODE_BACKEND: OpenCodeFamilyBackend = OpenCodeFamilyBackend(&MIMOCODE);

impl Flavor {
    fn default_local_path(&self) -> String {
        format!(
            "{}\\.config\\{}\\{}",
            home_dir_string(),
            self.dir,
            self.file
        )
    }

    fn default_wsl_path(&self) -> Option<String> {
        Some(format!(
            "{}/.config/{}/{}",
            wsl_home()?,
            self.dir,
            self.file
        ))
    }

    /// 主配置的 `.jsonc` 变体路径（`kilo.json` → `kilo.jsonc`）。
    fn jsonc_variant(&self, path: &str) -> String {
        let stem = self.file.trim_end_matches(".json");
        // 只替换文件名那一段。
        match path.rfind(&self.file) {
            Some(at) => format!("{}{}.jsonc", &path[..at], stem),
            // 路径里没有主文件名：按扩展名整体换。
            None => format!("{}.jsonc", path.trim_end_matches(".json")),
        }
    }
}

impl Backend for OpenCodeFamilyBackend {
    fn id(&self) -> ConfigFormat {
        self.0.id
    }

    fn default_local_path(&self) -> String {
        self.0.default_local_path()
    }

    fn default_wsl_path(&self) -> Option<String> {
        self.0.default_wsl_path()
    }

    /// 默认名不存在时回退 `.jsonc` 变体；候选幂等，传入的已是 `.jsonc` 时不重复追加。
    fn path_candidates(&self, path: &str) -> Vec<String> {
        let variant = self.0.jsonc_variant(path);
        if variant == path {
            vec![path.to_string()]
        } else {
            vec![path.to_string(), variant]
        }
    }

    fn local_available(&self, local_path: &str) -> bool {
        self.path_candidates(local_path)
            .iter()
            .any(|p| Path::new(p.as_str()).exists())
    }

    fn wsl_available(&self, probe: WslPathProbe) -> bool {
        // 已安装判定：配置文件或其目录存在
        probe.file_exists || probe.parent_dir_exists
    }

    /// 判别：先按路径（目录名或文件名）认领，再退回内容特征。
    ///
    /// 内容兜底只对 opencode 本家生效，且要求同族无人按路径认领。
    fn detect(&self, content: &str, path: &str) -> bool {
        if path_matches(self.0, path) {
            return true;
        }
        // 同族任一成员已按路径认领，内容兜底不再生效。
        if FAMILY.iter().any(|f| path_matches(f, path)) {
            return false;
        }
        if self.0.id != ConfigFormat::Opencode {
            return false;
        }
        // 正向特征：顶层含 provider 对象。
        parse_config_content(content)
            .map(|v| v.get("provider").and_then(|x| x.as_object()).is_some())
            .unwrap_or(false)
    }

    fn parse(&self, content: &str) -> Result<BackendLoad, String> {
        let v = parse_config_content(content)?;
        let agents = v
            .get("agent")
            .and_then(|x| x.as_object())
            .map(|o| o.iter().map(|(k, av)| AgentRow::from(k, av)).collect())
            .unwrap_or_default();
        let providers = v
            .get("provider")
            .and_then(|x| x.as_object())
            .map(|o| o.iter().map(|(k, pv)| ProviderRow::from(k, pv)).collect())
            .unwrap_or_default();
        Ok(BackendLoad {
            root: v.clone(),
            agents,
            providers,
            // extras 载体就是整个 root（agent/provider 保存时整体替换）
            extras: v,
        })
    }

    fn serialize_root(
        &self,
        agents: &[AgentRow],
        providers: &[ProviderRow],
        extras: &Value,
        target_root: Option<&Value>,
    ) -> Value {
        let schema = self.0.schema;
        let mut root = match target_root {
            None => {
                // 当前文件：以 UI 状态为准整体替换 agent / provider（删除即生效）；
                // 列表为空时移除对应键，不写空对象
                let mut r = extras.clone();
                if let Value::Object(o) = &mut r {
                    let mut am = Map::new();
                    for a in agents {
                        if !a.key.is_empty() {
                            am.insert(a.key.clone(), a.to_value());
                        }
                    }
                    if am.is_empty() {
                        o.remove("agent");
                    } else {
                        o.insert("agent".into(), Value::Object(am));
                    }

                    let mut pm = Map::new();
                    for p in providers {
                        if !p.key.is_empty() {
                            pm.insert(p.key.clone(), p.to_value());
                        }
                    }
                    if pm.is_empty() {
                        o.remove("provider");
                    } else {
                        o.insert("provider".into(), Value::Object(pm));
                    }
                }
                with_schema(r, schema)
            }
            Some(target) => with_schema(merge_opencode_root(target, agents, providers), schema),
        };
        // 按目标方言补齐必需字段，含目标文件里保留下来的旧条目。
        complete_required_fields_in(&mut root, self.0.id);
        root
    }

    fn load_target_root(&self, path: &str) -> Result<Value, String> {
        super::load_target_root_with(path, parse_config_content, || Value::Object(Map::new()))
    }

    fn icon_rgba(&self) -> Option<(&'static [u8], u32, u32)> {
        Some((self.0.icon, 32, 32))
    }
}

/// opencode 系全体成员。
pub static FAMILY: &[&Flavor] = &[&OPENCODE, &KILOCODE, &MIMOCODE];

/// 路径是否属于某个 opencode 系成员：路径段里含其目录名或文件名。
///
/// 同时接受 `/` 与 `\` 分隔，且大小写不敏感。
fn path_matches(flavor: &Flavor, path: &str) -> bool {
    if path.trim().is_empty() {
        return false;
    }
    let lower = path.to_lowercase().replace('\\', "/");
    // 目录名命中即认领。
    let dir_seg = format!("/{}/", flavor.dir);
    if lower.contains(&dir_seg) {
        return true;
    }
    // 同族其他成员的目录名出现时，文件名不再是本成员的线索。
    let other_dir = FAMILY
        .iter()
        .any(|f| f.dir != flavor.dir && lower.contains(&format!("/{}/", f.dir)));
    if other_dir {
        return false;
    }
    // 文件名：主名与其 `.jsonc` 变体。
    let stem = flavor.file.trim_end_matches(".json");
    let names = [format!("/{}", flavor.file), format!("/{}.jsonc", stem)];
    names.iter().any(|n| lower.ends_with(n.as_str()))
}

/// 保证 `$schema` 存在、且排在首位；已有值一律保留。
fn with_schema(root: Value, schema: &str) -> Value {
    let Value::Object(mut o) = root else {
        return root;
    };
    if !o.contains_key("$schema") {
        o.insert("$schema".into(), Value::String(schema.to_string()));
    }
    Value::Object(convert::order_fields(o, &["$schema"]))
}

/// 按目标方言补齐模型级**必需成对字段**。
///
/// 三家 CLI 的 required 不同：`limit.context` / `limit.output` 三家都要求成对；
/// `modalities.input` / `modalities.output` 仅 mimocode 要求成对。
///
/// 补法：`modalities` 缺的一侧补 `["text"]`；`limit` 只给一半时整块删除
/// （`limit` 在模型级可选）。`modalities` 两侧都没写时整块删除。
fn complete_required_model_fields(model: &mut Value, flavor: ConfigFormat) {
    let Some(obj) = model.as_object_mut() else {
        return;
    };

    // limit：三家都要求 context + output 成对（整块可省略，不能只给一半）。
    let limit_sides = obj
        .get("limit")
        .and_then(Value::as_object)
        .map(|l| (l.contains_key("context"), l.contains_key("output")));
    if let Some((has_context, has_output)) = limit_sides {
        if has_context != has_output {
            obj.remove("limit");
        }
    }

    // modalities：只有 mimocode 要求 input + output 成对。
    if flavor != ConfigFormat::Mimocode {
        return;
    }
    let modality_sides = obj
        .get("modalities")
        .and_then(Value::as_object)
        .map(|m| (m.contains_key("input"), m.contains_key("output")));
    let Some((has_input, has_output)) = modality_sides else {
        return;
    };
    match (has_input, has_output) {
        (true, true) => {}
        // 两侧都没写：整块删掉（`{}` 非法）。
        (false, false) => {
            obj.remove("modalities");
        }
        (true, false) => fill_text_modality(obj, "output"),
        (false, true) => fill_text_modality(obj, "input"),
    }
}

/// 把 `modalities.<side>` 补成 `["text"]`。
fn fill_text_modality(obj: &mut Map<String, Value>, side: &str) {
    if let Some(modalities) = obj.get_mut("modalities").and_then(Value::as_object_mut) {
        modalities.insert(
            side.into(),
            Value::Array(vec![Value::String("text".into())]),
        );
    }
}

/// 对整个 root 的 `provider.*.models.*` 逐个补齐必需字段，含目标文件里原有的模型。
fn complete_required_fields_in(root: &mut Value, flavor: ConfigFormat) {
    let Some(providers) = root.get_mut("provider").and_then(Value::as_object_mut) else {
        return;
    };
    for provider in providers.values_mut() {
        let Some(models) = provider.get_mut("models").and_then(Value::as_object_mut) else {
            continue;
        };
        for model in models.values_mut() {
            complete_required_model_fields(model, flavor);
        }
    }
}

/// 将 UI 状态合并进 opencode 系目标 root（跨格式保存用）：
/// agent / provider 以 UI 状态 upsert，目标已有同名条目按字段保守合并
/// （UI 提供的键覆盖，目标独有键与目标独有条目一律保留），
/// 其余顶层字段原样保留。
pub fn merge_opencode_root(
    target_root: &Value,
    agents: &[AgentRow],
    providers: &[ProviderRow],
) -> Value {
    let mut root = target_root.clone();
    if let Value::Object(o) = &mut root {
        let mut am = o
            .get("agent")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        for a in agents {
            if !a.key.is_empty() {
                let value = a.to_value();
                let entry = match am.get(&a.key) {
                    Some(target) => convert::merge_conservative(target, &value),
                    None => value,
                };
                am.insert(a.key.clone(), entry);
            }
        }
        o.insert("agent".into(), Value::Object(am));

        let mut pm = o
            .get("provider")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        for p in providers {
            if !p.key.is_empty() {
                let value = p.to_value();
                let entry = match pm.get(&p.key) {
                    Some(target) => convert::merge_conservative(target, &value),
                    None => value,
                };
                pm.insert(p.key.clone(), entry);
            }
        }
        o.insert("provider".into(), Value::Object(pm));
    }
    root
}
