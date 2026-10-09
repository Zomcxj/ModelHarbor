use super::parse::{combine, parse_kilo_free, parse_live_models, parse_models_dev_free};
use super::{KILO_LIVE_URL, OPENCODE_LIVE_URL, SOURCE_URL};
use crate::format::ConfigFormat;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 缓存有效期（秒）：24 小时；超过则在后台重新拉取。
pub const CACHE_TTL_SECS: i64 = 24 * 60 * 60;

/// 落盘缓存的内容（`fetched_at` + 模型 id 列表）。
pub struct Cache {
    pub models: Vec<String>,
    /// 是否仍在有效期内；过期也照样返回 `models`。
    pub fresh: bool,
}

/// 当前时间（秒级 Unix 时间戳）。
pub(super) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// 缓存文件名：按后端 `label()` 区分。
pub(super) fn cache_file(format: ConfigFormat) -> String {
    format!("free-models-{}.json", format.label())
}

/// 某个后端的缓存文件路径。
pub fn cache_path(format: ConfigFormat) -> PathBuf {
    crate::prefs::Prefs::config_dir().join(cache_file(format))
}

pub(super) fn agent(read_secs: u64) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(read_secs))
        .build()
}

/// 发起一次 GET 并取回正文，错误文本统一走人话映射。
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

/// 后台线程内执行：按后端拉取免费模型列表；没有免费层的后端返回空列表。
pub fn fetch_remote(format: ConfigFormat) -> Result<Vec<String>, String> {
    match format {
        // opencode：models.dev 的价格 + Zen 网关的可用性，两者求交。
        ConfigFormat::Opencode => {
            let free = parse_models_dev_free(&get_text(SOURCE_URL, 60)?, "opencode")?;
            // 网关列表拉不到就回退。
            let live = get_text(OPENCODE_LIVE_URL, 15)
                .ok()
                .and_then(|text| parse_live_models(&text).ok());
            Ok(combine(free, live.as_deref()))
        }
        // Kilo：网关响应自带 isFree。
        ConfigFormat::Kilocode => parse_kilo_free(&get_text(KILO_LIVE_URL, 60)?),
        // 其余后端没有免费层。
        _ => Ok(Vec::new()),
    }
}

/// 从指定路径读取缓存；文件缺失 / 读不出 / 结构不符都返回 `None`。
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
        // 时间戳落在未来时按「刚取过」处理。
        fresh: age < CACHE_TTL_SECS,
    })
}

/// 从指定路径写入缓存（原子写）。
pub(super) fn save_cache_at(path: &Path, models: &[String]) -> Result<(), String> {
    let payload = serde_json::json!({
        "fetched_at": unix_now(),
        "models": models,
    });
    crate::util::atomic_write_text(path, &crate::serialize::pretty_json(&payload))
}

/// 读取某个后端的缓存。
pub fn load_cache(format: ConfigFormat) -> Option<Cache> {
    load_cache_at(&cache_path(format))
}

/// 把结果写入某个后端的缓存。
pub fn save_cache(format: ConfigFormat, models: &[String]) -> Result<(), String> {
    save_cache_at(&cache_path(format), models)
}
