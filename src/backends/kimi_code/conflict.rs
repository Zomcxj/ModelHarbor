use super::*;
use crate::model::ProviderRow;
use serde_json::Value;

/// 三选一的凭据冲突描述；`None` = 无冲突。
///
/// 判据与源码 `declaredProviderCredential` 逐条对齐（顺序也一致，报错信息才会一样）。
/// 返回的是**人话**，直接进状态栏 / 保存错误。
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
///
/// 为什么要挡：同时写 `api_key` 与 `api_key_env` 会让 Kimi Code **启动失败**
/// （源码把这种情况判成 `kind: "conflict"` 并拒绝）。那比「配置不生效」严重得多——
/// 用户会以为是自己把 Kimi Code 弄坏了。宁可不让保存，也不能写出一个启动不了的文件。
/// 写盘前的结构冲突体检：保留前缀与别名撞名。
///
/// 两者都不会让 Kimi Code 报错——文件合法、模型也在——但界面**读不回来**：
/// `managed:*` 在 [`build_load`] 里被跳过，撞名的别名在模型表里后写覆盖先写。
/// 静默消失比保存失败糟糕，所以这里宁可不让存。
pub(crate) fn first_structural_conflict(providers: &[ProviderRow]) -> Option<String> {
    // `managed:` 是 `/login` 的保留前缀：这类 provider 本就不进界面，用户手起这个名字
    // 的话，它名下的模型下次加载就凭空消失。
    if let Some(p) = providers.iter().find(|p| is_managed_provider(p.key.trim())) {
        return Some(format!(
            "provider \"{}\" 占用了保留前缀 managed:（那是 Kimi 登录态的命名空间），请改名",
            p.key.trim()
        ));
    }
    // 别名就是 TOML 表键，撞名 = 有一条被静默覆盖。同一 provider 内 model id 重复已被
    // 保存入口的查重拦下，这里拦的是跨 provider 的组合撞名
    // （如 provider "a" + model "b/c" 与 provider "a/b" + model "c"）。
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
