use super::*;

/// 写盘前的总闸：两个 provider 的凭据若落到**同一个** `envKey` 变量名上，
/// [`sync_env`] 后写的会覆盖先写的——两个条目都指向它，其中一个必然拿错密钥，
/// 且无任何报错。这种冲突宁可不让存，请给卡片手填不同的变量名。
pub(crate) fn first_env_name_conflict(providers: &[ProviderRow]) -> Option<String> {
    // name -> (先到的 provider key, 它的密钥值)
    let mut seen: HashMap<String, (&str, &str)> = HashMap::new();
    for p in providers.iter().filter(|p| !p.key.trim().is_empty()) {
        let name = env_key_name(p);
        if name.is_empty() {
            continue;
        }
        if let Some((prev_key, prev_value)) = seen.get(name.as_str()) {
            // 同一个变量名 + 同一个密钥值（比如同一站点复制出两条）：共享无妨。
            if *prev_value == p.api_key.trim() {
                continue;
            }
            return Some(format!(
                "envKey \"{}\" 被 provider \"{}\" 与 \"{}\" 同时占用，密钥却不同",
                name,
                prev_key,
                p.key.trim()
            ));
        }
        seen.insert(name, (p.key.trim(), p.api_key.trim()));
    }
    None
}
