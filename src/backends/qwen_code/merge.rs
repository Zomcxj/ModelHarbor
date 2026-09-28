use super::*;

/// 一张卡片该落到哪个 pid、要不要写 `providerProtocol`、条目要写什么 `wireApi`。
///
/// 优先沿用卡片上原有的**自定义** pid（保留用户自己的命名），只把映射刷新成当前协议；
/// 新建卡片（或从别的后端转来）没有 pid，就回落到内置 pid。
///
/// 沿用自定义 pid 有个前提：它**本来就在映射表里**。映射表里没有的自定义 pid 是个没人
/// 认得的键，写出去只会让 Qwen Code 把整条静默跳过——那种情况回落到内置 pid 才是对的。
pub(crate) fn route(
    p: &ProviderRow,
    api: &str,
    protocols: Option<&Map<String, Value>>,
) -> (String, Option<String>, &'static str) {
    let (builtin_pid, wire) = pid_for_api(api);
    let custom = p.qwen_pid.trim();
    if !custom.is_empty() && !is_builtin_pid(custom) && !is_readonly_provider(custom) {
        let known = protocols
            .and_then(|m| m.get(custom))
            .and_then(Value::as_str)
            .is_some();
        if known {
            return (custom.to_string(), Some(builtin_pid.to_string()), wire);
        }
    }
    (builtin_pid.to_string(), None, wire)
}

/// 一条待写入的条目：落在哪个 pid、该 pid 需要的映射、条目内容。
pub(crate) struct Placed {
    pid: String,
    /// `Some(协议桶名)` = 这个 pid 是自定义的，要在 `providerProtocol` 里声明。
    mapping: Option<String>,
    entry: Value,
}

/// 界面状态 → 全部条目（`serialize_root` 唯一的条目来源）。
pub(crate) fn place(providers: &[ProviderRow], base: &Value) -> Vec<Placed> {
    let protocols = provider_protocols(base);
    let mut out: Vec<Placed> = Vec::new();
    for p in providers.iter().filter(|p| !p.key.trim().is_empty()) {
        let api = p.effective_api();
        let (pid, mapping, wire) = route(p, &api, protocols);
        if is_readonly_provider(&pid) {
            // 理论到不了：`route` 不会产出只读 pid。留一道闸门，别让只读条目被重建。
            continue;
        }
        // 旧条目按 id 建索引：界面没接管的键（`capabilities.agent`、
        // `generationConfig.maxRetries` 等）要从这里继承。
        let index: HashMap<&str, &Value> = entries_of_pid(base, &pid)
            .iter()
            .filter_map(|e| entry_id(e).map(|id| (id, e)))
            .collect();
        // 没有模型的卡片也要留一条（id 回落到 key），否则刚建好还没填模型的卡片
        // 一保存就消失。
        let models: Vec<Option<&ModelRow>> = if p.models.is_empty() {
            vec![None]
        } else {
            p.models.iter().map(Some).collect()
        };
        for m in models {
            let id = m
                .map(|m| m.id.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| p.key.trim())
                .to_string();
            let old = index.get(id.as_str()).copied();
            let entry = entry_from_provider(p, m, &id, wire, old);
            out.push(Placed {
                pid: pid.clone(),
                mapping: mapping.clone(),
                entry,
            });
        }
    }
    out
}

/// 按 pid 分组（没有停用概念，全部条目都进主配置）。
pub(crate) fn group_by_pid(placed: &[Placed]) -> Map<String, Value> {
    let mut out: Map<String, Value> = Map::new();
    for item in placed {
        push_entry(&mut out, &item.pid, item.entry.clone());
    }
    out
}

