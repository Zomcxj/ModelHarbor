use super::*;

/// 两个 provider 的凭据落到同一个 `envKey` 变量名、且密钥不同时，返回冲突描述。
pub(crate) fn first_env_name_conflict(providers: &[ProviderRow]) -> Option<String> {
    // name -> (先到的 provider key, 它的密钥值)
    let mut seen: HashMap<String, (&str, &str)> = HashMap::new();
    for p in providers.iter().filter(|p| !p.key.trim().is_empty()) {
        let name = env_key_name(p);
        if name.is_empty() {
            continue;
        }
        if let Some((prev_key, prev_value)) = seen.get(name.as_str()) {
            // 同一个变量名 + 同一个密钥值（如同一站点复制出两条）：不算冲突。
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
