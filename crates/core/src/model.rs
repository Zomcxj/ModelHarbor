use crate::format::ConfigFormat;
use crate::util::{
    bool_at, nested_list_str, nested_num, nested_str, num_at, parse_number_text, set_num_opt,
    set_str, str_at,
};
use serde_json::{Map, Value};

/// 推理档位的规范顺序（各方言并集）：写入时按此排序。
const VARIANT_ORDER: &[&str] = &[
    "off", "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
];

/// 将档位名按规范顺序排列（未收录的自定义档位保持相对顺序，排在最后）。
pub fn ordered_variants<'a, I>(names: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut list: Vec<String> = names
        .into_iter()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    list.sort_by_key(|name| {
        VARIANT_ORDER
            .iter()
            .position(|known| known == name)
            .unwrap_or(VARIANT_ORDER.len())
    });
    list
}

/// 解析逗号分隔的档位文本并按规范顺序返回。
pub fn ordered_variants_text(text: &str) -> Vec<String> {
    ordered_variants(text.split(','))
}

/// 按规范档位顺序重建映射（键为档位名，值为该档位的映射细节）。
pub fn order_variant_map(map: &Map<String, Value>) -> Map<String, Value> {
    ordered_variants(map.keys().map(String::as_str))
        .into_iter()
        .filter_map(|key| map.get(&key).cloned().map(|value| (key, value)))
        .collect()
}

/// opencode 系配置的 schema 版本。
///
/// v1（旧版）与 v2（新版）同概念不同键名：provider 用 `npm`/`options` 对
/// `package`/`settings`，agent 的档位从独立 `variant` 键并入 `model#variant`，
/// 温度从顶层挪到 `request.body`，模型的 `reasoning`/`tool_call`/`modalities`
/// 合并为 `capabilities`，`variants` 从对象变数组。`$schema` 两版相同，只能
/// 靠这些结构特征区分。写回时**跟随目标文件**的版本：给 v1 更新 v1，
/// 给 v2 更新 v2。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum OpencodeSchemaVersion {
    #[default]
    V1,
    V2,
}

impl OpencodeSchemaVersion {
    /// agent 条目自探测：v2 的 `model` 携带 `#variant`，温度在 `request.body`。
    pub fn of_agent(v: &Value) -> Self {
        if v.get("request").is_some() || str_at(v, "model").contains('#') {
            Self::V2
        } else {
            Self::V1
        }
    }

    /// provider 条目自探测：v2 用 `package` / `settings`。
    pub fn of_provider(v: &Value) -> Self {
        if v.get("package").is_some() || v.get("settings").is_some() {
            Self::V2
        } else {
            Self::V1
        }
    }

    /// model 条目自探测：v2 用 `capabilities`。
    pub fn of_model(v: &Value) -> Self {
        if v.get("capabilities").is_some() {
            Self::V2
        } else {
            Self::V1
        }
    }
}

/// v1 / v2 包名互译表：同一协议在两版里名字不同。
const PACKAGE_VERSION_PAIRS: &[(&str, &str)] = &[
    (
        "@ai-sdk/openai-compatible",
        "@opencode/ai/providers/openai-compatible",
    ),
    ("@ai-sdk/anthropic", "@opencode/ai/providers/anthropic"),
    ("@ai-sdk/google", "@opencode/ai/providers/google"),
    ("@ai-sdk/openai", "@opencode/ai/providers/openai"),
    ("@ai-sdk/mistral", "@opencode/ai/providers/mistral"),
];

/// 按目标 schema 版本翻译包名（v1 `@ai-sdk/*` ↔ v2 `@opencode/ai/providers/*`）。
/// 未收录的包名原样返回（自定义包不受版本影响）。UI 下拉写的是 v1 包名，
/// 落盘前统一按目标版本翻译。
fn translate_package(pkg: &str, schema: OpencodeSchemaVersion) -> String {
    for (v1, v2) in PACKAGE_VERSION_PAIRS {
        match schema {
            OpencodeSchemaVersion::V1 if pkg == *v2 => return (*v1).to_string(),
            OpencodeSchemaVersion::V2 if pkg == *v1 => return (*v2).to_string(),
            _ => {}
        }
    }
    pkg.to_string()
}

/// 包名是否等价 opencode 的 openai 缺省兼容层（未显式声明 developer role 时不勾选）。
/// 同时覆盖 v1（@ai-sdk/*）与 v2（@opencode/ai/providers/*）两种写法。
fn is_default_openai_package(pkg: &str) -> bool {
    matches!(
        pkg,
        "" | "@ai-sdk/openai"
            | "@ai-sdk/openai-compatible"
            | "@opencode/ai/providers/openai"
            | "@opencode/ai/providers/openai-compatible"
    )
}

