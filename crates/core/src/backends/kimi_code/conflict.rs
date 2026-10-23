use super::*;
use crate::model::ProviderRow;
use serde_json::Value;

/// The one-of-three credential conflict's description; `None` = no conflict.
///
/// Criteria align item-by-item with the source's `declaredProviderCredential`; the returned text goes
/// straight into the status bar / save errors.
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

/// Credential conflicts across every provider (blocked before saving).
/// The pre-write structural health check: reserved-prefix vs alias collisions.
///
/// `managed:*` is skipped in [`build_load`]; colliding aliases in the model table mean later writes overwrite earlier ones.
pub(crate) fn first_structural_conflict(providers: &[ProviderRow]) -> Option<String> {
    // `managed:` is /login's reserved prefix; such providers never enter the UI.
    if let Some(p) = providers.iter().find(|p| is_managed_provider(p.key.trim())) {
        return Some(format!(
            "provider \"{}\" 占用了保留前缀 managed:（那是 Kimi 登录态的命名空间），请改名",
            p.key.trim()
        ));
    }
    // The alias IS the TOML table key — collisions silently overwrite. Cross-provider combination collisions are blocked here.
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
