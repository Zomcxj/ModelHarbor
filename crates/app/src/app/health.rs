//! Config health check: gathers the scattered validations into one list, reviewed in one pass before saving.
//!
//! Every item reuses the existing judgment logic ([`crate::util::url_suspicions`],
//! [`crate::opencode_models::model_is_valid_on`], etc.); the check itself is read-only, touching nothing.

use crate::format::ConfigFormat;
use crate::model::{AgentRow, ProviderRow};

/// Severity of a health finding; decides ordering and coloring in the list.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) enum Severity {
    /// The save will be blocked.
    Blocker,
    /// The save passes, but the result may not be what the user wants.
    Warn,
    /// Just a heads-up.
    Info,
}

impl Severity {
    pub(super) fn label(self) -> &'static str {
        match self {
            Severity::Blocker => "会阻止保存",
            Severity::Warn => "需要注意",
            Severity::Info => "提示",
        }
    }
}

/// One health finding. `where_` is the locating name (provider key / agent name / field name).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Issue {
    pub severity: Severity,
    /// The problem in one sentence.
    pub title: String,
    /// Locating: the problematic provider / agent / entry.
    pub where_: String,
    /// What's wrong and how to handle it.
    pub detail: String,
}

/// Health-check input: depends only on UI state; no file reads, no network requests.
pub(super) struct HealthInput<'a> {
    pub page: ConfigFormat,
    pub providers: &'a [ProviderRow],
    pub agents: &'a [AgentRow],
    /// Whether the source format is opencode-family (deciding whether agents land in the target file).
    pub source_is_opencode: bool,
}

/// Gather all findings, sorted by severity (blockers first).
pub(super) fn collect(input: &HealthInput<'_>) -> Vec<Issue> {
    let mut issues = Vec::new();
    check_duplicate_keys(input, &mut issues);
    check_empty_identifiers(input, &mut issues);
    check_invalid_numbers(input, &mut issues);
    check_base_urls(input, &mut issues);
    check_agent_models(input, &mut issues);
    check_api_keys(input, &mut issues);
    check_agents_not_written(input, &mut issues);
    // Stable sort: within a level, keep each check's discovery order (provider order = UI order).
    issues.sort_by_key(|issue| issue.severity);
    issues
}

/// Duplicate keys / model ids. Same criteria as the save path (`find_duplicate_keys`).
fn check_duplicate_keys(input: &HealthInput<'_>, out: &mut Vec<Issue>) {
    let mut seen_agents = std::collections::HashSet::new();
    for agent in input.agents {
        let key = agent.key.trim();
        if !key.is_empty() && !seen_agents.insert(key.to_string()) {
            out.push(Issue {
                severity: Severity::Blocker,
                title: "agent 名重复".into(),
                where_: key.to_string(),
                detail: "同名 agent 在配置文件里会互相覆盖，且保存会被拦下。改掉其中一个的名字。"
                    .into(),
            });
        }
    }
    let mut seen_providers = std::collections::HashSet::new();
    for provider in input.providers {
        let key = provider.key.trim();
        if key.is_empty() {
            continue;
        }
        if !seen_providers.insert(key.to_string()) {
            out.push(Issue {
                severity: Severity::Blocker,
                title: "provider key 重复".into(),
                where_: key.to_string(),
                detail: "provider 容器以 key 为键，重名会让其中一份被覆盖，保存也会被拦下。".into(),
            });
        }
        let mut seen_models = std::collections::HashSet::new();
        for model in &provider.models {
            let id = model.id.trim();
            if !id.is_empty() && !seen_models.insert(id.to_string()) {
                out.push(Issue {
                    severity: Severity::Blocker,
                    title: "同一 provider 内 model id 重复".into(),
                    where_: format!("{} / {}", key, id),
                    detail: "同一 provider 下同名的模型只会生效一条，保存也会被拦下。".into(),
                });
            }
        }
    }
}

