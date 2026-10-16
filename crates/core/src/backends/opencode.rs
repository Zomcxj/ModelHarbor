//! opencode 系后端：`opencode` / `kilocode` / `mimocode` 三个同源格式共用一套实现。
//! 顶层 `agent`（subagent 定义 map）、`provider`（map）与其他顶层字段（如 `mcp`）原样保留。
//! 三者的差别只有配置目录名、主配置文件名、`$schema` 与图标，由 [`Flavor`] 区分。

use super::{Backend, BackendLoad};
use crate::convert;
use crate::format::ConfigFormat;
use crate::model::{AgentRow, OpencodeSchemaVersion, ProviderRow};
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
        // 正向特征：顶层含 provider 对象（单复数键都认）。
        parse_config_content(content)
            .map(|v| providers_table(&v).is_some())
            .unwrap_or(false)
    }

    fn parse(&self, content: &str) -> Result<BackendLoad, String> {
        let v = parse_config_content(content)?;
        // agent 条目的 schema 版本跟**文件**走：v2 文件里的极简 agent
        //（无 request、model 不带 #）按逐条嗅探会误判成 v1，温度从顶层
        // 死键读、保存时把 request.body.temperature 静默删掉。
        let file_schema = detect_schema_version(&v);
        let agents = agents_table(&v)
            .map(|o| {
                o.iter()
                    .map(|(k, av)| AgentRow::from_with_schema(k, av, file_schema))
                    .collect()
            })
            .unwrap_or_default();
        let providers = providers_table(&v)
            .map(|o| o.iter().map(|(k, pv)| ProviderRow::from(k, pv)).collect())
            .unwrap_or_default();
        // 行的 schema 记录 raw 的真实形状（供同版本增量写用）；「kilo / mimo
        // 永远 v1」由写盘路径的 target_version 钳制（见 serialize_root），
        // 不在这里改行 schema —— 否则同版本写会保留 raw 里的 v2 键。
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
                // 键名与写回形状跟随源文件：复数键存在就写复数，否则写单数；
                // v2 文件写 v2 形状（package / settings / capabilities 等）。
                // 在可变借用之前取，避免借用冲突。
                let ak = agents_key(&r);
                let pk = providers_key(&r);
                let version = target_version(self.0.id, &r);
                if let Value::Object(o) = &mut r {
                    let mut am = Map::new();
                    for a in agents {
                        if !a.key.is_empty() {
                            am.insert(a.key.clone(), a.to_value_as(version));
                        }
                    }
                    if am.is_empty() {
                        o.remove(ak);
                    } else {
                        o.insert(ak.into(), Value::Object(am));
                    }

                    let mut pm = Map::new();
                    for p in providers {
                        if !p.key.is_empty() {
                            pm.insert(p.key.clone(), p.to_value_as(version));
                        }
                    }
                    if pm.is_empty() {
                        o.remove(pk);
                    } else {
                        o.insert(pk.into(), Value::Object(pm));
                    }
                }
                with_schema(r, schema)
            }
            Some(target) => with_schema(
                merge_opencode_root_as(
                    target,
                    agents,
                    providers,
                    target_version(self.0.id, target),
                ),
                schema,
            ),
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

/// 路径是否属于 opencode 系任一成员。
///
/// 供其他后端的内容兜底判别让路：新版 opencode 顶层写 `providers`
/// （无 `provider`），与 pi 系的内容特征重合，光看内容会误认领。
pub fn family_path_matches(path: &str) -> bool {
    FAMILY.iter().any(|f| path_matches(f, path))
}

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
/// 模态成对（v1 在 `modalities`，v2 在 `capabilities`）仅 mimocode 要求。
///
/// 补法：缺的一侧补 `["text"]`；`limit` 只给一半时整块删除
/// （`limit` 在模型级可选）。模态两侧都没写时：v1 整块删除（`{}` 非法），
/// v2 若还留着 `tools` 等键则补齐两侧、否则删除。
fn complete_required_model_fields(
    model: &mut Value,
    flavor: ConfigFormat,
    version: OpencodeSchemaVersion,
) {
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

    // 模态成对：只有 mimocode 要求。
    if flavor != ConfigFormat::Mimocode {
        return;
    }
    let parent = match version {
        OpencodeSchemaVersion::V1 => "modalities",
        OpencodeSchemaVersion::V2 => "capabilities",
    };
    let modality_sides = obj
        .get(parent)
        .and_then(Value::as_object)
        .map(|m| (m.contains_key("input"), m.contains_key("output")));
    let Some((has_input, has_output)) = modality_sides else {
        return;
    };
    match (has_input, has_output) {
        (true, true) => {}
        (false, false) => {
            let has_other = obj
                .get(parent)
                .and_then(Value::as_object)
                .is_some_and(|m| !m.is_empty());
            if has_other {
                fill_text_modality(obj, parent, "input");
                fill_text_modality(obj, parent, "output");
            } else {
                obj.remove(parent);
            }
        }
        (true, false) => fill_text_modality(obj, parent, "output"),
        (false, true) => fill_text_modality(obj, parent, "input"),
    }
}

/// 把 `<parent>.<side>` 补成 `["text"]`。
fn fill_text_modality(obj: &mut Map<String, Value>, parent: &str, side: &str) {
    if let Some(target) = obj.get_mut(parent).and_then(Value::as_object_mut) {
        target.insert(
            side.into(),
            Value::Array(vec![Value::String("text".into())]),
        );
    }
}