/// 把值写入嵌套路径（逐层建对象；不存在则创建）。
fn set_nested_value(target: &mut Map<String, Value>, path: &[&str], value: Value) {
    let Some((&last, rest)) = path.split_last() else {
        return;
    };
    let mut cursor = target;
    for key in rest {
        if !cursor.get(*key).is_some_and(|v| v.is_object()) {
            cursor.insert((*key).to_string(), Value::Object(Map::new()));
        }
        cursor = cursor
            .get_mut(*key)
            .and_then(Value::as_object_mut)
            .expect("刚刚创建");
    }
    cursor.insert(last.to_string(), value);
}

/// 删除嵌套路径末级键。父级对象即使变空也保留（无害，避免重下钻逻辑）。
fn remove_nested_key(target: &mut Map<String, Value>, path: &[&str]) {
    let Some((&last, rest)) = path.split_last() else {
        return;
    };
    // 逐层下钻；任一层不是对象就无事可做。
    let mut cursor = target;
    for key in rest {
        let Some(next) = cursor.get_mut(*key).and_then(Value::as_object_mut) else {
            return;
        };
        cursor = next;
    }
    cursor.remove(last);
}

fn changed_str(raw: &Value, key: &str, current: &str) -> bool {
    current != str_at(raw, key)
}

fn changed_num(raw: &Value, key: &str, current: &str) -> bool {
    raw.get(key)
        .and_then(Value::as_str)
        .map(|value| value != current)
        .unwrap_or_else(|| current != num_at(raw, key))
}

fn changed_nested_num(raw: &Value, path: &[&str], current: &str) -> bool {
    let mut value = raw;
    for key in path {
        let Some(next) = value.get(*key) else {
            return !current.is_empty();
        };
        value = next;
    }
    value
        .as_str()
        .map(|original| original != current)
        .unwrap_or_else(|| current != nested_num(raw, path))
}

fn changed_nested_str(raw: &Value, path: &[&str], current: &str) -> bool {
    current != nested_str(raw, path)
}

fn changed_list(raw: &Value, path: &[&str], current: &str) -> bool {
    current != nested_list_str(raw, path)
}

fn changed_bool(raw: &Value, key: &str, current: bool) -> bool {
    raw.get(key).is_some() && current != bool_at(raw, key) || raw.get(key).is_none() && current
}

/// 嵌套路径版的 [`changed_bool`]：路径断裂或值非布尔时按 false 处理。
fn changed_nested_bool(raw: &Value, path: &[&str], current: bool) -> bool {
    let mut value = raw;
    for key in path {
        let Some(next) = value.get(*key) else {
            return current;
        };
        value = next;
    }
    current != value.as_bool().unwrap_or(false)
}

fn set_changed_str(m: &mut Map<String, Value>, raw: &Value, key: &str, current: &str) {
    if changed_str(raw, key, current) {
        set_str(m, key, current);
    }
}

#[derive(Clone)]
pub struct AgentRow {
    pub key: String,
    pub mode: String,
    pub description: String,
    pub model: String,
    pub variant: String,
    pub temperature: String,
    pub color: String,
    pub system: String,
    /// 来源文件的 opencode schema 版本：决定 to_value 的写回形状。
    pub schema: OpencodeSchemaVersion,
    pub raw: Value,
}

impl Default for AgentRow {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentRow {
    pub fn from(key: &str, v: &Value) -> Self {
        let schema = OpencodeSchemaVersion::of_agent(v);
        let (model, variant, temperature) = match schema {
            OpencodeSchemaVersion::V1 => (
                str_at(v, "model").to_string(),
                str_at(v, "variant").to_string(),
                num_at(v, "temperature"),
            ),
            // v2 把档位并进 model（`base#variant`，按最后一个 # 拆），
            // 温度挪到 request.body。
            OpencodeSchemaVersion::V2 => {
                let raw_model = str_at(v, "model");
                let (model, variant) = match raw_model.rsplit_once('#') {
                    Some((base, v)) => (base.to_string(), v.to_string()),
                    None => (raw_model.to_string(), String::new()),
                };
                (
                    model,
                    variant,
                    nested_num(v, &["request", "body", "temperature"]),
                )
            }
        };
        Self {
            key: key.to_string(),
            mode: str_at(v, "mode").to_string(),
            description: str_at(v, "description").to_string(),
            model,
            variant,
            temperature,
            color: str_at(v, "color").to_string(),
            system: str_at(v, "system").to_string(),
            schema,
            raw: v.clone(),
        }
    }

    pub fn new() -> Self {
        Self {
            key: String::new(),
            mode: "subagent".into(),
            description: String::new(),
            model: String::new(),
            variant: String::new(),
            temperature: String::new(),
            color: String::new(),
            system: String::new(),
            schema: OpencodeSchemaVersion::default(),
            raw: Value::Object(Map::new()),
        }
    }

    pub fn to_value(&self) -> Value {
        self.to_value_as(self.schema)
    }

