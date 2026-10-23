use super::*;

/// When two providers' credentials land on the same `envKey` variable name with different keys, return the conflict description.
pub(crate) fn first_env_name_conflict(providers: &[ProviderRow]) -> Option<String> {
    // name -> (the earlier provider key, its key value)
    let mut seen: HashMap<String, (&str, &str)> = HashMap::new();
    for p in providers.iter().filter(|p| !p.key.trim().is_empty()) {
        let name = env_key_name(p);
        if name.is_empty() {
            continue;
        }
        if let Some((prev_key, prev_value)) = seen.get(name.as_str()) {
            // The same variable name + the same key value (two entries copied from one station): not a conflict.
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
