//! opencode 系后端：`opencode` / `kilocode` / `mimocode` 三个同源格式共用一套实现。
//!
//! 结构（三者字段与结构一致）：顶层 `agent`（subagent 定义 map）+ `provider`（map）+
//! 其他顶层字段（如 `mcp`）原样保留。
//!
//! **为什么合成一个模块**：Kilo Code 与 MiMo Code 都是 opencode 的 fork，配置 schema
//! 的字段与结构一致——顶层 `provider` / `agent`，provider 的 `options.baseURL` /
//! `options.apiKey` / `options.timeout`，模型的 `models.<id>.limit.context|output` /
//! `tool_call` / `reasoning`。差别只有三处：**配置目录名、主配置文件名、图标**。复制三份
//! 解析器只会让以后的格式修正要改三遍、且迟早改漏一处，所以这里把这三处做成 [`Flavor`]
//! 参数，其余全部共用。
//!
//! **但「字段相同」不等于「required 相同」**：mimocode 额外要求 `modalities` 的
//! `input` / `output` 成对，opencode 与 kilo 都只把它当可选。写盘前必须按目标方言补齐
//! （见 [`complete_required_model_fields`]），否则源方言允许的半截字段写过去就是非法文件。
//!
//! **判别只能靠路径**：三者的内容形状一致，任何基于内容特征的判别都无法区分它们
//! （比如「顶层有 provider 对象」对三者同时为真）。所以 [`detect`] 按**目录名或文件名**
//! 认领，内容特征只作为最后的兜底（归 opencode）。这也意味着用户把 `kilo.json` 的内容
//! 拷到 `opencode.json` 里时，判出来的是 opencode——按路径认领的必然结果，且无害：
//! 三者写盘用的字段口径完全相同。

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
    /// 主配置文件名。Kilo 也接受 `kilo.jsonc`、MiMo 也接受 `mimocode.jsonc`，
    /// 这里取官方文档给出的首选（`kilo.json` / `mimocode.json`）。
    pub file: &'static str,
    /// 官方 `$schema` 地址。**新建 / 合并后缺失时会写进配置**：CLI 自己生成的
    /// 配置就带这个字段，缺了编辑器与 CLI 都拿不到 schema 提示。已有值一律保留。
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

/// Kilo Code（`@kilocode/cli`）：opencode 的 fork，配置目录 `~/.config/kilo/`，
/// 主配置 `kilo.json`（也接受 `kilo.jsonc`；文档另注它会读同目录下遗留的
/// `opencode.json`，但**不再**回退 `.opencode` 目录）。
pub static KILOCODE: Flavor = Flavor {
    id: ConfigFormat::Kilocode,
    dir: "kilo",
    file: "kilo.json",
    schema: "https://app.kilo.ai/config.json",
    icon: include_bytes!("../../assets/agents/kilocode_32.bin"),
};

/// MiMo Code（`@mimo-ai/cli`，小米）：opencode 的 fork，配置目录
/// `~/.config/mimocode/`（Windows 亦为 `%LOCALAPPDATA%\mimocode\`，两者同源解析），
/// 主配置 `mimocode.json`（也接受 `mimocode.jsonc`）。它**不读** `opencode.json`。
///
/// 图标用小米官方 logo（橙底白 `mi`），**不取 MiMo Code 的 favicon**：MiMo 仓库里的
/// favicon / 桌面应用图标 / console logo 全都沿用 opencode 的同一份图形
/// （`favicon.svg` 与 opencode 官方逐字节相同），照抄会得到一个和 opencode 页签
/// 看起来一样的图标。
pub static MIMOCODE: Flavor = Flavor {
    id: ConfigFormat::Mimocode,
    dir: "mimocode",
    file: "mimocode.json",
    schema: "https://mimo.xiaomi.com/mimocode/config.json",
    icon: include_bytes!("../../assets/agents/mimocode_32.bin"),
};

