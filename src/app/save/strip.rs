use crate::format::ConfigFormat;
use serde_json::{Map, Value};

/// 跨格式转换前，从目标 root 中剔除由当前组件状态接管的容器：
/// provider（opencode 的 provider / pi 系与 DSH 的 providers）与 opencode 的 agent。
///
/// 目的：跨格式保存/预览时，provider 条目与顺序完全以界面为准（干净转换），
/// 同时目标文件的其他顶层字段（如 DSH 的 llm-pi-ai 下其他设置）原样保留。
/// 同格式目标（WSL 同步等）不走这里，仍用保守合并。
///
/// **只剔界面接管得了的部分。** 目标文件里那些界面不显示、也无从重建的只读内容
/// （Kimi 的 `managed:*` OAuth provider 与它名下的模型、Qwen 的 `qwen-oauth`）必须留下：
/// 下游的 `unmanaged_*` 正是从**剔完之后的 root** 里把它们捡回来的，剔干净就再也捡不回来，
/// 一次跨页保存就成了静默删除。
///
/// **删除必须用 `shift_remove`，不能用 `remove`。** `serde_json` 开了 `preserve_order`
/// （底层 `IndexMap`），它的 `remove` 是 `swap_remove`：删一个键会把**最后一个**键搬到
/// 空出来的槽位，其余键的相对顺序随之打乱。跨格式保存 ZCode 时先删 `providerOrder`，
/// 写出的文件就成了 `modelConfigRules, providerConfigRules, providerOrder` —— ZCode 读得懂，
/// 但与它自己写出的顺序不一致。`shift_remove` 保留其余键的顺序。
///
/// `agents_owned` 表示界面确实持有 agents 数据。agents 只属于 opencode 页：
/// 数据来自 pi / oh-my-pi / DSH（或空载启动）时界面无从表达 agents，
/// 此时必须保留目标文件里的 agent 容器，否则会把它们静默删掉。
pub fn strip_cross_format_containers(fmt: ConfigFormat, root: &mut Value, agents_owned: bool) {
    // workbuddy 的根是数组：整体清空即是「剔除界面接管的容器」。
    if fmt == ConfigFormat::WorkBuddy {
        *root = Value::Array(Vec::new());
        return;
    }
    let Some(obj) = root.as_object_mut() else {
        return;
    };
    match fmt {
        // opencode 系（opencode / kilocode / mimocode）容器名相同，共用一套剔除。
        ConfigFormat::Opencode | ConfigFormat::Kilocode | ConfigFormat::Mimocode => {
            obj.shift_remove("provider");
            if agents_owned {
                obj.shift_remove("agent");
            }
        }
        ConfigFormat::Pi | ConfigFormat::OhMyPi => {
            obj.shift_remove("providers");
        }
        ConfigFormat::DeepSeekHarness => {
            if let Some(llm) = obj.get_mut("llm-pi-ai").and_then(Value::as_object_mut) {
                llm.shift_remove("providers");
            }
        }
        ConfigFormat::ZCode => {
            if let Some(cfg) = obj.get_mut("config").and_then(Value::as_object_mut) {
                cfg.shift_remove("providerOrder");
                if let Some(rules) = cfg
                    .get_mut("providerConfigRules")
                    .and_then(Value::as_object_mut)
                {
                    rules.shift_remove("providerRules");
                }
                if let Some(rules) = cfg
                    .get_mut("modelConfigRules")
                    .and_then(Value::as_object_mut)
                {
                    rules.shift_remove("providerModelRules");
                }
            }
        }
        // 已在上面提前返回，这里不会到达；列出以保持 match 穷尽。
        ConfigFormat::WorkBuddy => {}
        // QwenCode：只剔**界面接管的那部分**，`qwen-oauth` 与认不出来的内容都要留下。
        //
        // 不能整表剔掉：`unmanaged_of` 读的就是这个被剔过的 root，它在里面找不到的东西
        // 就带不过去。而除 `qwen-oauth`（官方硬编码、界面不显示）之外，还有两类内容界面
        // 也认不出来——没有 `id` 的条目、值不是数组的旧包装形状——整表剔掉，一次跨页保存
        // 就把它们一起删了。（同格式保存不会：那条路上基底是没剔过的原文件。）
        ConfigFormat::QwenCode => {
            if let Some(providers) = obj.get_mut("modelProviders").and_then(Value::as_object_mut) {
                let pids: Vec<String> = providers.keys().cloned().collect();
                for pid in pids {
                    if crate::backends::qwen_code::is_readonly_provider(&pid) {
                        continue;
                    }
                    // 值不是数组（旧包装形状）：整块不归界面管，原样留着。
                    let Some(items) = providers.get_mut(&pid).and_then(Value::as_array_mut) else {
                        continue;
                    };
                    items.retain(|e| !crate::backends::qwen_code::is_ui_managed_entry(e));
                    if items.is_empty() {
                        providers.shift_remove(&pid);
                    }
                }
            }
            if obj
                .get("modelProviders")
                .and_then(Value::as_object)
                .is_some_and(Map::is_empty)
            {
                obj.shift_remove("modelProviders");
            }
            // `providerProtocol` 只留**还存在的 pid** 的映射：界面接管的自定义 pid 与它的
            // 条目一起被剔掉了，映射留着就是指向不存在 provider 的孤儿声明；`qwen-oauth`
            // 是内置 pid、本来就不需要映射，而旧包装形状的 pid 整块留着，它的映射也得留。
            let kept: Vec<String> = obj
                .get("modelProviders")
                .and_then(Value::as_object)
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default();
            if let Some(protocols) = obj
                .get_mut("providerProtocol")
                .and_then(Value::as_object_mut)
            {
                let drop: Vec<String> = protocols
                    .keys()
                    .filter(|pid| !kept.iter().any(|k| k == *pid))
                    .cloned()
                    .collect();
                for pid in drop {
                    protocols.shift_remove(&pid);
                }
                if protocols.is_empty() {
                    obj.shift_remove("providerProtocol");
                }
            }
        }
        // KimiCode：同样只剔界面接管的部分，**留下 `managed:*`**。
        //
        // `[providers."managed:*"]` 是 `/login` 的 OAuth 登录态（与敏感目录
        // `credentials/` 配对），它名下的 `[models.*]` 条目是登录时写进去的官方模型。
        // 两者都不进界面、界面也无从重建；整表剔掉之后 `unmanaged_providers` /
        // `unmanaged_models` 在基底里什么也找不到，一次跨页保存就把登录态和官方模型一起删了。
        ConfigFormat::KimiCode => {
            // 「哪些模型归界面接管」要按**剔之前**的 provider 表判断：先剔 provider 再看
            // 模型的话，owner 已经不在表里，界面接管的模型会被误认成孤儿而留下。
            let owners: Vec<String> = obj
                .get("providers")
                .and_then(Value::as_object)
                .map(|provs| provs.keys().cloned().collect())
                .unwrap_or_default();
            if let Some(providers) = obj.get_mut("providers").and_then(Value::as_object_mut) {
                let drop: Vec<String> = providers
                    .keys()
                    .filter(|name| !crate::backends::kimi_code::is_managed_provider(name))
                    .cloned()
                    .collect();
                for name in drop {
                    providers.shift_remove(&name);
                }
            }
            if let Some(models) = obj.get_mut("models").and_then(Value::as_object_mut) {
                let drop: Vec<String> = models
                    .iter()
                    .filter(|(_, entry)| {
                        let owner = entry.get("provider").and_then(Value::as_str).unwrap_or("");
                        // `managed:*` 名下的留下。
                        if crate::backends::kimi_code::is_managed_provider(owner) {
                            return false;
                        }
                        // 孤儿（没写 `provider`，或指向表里本来就没有的键）也留下：那是本工具
                        // 认不出的内容，去留不该由一次格式转换来决定。只有「指向本次要剔掉的
                        // 那个 provider」的条目才是界面接管的。
                        !owner.is_empty() && owners.iter().any(|o| o == owner)
                    })
                    .map(|(alias, _)| alias.clone())
                    .collect();
                for alias in drop {
                    models.shift_remove(&alias);
                }
            }
            // 表空了整个删掉（`[providers]` 空表合法但多余）。
            for key in ["providers", "models"] {
                if obj
                    .get(key)
                    .and_then(Value::as_object)
                    .is_some_and(Map::is_empty)
                {
                    obj.shift_remove(key);
                }
            }
        }
    }
}
