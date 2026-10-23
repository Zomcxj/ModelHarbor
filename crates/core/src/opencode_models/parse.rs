use serde_json::Value;

/// A free model in models.dev.
#[derive(Debug)]
pub struct FreeModel {
    pub id: String,
    /// Whether the data source has flagged it delisted (a conservative flag — not proof the gateway stopped serving).
    pub deprecated: bool,
}

/// Whether the model's "price is zero".
///
/// Prices must be explicit zeros; missing fields count as paid.
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

/// Parse the response JSON, attaching a leading excerpt to errors.
///
/// The excerpt takes 120 characters. All three parsers share it, keeping error wording uniform.
fn parse_json(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|err| {
        let snippet = text.chars().take(120).collect::<String>();
        format!("响应不是合法 JSON（{}）：{}", err, snippet)
    })
}

/// Take the response's `data` array (OpenAI-style `/models` responses).
fn data_items(root: &Value) -> Result<&Vec<Value>, String> {
    root.get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "响应里没有 data 数组".to_string())
}

/// Collect model ids from `data[].id`, sorted and deduped.
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

/// Filter a vendor's free models out of models.dev's full `api.json`.
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

/// Parse ids out of an OpenAI-style `/models` response (`data[].id`).
pub fn parse_live_models(text: &str) -> Result<Vec<String>, String> {
    let root = parse_json(text)?;
    Ok(ids_from_data(data_items(&root)?, |_| true))
}

/// Parse model ids with `isFree == true` out of a Kilo gateway response.
///
/// Filters by the response's own `isFree` field (never by id suffix or price inference).
pub fn parse_kilo_free(text: &str) -> Result<Vec<String>, String> {
    let root = parse_json(text)?;
    Ok(ids_from_data(data_items(&root)?, |item| {
        item.get("isFree").and_then(Value::as_bool) == Some(true)
    }))
}

/// Merge the two sources: with `live` Some, take the intersection (free AND gateway-offered);
/// with None (the gateway request failed), fall back to "trust only models.dev, excluding deprecated".
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