/// Empty key / empty id: these entries are skipped at save time.
fn check_empty_identifiers(input: &HealthInput<'_>, out: &mut Vec<Issue>) {
    for (idx, provider) in input.providers.iter().enumerate() {
        if provider.key.trim().is_empty() {
            out.push(Issue {
                severity: Severity::Warn,
                title: "provider 没有名字".into(),
                where_: format!("第 {} 个 provider", idx + 1),
                detail: "没有 key 的 provider 不会写进配置文件。填上 key，或删掉这张卡片。".into(),
            });
            continue;
        }
        let nameless = provider
            .models
            .iter()
            .filter(|model| model.id.trim().is_empty())
            .count();
        if nameless > 0 {
            out.push(Issue {
                severity: Severity::Warn,
                title: "model 没有 id".into(),
                where_: format!("{}（{} 个）", provider.key, nameless),
                detail: "没有 id 的模型不会写进配置文件。填上 id，或删掉这些模型行。".into(),
            });
        }
    }
}

/// Invalid numeric fields: ignored at save time.
fn check_invalid_numbers(input: &HealthInput<'_>, out: &mut Vec<Issue>) {
    let bad =
        |text: &str| !text.trim().is_empty() && crate::util::parse_number_text(text).is_none();
    for agent in input.agents {
        if bad(&agent.temperature) {
            out.push(Issue {
                severity: Severity::Warn,
                title: "temperature 不是数字".into(),
                where_: format!("agent {}", agent.key.trim()),
                detail: format!(
                    "「{}」无法解析成数字，保存时会忽略该字段。改成数字（如 0.7），或留空。",
                    agent.temperature.trim()
                ),
            });
        }
    }
    for provider in input.providers {
        let key = provider.key.trim();
        if bad(&provider.timeout) {
            out.push(Issue {
                severity: Severity::Warn,
                title: "timeout 不是数字".into(),
                where_: key.to_string(),
                detail: format!(
                    "「{}」无法解析成数字，保存时会忽略该字段。单位是毫秒，例如 180000。",
                    provider.timeout.trim()
                ),
            });
        }
        for model in &provider.models {
            for (field, text) in [("context", &model.context), ("output", &model.output)] {
                if bad(text) {
                    out.push(Issue {
                        severity: Severity::Warn,
                        title: format!("{} 不是数字", field),
                        where_: format!("{} / {}", key, model.id.trim()),
                        detail: format!(
                            "「{}」无法解析成数字，保存时会忽略该字段（不会写成 0）。",
                            text.trim()
                        ),
                    });
                }
            }
        }
    }
}

/// Suspicious baseUrl spellings (missing scheme / doubled slashes / trailing slash / embedded whitespace).
///
/// Hints only; never auto-rewritten.
fn check_base_urls(input: &HealthInput<'_>, out: &mut Vec<Issue>) {
    for provider in input.providers {
        let suspicions = crate::util::url_suspicions(&provider.base_url);
        if suspicions.is_empty() {
            continue;
        }
        out.push(Issue {
            severity: Severity::Warn,
            title: "baseUrl 写法可疑".into(),
            where_: provider.key.trim().to_string(),
            detail: format!(
                "{}。当前值：{}",
                suspicions.join("、"),
                provider.base_url.trim()
            ),
        });
    }
}

/// An agent's `model` points at a gateway this page doesn't recognize.
fn check_agent_models(input: &HealthInput<'_>, out: &mut Vec<Issue>) {
    if !input.page.is_opencode_family() {
        return;
    }
    let keys: Vec<String> = input
        .providers
        .iter()
        .map(|provider| provider.key.clone())
        .collect();
    for agent in input.agents {
        let model = agent.model.trim();
        // An empty model means "not chosen yet".
        if model.is_empty() {
            continue;
        }
        if !crate::opencode_models::model_is_valid_on(input.page, &keys, model) {
            out.push(Issue {
                severity: Severity::Warn,
                title: "agent 的 model 本页网关不认".into(),
                where_: format!("agent {}", agent.key.trim()),
                detail: format!(
                    "「{}」的前半段既不是 {} 页的网关，也不是本页已配的 provider。\
                     保存到本页时会被换成本页网关的首选模型；若这是有意的跨页配置，可忽略。",
                    model,
                    input.page.label()
                ),
            });
        }
    }
}

