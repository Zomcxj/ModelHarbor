//! models.dev 官方模型目录（裁剪版）：为「使用官方推荐」按钮提供上下文与输出上限。
//!
//! ## 为什么需要它
//!
//! 表单里的 `context`（上下文窗口）与 `output`（最大输出）是**发给上游的声明**：
//! 填小了白白浪费模型能力，填大了会被上游拒绝。models.dev 收录了各厂商自己声明的
//! `limit.context` / `limit.output`，是这些取值最可靠的来源。
//!
//! ## 为什么不能直接按模型 id 取
//!
//! 同一个模型 id 在 models.dev 里常被几十家 provider 同时收录（实测
//! `deepseek-v4-flash` 有 29 家），而各家声明的上限**并不一致**——同一个 id 能出现
//! 六组不同的上下文值。直接按 id 取会随机撞上某个中转站的数值，所以按可信度分级
//! 判定，见 [`Catalog::resolve`]。
//!
//! ## 缓存
//!
//! 原始 `api.json` 约 4.8 MB，而这里只需要每个 provider 的 `api` 与每个模型的
//! `limit`，裁剪后约 0.55 MB（紧凑 JSON）。做法与免费模型列表一致：落盘到
//! `.modelharbor/official-limits.json`，超过 [`CACHE_TTL_SECS`] 才在后台重取。
//!
//! 裁剪后的结构刻意与 `api.json` 保持同形（`providers.<id>.api` /
//! `.models.<id>.limit`），于是 [`Catalog::parse_value`] 一份代码能同时读原始目录与
//! 缓存，不需要为缓存单写一套解析分支。

use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

/// 缓存文件名（落在 `.modelharbor/` 下）。
const CACHE_FILE: &str = "official-limits.json";

/// 缓存有效期（秒）。模型上限不是分钟级事件，一天一次足够。
pub const CACHE_TTL_SECS: i64 = 24 * 60 * 60;

/// 厂商标前缀 → models.dev 里的原厂 provider id（一律小写，匹配时先把 id 转小写）。
///
/// 只用于第 2 级判定（见 [`Catalog::resolve`]）：中转站的 `baseURL` 认不出来时，
/// 靠模型名的前缀判断「这是谁家的模型」，再取那一家的官方条目。
///
/// 前缀猜错不会取到别家的数值：取值前必须确认该 provider 下确实有这个名字
/// （[`Catalog::limits_of`]），落空就自动降级到下一级。
const VENDOR_PREFIXES: &[(&str, &str)] = &[
    ("claude-", "anthropic"),
    ("gpt-", "openai"),
    ("o1", "openai"),
    ("o3", "openai"),
    ("o4", "openai"),
    ("grok-", "xai"),
    ("glm-", "zai"),
    ("kimi-", "moonshotai"),
    ("deepseek-", "deepseek"),
    ("qwen", "alibaba"),
    ("gemini-", "google"),
    ("mimo-", "xiaomi"),
    ("sensenova-", "sensenova"),
    ("mistral-", "mistral"),
    ("minimax-", "minimax"),
    ("command-", "cohere"),
    ("sonar", "perplexity"),
];

/// 规范化 baseURL：去掉首尾空白与结尾斜杠并转小写。
///
/// 只做这些——用户填的可能是 `https://API.OpenAI.com/v1/`，而目录里写的是
/// `https://api.openai.com/v1`，两者是同一个端点。更复杂的等价（不同路径别名、
/// 带查询串）不做：宁可判定不出而让按钮变灰，也不要错误地认成官方端点。
fn normalize_api(url: &str) -> String {
    url.trim().trim_end_matches('/').to_ascii_lowercase()
}

/// 一个模型的官方上限。两个字段都可能缺（数据源只收录了其中一个）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Limits {
    context: Option<u64>,
    output: Option<u64>,
}

