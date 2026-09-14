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
        let mut models = Map::new();
        for mdl in &self.models {
            models.insert(mdl.id.clone(), mdl.to_value());
        }
        m.insert("models".into(), Value::Object(models));
        Value::Object(m)
    }
}