/// Providers with no key set.
///
/// Severity is `Info`.
fn check_api_keys(input: &HealthInput<'_>, out: &mut Vec<Issue>) {
    for provider in input.providers {
        if provider.key.trim().is_empty() {
            continue;
        }
        let has_secret = !provider.api_key_secret.trim().is_empty();
        // DSH goes through apiKeyEnv references; other backends look at apiKey.
        let filled = if input.page == ConfigFormat::DeepSeekHarness {
            !provider.api_key_env.trim().is_empty() || has_secret
        } else {
            !provider.api_key.trim().is_empty() || has_secret
        };
        if !filled {
            out.push(Issue {
                severity: Severity::Info,
                title: "未填密钥".into(),
                where_: provider.key.trim().to_string(),
                detail:
                    "该 provider 没有密钥。本地推理或公开网关可忽略；需要鉴权的站点会请求失败。"
                        .into(),
            });
        }
    }
}

/// A one-line reminder when the current format cannot store agents.
fn check_agents_not_written(input: &HealthInput<'_>, out: &mut Vec<Issue>) {
    if input.agents.is_empty() || input.source_is_opencode {
        return;
    }
    out.push(Issue {
        severity: Severity::Info,
        title: "agents 不会写进本页格式".into(),
        where_: format!("{} 个 agent", input.agents.len()),
        detail: format!(
            "{} 的配置格式没有 agent 定义，保存时会忽略这些 agent（目标文件里已有的不会被清空）。",
            input.page.label()
        ),
    });
}

/// Count findings per severity, for the badge on the entry button.
pub(super) fn count_by_severity(issues: &[Issue]) -> (usize, usize) {
    let blockers = issues
        .iter()
        .filter(|issue| issue.severity == Severity::Blocker)
        .count();
    let warnings = issues
        .iter()
        .filter(|issue| issue.severity == Severity::Warn)
        .count();
    (blockers, warnings)
}

#[cfg(test)]
mod tests {
    use super::{collect, count_by_severity, HealthInput, Severity};
    use crate::format::ConfigFormat;
    use crate::model::{AgentRow, ModelRow, ProviderRow};

    fn provider(key: &str) -> ProviderRow {
        let mut provider = ProviderRow::new();
        provider.key = key.to_string();
        provider
    }

