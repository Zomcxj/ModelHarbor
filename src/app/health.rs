//! 配置体检：把散落在各处的检查汇总成一张清单，保存前一次看完。
//!
//! ## 为什么需要
//!
//! 这些检查此前各自为政：key 重复只在**保存失败时**从状态栏冒一句、非法数字只在
//! 保存**之后**说「忽略了 N 项」、baseUrl 可疑只在卡片上挂个小 ⚠、agent 的 model
//! 指向别家网关则**切页时被静默替换**（`normalize_agent_models_for_page`）。
//! 用户想知道「我这套配置到底有没有毛病」，得逐页逐卡片翻。
//!
//! 这里不新增判定口径，只把既有口径**汇总**：每一条都复用原来的实现
//! （[`crate::util::url_suspicions`]、[`crate::opencode_models::model_is_valid_on`] 等），
//! 避免出现「体检说没问题、保存却失败」这种两套标准。
//!
//! ## 只读
//!
//! 体检**不改任何东西**，也不提供「一键修复」：这些问题的正确修法取决于用户意图
//! （重复的 model id 该留哪一条？可疑的 baseUrl 该不该改？），自动改就是替用户做决定。
//! 清单只回答「哪里有疑点」，改还是不改、怎么改，由用户在表单里定。

use crate::format::ConfigFormat;
use crate::model::{AgentRow, ProviderRow};

/// 体检结论的严重度，决定清单里的排序与配色。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) enum Severity {
    /// 保存会被拦住，必须先处理。
    Blocker,
    /// 保存能过，但结果可能不是用户想要的。
    Warn,
    /// 只是提个醒（例如某些后端本就不需要密钥）。
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

/// 一条体检结论。`where_` 是定位用的名字（provider key / agent 名 / 字段名）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Issue {
    pub severity: Severity,
    /// 一句话说清问题。
    pub title: String,
    /// 定位：出问题的 provider / agent / 条目。
    pub where_: String,
    /// 为什么这是问题、以及怎么处理。
    pub detail: String,
}

/// 体检的输入：只依赖界面状态，不读文件、不发网络请求。
pub(super) struct HealthInput<'a> {
    pub page: ConfigFormat,
    pub providers: &'a [ProviderRow],
    pub agents: &'a [AgentRow],
    /// 来源格式是否为 opencode 系（决定 agents 是否会写进目标文件）。
    pub source_is_opencode: bool,
}

/// 汇总所有体检结论，按严重度排序（阻断在前）。
pub(super) fn collect(input: &HealthInput<'_>) -> Vec<Issue> {
    let mut issues = Vec::new();
    check_duplicate_keys(input, &mut issues);
    check_empty_identifiers(input, &mut issues);
    check_invalid_numbers(input, &mut issues);
    check_base_urls(input, &mut issues);
    check_agent_models(input, &mut issues);
    check_api_keys(input, &mut issues);
    check_agents_not_written(input, &mut issues);
    // 稳定排序：同级内保持上面各检查的发现顺序（provider 顺序 = 界面顺序）。
    issues.sort_by_key(|issue| issue.severity);
    issues
}

/// 重复的 key / model id。与保存的判定**同一口径**（`find_duplicate_keys`）：
/// 这里报了、那里就一定会拦。
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

/// 空 key / 空 id：这些条目保存时会被跳过，等于白填。
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

/// 非法数字字段：保存时被忽略（不写坏配置，但用户以为填进去了）。
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

/// baseUrl 可疑写法（缺协议头 / 重复斜杠 / 末尾斜杠 / 含空白）。
///
/// 只提示、不自动改写：末尾斜杠是否该去掉取决于目标客户端的拼 URL 行为，
/// 见 `docs/DETAILS.md` 里各家的 `/v1` 归一规则。
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

/// agent 的 `model` 指向本页不认的网关。
///
/// 这是最值得提前看见的一条：切到 opencode 系页面时这种引用会被**静默替换**成
/// 本页网关的首选模型（`normalize_agent_models_for_page`），保存时也按目标页归一。
/// 与其等它被换掉，不如先告诉用户「这一条在本页无效」。
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
        // 空 model 是「还没选」，不是「选错了」。
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

/// 没填密钥的 provider。
///
/// 定为「提示」而不是「问题」：本地推理（Ollama 等）与公开网关本就不需要密钥。
fn check_api_keys(input: &HealthInput<'_>, out: &mut Vec<Issue>) {
    for provider in input.providers {
        if provider.key.trim().is_empty() {
            continue;
        }
        let has_secret = !provider.api_key_secret.trim().is_empty();
        // DSH 走 apiKeyEnv 引用（真实值在同级 .credentials.yaml），其余后端看 apiKey。
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

/// 当前格式写不了 agents 时提醒一句：界面上改得好好的，保存却不会落盘。
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

/// 按严重度统计条数，供入口按钮上显示角标。
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
        // timeout / context 留空是合法状态（表示不指定），不该报。
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
        // opencode 页不认 kilo 前缀，且没有同名 provider。
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
        // DSH 的密钥是 apiKeyEnv 引用，apiKey 字段对它没有意义。
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
        // 一个重复 key（阻断）+ 一个可疑 baseUrl（注意）+ 一个缺密钥（提示）。
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