/// 目录里一个 provider 的条目。
struct ProviderEntry {
    /// 该 provider 的官方端点（models.dev 的 `api`），用于 baseURL 精确匹配。
    api: Option<String>,
    /// 模型名（小写）→ 官方上限。
    models: HashMap<String, Limits>,
}

/// 判定成功后的官方推荐值。
pub struct Official {
    /// 官方推荐的上下文窗口（必然有值：取不到这个值就不会判定成功）。
    pub context: u64,
    /// 官方推荐的最大输出；数据源没收录时为 `None`（此时只填上下文）。
    pub output: Option<u64>,
    /// 判定依据，显示在悬停提示里，让用户知道这个数是从哪来的。
    pub source: String,
}

/// 判定结果。分得这么细是为了让按钮的悬停提示说实话：「数据源没收录这个名字」
/// 与「各家收录的取值互相矛盾」是两件不同的事，用户能做的处置也不同
/// （前者只能手填，后者可以去核对厂商文档）。
pub enum Verdict {
    /// 判定出了官方推荐值。
    Found(Official),
    /// 目录里没有这个模型名。
    UnknownModel,
    /// 收录了，但各来源给出的上下文上限互相矛盾（`n` 组不同取值）。
    Ambiguous(usize),
    /// 收录了，但没有任何来源给出上下文上限（只有 output 之类）。
    NoContext,
}

impl Official {
    /// 从上限构造；没有上下文值时判定失败（返回 `None`）。
    fn new(limits: Limits, source: String) -> Option<Self> {
        Some(Self {
            context: limits.context?,
            output: limits.output,
            source,
        })
    }
}

/// 裁剪版官方目录。构造后只读，可跨线程共享。
pub struct Catalog {
    /// provider id（小写）→ 条目。用 `BTreeMap` 而非 `HashMap`：`by_api` 的
    /// 冲突消解依赖遍历顺序，字典序能保证结果与迭代顺序无关、可复现。
    providers: BTreeMap<String, ProviderEntry>,
    /// 规范化 baseURL → provider id（同一端点有多个 provider 时取字典序最小者）。
    by_api: HashMap<String, String>,
    /// 模型名（小写）→ 该名字在**全部** provider 中出现过的上限（已去重、已排序）。
    /// 长度为 1 表示没有分歧，第 3 级判定才能用。
    by_model: HashMap<String, Vec<Limits>>,
}

impl Catalog {
    /// 解析 models.dev 的 `api.json` 全文。
    pub fn parse(text: &str) -> Result<Self, String> {
        let root = crate::opencode_models::parse_json(text)?;
        Self::parse_value(&root)
    }

    /// 解析 provider map（`api.json` 的顶层对象，也是缓存文件里的 `providers`）。
    fn parse_value(root: &Value) -> Result<Self, String> {
        let obj = root
            .as_object()
            .ok_or_else(|| "官方目录不是 JSON 对象".to_string())?;
        let mut providers: BTreeMap<String, ProviderEntry> = BTreeMap::new();
        for (pid, pv) in obj {
            let api = pv.get("api").and_then(Value::as_str).map(str::to_string);
            let mut models: HashMap<String, Limits> = HashMap::new();
            if let Some(entries) = pv.get("models").and_then(Value::as_object) {
                for (mid, mv) in entries {
                    let limit = mv.get("limit");
                    let limits = Limits {
                        context: limit.and_then(|l| l.get("context")).and_then(Value::as_u64),
                        output: limit.and_then(|l| l.get("output")).and_then(Value::as_u64),
                    };
                    // 两个上限都没有的模型对本功能没有意义，直接丢掉（目录里绝大多数
                    // 条目属于此类，裁掉的正是它们）。
                    if limits.context.is_none() && limits.output.is_none() {
                        continue;
                    }
                    models.insert(mid.to_ascii_lowercase(), limits);
                }
            }
            if api.is_none() && models.is_empty() {
                continue;
            }
            providers.insert(pid.to_ascii_lowercase(), ProviderEntry { api, models });
        }
        if providers.is_empty() {
            return Err("官方目录里没有任何 provider".to_string());
        }
        Ok(Self::build(providers))
    }