    /// 按目标 schema 版本写出。同版本增量写（只写改动字段，保留 raw 键序与
    /// 未动内容）；跨版本视为转换：删源版特有键、按目标形状全量写。
    pub fn to_value_as(&self, schema: OpencodeSchemaVersion) -> Value {
        let cross = self.schema != schema;
        let mut m = self.raw.as_object().cloned().unwrap_or_default();
        // 仅在 UI 值相对原始值发生变化时写入；未修改字段保留原始键、类型和内容。
        if cross {
            set_str(&mut m, "mode", &self.mode);
            set_str(&mut m, "description", &self.description);
            set_str(&mut m, "color", &self.color);
            set_str(&mut m, "system", &self.system);
        } else {
            set_changed_str(&mut m, &self.raw, "mode", &self.mode);
            set_changed_str(&mut m, &self.raw, "description", &self.description);
            set_changed_str(&mut m, &self.raw, "color", &self.color);
            set_changed_str(&mut m, &self.raw, "system", &self.system);
        }
        match schema {
            OpencodeSchemaVersion::V1 => {
                if cross {
                    // v2 → v1：拆掉 request.body，平铺 variant / temperature。
                    m.remove("request");
                    set_str(&mut m, "model", &self.model);
                    set_str(&mut m, "variant", &self.variant);
                } else {
                    set_changed_str(&mut m, &self.raw, "model", &self.model);
                    set_changed_str(&mut m, &self.raw, "variant", &self.variant);
                }
                if cross || changed_num(&self.raw, "temperature", &self.temperature) {
                    match parse_number_text(&self.temperature) {
                        Some(v) => {
                            m.insert("temperature".into(), v);
                        }
                        None => {
                            m.remove("temperature");
                        }
                    }
                }
            }
            OpencodeSchemaVersion::V2 => {
                // v2：档位并入 model（`base#variant`），无独立 variant 键；
                // 温度在 request.body.temperature。
                let merged = if self.variant.is_empty() {
                    self.model.clone()
                } else {
                    format!("{}#{}", self.model, self.variant)
                };
                if cross {
                    m.remove("variant");
                    m.remove("temperature");
                    set_str(&mut m, "model", &merged);
                    match parse_number_text(&self.temperature) {
                        Some(v) => set_nested_value(&mut m, &["request", "body", "temperature"], v),
                        None => remove_nested_key(&mut m, &["request", "body", "temperature"]),
                    }
                } else {
                    if str_at(&self.raw, "model") != merged {
                        m.insert("model".into(), Value::String(merged));
                    }
                    if nested_num(&self.raw, &["request", "body", "temperature"])
                        != self.temperature
                    {
                        match parse_number_text(&self.temperature) {
                            Some(v) => {
                                set_nested_value(&mut m, &["request", "body", "temperature"], v)
                            }
                            None => remove_nested_key(&mut m, &["request", "body", "temperature"]),
                        }
                    }
                }
            }
        }
        Value::Object(m)
    }
}

#[derive(Clone)]
pub struct ModelRow {
    pub id: String,
    pub name: String,
    pub reasoning: bool,
    pub tool_call: bool,
    pub store: bool,
    /// WorkBuddy 专属：关闭的模型不写入配置文件（保存时整条跳过）。
    ///
    /// 同一 id 全局去重，UI 上只允许开一个（开关互斥）；`id` 是发给上游的
    /// 模型名，不改名去重。
    pub disabled: bool,
    pub context: String,
    pub output: String,
    pub modalities_input: String,
    pub modalities_output: String,
    pub variants: String,
    /// 加载时的思考档位投影。
    pub original_variants: String,
    /// raw 所属格式；None 表示在当前页面中新建的条目。
    pub source_format: Option<ConfigFormat>,
    /// 来源文件的 opencode schema 版本：决定写回形状。
    pub schema: OpencodeSchemaVersion,
    pub raw: Value,
    /// KimiCode 专属：模型在顶层 `[models."<alias>"]` 表里的表键。
    ///
    /// Kimi 把模型放在全局表里，键是 alias，表内的 `model` 才是发给上游的 wire id，
    /// 两者可以不同。`id` 只装 wire id（界面显示它），alias 单独存这里，
    /// 写回时用它作表键。新建模型时 alias 缺省 = `id`。
    pub kimi_alias: String,
}

impl Default for ModelRow {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelRow {
    pub fn from(id: &str, v: &Value) -> Self {
        let schema = OpencodeSchemaVersion::of_model(v);
        let (variants, store, reasoning, tool_call, modalities_input, modalities_output) =
            match schema {
                OpencodeSchemaVersion::V1 => {
                    let variants = v
                        .get("variants")
                        .map(|val| {
                            if let Some(obj) = val.as_object() {
                                obj.keys().cloned().collect::<Vec<_>>().join(", ")
                            } else {
                                String::new()
                            }
                        })
                        .unwrap_or_default();
                    let store = v
                        .get("options")
                        .and_then(|o| o.get("store"))
                        .and_then(|s| s.as_bool())
                        .unwrap_or(false);
                    (
                        variants,
                        store,
                        bool_at(v, "reasoning"),
                        bool_at(v, "tool_call"),
                        nested_list_str(v, &["modalities", "input"]),
                        nested_list_str(v, &["modalities", "output"]),
                    )
                }
                // v2：tool_call / 模态并入 capabilities、variants 变数组、
                // store 挪到 body；无 reasoning 键。
                OpencodeSchemaVersion::V2 => {
                    let variants = v
                        .get("variants")
                        .and_then(Value::as_array)
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|item| item.get("id").and_then(Value::as_str))
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    let store = v
                        .get("body")
                        .and_then(|b| b.get("store"))
                        .and_then(|s| s.as_bool())
                        .unwrap_or(false);
                    let tool_call = v
                        .get("capabilities")
                        .and_then(|c| c.get("tools"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    (
                        variants,
                        store,
                        false,
                        tool_call,
                        nested_list_str(v, &["capabilities", "input"]),
                        nested_list_str(v, &["capabilities", "output"]),
                    )
                }
            };
        Self {
            id: id.to_string(),
            name: str_at(v, "name").to_string(),
            reasoning,
            tool_call,
            store,
            disabled: bool_at(v, "disabled"),
            context: nested_num(v, &["limit", "context"]),
            output: nested_num(v, &["limit", "output"]),
            modalities_input,
            modalities_output,
            original_variants: variants.clone(),
            variants,
            source_format: Some(ConfigFormat::Opencode),
            schema,
            raw: v.clone(),
            kimi_alias: String::new(),
        }
    }

    pub fn new() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            // 新增模型的缺省值：默认支持思考与工具调用，预填常用上下文/输出上限、
            // 输入输出模态与思考档位。
            reasoning: true,
            tool_call: true,
            store: false,
            // 新建模型默认启用，`disabled` 只由 WorkBuddy 页的开关显式设置。
            disabled: false,
            context: "262000".into(),
            output: "131000".into(),
            modalities_input: "text, image".into(),
            modalities_output: "text".into(),
            variants: "medium, high, xhigh, max".into(),
            original_variants: String::new(),
            source_format: None,
            schema: OpencodeSchemaVersion::default(),
            raw: Value::Object(Map::new()),
            kimi_alias: String::new(),
        }
    }