    fn input<'a>(
        page: ConfigFormat,
        providers: &'a [ProviderRow],
        agents: &'a [AgentRow],
    ) -> HealthInput<'a> {
        HealthInput {
            page,
            providers,
            agents,
            source_is_opencode: true,
        }
    }

    fn titles(issues: &[super::Issue]) -> Vec<&str> {
        issues.iter().map(|issue| issue.title.as_str()).collect()
    }

    #[test]
    fn a_clean_config_reports_nothing_that_blocks() {
        let mut provider = provider("p1");
        provider.base_url = "https://api.example.com/v1".into();
        provider.api_key = "sk-x".into();
        let providers = vec![provider];
        let issues = collect(&input(ConfigFormat::Opencode, &providers, &[]));
        let (blockers, _) = count_by_severity(&issues);
        assert_eq!(blockers, 0, "干净配置不该有阻断项: {issues:?}");
    }

    #[test]
    fn duplicate_provider_keys_are_blockers() {
        let providers = vec![provider("dup"), provider("dup")];
        let issues = collect(&input(ConfigFormat::Opencode, &providers, &[]));
        assert_eq!(issues[0].severity, Severity::Blocker);
        assert_eq!(issues[0].title, "provider key 重复");
        assert_eq!(issues[0].where_, "dup");
    }

    #[test]
    fn duplicate_model_ids_inside_one_provider_are_blockers() {
        let mut p = provider("p1");
        p.models = vec![
            ModelRow::from("m", &serde_json::json!({})),
            ModelRow::from("m", &serde_json::json!({})),
        ];
        let providers = vec![p];
        let issues = collect(&input(ConfigFormat::Opencode, &providers, &[]));
        assert!(
            titles(&issues).contains(&"同一 provider 内 model id 重复"),
            "{issues:?}"
        );
    }

    #[test]
    fn empty_provider_key_is_reported_once_and_skips_its_models() {
        let mut p = provider("");
        p.models = vec![ModelRow::new(), ModelRow::new()];
        let providers = vec![p];
        let issues = collect(&input(ConfigFormat::Opencode, &providers, &[]));
        assert_eq!(
            titles(&issues),
            vec!["provider 没有名字"],
            "没有 key 时只报一条，不再叠加 model 与密钥的提示: {issues:?}"
        );
    }

    #[test]
    fn a_nameless_model_is_reported_with_a_count() {
        let mut p = provider("p1");
        p.api_key = "sk-x".into();
        p.models = vec![
            ModelRow::from("ok", &serde_json::json!({})),
            ModelRow::new(),
        ];
        let providers = vec![p];
        let issues = collect(&input(ConfigFormat::Opencode, &providers, &[]));
        let issue = issues
            .iter()
            .find(|issue| issue.title == "model 没有 id")
            .expect("应报出无 id 的模型");
        assert!(issue.where_.contains("1 个"), "带上条数: {}", issue.where_);
    }

    #[test]
    fn an_unparseable_number_is_a_warning_not_a_blocker() {
        let mut p = provider("p1");
        p.api_key = "sk-x".into();
        p.timeout = "abc".into();
        p.models = vec![ModelRow::from("m", &serde_json::json!({}))];
        p.models[0].context = "1e999".into();
        let providers = vec![p];
        let issues = collect(&input(ConfigFormat::Opencode, &providers, &[]));
        assert!(issues.iter().any(|issue| issue.title == "timeout 不是数字"));
        assert!(
            issues.iter().any(|issue| issue.title == "context 不是数字"),
            "非有限浮点也要算无效: {issues:?}"
        );
        assert!(issues
            .iter()
            .all(|issue| issue.severity != Severity::Blocker));
    }

    #[test]
    fn an_empty_number_field_is_not_reported() {
        let mut p = provider("p1");
        p.api_key = "sk-x".into();
        // An empty timeout / context means unspecified — legal.
        p.models = vec![ModelRow::from("m", &serde_json::json!({}))];
        let providers = vec![p];
        let issues = collect(&input(ConfigFormat::Opencode, &providers, &[]));
        assert!(
            !titles(&issues).iter().any(|t| t.contains("不是数字")),
            "空字段不是错误: {issues:?}"
        );
    }

    #[test]
    fn a_suspicious_base_url_is_reported_with_the_current_value() {
        let mut p = provider("p1");
        p.api_key = "sk-x".into();
        p.base_url = "https://api.example.com//v1/".into();
        let providers = vec![p];
        let issues = collect(&input(ConfigFormat::Opencode, &providers, &[]));
        let issue = issues
            .iter()
            .find(|issue| issue.title == "baseUrl 写法可疑")
            .expect("应报出可疑 baseUrl");
        assert!(issue.detail.contains("重复斜杠"), "{}", issue.detail);
        assert!(
            issue.detail.contains("https://api.example.com//v1/"),
            "{}",
            issue.detail
        );
    }

    #[test]
    fn an_agent_model_from_another_gateway_is_reported() {
        let mut agent = AgentRow::new();
        agent.key = "build".into();
        agent.model = "kilo/kilo-auto/free".into();
        let agents = vec![agent];
        // The opencode page doesn't accept the kilo prefix, and no same-named provider exists.
        let issues = collect(&input(ConfigFormat::Opencode, &[], &agents));
        let issue = issues
            .iter()
            .find(|issue| issue.title.contains("本页网关不认"))
            .expect("应报出跨网关引用");
        assert!(issue.where_.contains("build"));
        assert!(issue.detail.contains("kilo/kilo-auto/free"));
    }

    #[test]
    fn an_agent_model_pointing_at_a_configured_provider_is_fine() {
        let mut p = provider("my-gw");
        p.api_key = "sk-x".into();
        let providers = vec![p];
        let mut agent = AgentRow::new();
        agent.key = "build".into();
        agent.model = "my-gw/some-model".into();
        let agents = vec![agent];
        let issues = collect(&input(ConfigFormat::Opencode, &providers, &agents));
        assert!(
            !titles(&issues).iter().any(|t| t.contains("本页网关不认")),
            "指向自己配的 provider 是合法的: {issues:?}"
        );
    }

    #[test]
    fn an_agent_with_no_model_is_not_reported() {
        let mut agent = AgentRow::new();
        agent.key = "build".into();
        agent.model = String::new();
        let agents = vec![agent];
        let issues = collect(&input(ConfigFormat::Opencode, &[], &agents));
        assert!(
            !titles(&issues).iter().any(|t| t.contains("本页网关不认")),
            "空 model 是「还没选」，不是错误: {issues:?}"
        );
    }

    #[test]
    fn agent_model_check_is_skipped_on_non_opencode_pages() {
        let mut agent = AgentRow::new();
        agent.key = "build".into();
        agent.model = "kilo/kilo-auto/free".into();
        let agents = vec![agent];
        let issues = collect(&input(ConfigFormat::Pi, &[], &agents));
        assert!(
            !titles(&issues).iter().any(|t| t.contains("本页网关不认")),
            "非 opencode 系页面不参与网关判定: {issues:?}"
        );
    }

    #[test]
    fn a_missing_key_is_only_an_info() {
        let providers = vec![provider("p1")];
        let issues = collect(&input(ConfigFormat::Opencode, &providers, &[]));
        let issue = issues
            .iter()
            .find(|issue| issue.title == "未填密钥")
            .expect("应报出未填密钥");
        assert_eq!(issue.severity, Severity::Info);
    }

    #[test]
    fn dsh_reads_the_env_var_field_instead_of_the_literal_key() {
        let mut p = provider("p1");
        // DSH's key is an apiKeyEnv reference; the apiKey field means nothing to it.
        p.api_key_env = "MY_KEY".into();
        let providers = vec![p];
        let issues = collect(&input(ConfigFormat::DeepSeekHarness, &providers, &[]));
        assert!(
            !titles(&issues).contains(&"未填密钥"),
            "填了 apiKeyEnv 就不该报未填密钥: {issues:?}"
        );
    }

    #[test]
    fn agents_that_will_not_be_written_are_reported_on_other_formats() {
        let mut agent = AgentRow::new();
        agent.key = "build".into();
        let agents = vec![agent];
        let issues = collect(&HealthInput {
            page: ConfigFormat::Pi,
            providers: &[],
            agents: &agents,
            source_is_opencode: false,
        });
        let issue = issues
            .iter()
            .find(|issue| issue.title.contains("不会写进本页格式"))
            .expect("应提示 agents 不会被写入");
        assert!(issue.where_.contains("1 个 agent"));
    }

    #[test]
    fn blockers_sort_before_warnings_and_info() {
        // One duplicate key (blocker) + one suspicious baseUrl (caution) + one missing key (info).
        let mut a = provider("dup");
        a.base_url = "api.example.com".into();
        let b = provider("dup");
        let providers = vec![a, b];
        let issues = collect(&input(ConfigFormat::Opencode, &providers, &[]));
        let severities: Vec<Severity> = issues.iter().map(|issue| issue.severity).collect();
        let mut sorted = severities.clone();
        sorted.sort();
        assert_eq!(severities, sorted, "必须按严重度排序: {issues:?}");
        assert_eq!(severities[0], Severity::Blocker);
    }

    #[test]
    fn severity_labels_are_distinct() {
        let labels = [
            Severity::Blocker.label(),
            Severity::Warn.label(),
            Severity::Info.label(),
        ];
        assert_eq!(labels[0], "会阻止保存");
        assert_ne!(labels[0], labels[1]);
        assert_ne!(labels[1], labels[2]);
    }
}