    /// 由 provider 条目建立查询索引。
    fn build(providers: BTreeMap<String, ProviderEntry>) -> Self {
        let mut by_api: HashMap<String, String> = HashMap::new();
        let mut by_model: HashMap<String, HashSet<Limits>> = HashMap::new();
        for (pid, entry) in &providers {
            if let Some(api) = &entry.api {
                let key = normalize_api(api);
                if !key.is_empty() {
                    // `BTreeMap` 保证按 pid 字典序访问，冲突时稳定保留最小者。
                    by_api.entry(key).or_insert_with(|| pid.clone());
                }
            }
            for (mid, limits) in &entry.models {
                by_model.entry(mid.clone()).or_default().insert(*limits);
            }
        }
        let by_model = by_model
            .into_iter()
            .map(|(mid, set)| {
                let mut values: Vec<Limits> = set.into_iter().collect();
                values.sort();
                (mid, values)
            })
            .collect();
        Self {
            providers,
            by_api,
            by_model,
        }
    }

    /// 判定某个模型的官方推荐值（见 [`Verdict`]）。
    ///
    /// 四级判定，从可信到宽松：
    ///
    /// 1. **baseURL 精确匹配**——用户配的就是某家官方端点，这一家的声明最可信；
    /// 2. **模型原厂条目**——中转站的 baseURL 认不出来时，按模型名前缀找原厂
    ///    （`claude-*` → anthropic，见 [`VENDOR_PREFIXES`]）；
    /// 3. **全库一致**——所有收录来源给出同一组上限，说明没有分歧；
    /// 4. 否则放弃：宁可让用户手填，也不要塞一个猜出来的数。
    ///
    /// 前两级都必须在该 provider 下**确实存在这个名字**才算命中，所以前缀猜错只会
    /// 降级，不会取到别家数值。
    pub fn verdict(&self, base_url: &str, model_id: &str) -> Verdict {
        let model_id = model_id.trim();
        if model_id.is_empty() {
            return Verdict::UnknownModel;
        }
        let key = model_id.to_ascii_lowercase();
        if let Some(pid) = self.provider_for_api(base_url) {
            if let Some(limits) = self.limits_of(pid, &key) {
                if let Some(found) = Official::new(limits, format!("{}（baseURL 精确匹配）", pid))
                {
                    return Verdict::Found(found);
                }
            }
        }
        if let Some(pid) = vendor_provider(&key) {
            if let Some(limits) = self.limits_of(pid, &key) {
                if let Some(found) = Official::new(limits, format!("{}（模型原厂条目）", pid))
                {
                    return Verdict::Found(found);
                }
            }
        }
        let Some(candidates) = self.by_model.get(&key) else {
            return Verdict::UnknownModel;
        };
        if candidates.len() == 1 {
            match Official::new(candidates[0], "models.dev 全部收录来源一致".to_string()) {
                Some(found) => Verdict::Found(found),
                // 唯一一组候选里也没有上下文值。
                None => Verdict::NoContext,
            }
        } else {
            Verdict::Ambiguous(candidates.len())
        }
    }

    /// baseURL 对应的 provider id。
    fn provider_for_api(&self, base_url: &str) -> Option<&str> {
        let key = normalize_api(base_url);
        if key.is_empty() {
            return None;
        }
        self.by_api.get(&key).map(String::as_str)
    }

    /// 取某 provider 下某个模型的上限；该名字不存在、或没有上下文值时返回 `None`
    /// （后者让判定继续往下一级走）。
    fn limits_of(&self, provider: &str, model_key: &str) -> Option<Limits> {
        let limits = *self.providers.get(provider)?.models.get(model_key)?;
        limits.context.map(|_| limits)
    }