    pub fn to_value(&self) -> Value {
        self.to_value_as(self.schema)
    }

    /// 按目标 schema 版本写出。同版本增量写（只更新改动字段，保留 raw 键序与
    /// 未动内容）；跨版本 / 跨格式视为转换：按目标形状全量重写。
    pub fn to_value_as(&self, schema: OpencodeSchemaVersion) -> Value {
        // pi/omp/DSH 来源从干净对象构造；opencode 系来源以 raw 为基底，
        // 并只更新 UI 实际改动过的字段。
        let convert_dialect = self
            .source_format
            .is_some_and(|format| !format.is_opencode_family());
        let cross = convert_dialect || self.schema != schema;
        let raw_empty = raw_object_is_empty(&self.raw);
        let mut m = if convert_dialect {
            Map::new()
        } else {
            self.raw.as_object().cloned().unwrap_or_default()
        };
        // 跨版本：清掉源版特有键，按目标形状重写。
        if cross {
            match schema {
                OpencodeSchemaVersion::V1 => {
                    m.remove("capabilities");
                    m.remove("body");
                }
                OpencodeSchemaVersion::V2 => {
                    m.remove("reasoning");
                    m.remove("tool_call");
                    m.remove("modalities");
                    m.remove("options");
                }
            }
        }

        self.apply_identity(&mut m, cross, schema);
        self.apply_store(&mut m, cross, raw_empty, schema);
        self.apply_limits(&mut m, cross);
        self.apply_modalities(&mut m, cross, schema);
        self.apply_variants(&mut m, cross, schema);

        // 新建模型 / 转换写入时按 opencode 惯例键顺序输出；
        // 同版本已有 raw 的模型保留原有键顺序。
        if cross || raw_empty {
            m = canonical_model_order(m, schema);
        }
        Value::Object(m)
    }