/// 对整个 root 的 provider 表逐个补齐必需字段，含目标文件里原有的模型。
/// 兼容新旧顶层键：新版写 `providers`，旧版写 `provider`。
fn complete_required_fields_in(root: &mut Value, flavor: ConfigFormat) {
    let version = detect_schema_version(root);
    let pk = providers_key(root);
    let Some(providers) = root.get_mut(pk).and_then(Value::as_object_mut) else {
        return;
    };
    for provider in providers.values_mut() {
        let Some(models) = provider.get_mut("models").and_then(Value::as_object_mut) else {
            continue;
        };
        for model in models.values_mut() {
            complete_required_model_fields(model, flavor, version);
        }
    }
}

/// 将 UI 状态合并进 opencode 系目标 root（跨格式保存用）：
/// agent / provider 以 UI 状态 upsert，目标已有同名条目按字段保守合并
/// （UI 提供的键覆盖，目标独有键与目标独有条目一律保留），
/// 其余顶层字段原样保留。
/// 顶层 agents / providers 键名跟随源文件：新版 opencode 写复数
/// `agents` / `providers`，旧版写单数 `agent` / `provider`。
/// 复数存在即用复数，否则回退单数。
fn agents_key(root: &Value) -> &'static str {
    if root.get("agents").and_then(|x| x.as_object()).is_some() {
        "agents"
    } else {
        "agent"
    }
}

fn providers_key(root: &Value) -> &'static str {
    // opencode 的惯例键是单数 `provider`：目标里单、复并存时（pi 遗留的
    // 顶层 `providers` 被有意保留的场景）必须仍写单数，否则 provider 条目
    // 被误 upsert 进 pi 形状的容器。仅当目标只有复数键时才跟随复数。
    if root.get("provider").and_then(|x| x.as_object()).is_none()
        && root.get("providers").and_then(|x| x.as_object()).is_some()
    {
        "providers"
    } else {
        "provider"
    }
}

fn agents_table(root: &Value) -> Option<&Map<String, Value>> {
    root.get(agents_key(root)).and_then(|x| x.as_object())
}

fn providers_table(root: &Value) -> Option<&Map<String, Value>> {
    root.get(providers_key(root)).and_then(|x| x.as_object())
}

/// 整文件判定 opencode schema 版本。
///
/// provider 用 `package` / `settings` 是 v2 最强证据；顶层复数
/// `agents` / `plugins`（pi 从不具备、旧版 opencode 写单数）与 agent 的
/// `model#variant` 作后备。顶层复数 `providers` **单独不作证据**：
/// 那也是 pi 的内容形状（顶层就是 providers），裸复数键归 pi 认领。
/// 写回形状跟随此判定：给 v1 更新 v1，给 v2 更新 v2。
pub fn detect_schema_version(root: &Value) -> OpencodeSchemaVersion {
    if providers_table(root).is_some_and(|table| {
        table
            .values()
            .any(|p| p.get("package").is_some() || p.get("settings").is_some())
    }) {
        return OpencodeSchemaVersion::V2;
    }
    if root.get("agents").and_then(Value::as_object).is_some() || root.get("plugins").is_some() {
        return OpencodeSchemaVersion::V2;
    }
    if agents_table(root).is_some_and(|table| {
        table
            .values()
            .any(|a| crate::util::str_at(a, "model").contains('#'))
    }) {
        return OpencodeSchemaVersion::V2;
    }
    OpencodeSchemaVersion::V1
}

/// 某个 opencode 系后端的读写版本：opencode 跟随文件内容（v1 或 v2）；
/// kilo / mimo 永远 v1 —— v2 只是 opencode 本家的演进，它们的 CLI
/// 没有跟进 v2 schema（用户拍板）。
fn target_version(id: ConfigFormat, root: &Value) -> OpencodeSchemaVersion {
    if id == ConfigFormat::Opencode {
        detect_schema_version(root)
    } else {
        OpencodeSchemaVersion::V1
    }
}

pub fn merge_opencode_root(
    target_root: &Value,
    agents: &[AgentRow],
    providers: &[ProviderRow],
) -> Value {
    merge_opencode_root_as(
        target_root,
        agents,
        providers,
        detect_schema_version(target_root),
    )
}

/// 同 [`merge_opencode_root`]，但写回版本由调用方指定
/// （opencode 系保存路径用它馴 kilo / mimo 永远写 v1）。
pub fn merge_opencode_root_as(
    target_root: &Value,
    agents: &[AgentRow],
    providers: &[ProviderRow],
    version: OpencodeSchemaVersion,
) -> Value {
    // 键名与写回形状跟随目标文件：复数键存在就写复数，否则写单数；
    // 目标是 v2 就写 v2 形状。
    // 在可变借用之前取，避免借用冲突。
    let ak = agents_key(target_root);
    let pk = providers_key(target_root);
    let mut root = target_root.clone();
    if let Value::Object(o) = &mut root {
        let mut am = o
            .get(ak)
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        for a in agents {
            if !a.key.is_empty() {
                let value = a.to_value_as(version);
                let entry = match am.get(&a.key) {
                    Some(target) => convert::merge_conservative(target, &value),
                    None => value,
                };
                am.insert(a.key.clone(), entry);
            }
        }
        o.insert(ak.into(), Value::Object(am));

        let mut pm = o
            .get(pk)
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        for p in providers {
            if !p.key.is_empty() {
                let value = p.to_value_as(version);
                let entry = match pm.get(&p.key) {
                    Some(target) => convert::merge_conservative(target, &value),
                    None => value,
                };
                pm.insert(p.key.clone(), entry);
            }
        }
        o.insert(pk.into(), Value::Object(pm));
    }
    root
}