    /// 落盘用的裁剪 JSON（`fetched_at` + provider 条目）。
    fn to_cache_json(&self) -> Value {
        let mut providers = serde_json::Map::new();
        for (pid, entry) in &self.providers {
            let mut models = serde_json::Map::new();
            for (mid, limits) in &entry.models {
                let mut limit = serde_json::Map::new();
                if let Some(context) = limits.context {
                    limit.insert("context".to_string(), Value::from(context));
                }
                if let Some(output) = limits.output {
                    limit.insert("output".to_string(), Value::from(output));
                }
                models.insert(mid.clone(), serde_json::json!({ "limit": limit }));
            }
            let mut item = serde_json::Map::new();
            if let Some(api) = &entry.api {
                item.insert("api".to_string(), Value::String(api.clone()));
            }
            if !models.is_empty() {
                item.insert("models".to_string(), Value::Object(models));
            }
            providers.insert(pid.clone(), Value::Object(item));
        }
        serde_json::json!({
            "fetched_at": crate::opencode_models::unix_now(),
            "providers": providers,
        })
    }
}

/// 按前缀判断模型原厂。
fn vendor_provider(model_key: &str) -> Option<&'static str> {
    VENDOR_PREFIXES
        .iter()
        .find(|(prefix, _)| model_key.starts_with(prefix))
        .map(|(_, provider)| *provider)
}

/// 目录缓存路径。
fn cache_path() -> PathBuf {
    crate::prefs::Prefs::config_dir().join(CACHE_FILE)
}

/// 读落盘缓存；缺失 / 读不出 / 结构不符都返回 `None`。
fn load_cache() -> Option<(Catalog, bool)> {
    let text = std::fs::read_to_string(cache_path()).ok()?;
    let root: Value = serde_json::from_str(&text).ok()?;
    let catalog = Catalog::parse_value(root.get("providers")?).ok()?;
    let fetched_at = root.get("fetched_at").and_then(Value::as_i64).unwrap_or(0);
    let age = crate::opencode_models::unix_now().saturating_sub(fetched_at);
    // 时间戳落在未来（改过系统时间）时按「刚取过」处理，不必重取。
    Some((catalog, age < CACHE_TTL_SECS))
}

/// 把目录写入缓存（原子写；失败只返回错误文本，不影响界面）。
///
/// 用紧凑 JSON 而不是 `pretty_json`：这份文件**只给程序读**（用户不会去翻），
/// 而缩进会让它从 0.33 MB 涨到 0.85 MB，启动时解析也跟着慢一倍。
pub fn save_cache(catalog: &Catalog) -> Result<(), String> {
    let payload = catalog.to_cache_json();
    let text = serde_json::to_string(&payload).map_err(|err| err.to_string())?;
    crate::util::atomic_write_text(&cache_path(), &text)
}

/// 后台线程内执行：拉取并裁剪官方目录。
pub fn fetch_remote() -> Result<Catalog, String> {
    let text = crate::opencode_models::get_text(crate::opencode_models::SOURCE_URL, 60)?;
    Catalog::parse(&text)
}

/// 官方目录的运行时状态（界面只读；刷新由后台线程负责）。
#[derive(Default)]
pub struct CatalogState {
    /// 已就绪的目录；`None` = 还没拿到（首次启动且缓存缺失，或拉取还没回来）。
    pub catalog: Option<Arc<Catalog>>,
    /// 后台拉取通道（`Some` = 正在飞）。
    pub rx: Option<std::sync::mpsc::Receiver<Result<Catalog, String>>>,
    /// 最近一次拉取失败的原因（成功后清空）。
    pub error: Option<String>,
    /// 启动时是否需要自动拉一次（缓存缺失或过期）。
    pub auto: bool,
}

impl CatalogState {
    /// 读落盘缓存；缺失或过期时标记「首帧后台重取」。
    ///
    /// 过期也照样先用旧目录渲染：一天前的上限值远比「按钮全灰」有用。
    pub fn load_from_disk() -> Self {
        match load_cache() {
            Some((catalog, fresh)) => Self {
                catalog: Some(Arc::new(catalog)),
                auto: !fresh,
                ..Self::default()
            },
            None => Self {
                auto: true,
                ..Self::default()
            },
        }
    }