    /// 名称与身份开关（v1：reasoning / tool_call；v2：capabilities.tools）。
    fn apply_identity(
        &self,
        m: &mut Map<String, Value>,
        cross: bool,
        schema: OpencodeSchemaVersion,
    ) {
        if cross {
            set_str(m, "name", &self.name);
        } else {
            set_changed_str(m, &self.raw, "name", &self.name);
        }
        match schema {
            OpencodeSchemaVersion::V1 => {
                if cross {
                    // reasoning/tool_call 是 opencode 专属控件：跨格式来源不继承默认值，
                    // 但用户在 opencode 页明确勾选后仍可写入。
                    if self.reasoning {
                        m.insert("reasoning".into(), true.into());
                    }
                    if self.tool_call {
                        m.insert("tool_call".into(), true.into());
                    }
                } else {
                    if changed_bool(&self.raw, "reasoning", self.reasoning) {
                        m.insert("reasoning".into(), self.reasoning.into());
                    }
                    if changed_bool(&self.raw, "tool_call", self.tool_call) {
                        m.insert("tool_call".into(), self.tool_call.into());
                    }
                }
            }
            OpencodeSchemaVersion::V2 => {
                // v2 无 reasoning；tool_call 位于 capabilities.tools。
                let tools_changed =
                    changed_nested_bool(&self.raw, &["capabilities", "tools"], self.tool_call);
                if cross {
                    // 与 v1 转换分支同策略：仅在明确勾选时创建。
                    if self.tool_call {
                        set_capability(m, "tools", self.tool_call.into());
                    }
                } else if tools_changed {
                    set_capability(m, "tools", self.tool_call.into());
                }
            }
        }
    }

    /// 存储开关（v1：options.store；v2：body.store）。
    /// 新建模型（raw 为空）与既有配置一致写出 false，已有模型只在改动时写入。
    fn apply_store(
        &self,
        m: &mut Map<String, Value>,
        cross: bool,
        raw_empty: bool,
        schema: OpencodeSchemaVersion,
    ) {
        // raw 里的 store 位置跟随来源版本；写出位置跟随目标版本。
        let raw_store = match self.schema {
            OpencodeSchemaVersion::V1 => self.raw.get("options"),
            OpencodeSchemaVersion::V2 => self.raw.get("body"),
        }
        .and_then(|o| o.get("store"))
        .and_then(Value::as_bool);
        let parent_key = match schema {
            OpencodeSchemaVersion::V1 => "options",
            OpencodeSchemaVersion::V2 => "body",
        };
        if !cross && raw_store == Some(self.store) {
            return;
        }
        if self.store {
            let mut parent = m
                .get(parent_key)
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            parent.insert("store".into(), true.into());
            m.insert(parent_key.into(), Value::Object(parent));
        } else if raw_empty && !cross {
            let mut parent = Map::new();
            parent.insert("store".into(), Value::Bool(false));
            m.insert(parent_key.into(), Value::Object(parent));
        } else if let Some(parent) = m.get_mut(parent_key).and_then(Value::as_object_mut) {
            parent.remove("store");
            if parent.is_empty() {
                m.remove(parent_key);
            }
        }
    }

    /// `limit.context` / `limit.output`。
    fn apply_limits(&self, m: &mut Map<String, Value>, cross: bool) {
        let context_changed =
            cross || changed_nested_num(&self.raw, &["limit", "context"], &self.context);
        let output_changed =
            cross || changed_nested_num(&self.raw, &["limit", "output"], &self.output);
        if !context_changed && !output_changed {
            return;
        }
        let mut limit = m
            .get("limit")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if context_changed {
            set_num_opt(&mut limit, "context", &self.context);
        }
        if output_changed {
            set_num_opt(&mut limit, "output", &self.output);
        }
        if limit.is_empty() {
            m.remove("limit");
        } else {
            m.insert("limit".into(), Value::Object(limit));
        }
    }

    /// 模态列表（v1：modalities.input/output；v2：capabilities.input/output）。
    fn apply_modalities(
        &self,
        m: &mut Map<String, Value>,
        cross: bool,
        schema: OpencodeSchemaVersion,
    ) {
        let parent = match schema {
            OpencodeSchemaVersion::V1 => "modalities",
            OpencodeSchemaVersion::V2 => "capabilities",
        };
        let input_changed =
            cross || changed_list(&self.raw, &[parent, "input"], &self.modalities_input);
        let output_changed =
            cross || changed_list(&self.raw, &[parent, "output"], &self.modalities_output);
        if !input_changed && !output_changed {
            return;
        }
        let mut target = m
            .get(parent)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        set_list(&mut target, "input", input_changed, &self.modalities_input);
        set_list(
            &mut target,
            "output",
            output_changed,
            &self.modalities_output,
        );
        if target.is_empty() {
            m.remove(parent);
        } else {
            m.insert(parent.into(), Value::Object(target));
        }
    }