/// 兼容旧引用：本家 opencode 的后端实例。
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
        // 只替换**文件名**那一段，别动父目录里可能出现的同名子串。
        match path.rfind(&self.file) {
            Some(at) => format!("{}{}.jsonc", &path[..at], stem),
            // 路径里没有主文件名（用户自定义名）：按扩展名整体换。
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

    /// 默认名不存在时回退 `.jsonc` 变体（CLI 首次运行可能只生成那个）。
    ///
    /// 传进来的已经是 `.jsonc` 时不重复追加（解析结果会被再喂回来，
    /// 候选必须幂等，否则会滚出 `kilo.jsonc.jsonc` 这种路径）。
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

    /// 判别：**先按路径认领，再退回内容特征**。
    ///
    /// 三者的内容形状一模一样，内容判别分不开它们；能分开的只有路径。所以路径里出现
    /// 本成员的目录名或文件名时直接认领。都不匹配时用内容特征兜底，且只有 opencode
    /// 本家会认（它是这一族的代表，也是 `BACKENDS` 的判别回落项）——否则三个同源后端
    /// 会为同一份内容互相抢，判出哪个取决于注册顺序，反而不可预期。
    ///
    /// 内容兜底有个前提：**同族没人靠路径认领**。Kilo 会读同目录下遗留的
    /// `~/.config/kilo/opencode.json`，该文件按路径属于 kilocode；若此时还让 opencode
    /// 按内容（顶层 `provider`）兜底，注册顺序在前的 opencode 会把它抢走。
    fn detect(&self, content: &str, path: &str) -> bool {
        if path_matches(self.0, path) {
            return true;
        }
        // 同族任一成员已按路径认领，内容兜底不得再抢。
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
        // 各家 required 不同：源方言允许的半截字段写进目标方言会变成非法文件
        // （详见 `complete_required_model_fields`）。补齐放在最后，连目标文件里
        // 保留下来的旧条目一起过一遍。
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

/// opencode 系全体成员。`path_matches` 需要知道「同族还有谁」，才能判断某个目录名
/// 是否已经明确把文件判给了别的成员。
pub static FAMILY: &[&Flavor] = &[&OPENCODE, &KILOCODE, &MIMOCODE];

/// 路径是否属于某个 opencode 系成员：看路径段里有没有它的目录名或文件名。
///
/// 同时接受 `/` 与 `\` 分隔（WSL 路径用 `/`），且大小写不敏感（Windows）。
fn path_matches(flavor: &Flavor, path: &str) -> bool {
    if path.trim().is_empty() {
        return false;
    }
    let lower = path.to_lowercase().replace('\\', "/");
    // 目录名是最强信号：命中即认领。
    let dir_seg = format!("/{}/", flavor.dir);
    if lower.contains(&dir_seg) {
        return true;
    }
    // 同族其他成员的目录名出现时，文件名不再是本成员的线索。Kilo 会读同目录下遗留的
    // `~/.config/kilo/opencode.json`——那个文件属于 kilocode，若按文件名判给 opencode，
    // 页面与保存路径都会认错门。
    let other_dir = FAMILY
        .iter()
        .any(|f| f.dir != flavor.dir && lower.contains(&format!("/{}/", f.dir)));
    if other_dir {
        return false;
    }
    // 文件名：主名（`kilo.json`）与其 `.jsonc` 变体（`kilo.jsonc`）。
    let stem = flavor.file.trim_end_matches(".json");
    let names = [format!("/{}", flavor.file), format!("/{}.jsonc", stem)];
    names.iter().any(|n| lower.ends_with(n.as_str()))
}

/// 保证 `$schema` 存在、且排在首位。
///
/// CLI 自己生成的配置就带这个字段（`kilo.json` 里只有一行 `$schema`），它让编辑器与
/// CLI 拿到字段补全。此前新建 / 合并到不存在的目标时完全不写它，写出的配置是个裸的
/// `{"agent":{},"provider":{}}`，用户会看到「预览里没有 $schema」。
///
/// **已有值一律保留**：用户可能手动改成别的 schema 地址（或旧版本地址），
/// 覆盖成官方值等于替用户改配置。
fn with_schema(root: Value, schema: &str) -> Value {
    let Value::Object(mut o) = root else {
        return root;
    };
    if !o.contains_key("$schema") {
        o.insert("$schema".into(), Value::String(schema.to_string()));
    }
    Value::Object(convert::order_fields(o, &["$schema"]))
}

/// 按目标方言补齐模型级**必需成对字段**，让写出的配置一定通过 CLI 的 schema 校验。
///
/// ## 为什么必须做
///
/// 三个 CLI 都用 zod 校验配置，schema 里的 `required` 是真会拒绝启动的（实测
/// `mimo models` / `kilo models` / `opencode models` 对半截 `limit` 一律报
/// `Missing key … limit.output`）。而三家的 required **并不相同**：
///
/// | 字段 | opencode | kilo | mimocode |
/// |---|---|---|---|
/// | `limit.context` + `limit.output` | 必需 | 必需 | 必需 |
/// | `modalities.input` + `modalities.output` | 可选 | 可选 | **必需** |
///
/// 于是同一份配置在三页之间并不等价：源方言（opencode）允许只写
/// `modalities.output`，写进 mimocode 就变成**非法文件**——用户遇到的正是这个
/// （opencode 里那几个只写了 output 的模型，切到 mimo 页保存后 `mimo` 拒绝加载）。
///
/// ## 补法
///
/// 按「缺的那一侧有没有安全的默认值」分两种，不搞一刀切：
///
/// - `modalities`：缺的一侧补 `["text"]`。整块省略时 CLI 就是按「纯文本」理解的，
///   补 text 是对源方言语义的**忠实**表达，同时保住了用户明确写出的那一侧。
/// - `limit`：半截 limit **无法表达**（schema 要数字，而「不限」没有对应值），凭空
///   编一个上下文窗口比交给 CLI 自己的模型库更糟，所以整块删掉——`limit` 在模型级
///   本来就是可选的，省略后 CLI 用它自己的数据。
fn complete_required_model_fields(model: &mut Value, flavor: ConfigFormat) {
    let Some(obj) = model.as_object_mut() else {
        return;
    };

    // limit：三家都要求 context + output 成对（limit 整块可省略，但不能只给一半）。
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
        // 两侧都没写等于没声明模态：整块删掉才是合法形状（`{}` 会被拒）。
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

/// 对整个 root 的 `provider.*.models.*` 逐个补齐必需字段。
///
/// 覆盖**目标文件里原有的模型**，不只是界面接管的那些：合并写入时目标文件里的条目
/// 会保留下来，它们同样要能通过校验，否则照样是「CLI 起不来」。
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
