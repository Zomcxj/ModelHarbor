use serde_json::Value;

/// models.dev 里的一个免费模型。
#[derive(Debug)]
pub struct FreeModel {
    pub id: String,
    /// 数据源是否已标记其下架（保守标记，不代表网关已停供）。
    pub deprecated: bool,
}

/// 该模型是否「价格为零」。
///
/// 价格必须是**显式的 0**：字段缺失说明数据源还没收录价格，按收费处理。
fn is_zero_cost(model: &Value) -> bool {
    let zero = |key: &str| {
        model
            .get("cost")
            .and_then(|cost| cost.get(key))
            .and_then(Value::as_f64)
            == Some(0.0)
    };
    zero("input") && zero("output")
}

/// 解析响应 JSON，失败时给出**带开头片段**的错误。
///
/// 片段只取 120 个字符：models.dev 的正文约 4.8 MB，整个塞进提示里既没人看也拖慢界面。
/// 三个解析器（models.dev / 网关 `/models` / Kilo 网关）共用这一段——错误文案必须一致，
/// 否则同一个网络故障在不同后端下会显示成不同的话。
fn parse_json(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|err| {
        let snippet = text.chars().take(120).collect::<String>();
        format!("响应不是合法 JSON（{}）：{}", err, snippet)
    })
}

/// 取响应里的 `data` 数组（OpenAI 风格的 `/models` 响应）。
fn data_items(root: &Value) -> Result<&Vec<Value>, String> {
    root.get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "响应里没有 data 数组".to_string())
}

/// 从 `data[].id` 收集模型 id，排序去重。
fn ids_from_data(items: &[Value], keep: impl Fn(&Value) -> bool) -> Vec<String> {
    let mut ids: Vec<String> = items
        .iter()
        .filter(|item| keep(item))
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// 从 models.dev 的 `api.json` 全文里筛出指定厂商的免费模型。
pub fn parse_models_dev_free(text: &str, provider: &str) -> Result<Vec<FreeModel>, String> {
    let root = parse_json(text)?;
    let provider = root
        .get(provider)
        .ok_or_else(|| format!("数据里没有 {} 厂商", provider))?;
    let models = provider
        .get("models")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{} 厂商下没有 models", provider))?;
    let mut found: Vec<FreeModel> = models
        .iter()
        .filter(|(_, model)| is_zero_cost(model))
        .map(|(id, model)| FreeModel {
            id: id.clone(),
            deprecated: model.get("status").and_then(Value::as_str) == Some("deprecated"),
        })
        .collect();
    found.sort_by(|a, b| a.id.cmp(&b.id));
    found.dedup_by(|a, b| a.id == b.id);
    Ok(found)
}

/// 解析 OpenAI 风格 `/models` 响应里的 id（`data[].id`）。
pub fn parse_live_models(text: &str) -> Result<Vec<String>, String> {
    let root = parse_json(text)?;
    Ok(ids_from_data(data_items(&root)?, |_| true))
}

/// 解析 Kilo 网关响应里 `isFree == true` 的模型 id。
///
/// 用响应自带的 `isFree` 字段而不是「id 以 `:free` 结尾」或价格推断：
/// `kilo-auto/free` 与 `openrouter/free` 两个免费项并不带 `:free` 后缀，
/// 按后缀筛会漏掉它们。
pub fn parse_kilo_free(text: &str) -> Result<Vec<String>, String> {
    let root = parse_json(text)?;
    Ok(ids_from_data(data_items(&root)?, |item| {
        item.get("isFree").and_then(Value::as_bool) == Some(true)
    }))
}

/// 合并两个数据源：`live` 为 `Some` 时取交集（免费且网关在供）；
/// 为 `None`（网关请求失败）时退回「只信 models.dev，并排除 deprecated」。
pub(super) fn combine(free: Vec<FreeModel>, live: Option<&[String]>) -> Vec<String> {
    let mut ids: Vec<String> = match live {
        Some(live) => free
            .into_iter()
            .filter(|model| live.iter().any(|id| id == &model.id))
            .map(|model| model.id)
            .collect(),
        None => free
            .into_iter()
            .filter(|model| !model.deprecated)
            .map(|model| model.id)
            .collect(),
    };
    ids.sort();
    ids.dedup();
    ids
}