    /// `variants`：按规范顺序写出，并保留 raw 中各档位的原有内容
    /// （v1 对象键集合 ↔ v2 数组 `id` 集合）。
    fn apply_variants(
        &self,
        m: &mut Map<String, Value>,
        cross: bool,
        schema: OpencodeSchemaVersion,
    ) {
        match schema {
            OpencodeSchemaVersion::V1 => {
                let raw_names = self
                    .raw
                    .get("variants")
                    .and_then(Value::as_object)
                    .map(|v| v.keys().cloned().collect::<Vec<_>>().join(", "))
                    .unwrap_or_default();
                if !cross && self.variants == raw_names {
                    return;
                }
                if self.variants.trim().is_empty() {
                    m.remove("variants");
                    return;
                }
                let raw_variants = m
                    .get("variants")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                let variants_map: Map<String, Value> = ordered_variants_text(&self.variants)
                    .into_iter()
                    .map(|name| {
                        let value = raw_variants
                            .get(&name)
                            .cloned()
                            .unwrap_or_else(|| Value::Object(Map::new()));
                        (name, value)
                    })
                    .collect();
                m.insert("variants".into(), Value::Object(variants_map));
            }
            OpencodeSchemaVersion::V2 => {
                // v2：variants 是 `[{id}]` 数组，保留 raw 中同名条目的原有内容。
                let raw_names = self
                    .raw
                    .get("variants")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|item| item.get("id").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                if !cross && self.variants == raw_names {
                    return;
                }
                if self.variants.trim().is_empty() {
                    m.remove("variants");
                    return;
                }
                let raw_items: Vec<Value> = m
                    .get("variants")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let items: Vec<Value> = ordered_variants_text(&self.variants)
                    .into_iter()
                    .map(|name| {
                        raw_items
                            .iter()
                            .find(|item| {
                                item.get("id").and_then(Value::as_str) == Some(name.as_str())
                            })
                            .cloned()
                            .unwrap_or_else(|| {
                                let mut obj = Map::new();
                                obj.insert("id".into(), Value::String(name));
                                Value::Object(obj)
                            })
                    })
                    .collect();
                m.insert("variants".into(), Value::Array(items));
            }
        }
    }
}

/// 写入 capabilities.<key>（保留 capabilities 里已有的其他键）。
fn set_capability(m: &mut Map<String, Value>, key: &str, value: Value) {
    let mut cap = m
        .get("capabilities")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    cap.insert(key.into(), value);
    m.insert("capabilities".into(), Value::Object(cap));
}
/// 逗号分隔的模态列表写入：改动过才写，空串删除该键。
fn set_list(target: &mut Map<String, Value>, key: &str, changed: bool, text: &str) {
    if !changed {
        return;
    }
    let values: Vec<Value> = text
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|value| Value::String(value.to_string()))
        .collect();
    if values.is_empty() {
        target.remove(key);
    } else {
        target.insert(key.into(), Value::Array(values));
    }
}

/// raw 是否为空对象（视为新建条目）。
fn raw_object_is_empty(raw: &Value) -> bool {
    match raw.as_object() {
        Some(obj) => obj.is_empty(),
        None => true,
    }
}

/// opencode 模型的惯例键顺序（未列举的键按原顺序追加在后）。
const MODEL_KEY_ORDER: &[&str] = &[
    "name",
    "modalities",
    "reasoning",
    "tool_call",
    "limit",
    "options",
    "variants",
];

/// v2 形状的惯例键顺序。
const MODEL_KEY_ORDER_V2: &[&str] = &["name", "capabilities", "limit", "body", "variants"];

fn canonical_model_order(
    m: Map<String, Value>,
    schema: OpencodeSchemaVersion,
) -> Map<String, Value> {
    let order: &[&str] = match schema {
        OpencodeSchemaVersion::V1 => MODEL_KEY_ORDER,
        OpencodeSchemaVersion::V2 => MODEL_KEY_ORDER_V2,
    };
    let mut out = Map::new();
    for key in order {
        if let Some(value) = m.get(*key) {
            out.insert((*key).to_string(), value.clone());
        }
    }
    for (key, value) in m {
        if !order.contains(&key.as_str()) {
            out.insert(key, value);
        }
    }
    out
}

#[derive(Clone)]
pub struct ProviderRow {
    pub key: String,
    pub description: String,
    pub npm: String,
    pub base_url: String,
    pub api_key: String,
    /// DSH 中保存于主配置的凭据引用名（apiKeyEnv）。
    pub api_key_env: String,
    /// 加载时的 apiKeyEnv。
    pub original_api_key_env: String,
    /// DSH 同级 `.credentials.yaml` 中 refs 下的实际密钥。
    pub api_key_secret: String,
    /// 加载时的密钥。
    pub original_api_key_secret: String,
    /// DSH provider 的 timeoutMs。
    pub dsh_timeout_ms: String,
    /// DSH provider 的 retryPolicy.mode。
    pub dsh_retry_mode: String,
    /// DSH provider 的 retryPolicy.maxRetries。
    pub dsh_max_retries: String,
    /// 加载时的 DSH provider 参数。
    pub original_dsh_timeout_ms: String,
    pub original_dsh_retry_mode: String,
    pub original_dsh_max_retries: String,
    pub timeout: String,
    /// 加载时的 timeout（含缺省默认化）。
    pub original_timeout: String,
    pub compat: bool,
    /// pi: compat.requiresReasoningContentOnAssistantMessages；
    /// omp: compat.requiresReasoningContentForAllAssistantTurns（相互映射）。
    /// 加载 opencode/dsh 或新建时缺省为 false（不勾选）。
    pub requires_reasoning_content: bool,
    /// 加载时的值，用于同格式未修改时不改写 raw。
    pub original_requires_reasoning_content: bool,
    pub models: Vec<ModelRow>,
    pub new_model: ModelRow,
    /// raw 所属格式；None 表示在当前页面中新建的条目。
    pub source_format: Option<ConfigFormat>,
    /// 来源文件的 opencode schema 版本：决定写回形状。
    pub schema: OpencodeSchemaVersion,
    pub raw: Value,
    pub pi_api: String,
    /// QwenCode 专属：该条目在 `modelProviders` 里的 provider id（map 键）。
    ///
    /// QwenCode 的结构是 `modelProviders[<pid>] = [<模型条目>, ...]`，一个 pid 下
    /// 可以挂多条条目，且每条条目自带 `baseUrl` / `envKey`。界面按「一条 = 一张卡片」
    /// 建模，`key` 只存条目自己的 `id`，本字段记录它属于哪个 pid。
    pub qwen_pid: String,
}

