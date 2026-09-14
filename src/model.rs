use crate::format::ConfigFormat;
use crate::util::{
    bool_at, nested_list_str, nested_num, nested_str, num_at, parse_number_text, set_num_opt,
    set_str, str_at,
};
use serde_json::{Map, Value};

/// 推理档位的规范顺序（各方言并集）：写入时按此排序，删除后重新勾选
/// 也会回到原本位置，而不是被追加到末尾。
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
    pub raw: Value,
}

impl Default for AgentRow {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentRow {
    pub fn from(key: &str, v: &Value) -> Self {
        let row = Self {
            key: key.to_string(),
            mode: str_at(v, "mode").to_string(),
            description: str_at(v, "description").to_string(),
            model: str_at(v, "model").to_string(),
            variant: str_at(v, "variant").to_string(),
            temperature: num_at(v, "temperature"),
            color: str_at(v, "color").to_string(),
            system: str_at(v, "system").to_string(),
            raw: v.clone(),
        };
        row
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
            raw: Value::Object(Map::new()),
        }
    }

    pub fn to_value(&self) -> Value {
        let mut m = self.raw.as_object().cloned().unwrap_or_default();
        // 仅在 UI 值相对原始值发生变化时写入；未修改字段保留原始键、类型和内容。
        set_changed_str(&mut m, &self.raw, "mode", &self.mode);
        set_changed_str(&mut m, &self.raw, "description", &self.description);
        set_changed_str(&mut m, &self.raw, "model", &self.model);
        set_changed_str(&mut m, &self.raw, "variant", &self.variant);
        set_changed_str(&mut m, &self.raw, "color", &self.color);
        set_changed_str(&mut m, &self.raw, "system", &self.system);
        if changed_num(&self.raw, "temperature", &self.temperature) {
            match parse_number_text(&self.temperature) {
                Some(v) => {
                    m.insert("temperature".into(), v);
                }
                None => {
                    m.remove("temperature");
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
    pub context: String,
    pub output: String,
    pub modalities_input: String,
    pub modalities_output: String,
    pub variants: String,
    /// 加载时的思考档位投影，用于区分跨格式继承值与用户在目标页的手动输入。
    pub original_variants: String,
    /// 未在界面建模的方言字段（JSON 文本），见 [`advanced_json_of`]。
    pub advanced: String,
    /// 加载时的同名字段，用于「未编辑不写回」。
    pub original_advanced: String,
    /// raw 所属格式；None 表示在当前页面中新建的条目。
    pub source_format: Option<ConfigFormat>,
    pub raw: Value,
}

impl Default for ModelRow {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelRow {
    pub fn from(id: &str, v: &Value) -> Self {
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
        let advanced = advanced_json_of(v, MODEL_UI_KEYS);
        Self {
            id: id.to_string(),
            name: str_at(v, "name").to_string(),
            reasoning: bool_at(v, "reasoning"),
            tool_call: bool_at(v, "tool_call"),
            store,
            context: nested_num(v, &["limit", "context"]),
            output: nested_num(v, &["limit", "output"]),
            modalities_input: nested_list_str(v, &["modalities", "input"]),
            modalities_output: nested_list_str(v, &["modalities", "output"]),
            original_variants: variants.clone(),
            variants,
            advanced: advanced.clone(),
            original_advanced: advanced,
            source_format: Some(ConfigFormat::Opencode),
            raw: v.clone(),
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
            context: "272000".into(),
            output: "128000".into(),
            modalities_input: "text, image".into(),
            modalities_output: "text".into(),
            variants: "medium, high, xhigh, max".into(),
            original_variants: String::new(),
            advanced: String::new(),
            original_advanced: String::new(),
            source_format: None,
            raw: Value::Object(Map::new()),
        }
    }

    pub fn to_value(&self) -> Value {
        // pi/omp/DSH 来源需要转换方言，因此从干净对象构造；opencode 来源
        // 则以 raw 为基底，并只更新 UI 实际改动过的字段。
        let convert_dialect = self
            .source_format
            .is_some_and(|format| format != ConfigFormat::Opencode);
        let raw_empty = raw_object_is_empty(&self.raw);
        let mut m = if convert_dialect {
            Map::new()
        } else {
            self.raw.as_object().cloned().unwrap_or_default()
        };

        self.apply_identity(&mut m, convert_dialect);
        self.apply_store(&mut m, convert_dialect, raw_empty);
        self.apply_limits(&mut m, convert_dialect);
        self.apply_modalities(&mut m, convert_dialect);
        self.apply_variants(&mut m, convert_dialect);

        // 高级字段（未建模的方言键，如 cost / samplingParams / headers / tokenizer）：
        // 只在用户改过文本且文本合法时写回；跨格式转换不改写，
        // 避免把源方言专属键带进目标格式（沿用既有转换语义）。
        if !convert_dialect {
            merge_advanced_model(&mut m, self);
        }

        // 新建模型 / 跨格式写入时按 opencode 惯例键顺序输出（name、modalities、
        // reasoning、tool_call、limit、options、variants），与配置文件保持一致；
        // 同格式已有 raw 的模型保留原有键顺序，避免无意义的整文件重排。
        if convert_dialect || raw_empty {
            m = canonical_model_order(m);
        }
        Value::Object(m)
    }

    /// 名称与 opencode 专属开关（reasoning / tool_call）。
    fn apply_identity(&self, m: &mut Map<String, Value>, convert_dialect: bool) {
        if convert_dialect {
            set_str(m, "name", &self.name);
            // reasoning/tool_call 是 opencode 专属控件。跨格式来源不继承默认值，
            // 但用户在 opencode 页明确勾选后仍可写入。
            if self.reasoning {
                m.insert("reasoning".into(), true.into());
            }
            if self.tool_call {
                m.insert("tool_call".into(), true.into());
            }
            return;
        }
        set_changed_str(m, &self.raw, "name", &self.name);
        if changed_bool(&self.raw, "reasoning", self.reasoning) {
            m.insert("reasoning".into(), self.reasoning.into());
        }
        if changed_bool(&self.raw, "tool_call", self.tool_call) {
            m.insert("tool_call".into(), self.tool_call.into());
        }
    }

    /// `options.store`：新建模型（raw 为空）与既有配置一致写出 false，
    /// 已有模型仍只在改动时写入，保持最小 diff。
    fn apply_store(&self, m: &mut Map<String, Value>, convert_dialect: bool, raw_empty: bool) {
        let raw_store = self
            .raw
            .get("options")
            .and_then(|o| o.get("store"))
            .and_then(Value::as_bool);
        if !convert_dialect && raw_store == Some(self.store) {
            return;
        }
        if self.store {
            let mut options = m
                .get("options")
                .and_then(|o| o.as_object())
                .cloned()
                .unwrap_or_default();
            options.insert("store".into(), true.into());
            m.insert("options".into(), Value::Object(options));
        } else if raw_empty && !convert_dialect {
            let mut options = Map::new();
            options.insert("store".into(), Value::Bool(false));
            m.insert("options".into(), Value::Object(options));
        } else if let Some(options) = m.get_mut("options").and_then(Value::as_object_mut) {
            options.remove("store");
            if options.is_empty() {
                m.remove("options");
            }
        }
    }

    /// `limit.context` / `limit.output`。
    fn apply_limits(&self, m: &mut Map<String, Value>, convert_dialect: bool) {
        let context_changed =
            convert_dialect || changed_nested_num(&self.raw, &["limit", "context"], &self.context);
        let output_changed =
            convert_dialect || changed_nested_num(&self.raw, &["limit", "output"], &self.output);
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

    /// `modalities.input` / `modalities.output`（逗号分隔列表）。
    fn apply_modalities(&self, m: &mut Map<String, Value>, convert_dialect: bool) {
        let input_changed = convert_dialect
            || changed_list(&self.raw, &["modalities", "input"], &self.modalities_input);
        let output_changed = convert_dialect
            || changed_list(
                &self.raw,
                &["modalities", "output"],
                &self.modalities_output,
            );
        if !input_changed && !output_changed {
            return;
        }
        let mut modalities = m
            .get("modalities")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        set_list(
            &mut modalities,
            "input",
            input_changed,
            &self.modalities_input,
        );
        set_list(
            &mut modalities,
            "output",
            output_changed,
            &self.modalities_output,
        );
        if modalities.is_empty() {
            m.remove("modalities");
        } else {
            m.insert("modalities".into(), Value::Object(modalities));
        }
    }

    /// `variants`：按规范顺序写出，并保留 raw 中各档位的原有内容。
    fn apply_variants(&self, m: &mut Map<String, Value>, convert_dialect: bool) {
        let raw_names = self
            .raw
            .get("variants")
            .and_then(Value::as_object)
            .map(|v| v.keys().cloned().collect::<Vec<_>>().join(", "))
            .unwrap_or_default();
        if !convert_dialect && self.variants == raw_names {
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

/// 界面已建模的 model 键（各方言并集）：其余键归入「高级字段」JSON 编辑区。
///
/// 新增建模字段时必须同步这里，否则该字段会在高级区重复出现（保存仍最小 diff，
/// 但界面会同时存在两个入口）。
const MODEL_UI_KEYS: &[&str] = &[
    "id",
    "name",
    "api",
    "baseUrl",
    "provider",
    "reasoning",
    "tool_call",
    "options",
    "limit",
    "modalities",
    "variants",
    "thinkingLevelMap",
    "thinking",
    "reasoningEfforts",
    "contextWindow",
    "maxTokens",
    "input",
];

/// 界面已建模的 provider 键（各方言并集）。
const PROVIDER_UI_KEYS: &[&str] = &[
    "description",
    "npm",
    "baseUrl",
    "baseURL",
    "apiKey",
    "apiKeyEnv",
    "api",
    "timeout",
    "timeoutMs",
    "options",
    "models",
    "retryPolicy",
    "compat",
];

/// 从 raw 中摘出未建模字段并格式化为 JSON 文本（无高级字段时返回空串，
/// 让界面保持安静；键序沿用 raw 原序，保存时不会触发无意义重排）。
fn advanced_json_of(raw: &Value, ui_keys: &[&str]) -> String {
    let Some(obj) = raw.as_object() else {
        return String::new();
    };
    let mut rest = Map::new();
    for (k, v) in obj {
        if !ui_keys.contains(&k.as_str()) {
            rest.insert(k.clone(), v.clone());
        }
    }
    if rest.is_empty() {
        return String::new();
    }
    serde_json::to_string_pretty(&Value::Object(rest)).unwrap_or_default()
}

/// 从方言 raw 摘出 model 的高级字段（供 convert / backends 构造 ModelRow 时复用）。
pub fn advanced_json_for_model(raw: &Value) -> String {
    advanced_json_of(raw, MODEL_UI_KEYS)
}

/// 从方言 raw 摘出 provider 的高级字段。
pub fn advanced_json_for_provider(raw: &Value) -> String {
    advanced_json_of(raw, PROVIDER_UI_KEYS)
}

/// 把 Row 上「高级字段」的改动合并进以 raw 为基底的目标对象。
///
/// 未改动或文本非法时不做任何事（保证最小 diff、不破坏配置）。
/// pi / omp / DSH 的写出路径都以 raw 为基底，因此各自需要显式调用本函数；
/// opencode 路径已在 `ModelRow::to_value` 内部调用。
pub fn merge_advanced_model(obj: &mut Map<String, Value>, row: &ModelRow) {
    if row.advanced == row.original_advanced {
        return;
    }
    if let (Ok(original), Ok(edited)) = (
        parse_advanced_json(&row.original_advanced),
        parse_advanced_json(&row.advanced),
    ) {
        merge_advanced(obj, &original, &edited);
    }
}

/// 把 provider 的「高级字段」改动合并进以 raw 为基底的目标对象（语义同
/// [`merge_advanced_model`]）。
pub fn merge_advanced_provider(obj: &mut Map<String, Value>, row: &ProviderRow) {
    if row.advanced == row.original_advanced {
        return;
    }
    if let (Ok(original), Ok(edited)) = (
        parse_advanced_json(&row.original_advanced),
        parse_advanced_json(&row.advanced),
    ) {
        merge_advanced(obj, &original, &edited);
    }
}

/// 解析「高级字段」文本：空文本表示没有高级字段；必须是 JSON 对象。
pub fn parse_advanced_json(text: &str) -> Result<Map<String, Value>, String> {
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(obj)) => Ok(obj),
        Ok(_) => Err("高级字段必须是一个 JSON 对象（以 { 开始）".to_string()),
        Err(e) => Err(format!("JSON 解析失败: {}", e)),
    }
}

/// 把编辑后的高级字段合并进目标对象：
/// 先移除原高级键，再写入新值 —— 用户删掉的键会被真正移除，
/// 而界面建模的键（未出现在这里）不受影响。
fn merge_advanced(
    m: &mut Map<String, Value>,
    original: &Map<String, Value>,
    edited: &Map<String, Value>,
) {
    for key in original.keys() {
        if !edited.contains_key(key) {
            m.remove(key);
        }
    }
    for (k, v) in edited {
        m.insert(k.clone(), v.clone());
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

fn canonical_model_order(m: Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for key in MODEL_KEY_ORDER {
        if let Some(value) = m.get(*key) {
            out.insert((*key).to_string(), value.clone());
        }
    }
    for (key, value) in m {
        if !MODEL_KEY_ORDER.contains(&key.as_str()) {
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
    /// 加载时的 apiKeyEnv，用于清理重命名后的旧 ref。
    pub original_api_key_env: String,
    /// DSH 同级 `.credentials.yaml` 中 refs 下的实际密钥。
    pub api_key_secret: String,
    /// 加载时的密钥，用于区分“原本缺失”与“用户明确清空”。
    pub original_api_key_secret: String,
    /// DSH provider 的 timeoutMs。
    pub dsh_timeout_ms: String,
    /// DSH provider 的 retryPolicy.mode。
    pub dsh_retry_mode: String,
    /// DSH provider 的 retryPolicy.maxRetries。
    pub dsh_max_retries: String,
    /// 加载时的 DSH provider 参数，用于只保存用户实际修改的字段。
    pub original_dsh_timeout_ms: String,
    pub original_dsh_retry_mode: String,
    pub original_dsh_max_retries: String,
    pub timeout: String,
    /// 加载时的 timeout（含缺省默认化），用于区分“未动过”与“用户修改”。
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
    pub raw: Value,
    pub pi_api: String,
    /// 未在界面建模的方言字段（JSON 文本），见 [`advanced_json_of`]。
    pub advanced: String,
    /// 加载时的同名字段，用于「未编辑不写回」。
    pub original_advanced: String,
}

impl Default for ProviderRow {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderRow {
    pub fn from(key: &str, v: &Value) -> Self {
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
                // opencode 缺省 npm 等价于 @ai-sdk/openai（chat/completions）；
                // 这类 api 不支持 developer role，没有显式声明时不勾选。
                !matches!(
                    str_at(v, "npm"),
                    "" | "@ai-sdk/openai" | "@ai-sdk/openai-compatible"
                )
            });
        let advanced = advanced_json_of(v, PROVIDER_UI_KEYS);
        let row = Self {
            key: key.to_string(),
            description: str_at(v, "description").to_string(),
            npm: str_at(v, "npm").to_string(),
            base_url: nested_str(v, &["options", "baseURL"]).to_string(),
            api_key: nested_str(v, &["options", "apiKey"]).to_string(),
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
                let t = nested_num(v, &["options", "timeout"]);
                if t.is_empty() {
                    "180000".to_string()
                } else {
                    t
                }
            },
            original_timeout: {
                let t = nested_num(v, &["options", "timeout"]);
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
            raw: v.clone(),
            pi_api: String::new(),
            advanced: advanced.clone(),
            original_advanced: advanced,
        };
        // opencode 的 anthropic-messages 必须带 /v1：读入即补齐，界面显示与落盘一致。
        let api = row.effective_api();
        let base_url = crate::convert::with_v1_for_messages(&api, &row.base_url);
        Self { base_url, ..row }
    }

    /// 生效的 api（线上协议）。UI 显示、写盘、延迟测试共用同一套优先级，避免三处口径不一致：
    /// 1. npm 非空 → 按 npm 包推导（opencode 侧以 npm 表达协议）；
    /// 2. pi_api 非空 → 直接用（若被误写成 `@ai-sdk/...` 包名，按 npm 解释并自愈）；
    /// 3. 原文件（raw）里的 api → 保留，兼容 pi-messages / google-vertex 等无 npm 对应的协议；
    /// 4. 都没有 → 默认兼容层 openai-completions（api 字段始终会写出，不产生非法配置）。
    pub fn effective_api(&self) -> String {
        if !self.npm.is_empty() {
            return crate::convert::npm_to_api(&self.npm);
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
    /// 与 [`Self::effective_api`] 的取值口径一致：二者都认为"没写"才是空。
    pub fn has_explicit_api(&self) -> bool {
        !self.npm.is_empty() || !self.pi_api.is_empty() || !str_at(&self.raw, "api").is_empty()
    }

    /// 选择「(空)」（不指定协议）：清掉 npm / pi_api，并抹掉 raw 里的 api。
    /// raw 必须一并清理，否则 [`Self::effective_api`] 会从 raw 回退把旧协议写回去，
    /// 造成界面显示「(空)」而落盘仍是旧协议。
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
            original_timeout: "180000".into(),
            compat: false,
            requires_reasoning_content: false,
            original_requires_reasoning_content: false,
            models: Vec::new(),
            new_model: ModelRow::new(),
            source_format: None,
            raw: Value::Object(Map::new()),
            pi_api: String::new(),
            advanced: String::new(),
            original_advanced: String::new(),
        }
    }

    pub fn to_value(&self) -> Value {
        // pi/omp/DSH 来源全新构造，防止方言键泄漏进 opencode；opencode
        // 来源则以 raw 为基底，只更新发生变化的 provider 字段。
        let convert_dialect = self
            .source_format
            .is_some_and(|format| format != ConfigFormat::Opencode);
        let mut m = if convert_dialect {
            Map::new()
        } else {
            self.raw.as_object().cloned().unwrap_or_default()
        };
        if convert_dialect {
            // opencode 专属字段只在用户于 opencode 页明确填写时创建。
            set_str(&mut m, "description", &self.description);
            set_str(&mut m, "npm", &self.npm);
        } else {
            set_changed_str(&mut m, &self.raw, "description", &self.description);
            set_changed_str(&mut m, &self.raw, "npm", &self.npm);
        }
        let base_changed = convert_dialect
            || changed_nested_str(&self.raw, &["options", "baseURL"], &self.base_url);
        let key_changed =
            convert_dialect || changed_nested_str(&self.raw, &["options", "apiKey"], &self.api_key);
        let timeout_changed = convert_dialect || self.timeout != self.original_timeout;
        if base_changed || key_changed || timeout_changed {
            let mut options = m
                .get("options")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if base_changed {
                // opencode 的 anthropic-messages 必须带 /v1：跨格式写入时源值可能不带（pi / omp / dsh
                // 读入会去掉 /v1），此处统一补齐；同格式且未改动时 base_changed 为 false，不会产生多余 diff。
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
                m.remove("options");
            } else {
                m.insert("options".into(), Value::Object(options));
            }
        }
        // 高级字段（如 omp 的 auth / authHeader / discovery / transport）：
        // 同样只在用户改过文本且文本合法时写回，跨格式转换不改写。
        if !convert_dialect {
            merge_advanced_provider(&mut m, self);
        }
        let mut models = Map::new();
        for mdl in &self.models {
            models.insert(mdl.id.clone(), mdl.to_value());
        }
        m.insert("models".into(), Value::Object(models));
        Value::Object(m)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        advanced_json_for_model, advanced_json_for_provider, parse_advanced_json, ModelRow,
    };
    use crate::backends::oh_my_pi::provider_to_omp;
    use crate::convert::{model_from_pi, model_to_pi, provider_from_pi};
    use serde_json::{json, Value};

    /// pi 方言 model（无 limit/modalities/options/variants → 非 opencode 形状）。
    fn pi_model_raw() -> Value {
        json!({
            "id": "m1",
            "name": "M1",
            "api": "openai-completions",
            "baseUrl": "https://api.example.com/v1",
            "reasoning": true,
            "contextWindow": 1000,
            "maxTokens": 100,
            "cost": {"input": 1.5, "output": 2.5},
            "samplingParams": {"temperature": 0.4}
        })
    }

    /// pi 方言 provider（models 是数组，不是对象 → 非 opencode 形状）。
    fn pi_provider_raw() -> Value {
        json!({
            "baseUrl": "https://api.example.com/v1",
            "apiKey": "sk-test",
            "api": "openai-completions",
            "auth": "none",
            "discovery": {"type": "ollama"},
            "models": [pi_model_raw()]
        })
    }

    #[test]
    fn advanced_json_excludes_ui_keys() {
        let model = advanced_json_for_model(&pi_model_raw());
        assert!(model.contains("\"cost\""), "cost 应归入高级字段: {model}");
        assert!(model.contains("samplingParams"));
        assert!(
            !model.contains("contextWindow"),
            "已建模字段不应出现: {model}"
        );
        assert!(!model.contains("\"name\""));

        let provider = advanced_json_for_provider(&pi_provider_raw());
        assert!(provider.contains("\"auth\""));
        assert!(provider.contains("\"discovery\""));
        assert!(!provider.contains("\"baseUrl\""));
        assert!(!provider.contains("\"models\""));
    }

    #[test]
    fn parse_advanced_json_accepts_empty_and_objects() {
        assert!(parse_advanced_json("   ").unwrap().is_empty());
        assert_eq!(parse_advanced_json("{\"cost\": 1}").unwrap().len(), 1);
        assert!(parse_advanced_json("{").is_err(), "非法 JSON 应报错");
        assert!(parse_advanced_json("[1, 2]").is_err(), "数组不是对象");
        assert!(parse_advanced_json("42").is_err());
    }

    #[test]
    fn pi_model_round_trip_keeps_unedited_advanced() {
        let raw = pi_model_raw();
        let row = model_from_pi(&raw);
        let out = model_to_pi(&row);
        assert_eq!(out.get("cost"), raw.get("cost"));
        assert_eq!(out.get("samplingParams"), raw.get("samplingParams"));
    }

    #[test]
    fn pi_model_round_trip_applies_edited_advanced() {
        let mut row = model_from_pi(&pi_model_raw());
        // 用户改掉 cost，并删掉了 samplingParams
        row.advanced = "{\"cost\": {\"input\": 9.0}}".to_string();
        let out = model_to_pi(&row);
        assert_eq!(out["cost"]["input"], json!(9.0));
        assert!(out.get("samplingParams").is_none(), "被删除的高级键应移除");
        // 已建模字段不受影响
        assert_eq!(out["contextWindow"], json!(1000));
    }

    #[test]
    fn pi_model_round_trip_ignores_invalid_advanced() {
        let mut row = model_from_pi(&pi_model_raw());
        row.advanced = "{ 这不是合法 JSON".to_string();
        let out = model_to_pi(&row);
        // 非法文本不写盘、不破坏 raw
        assert_eq!(out.get("cost"), pi_model_raw().get("cost"));
    }

    #[test]
    fn opencode_same_format_merge_is_minimal() {
        let raw = json!({
            "name": "M",
            "reasoning": true,
            "limit": {"context": 1000},
            "cost": {"input": 1.0},
            "attachment": true
        });
        let mut row = ModelRow::from("m", &raw);
        // 未编辑 → 高级键原样保留
        let untouched = row.to_value();
        assert_eq!(untouched.get("cost"), raw.get("cost"));
        assert_eq!(untouched.get("attachment"), raw.get("attachment"));

        row.advanced = "{\"cost\": {\"input\": 4.0}}".to_string();
        let edited = row.to_value();
        assert_eq!(edited["cost"]["input"], json!(4.0));
        assert!(edited.get("attachment").is_none(), "删除的高级键应移除");
        assert_eq!(edited["name"], json!("M"));
        assert_eq!(edited["limit"]["context"], json!(1000));
    }

    #[test]
    fn omp_provider_round_trip_applies_edited_advanced() {
        let mut row = provider_from_pi("p1", &pi_provider_raw());
        row.advanced = "{\"auth\": \"oauth\"}".to_string();
        let out = provider_to_omp(&row);
        assert_eq!(out["auth"], json!("oauth"));
        assert!(out.get("discovery").is_none(), "被删除的高级键应移除");
        assert_eq!(out["apiKey"], json!("sk-test"));
    }

    #[test]
    fn cross_format_does_not_leak_advanced() {
        // opencode 形状的 row 转到 pi：高级字段（opencode 专属键）不应被带过去
        let raw = json!({
            "name": "M",
            "limit": {"context": 1000},
            "attachment": true
        });
        let mut row = ModelRow::from("m", &raw);
        row.advanced = "{\"attachment\": false}".to_string();
        let out = model_to_pi(&row);
        assert!(out.get("attachment").is_none(), "跨格式转换不写入高级字段");
    }
}
