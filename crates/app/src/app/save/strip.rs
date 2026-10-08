use crate::format::ConfigFormat;
use serde_json::{Map, Value};

/// 跨格式转换前，从目标 root 中剔除由当前组件状态接管的容器：
/// provider（opencode 的 provider / pi 系与 DSH 的 providers）与 opencode 的 agent。
///
/// 只剔界面接管得了的部分：界面不显示、也无从重建的只读内容（Kimi 的 `managed:*`
/// OAuth provider 与它名下的模型、Qwen 的 `qwen-oauth`）保留，其他顶层字段原样保留。
///
/// 删除一律用 `shift_remove`，保留其余键的相对顺序。
///
/// `agents_owned` 表示界面确实持有 agents 数据；为 `false` 时保留目标文件里的 agent 容器。
pub fn strip_cross_format_containers(fmt: ConfigFormat, root: &mut Value, agents_owned: bool) {
    // workbuddy 的根是数组：整体清空。
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
        // 已在上面提前返回，不会到达；列出以保持 match 穷尽。
        ConfigFormat::WorkBuddy => {}
        // QwenCode：只剔界面接管的那部分，`qwen-oauth` 与认不出来的内容留下。
        ConfigFormat::QwenCode => {
            if let Some(providers) = obj.get_mut("modelProviders").and_then(Value::as_object_mut) {
                let pids: Vec<String> = providers.keys().cloned().collect();
                for pid in pids {
                    if crate::backends::qwen_code::is_readonly_provider(&pid) {
                        continue;
                    }
                    // 值不是数组（旧包装形状）：原样留着。
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
            // `providerProtocol` 只留还存在的 pid 的映射。
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
        // KimiCode：同样只剔界面接管的部分，`managed:*` 与它名下的模型留下。
        ConfigFormat::KimiCode => {
            // 模型归属按剔之前的 provider 表判断。
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
                        // 孤儿（没写 `provider`，或指向表里没有的键）也留下；只剔指向本次要剔的
                        // 那个 provider 的条目。
                        !owner.is_empty() && owners.iter().any(|o| o == owner)
                    })
                    .map(|(alias, _)| alias.clone())
                    .collect();
                for alias in drop {
                    models.shift_remove(&alias);
                }
            }
            // 表空了整个删掉。
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