    pub fn fetching(&self) -> bool {
        self.rx.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一份最小目录，覆盖四级判定的每个分支：
    /// `openai` 与 `relay-a` 收录同名模型但取值不同（baseURL 匹配才能区分），
    /// `claude-x` 只有原厂收录（走前缀），`solo-model` 全库唯一，`dup-model` 有分歧。
    fn catalog() -> Catalog {
        Catalog::parse(
            &serde_json::json!({
                "openai": {
                    "api": "https://api.openai.com/v1",
                    "models": { "gpt-5": { "limit": { "context": 400000, "output": 128000 } } }
                },
                "relay-a": {
                    "api": "https://relay.example/v1",
                    "models": { "gpt-5": { "limit": { "context": 1000000, "output": 32000 } } }
                },
                "anthropic": {
                    "models": { "claude-x": { "limit": { "context": 200000, "output": 64000 } } }
                },
                "solo": {
                    "models": { "solo-model": { "limit": { "context": 123, "output": 45 } } }
                },
                "conflict-a": { "models": { "dup-model": { "limit": { "context": 100 } } } },
                "conflict-b": { "models": { "dup-model": { "limit": { "context": 200 } } } },
                "out-only": { "models": { "no-context": { "limit": { "output": 999 } } } },
                "no-limits": { "models": { "bare": { "name": "x" } } }
            })
            .to_string(),
        )
        .unwrap()
    }

    /// 判定成功时的 `(context, output, 来源)`；没判定出来时 panic 并说明原因
    /// （断言里直接写 `unwrap` 看不出为什么失败，这里带上判定分支）。
    fn found(base: &str, model: &str) -> (u64, Option<u64>, String) {
        match catalog().verdict(base, model) {
            Verdict::Found(official) => (official.context, official.output, official.source),
            Verdict::UnknownModel => panic!("{} 判定为「目录里没有这个名字」", model),
            Verdict::Ambiguous(n) => panic!("{} 判定为「{} 组取值互相矛盾」", model, n),
            Verdict::NoContext => panic!("{} 判定为「没有上下文上限」", model),
        }
    }

    #[test]
    fn a_matching_base_url_beats_the_relay_entry() {
        // 同名模型两家都收录：认对了端点才不会取到中转站的数。
        let (context, output, source) = found("https://api.openai.com/v1", "gpt-5");
        assert_eq!(context, 400000);
        assert_eq!(output, Some(128000));
        assert!(source.contains("openai"), "来源：{}", source);
    }

    #[test]
    fn the_base_url_comparison_tolerates_case_and_a_trailing_slash() {
        let (context, _, _) = found("  https://API.OpenAI.com/v1/  ", "gpt-5");
        assert_eq!(context, 400000);
    }

    #[test]
    fn an_unrecognised_base_url_falls_back_to_the_vendor_entry() {
        // 中转站的端点认不出来，靠模型名前缀找到 anthropic 的官方条目。
        let (context, _, source) = found("https://some-relay.example/v1", "claude-x");
        assert_eq!(context, 200000);
        assert!(source.contains("anthropic"), "来源：{}", source);
    }

    #[test]
    fn the_vendor_prefix_match_ignores_case() {
        // 用户实际写的是 `Qwen3.8-Flash` 这种大小写混排。
        let (context, _, _) = found("", "CLAUDE-X");
        assert_eq!(context, 200000);
    }

    #[test]
    fn a_prefix_without_a_matching_entry_falls_through() {
        // `glm-` 指向 zai，但这份目录里没有 zai：不能因此编出一个值。
        assert!(matches!(
            catalog().verdict("", "glm-5.3-flash"),
            Verdict::UnknownModel
        ));
    }

    #[test]
    fn a_single_uncontested_value_is_used() {
        let (context, output, source) = found("", "solo-model");
        assert_eq!(context, 123);
        assert_eq!(output, Some(45));
        assert!(source.contains("一致"), "来源：{}", source);
    }

    #[test]
    fn disagreeing_sources_are_refused_rather_than_guessed() {
        // 两家给出 100 / 200：挑任何一个都是在替用户做没有依据的决定。
        // 分支要精确到 Ambiguous：这决定按钮提示是「有分歧」而不是「没收录」。
        match catalog().verdict("", "dup-model") {
            Verdict::Ambiguous(n) => assert_eq!(n, 2),
            _ => panic!("取值有分歧时应判定为 Ambiguous"),
        }
    }

    #[test]
    fn a_model_without_a_context_limit_is_not_offered() {
        // 只有 output、没有 context：按钮要填的是上下文，填不了就判定失败。
        assert!(matches!(
            catalog().verdict("", "no-context"),
            Verdict::NoContext
        ));
        // 完全没有 limit 的条目在解析时就被丢掉了，等同「没收录」。
        assert!(matches!(
            catalog().verdict("", "bare"),
            Verdict::UnknownModel
        ));
    }

    #[test]
    fn an_empty_or_unknown_model_id_is_refused() {
        assert!(matches!(
            catalog().verdict("https://api.openai.com/v1", "   "),
            Verdict::UnknownModel
        ));
        assert!(matches!(
            catalog().verdict("", "nope"),
            Verdict::UnknownModel
        ));
    }

    #[test]
    fn the_cache_round_trip_preserves_every_verdict() {
        // 缓存写盘再读回，四级判定的结论必须一模一样——否则重启后按钮会变灰。
        let original = catalog();
        let payload = original.to_cache_json();
        let restored = Catalog::parse_value(payload.get("providers").unwrap()).unwrap();
        for (base, model) in [
            ("https://api.openai.com/v1", "gpt-5"),
            ("https://some-relay.example/v1", "claude-x"),
            ("", "solo-model"),
            ("", "dup-model"),
            ("", "no-context"),
            ("", "glm-5.3-flash"),
            ("", "bare"),
        ] {
            let describe = |c: &Catalog| match c.verdict(base, model) {
                Verdict::Found(o) => format!("Found({}, {:?})", o.context, o.output),
                Verdict::UnknownModel => "UnknownModel".to_string(),
                Verdict::Ambiguous(n) => format!("Ambiguous({})", n),
                Verdict::NoContext => "NoContext".to_string(),
            };
            assert_eq!(
                describe(&original),
                describe(&restored),
                "{} / {} 的判定在缓存往返后变了",
                base,
                model
            );
        }
    }

    #[test]
    fn the_cache_payload_carries_a_timestamp() {
        // 没有时间戳就永远算过期，每次启动都要重拉。
        let payload = catalog().to_cache_json();
        assert!(payload.get("fetched_at").and_then(Value::as_i64).is_some());
    }

    #[test]
    fn broken_payloads_are_reported_without_panicking() {
        assert!(Catalog::parse("not json").is_err());
        assert!(Catalog::parse("[]").is_err());
        // 合法对象但没有一个 provider 带 limit / api
        assert!(Catalog::parse(r#"{"a":{"models":{"x":{"name":"y"}}}}"#).is_err());
    }

    #[test]
    fn every_vendor_prefix_points_at_a_plausible_provider_id() {
        // 前缀表是手写的，写错了只会静默降级（不会出错），所以在这里守一道：
        // 前缀必须非空小写、provider id 非空，且没有重复前缀。
        let mut seen: HashSet<&str> = HashSet::new();
        for (prefix, provider) in VENDOR_PREFIXES {
            assert!(!prefix.is_empty() && prefix.chars().all(|c| !c.is_ascii_uppercase()));
            assert!(!provider.is_empty());
            assert!(seen.insert(prefix), "前缀重复：{}", prefix);
        }
    }
}
