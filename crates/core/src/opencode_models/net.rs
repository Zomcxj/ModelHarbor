use super::parse::{combine, parse_kilo_free, parse_live_models, parse_models_dev_free};
use super::{KILO_LIVE_URL, OPENCODE_LIVE_URL, SOURCE_URL};
use crate::format::ConfigFormat;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The cache's lifetime (seconds): 24 hours; past it, refetch in the background.
pub const CACHE_TTL_SECS: i64 = 24 * 60 * 60;

/// The persisted cache's content (`fetched_at` + the model id list).
pub struct Cache {
    pub models: Vec<String>,
    /// Whether still within its lifetime; expired ones return `models` all the same.
    pub fresh: bool,
}

/// Current time (second-resolution Unix timestamp).
pub(super) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// The cache file's name: distinguished per backend's `label()`.
pub(super) fn cache_file(format: ConfigFormat) -> String {
    format!("free-models-{}.json", format.label())
}

/// A backend's cache file path.
pub fn cache_path(format: ConfigFormat) -> PathBuf {
    crate::prefs::Prefs::config_dir().join(cache_file(format))
}

pub(super) fn agent(read_secs: u64) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(read_secs))
        .build()
}

/// Issue one GET and fetch the body; error text goes through the human mapping uniformly.
fn get_text(url: &str, read_secs: u64) -> Result<String, String> {
    let response = agent(read_secs)
        .get(url)
        .set("Accept", "application/json")
        .set("User-Agent", "ModelHarbor")
        .call()
        .map_err(|err| match err {
            ureq::Error::Status(code, _) => crate::http_status::label(code),
            ureq::Error::Transport(transport) => format!(
                "网络错误：{}",
                crate::util::sanitize_network_error(&transport.to_string())
            ),
        })?;
    response.into_string().map_err(|err| err.to_string())
}

/// Runs on the background thread: fetch a backend's free-model list; tier-less backends return an empty list.
pub fn fetch_remote(format: ConfigFormat) -> Result<Vec<String>, String> {
    match format {
        // opencode: models.dev's prices ∩ the Zen gateway's availability.
        ConfigFormat::Opencode => {
            let free = parse_models_dev_free(&get_text(SOURCE_URL, 60)?, "opencode")?;
            // Gateway list unfetchable → fall back.
            let live = get_text(OPENCODE_LIVE_URL, 15)
                .ok()
                .and_then(|text| parse_live_models(&text).ok());
            Ok(combine(free, live.as_deref()))
        }
        // Kilo: the gateway response carries isFree itself.
        ConfigFormat::Kilocode => parse_kilo_free(&get_text(KILO_LIVE_URL, 60)?),
        // Other backends have no free tier.
        _ => Ok(Vec::new()),
    }
}

/// Read the cache from a given path; missing / unreadable / wrong-shaped files all return `None`.
pub(super) fn load_cache_at(path: &Path) -> Option<Cache> {
    let text = std::fs::read_to_string(path).ok()?;
    let root: Value = serde_json::from_str(&text).ok()?;
    let models: Vec<String> = root
        .get("models")?
        .as_array()?
        .iter()
        .filter_map(|item| item.as_str())
        .map(str::to_string)
        .filter(|id| !id.trim().is_empty())
        .collect();
    if models.is_empty() {
        return None;
    }
    let fetched_at = root.get("fetched_at").and_then(Value::as_i64).unwrap_or(0);
    let age = unix_now().saturating_sub(fetched_at);
    Some(Cache {
        models,
        // A future timestamp counts as "just fetched".
        fresh: age < CACHE_TTL_SECS,
    })
}

/// Write the cache to a given path (atomic write).
pub(super) fn save_cache_at(path: &Path, models: &[String]) -> Result<(), String> {
    let payload = serde_json::json!({
        "fetched_at": unix_now(),
        "models": models,
    });
    crate::util::atomic_write_text(path, &crate::serialize::pretty_json(&payload))
}

/// Read a backend's cache.
pub fn load_cache(format: ConfigFormat) -> Option<Cache> {
    load_cache_at(&cache_path(format))
}

/// Write a result into a backend's cache.
pub fn save_cache(format: ConfigFormat, models: &[String]) -> Result<(), String> {
    save_cache_at(&cache_path(format), models)
}