impl Default for ProviderRow {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderRow {
    pub fn from(key: &str, v: &Value) -> Self {
        let schema = OpencodeSchemaVersion::of_provider(v);
        // v1：npm / options；v2：package / settings。键名不同、语义一致，
        // 统一读进内部字段。
        let (pkg_key, opt_key) = match schema {
            OpencodeSchemaVersion::V1 => ("npm", "options"),
            OpencodeSchemaVersion::V2 => ("package", "settings"),
        };
        let npm = str_at(v, pkg_key).to_string();
        let models = v
            .get("models")
            .and_then(|x| x.as_object())
            .map(|o| o.iter().map(|(k, mv)| ModelRow::from(k, mv)).collect())
            .unwrap_or_default();
        let compat = v
            .get("compat")
            .and_then(|c| c.get("supportsDeveloperRole"))
            .and_then(|v| v.as_bool())
            .unwrap_or_else(|| {
                // opencode 缺省包等价 @ai-sdk/openai（chat/completions），
                // 未显式声明 developer role 时不勾选。
                !is_default_openai_package(&npm)
            });
        let row = Self {
            key: key.to_string(),
            description: str_at(v, "description").to_string(),
            npm,
            base_url: nested_str(v, &[opt_key, "baseURL"]).to_string(),
            api_key: nested_str(v, &[opt_key, "apiKey"]).to_string(),
            api_key_env: String::new(),
            original_api_key_env: String::new(),
            api_key_secret: String::new(),
            original_api_key_secret: String::new(),
            // 跨格式保存到 DSH 时写出默认 timeoutMs；同格式未修改不写。
            dsh_timeout_ms: "180000".into(),
            dsh_retry_mode: "normal".into(),
            dsh_max_retries: String::new(),
            original_dsh_timeout_ms: "180000".into(),
            original_dsh_retry_mode: "normal".into(),
            original_dsh_max_retries: String::new(),
            timeout: {
                let t = nested_num(v, &[opt_key, "timeout"]);
                if t.is_empty() {
                    "180000".to_string()
                } else {
                    t
                }
            },
            original_timeout: {
                let t = nested_num(v, &[opt_key, "timeout"]);
                if t.is_empty() {
                    "180000".to_string()
                } else {
                    t
                }
            },
            compat,
            // opencode 无该字段：默认不勾选。
            requires_reasoning_content: false,
            original_requires_reasoning_content: false,
            models,
            new_model: ModelRow::new(),
            source_format: Some(ConfigFormat::Opencode),
            schema,
            raw: v.clone(),
            pi_api: String::new(),
            qwen_pid: String::new(),
        };
        // opencode 的 anthropic-messages 必须带 /v1：读入即补齐，界面显示与落盘一致。
        let api = row.effective_api();
        let base_url = crate::convert::with_v1_for_messages(&api, &row.base_url);
        Self { base_url, ..row }
    }

    /// 生效的 api（线上协议）。UI 显示、写盘、延迟测试共用同一套优先级：
    /// 1. npm 非空 → 按 npm 包推导；
    /// 2. pi_api 非空 → 直接用（若被误写成 `@ai-sdk/...` 包名，按 npm 解释并自愈）；
    /// 3. 原文件（raw）里的 api → 保留；
    /// 4. 都没有 → 默认兼容层 openai-completions。
    pub fn effective_api(&self) -> String {
        if !self.npm.is_empty() {
            // npm 字段可能装 v1 包名或 v2 package 名，统一映射。
            return crate::convert::package_or_npm_to_api(&self.npm);
        }
        let api = self.pi_api.trim();
        if !api.is_empty() {
            return if api.starts_with("@ai-sdk/") {
                crate::convert::npm_to_api(api)
            } else {
                api.to_string()
            };
        }
        let raw_api = str_at(&self.raw, "api");
        if !raw_api.is_empty() {
            return raw_api.to_string();
        }
        "openai-completions".to_string()
    }

