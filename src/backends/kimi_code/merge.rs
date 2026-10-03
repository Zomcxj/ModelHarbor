use super::*;
use crate::model::ProviderRow;
use serde_json::{Map, Value};

/// 一个 provider 条目 + 挂在它名下的模型条目（含 alias）。
pub(crate) struct Joined {
    pub(crate) name: String,
    pub(crate) provider: Value,
    pub(crate) models: Vec<(String, Value)>,
}

/// 按 `provider` 字段把顶层 `[models.*]` 挂到 `[providers.*]` 上。
///
/// `provider` 缺失、指向不存在的键或指向 `managed:*` 的模型不挂到任何 provider，
/// 因此不进界面；它们由 [`unmanaged_models`] 原样写回。
pub(crate) fn join(root: &Value) -> Vec<Joined> {
    let mut joined: Vec<Joined> = Vec::new();
    let empty = Map::new();
    let providers = providers_map(root).unwrap_or(&empty);
    for (name, provider) in providers {
        joined.push(Joined {
            name: name.clone(),
            provider: provider.clone(),
            models: Vec::new(),
        });
    }
    for (alias, model) in models_map(root).unwrap_or(&empty) {
        let owner = model.get("provider").and_then(Value::as_str).unwrap_or("");
        if let Some(entry) = joined.iter_mut().find(|j| j.name == owner) {
            entry.models.push((alias.clone(), model.clone()));
        }
    }
    joined
}

/// provider 条目 → [`ProviderRow`]（一个 provider 一张卡片，模型挂其下）。
pub(crate) fn provider_from_entry(
    name: &str,
    provider: &Value,
    models: &[(String, Value)],
) -> ProviderRow {
    let mut row = ProviderRow::new();
    row.key = name.to_string();
    row.base_url = provider
        .get("base_url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    row.api_key = provider
        .get("api_key")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    row.api_key_env = provider
        .get("api_key_env")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    row.original_api_key_env = row.api_key_env.clone();
    // `type` 逐字进 `pi_api`：它既是界面下拉的当前值，也是写回时的原值。
    // 空 type 留给 `effective_api()` 回落。
    row.pi_api = provider_type(provider).to_string();
    row.models = models
        .iter()
        .map(|(alias, m)| model_from_entry(alias, m))
        .collect();
    row.source_format = Some(ConfigFormat::KimiCode);
    row.raw = provider.clone();
    row
}

/// 界面状态 → 全部模型条目。
pub(crate) fn all_models(providers: &[ProviderRow], base: &Value) -> Vec<(String, String, Value)> {
    // 旧条目按 alias 建索引：界面没接管的键从这里继承。
    let old: Map<String, Value> = models_map(base).cloned().unwrap_or_default();
    let mut out: Vec<(String, String, Value)> = Vec::new();
    for p in providers.iter().filter(|p| !p.key.trim().is_empty()) {
        let provider = p.key.trim().to_string();
        // 没有模型的 provider 不写条目：Kimi 的模型必须挂在 provider 下，
        // 但 provider 本身没有模型是合法的，写空模型会产出缺 `model` 的非法条目。
        for m in &p.models {
            let wire = m.id.trim();
            if wire.is_empty() {
                // 没有 wire id 的模型行写出去就是非法条目（`model` 必填），保存时跳过。
                continue;
            }
            let alias = effective_alias(&provider, m);
            let entry = entry_from_model(m, &provider, old.get(&alias));
            out.push((provider.clone(), alias, entry));
        }
    }
    out
}

/// `[models.*]`：全部条目。
///
/// Kimi 没有「停用」概念，只有这一张表，没有「生效清单 / 全量副本」之分。
pub(crate) fn model_table(providers: &[ProviderRow], base: &Value) -> Map<String, Value> {
    let mut out: Map<String, Value> = Map::new();
    for (_, alias, entry) in all_models(providers, base) {
        out.insert(alias, entry);
    }
    out
}

/// 把**认不出来**的模型条目并进一张模型表（原样，不加 `disabled`）。
pub(crate) fn merge_unmanaged_models(
    models: &mut Map<String, Value>,
    base: &Value,
    ui_providers: &[ProviderRow],
) {
    for (alias, model) in unmanaged_models(base, ui_providers) {
        models.entry(alias).or_insert(model);
    }
}

/// `[providers.*]`：界面 provider + 只读 provider（原样）+ 认不出来的形状。
pub(crate) fn all_providers(providers: &[ProviderRow], base: &Value) -> Map<String, Value> {
    let mut out: Map<String, Value> = Map::new();
    let old = providers_map(base).cloned().unwrap_or_default();
    for p in providers.iter().filter(|p| !p.key.trim().is_empty()) {
        let name = p.key.trim();
        // 只读 provider 不归界面管，原样保留（见 [`unmanaged_providers`]）。
        if is_managed_provider(name) {
            continue;
        }
        out.insert(name.to_string(), provider_entry_from_row(p, old.get(name)));
    }
    for (name, value) in unmanaged_providers(base) {
        out.insert(name, value);
    }
    out
}

/// 基座里不归界面管的 provider：只有 `managed:*`（OAuth 登录态），原样带过。
///
/// 除 `managed:*` 外不再保留基座里的条目：用户删掉或改名的卡片不补回去。
pub(crate) fn unmanaged_providers(base: &Value) -> Vec<(String, Value)> {
    providers_map(base)
        .map(|m| {
            m.iter()
                .filter(|(name, _)| is_managed_provider(name))
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// 基座里认不出来的模型条目：孤儿（`provider` 指向不存在的键）与 `managed:*` 名下的。
///
/// 返回 `alias → 条目`；它们不进界面，但原样写回。
pub(crate) fn unmanaged_models(base: &Value, ui_providers: &[ProviderRow]) -> Map<String, Value> {
    let providers = providers_map(base).cloned().unwrap_or_default();
    let mut out: Map<String, Value> = Map::new();
    for (alias, model) in models_map(base).unwrap_or(&Map::new()) {
        let owner = model.get("provider").and_then(Value::as_str).unwrap_or("");
        // 界面认得的：provider 存在、不是 managed、且该 provider 有卡片。
        let known = !owner.is_empty()
            && !is_managed_provider(owner)
            && providers.contains_key(owner)
            && ui_providers.iter().any(|p| p.key.trim() == owner);
        if !known {
            out.insert(alias.clone(), model.clone());
        }
    }
    out
}

/// 按 alias 逐条判断勾选状态。
///
/// Kimi 没有「停用」概念，模型的 `disabled` 一律为 `false`。
pub(crate) fn build_load(joined: Vec<Joined>) -> BackendLoad {
    let mut providers: Vec<ProviderRow> = Vec::new();
    for j in &joined {
        // `managed:*` 不进界面：它由 OAuth 维护，其模型也是登录时自动写入的。
        if is_managed_provider(&j.name) {
            continue;
        }
        providers.push(provider_from_entry(&j.name, &j.provider, &j.models));
    }
    BackendLoad {
        root: Value::Object(Map::new()),
        agents: Vec::new(),
        providers,
        extras: Value::Object(Map::new()),
    }
}