/// `providerProtocol` 的内容：由界面条目推导，并保留「认不出来的 pid」原有的映射。
///
/// 界面不再需要的自定义 pid（卡片被删掉了）其映射会被清掉——留着就是一个指向不存在
/// provider 的孤儿声明。官方要求自定义 pid 必须有映射，所以这里也顺带保证了
/// 「有自定义 pid 条目 ⇒ 有映射」。
pub(crate) fn protocols_for(placed: &[Placed], base: &Value) -> Map<String, Value> {
    let mut out: Map<String, Value> = Map::new();
    for item in placed {
        if let Some(mapping) = &item.mapping {
            out.insert(item.pid.clone(), Value::String(mapping.clone()));
        }
    }
    if let (Some(old), Some(map)) = (provider_protocols(base), providers_map(base)) {
        for (pid, value) in old {
            // 值不是数组的 pid 是旧包装形状，整块由 `unmanaged_of` 原样带过去，
            // 它的映射自然也该留着。
            let unmanaged = map.get(pid).is_some_and(|v| !v.is_array());
            if unmanaged && !out.contains_key(pid) {
                out.insert(pid.clone(), value.clone());
            }
        }
    }
    out
}

/// 基座里**认不出来**的条目，必须原样带过去（见模块说明末节）。
///
/// 返回 `pid → 该 pid 要补写的内容`：数组表示「补进这个 pid 的数组里」，其它值表示
/// 「整块替换这个 pid」。后者只在该 pid 本次没有界面条目时才产出。
pub(crate) fn unmanaged_of(base: &Value, placed: &[Placed]) -> Vec<(String, Value)> {
    let managed: HashSet<&str> = placed.iter().map(|p| p.pid.as_str()).collect();
    let mut out: Vec<(String, Value)> = Vec::new();
    let Some(m) = providers_map(base) else {
        return out;
    };
    for (pid, value) in m {
        // `qwen-oauth` 整块原样保留：官方硬编码、不可覆盖，界面也不显示它。
        if is_readonly_provider(pid) {
            out.push((pid.clone(), value.clone()));
            continue;
        }
        match value.as_array() {
            Some(items) => {
                let extra: Vec<Value> = items
                    .iter()
                    .filter(|e| entry_id(e).is_none())
                    .cloned()
                    .collect();
                if !extra.is_empty() {
                    out.push((pid.clone(), Value::Array(extra)));
                }
            }
            // 值不是数组（旧包装形状 `{protocol, models}`）：整块保留。只有本次没有
            // 界面条目落到这个 pid 时才这么做——有的话我们的数组会把它顶掉，而那种
            // 情况本就不该出现（那个形状解析不出卡片）。
            None => {
                if !managed.contains(pid.as_str()) {
                    out.push((pid.clone(), value.clone()));
                }
            }
        }
    }
    out
}

/// 把界面上的密钥写回顶层 `env`。
///
/// **只增改，绝不删。** `env` 是跨 provider 共享的扁平命名空间，而且**不归本工具独有**：
/// Qwen Code 自己的 `/auth` 也往里写（例如 Coding Plan 的
/// `BAILIAN_CODING_PLAN_API_KEY`）。按「有没有条目引用」去修剪，就会把用户刚用 `/auth`
/// 配好的凭据静默删掉——那是不可恢复的。孤立键对 Qwen Code 无害（它只按 `envKey` 查），
/// 所以宁可留一个没人引用的键，也不删。
///
/// 界面清空密钥时同样不动 `env`：那个值可能是 `/auth` 写的、也可能是用户手填的，
/// 本工具无从区分「清空」与「不接管」。清掉字段保存后密钥还在，是可恢复的；
/// 反过来误删一个凭据则不可恢复。
pub(crate) fn sync_env(root: &mut Map<String, Value>, providers: &[ProviderRow]) {
    let mut env = root
        .get("env")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for p in providers {
        let key = env_key_name(p);
        if key.is_empty() || p.api_key.trim().is_empty() {
            continue;
        }
        env.insert(key, Value::String(p.api_key.clone()));
    }
    if !env.is_empty() {
        root.insert("env".into(), Value::Object(env));
    }
}

/// 条目列表 → [`BackendLoad`]。
pub(crate) fn build_load(
    entries: Vec<(String, Value)>,
    env: Option<&Map<String, Value>>,
    protocols: Option<&Map<String, Value>>,
) -> BackendLoad {
    let mut providers: Vec<ProviderRow> = Vec::new();
    for (pid, entry) in &entries {
        let Some(row) = provider_from_entry(pid, protocols, entry, env) else {
            continue;
        };
        providers.push(row);
    }
    BackendLoad {
        root: Value::Object(Map::new()),
        agents: Vec::new(),
        providers,
        extras: Value::Object(Map::new()),
    }
}