    /// 是否显式指定了协议（npm 或 api 任一侧有值），供下拉决定显示「(空)」还是协议名。
    pub fn has_explicit_api(&self) -> bool {
        !self.npm.is_empty() || !self.pi_api.is_empty() || !str_at(&self.raw, "api").is_empty()
    }

    /// 选择「(空)」（不指定协议）：清掉 npm / pi_api，并抹掉 raw 里的 api。
    pub fn clear_api(&mut self) {
        self.npm.clear();
        self.pi_api.clear();
        if let Value::Object(m) = &mut self.raw {
            m.remove("api");
        }
    }

    pub fn new() -> Self {
        Self {
            key: String::new(),
            description: String::new(),
            npm: String::new(),
            base_url: String::new(),
            api_key: String::new(),
            api_key_env: String::new(),
            original_api_key_env: String::new(),
            api_key_secret: String::new(),
            original_api_key_secret: String::new(),
            // 新建 provider 的默认值：跨格式保存时也写入目标文件的默认字段。
            dsh_timeout_ms: "180000".into(),
            dsh_retry_mode: "normal".into(),
            dsh_max_retries: String::new(),
            original_dsh_timeout_ms: "180000".into(),
            original_dsh_retry_mode: "normal".into(),
            original_dsh_max_retries: String::new(),
            timeout: "180000".into(),
            original_timeout: "".into(),
            compat: false,
            requires_reasoning_content: false,
            original_requires_reasoning_content: false,
            models: Vec::new(),
            new_model: ModelRow::new(),
            source_format: None,
            schema: OpencodeSchemaVersion::default(),
            raw: Value::Object(Map::new()),
            pi_api: String::new(),
            qwen_pid: String::new(),
        }
    }

    pub fn to_value(&self) -> Value {
        self.to_value_as(self.schema)
    }

    /// 按目标 schema 版本写出：v1 用 npm / options，v2 用 package / settings。
    /// 包名按目标版本互译（UI 下拉写的是 v1 包名）；同版本增量写，
    /// 跨版本 / 跨格式视为转换。
    pub fn to_value_as(&self, schema: OpencodeSchemaVersion) -> Value {
        // pi/omp/DSH 来源全新构造；opencode 系来源以 raw 为基底，
        // 只更新发生变化的 provider 字段。
        let convert_dialect = self
            .source_format
            .is_some_and(|format| !format.is_opencode_family());
        let cross = convert_dialect || self.schema != schema;
        let mut m = if convert_dialect {
            Map::new()
        } else {
            self.raw.as_object().cloned().unwrap_or_default()
        };
        let (pkg_key, opt_key) = match schema {
            OpencodeSchemaVersion::V1 => ("npm", "options"),
            OpencodeSchemaVersion::V2 => ("package", "settings"),
        };
        // 跨版本：清掉源版键，按目标形状重写。
        if cross {
            match schema {
                OpencodeSchemaVersion::V1 => {
                    m.remove("package");
                    m.remove("settings");
                }
                OpencodeSchemaVersion::V2 => {
                    m.remove("npm");
                    m.remove("options");
                }
            }
            set_str(&mut m, "description", &self.description);
        } else {
            set_changed_str(&mut m, &self.raw, "description", &self.description);
        }
        let pkg = translate_package(&self.npm, schema);
        if cross {
            set_str(&mut m, pkg_key, &pkg);
        } else {
            set_changed_str(&mut m, &self.raw, pkg_key, &pkg);
        }
        let base_changed =
            cross || changed_nested_str(&self.raw, &[opt_key, "baseURL"], &self.base_url);
        let key_changed =
            cross || changed_nested_str(&self.raw, &[opt_key, "apiKey"], &self.api_key);
        let timeout_changed = cross || self.timeout != self.original_timeout;
        if base_changed || key_changed || timeout_changed {
            let mut options = m
                .get(opt_key)
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if base_changed {
                // opencode 的 anthropic-messages 必须带 /v1。
                let api = self.effective_api();
                let base_url = crate::convert::with_v1_for_messages(&api, &self.base_url);
                set_str(&mut options, "baseURL", &base_url);
            }
            if key_changed {
                set_str(&mut options, "apiKey", &self.api_key);
            }
            if timeout_changed {
                set_num_opt(&mut options, "timeout", &self.timeout);
            }
            if options.is_empty() {
                m.remove(opt_key);
            } else {
                m.insert(opt_key.into(), Value::Object(options));
            }
        }
        let mut models = Map::new();
        for mdl in &self.models {
            models.insert(mdl.id.clone(), mdl.to_value_as(schema));
        }
        m.insert("models".into(), Value::Object(models));
        Value::Object(m)
    }
}
