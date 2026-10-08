use super::*;
use crate::model::ProviderRow;
use serde_json::Value;

/// 三选一的凭据冲突描述；`None` = 无冲突。
///
/// 判据与源码 `declaredProviderCredential` 逐条对齐；返回文本直接进状态栏 / 保存错误。
pub(crate) fn credential_conflict(name: &str, entry: &Value) -> Option<String> {
    let inline = non_empty_str(entry.get("api_key")).is_some();
    let env = non_empty_str(entry.get("api_key_env")).is_some();
    let oauth = entry.get("oauth").is_some();
    let pair = match (inline, env, oauth) {
        (true, true, _) => Some(("api_key", "api_key_env")),
        (_, true, true) => Some(("api_key_env", "oauth")),
        (true, _, true) => Some(("api_key", "oauth")),
        _ => None,
    };
    pair.map(|(first, second)| {
        format!(
            "Provider \"{name}\" has both {first} and {second} set in config.toml - \
             they are mutually exclusive. Remove one."
        )
    })
}

/// 全部 provider 里的凭据冲突（保存前挡下来）。
/// 写盘前的结构冲突体检：保留前缀与别名撞名。
///
/// `managed:*` 在 [`build_load`] 里被跳过；撞名的别名在模型表里后写覆盖先写。
pub(crate) fn first_structural_conflict(providers: &[ProviderRow]) -> Option<String> {
    // `managed:` 是 `/login` 的保留前缀，这类 provider 不进界面。
    if let Some(p) = providers.iter().find(|p| is_managed_provider(p.key.trim())) {
        return Some(format!(
            "provider \"{}\" 占用了保留前缀 managed:（那是 Kimi 登录态的命名空间），请改名",
            p.key.trim()
        ));
    }
    // 别名就是 TOML 表键，撞名会静默覆盖。这里拦跨 provider 的组合撞名。
    let mut seen: std::collections::HashMap<String, &str> = std::collections::HashMap::new();
    for p in providers.iter().filter(|p| !p.key.trim().is_empty()) {
        for m in &p.models {
            if m.id.trim().is_empty() {
                continue;
            }
            let alias = effective_alias(p.key.trim(), m);
            if let Some(prev) = seen.get(alias.as_str()) {
                return Some(format!(
                    "模型别名 \"{alias}\" 被 provider \"{prev}\" 与 \"{}\" 同时占用（表键撞名会静默丢条目），请改名",
                    p.key.trim()
                ));
            }
            seen.insert(alias, p.key.trim());
        }
    }
    None
}

pub(crate) fn first_credential_conflict(providers: &[ProviderRow]) -> Option<String> {
    providers
        .iter()
        .filter(|p| !p.key.trim().is_empty())
        .find_map(|p| credential_conflict(p.key.trim(), &provider_entry_view(p)))
}
