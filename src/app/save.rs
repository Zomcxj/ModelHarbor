//! 保存 / 预览文档的路径解析、校验、写盘与各后端加载入口。
use super::{App, SaveFormat};
use crate::backends;
use crate::credentials;
use crate::format::ConfigFormat;
use crate::model::{AgentRow, ProviderRow};
use crate::util::{self, is_wsl_path, parse_number_text};
use serde_json::{Map, Value};
use std::collections::HashSet;

/// 一个保存目标的运行时状态（可用性 / 解析路径 / 勾选）。
pub(super) struct SaveTarget {
    pub(super) backend: ConfigFormat,
    pub(super) available: bool,
    pub(super) path: String,
}

/// 本页写入路径的解析结果。
#[derive(Clone)]
pub(super) enum PageTarget {
    /// 当前文件（已加载）：整体替换 agent / provider。
    Current(String),
    /// 路径已修改但未重新加载：按“先读后合并”写入，不破坏目标文件已有配置。
    Modified(String),
    /// 该后端默认目标（Windows 本地路径；WSL 仅在勾选「WSL同步」后写入）。
    Default(String),
}

/// 跨格式转换前，从目标 root 中剔除由当前组件状态接管的容器：
/// provider（opencode 的 provider / pi 系与 DSH 的 providers）与 opencode 的 agent。
///
/// 目的：跨格式保存/预览时，provider 条目与顺序完全以界面为准（干净转换），
/// 同时目标文件的其他顶层字段（如 DSH 的 llm-pi-ai 下其他设置）原样保留。
/// 同格式目标（WSL 同步等）不走这里，仍用保守合并。
///
/// **删除必须用 `shift_remove`，不能用 `remove`。** `serde_json` 开了 `preserve_order`
/// （底层 `IndexMap`），它的 `remove` 是 `swap_remove`：删一个键会把**最后一个**键搬到
/// 空出来的槽位，其余键的相对顺序随之打乱。跨格式保存 ZCode 时先删 `providerOrder`，
/// 写出的文件就成了 `modelConfigRules, providerConfigRules, providerOrder` —— ZCode 读得懂，
/// 但与它自己写出的顺序不一致。`shift_remove` 保留其余键的顺序。
///
/// `agents_owned` 表示界面确实持有 agents 数据。agents 只属于 opencode 页：
/// 数据来自 pi / oh-my-pi / DSH（或空载启动）时界面无从表达 agents，
/// 此时必须保留目标文件里的 agent 容器，否则会把它们静默删掉。
pub fn strip_cross_format_containers(fmt: ConfigFormat, root: &mut Value, agents_owned: bool) {
    // workbuddy 的根是数组：整体清空即是「剔除界面接管的容器」。
    if fmt == ConfigFormat::WorkBuddy {
        *root = Value::Array(Vec::new());
        return;
    }
    let Some(obj) = root.as_object_mut() else {
        return;
    };
    match fmt {
        // opencode 系（opencode / kilocode / mimocode）容器名相同，共用一套剔除。
        ConfigFormat::Opencode | ConfigFormat::Kilocode | ConfigFormat::Mimocode => {
            obj.shift_remove("provider");
            if agents_owned {
                obj.shift_remove("agent");
            }
        }
        ConfigFormat::Pi | ConfigFormat::OhMyPi => {
            obj.shift_remove("providers");
        }
        ConfigFormat::DeepSeekHarness => {
            if let Some(llm) = obj.get_mut("llm-pi-ai").and_then(Value::as_object_mut) {
                llm.shift_remove("providers");
            }
        }
        ConfigFormat::ZCode => {
            if let Some(cfg) = obj.get_mut("config").and_then(Value::as_object_mut) {
                cfg.shift_remove("providerOrder");
                if let Some(rules) = cfg
                    .get_mut("providerConfigRules")
                    .and_then(Value::as_object_mut)
                {
                    rules.shift_remove("providerRules");
                }
                if let Some(rules) = cfg
                    .get_mut("modelConfigRules")
                    .and_then(Value::as_object_mut)
                {
                    rules.shift_remove("providerModelRules");
                }
            }
        }
        // 已在上面提前返回，这里不会到达；列出以保持 match 穷尽。
        ConfigFormat::WorkBuddy => {}
    }
}

/// 加载 opencode 配置；读取/解析失败返回 Err。
pub fn load_opencode_result(
    path: &str,
) -> Result<(Value, Vec<AgentRow>, Vec<ProviderRow>), String> {
    let load = crate::backends::load_backend(ConfigFormat::Opencode, path)?;
    Ok((load.root, load.agents, load.providers))
}

/// 兼容包装：失败时回退空状态（供测试与旧调用方使用）。
pub fn load_or_empty(path: &str) -> (Value, Vec<AgentRow>, Vec<ProviderRow>) {
    load_opencode_result(path)
        .unwrap_or_else(|_| (Value::Object(Map::new()), Vec::new(), Vec::new()))
}

impl App {
    /// 当前 agent 文件是否使用某个 provider 级字段。
    /// 判断范围是整个文件，不是单个 provider；切换到尚未加载的目标页时
    /// 使用完整 schema，保证新增 provider 可以输入所有专属字段。
    pub(super) fn page_has_provider_field(&self, field: &str) -> bool {
        if self.source_format != self.current_page {
            return true;
        }
        self.providers
            .iter()
            .any(|provider| match self.current_page {
                // opencode 系三者字段口径相同，共用一套判定。
                ConfigFormat::Opencode | ConfigFormat::Kilocode | ConfigFormat::Mimocode => {
                    match field {
                        "base_url" => provider
                            .raw
                            .get("options")
                            .and_then(|v| v.get("baseURL"))
                            .is_some(),
                        "timeout" => provider
                            .raw
                            .get("options")
                            .and_then(|v| v.get("timeout"))
                            .is_some(),
                        _ => false,
                    }
                }
                ConfigFormat::Pi | ConfigFormat::OhMyPi => match field {
                    "base_url" => provider.raw.get("baseUrl").is_some(),
                    _ => false,
                },
                ConfigFormat::DeepSeekHarness => match field {
                    "base_url" => provider.raw.get("baseURL").is_some(),
                    _ => false,
                },
                // ZCode 的端点/密钥落在 `api.baseUrl` / `access.apiKey`。
                ConfigFormat::ZCode => match field {
                    "base_url" => provider
                        .raw
                        .get("api")
                        .and_then(|v| v.get("baseUrl"))
                        .is_some(),
                    _ => false,
                },
                // WorkBuddy 每条模型自带 url / apiKey。
                ConfigFormat::WorkBuddy => match field {
                    "base_url" => provider.raw.get("url").is_some(),
                    _ => false,
                },
            })
    }

    /// 当前 agent 文件是否使用某个 model 级字段。字段存在性按整个
    /// agent 文件判断，避免单个模型缺字段时导致同一页面布局跳变。
    pub(super) fn page_has_model_field(&self, field: &str) -> bool {
        if self.source_format != self.current_page {
            return true;
        }
        self.providers.iter().any(|provider| {
            provider.models.iter().any(|model| match self.current_page {
                // opencode 系三者模型字段口径相同，共用一套判定。
                ConfigFormat::Opencode | ConfigFormat::Kilocode | ConfigFormat::Mimocode => {
                    match field {
                        "name" => model.raw.get("name").is_some(),
                        "reasoning" => model.raw.get("reasoning").is_some(),
                        "tool_call" => model.raw.get("tool_call").is_some(),
                        "store" => model
                            .raw
                            .get("options")
                            .and_then(|v| v.get("store"))
                            .is_some(),
                        "context" => model
                            .raw
                            .get("limit")
                            .and_then(|v| v.get("context"))
                            .is_some(),
                        "output" => model
                            .raw
                            .get("limit")
                            .and_then(|v| v.get("output"))
                            .is_some(),
                        "input" => model
                            .raw
                            .get("modalities")
                            .and_then(|v| v.get("input"))
                            .is_some(),
                        "variants" => model.raw.get("variants").is_some(),
                        _ => false,
                    }
                }
                ConfigFormat::Pi | ConfigFormat::OhMyPi => match field {
                    "name" => model.raw.get("name").is_some(),
                    // reasoning 也可由 pi/omp 的 thinking 块 / thinkingLevelMap 表达，
                    // 只写了这些键时同样应显示（并勾选）reasoning。
                    "reasoning" => {
                        model.raw.get("reasoning").is_some()
                            || model.raw.get("thinkingLevelMap").is_some()
                            || model.raw.get("thinking").is_some()
                            || model.raw.get("reasoningEfforts").is_some()
                    }
                    "context" => model.raw.get("contextWindow").is_some(),
                    "output" => model.raw.get("maxTokens").is_some(),
                    "input" => model.raw.get("input").is_some(),
                    "variants" => {
                        model.raw.get("thinkingLevelMap").is_some()
                            || model.raw.get("thinking").is_some()
                    }
                    _ => false,
                },
                ConfigFormat::DeepSeekHarness => match field {
                    "name" => model.raw.get("name").is_some(),
                    "context" => model.raw.get("contextWindow").is_some(),
                    "output" => model.raw.get("maxTokens").is_some(),
                    "input" => model.raw.get("input").is_some(),
                    "variants" => model.raw.get("reasoningEfforts").is_some(),
                    _ => false,
                },
                // ZCode 的模型属性落在 config.properties / config.optionSpecs。
                ConfigFormat::ZCode => match field {
                    "context" => model
                        .raw
                        .get("properties")
                        .and_then(|v| v.get("contextWindow"))
                        .is_some(),
                    "output" => model
                        .raw
                        .get("optionSpecs")
                        .and_then(|v| v.get("maxOutputTokens"))
                        .is_some(),
                    "input" => model
                        .raw
                        .get("properties")
                        .and_then(|v| v.get("inputFormat"))
                        .is_some(),
                    "tool_call" => model
                        .raw
                        .get("properties")
                        .and_then(|v| v.get("supportsToolCall"))
                        .is_some(),
                    "variants" => model
                        .raw
                        .get("optionSpecs")
                        .and_then(|v| v.get("reasoningLevel"))
                        .is_some(),
                    _ => false,
                },
                // WorkBuddy 每条模型自带全部属性。
                ConfigFormat::WorkBuddy => match field {
                    // 模型名已落到模型行的 id 字段（WB 无独立模型 id），不再单列 name 字段，
                    // 否则 id / name 两个框都显示同一个模型名，反而误导。
                    "name" => false,
                    // 「启用开关」不走这里：它是**语义**字段（不是「文件里有没有」），
                    // 由 `ConfigFormat::has_model_enable()` 判定，见 providers.rs。
                    // 在这里返回 true 会把开关漏到所有页面（曾经就是这样）。
                    "context" => model.raw.get("maxInputTokens").is_some(),
                    "output" => model.raw.get("maxOutputTokens").is_some(),
                    "input" => model.raw.get("supportsImages").is_some(),
                    "tool_call" => model.raw.get("supportsToolCall").is_some(),
                    "reasoning" => model.raw.get("supportsReasoning").is_some(),
                    _ => false,
                },
            })
        })
    }

    /// 将已加载的公共 provider 凭据投影到 DSH 专属字段。
    /// 只在进入 DSH 页面时执行一次，避免用户在页面内主动清空后被立即回填。
    pub(super) fn project_dsh_credentials(&mut self) {
        for provider in &mut self.providers {
            if provider.api_key_env.trim().is_empty() {
                provider.api_key_env = credentials::default_env_name(&provider.key);
            }
        }
    }

    /// 页面切换时保持 provider 密钥一致：DSH 页使用 api_key_secret（对应
    /// .credentials.yaml 的 refs），其他页面使用 api_key。切换时把非空值
    /// 同步到目标页字段；若用户在 DSH 页明确清空过密钥（原本有、当前空），
    /// 不再用其他页面的旧值覆盖。
    pub(super) fn sync_provider_secrets(&mut self, target: ConfigFormat) {
        for provider in &mut self.providers {
            if target == ConfigFormat::DeepSeekHarness {
                let cleared_on_dsh = !provider.original_api_key_secret.is_empty()
                    && provider.api_key_secret.is_empty();
                if !provider.api_key.trim().is_empty() && !cleared_on_dsh {
                    provider.api_key_secret = provider.api_key.clone();
                }
            } else {
                let cleared_on_dsh = !provider.original_api_key_secret.is_empty()
                    && provider.api_key_secret.is_empty();
                if cleared_on_dsh {
                    // 与 DSH 方向对称：在 DSH 页明确清空过的密钥（原本有、当前空）
                    // 不再用旧值填充其他页面，避免已清空的密钥被写回 opencode 等配置。
                    provider.api_key = String::new();
                } else if !provider.api_key_secret.trim().is_empty() {
                    provider.api_key = provider.api_key_secret.clone();
                }
            }
        }
    }

    /// 统计非法数字字段数（非空且解析失败），保存后提示用户它们被忽略。
    pub(super) fn count_invalid_numeric_fields(&self) -> usize {
        fn bad(s: &str) -> bool {
            !s.trim().is_empty() && parse_number_text(s).is_none()
        }
        let mut n = 0;
        for a in &self.agents {
            if bad(&a.temperature) {
                n += 1;
            }
        }
        for p in &self.providers {
            if bad(&p.timeout) {
                n += 1;
            }
            for m in &p.models {
                if bad(&m.context) || bad(&m.output) {
                    n += 1;
                }
            }
        }
        n
    }

    /// 校验 agent / provider / model key 唯一性，返回首个冲突描述。
    pub(super) fn find_duplicate_keys(&self) -> Option<String> {
        let mut seen = HashSet::new();
        for a in &self.agents {
            let k = a.key.trim();
            if !k.is_empty() && !seen.insert(k.to_string()) {
                return Some(format!("agent \"{}\"", k));
            }
        }
        let mut seen_p = HashSet::new();
        for p in &self.providers {
            let k = p.key.trim();
            if k.is_empty() {
                continue;
            }
            if !seen_p.insert(k.to_string()) {
                return Some(format!("provider \"{}\"", k));
            }
            let mut seen_m = HashSet::new();
            for m in &p.models {
                let mk = m.id.trim();
                if !mk.is_empty() && !seen_m.insert(mk.to_string()) {
                    return Some(format!("provider \"{}\" 的 model \"{}\"", k, mk));
                }
            }
        }
        None
    }

    /// 本页写入路径：
    /// - 当前文件属于本页格式且已加载 → 当前文件（整体替换）；
    /// - 路径已修改但未加载 → 仍写该路径，但按“先读后合并”（防止覆盖目标文件已有配置）；
    /// - 其余 → 该后端默认目标（Windows 本地；WSL 需勾选「WSL同步」）。
    pub(super) fn page_save_path(&self, fmt: ConfigFormat) -> PageTarget {
        if !self.config_path.is_empty() && self.config_path != self.loaded_path {
            // 用户已经在路径框中明确指定了目标文件，即使尚未点击“加载”，
            // 也必须使用该路径；保存流程会先读目标并按目标格式合并，不能静默回落默认路径。
            return PageTarget::Modified(self.config_path.clone());
        }
        if self.source_format == fmt && !self.config_path.is_empty() {
            return PageTarget::Current(self.config_path.clone());
        }
        let path = self
            .targets
            .iter()
            .find(|t| t.backend == fmt)
            .map(|t| t.path.clone())
            .unwrap_or_default();
        PageTarget::Default(path)
    }

    /// 保存当前页面：写入该页格式对应的路径，并按需同步 WSL。
    pub(super) fn save_page(&mut self, fmt: ConfigFormat) {
        self.status = self.save_page_report(fmt);
    }

    /// 一键保存：按页面顺序把所有**已安装**后端各写一遍。
    ///
    /// 每个页面的 provider / agent 都来自同一份界面状态，逐页切换再逐个点保存只是
    /// 重复劳动；这里一次写完，并把每页结果拼进状态栏，写没写、写到哪一目了然。
    ///
    /// 路径解析必须分两种：当前页走 [`Self::page_save_path`]（尊重输入框里「已改未加载」
    /// 的路径）；其他页面必须用各自目标路径——`page_save_path` 里的 `config_path` 是
    /// **当前页**的输入框，拿它写别的页面会把全部配置都塞进同一个文件。
    /// 未安装（本地与 WSL 都探测不到）的后端跳过，不去凭空创建配置文件。
    pub(super) fn save_all(&mut self) {
        if let Some(dup) = self.find_duplicate_keys() {
            self.status = format!("key 重复: {}，已取消保存", dup);
            return;
        }
        let current = self.current_page;
        let mut parts: Vec<String> = Vec::new();
        for i in 0..self.targets.len() {
            let (fmt, available, path) = {
                let t = &self.targets[i];
                (t.backend, t.available, t.path.clone())
            };
            if fmt == current {
                parts.push(self.save_page_report(fmt));
            } else if available && !path.trim().is_empty() {
                parts.push(self.save_report_to(fmt, &path));
            }
        }
        self.status = if parts.is_empty() {
            "一键保存: 没有可写的目标（本地与 WSL 均未探测到已安装的配置）".to_string()
        } else {
            format!("一键保存: {}", parts.join(" | "))
        };
    }

    /// 保存一页并返回状态文本（不写 `self.status`，供单页保存与一键保存共用）。
    pub(super) fn save_page_report(&mut self, fmt: ConfigFormat) -> String {
        if let Some(dup) = self.find_duplicate_keys() {
            return format!("key 重复: {}，已取消保存", dup);
        }
        let target = self.page_save_path(fmt);
        let path = match &target {
            PageTarget::Current(p) | PageTarget::Modified(p) | PageTarget::Default(p) => p.clone(),
        };
        match &target {
            PageTarget::Current(_) => {
                if let Some(err) = self.load_error.clone() {
                    return format!("当前文件: 加载失败({})，已跳过", err);
                }
            }
            PageTarget::Default(_) => {
                if !self.targets.iter().any(|t| t.backend == fmt && t.available) {
                    return format!("{}: 未安装（本地与 WSL 均未找到配置）", fmt.label());
                }
            }
            PageTarget::Modified(_) => {}
        }
        self.save_report_to(fmt, &path)
    }

    /// 按已解析好的路径写入一页并返回状态文本（含 WSL 同步）。
    fn save_report_to(&mut self, fmt: ConfigFormat, path: &str) -> String {
        let res = self.save_backend_to(fmt, path);
        let ok = res.is_ok();
        let mut status = match res {
            Ok(backup) => {
                let mut msg = format!("{}: 已保存", fmt.label());
                if let Some(backup) = backup {
                    msg.push_str(&format!("（原文件已备份为 {}）", backup));
                }
                msg
            }
            Err(e) => format!("{}: 保存失败({})", fmt.label(), e),
        };
        if ok {
            // 该格式不支持 agents 时明确告知，避免误以为已写入
            if !fmt.is_opencode_family() && !self.agents.is_empty() {
                status.push_str(&format!(
                    "（{} 个 agents 未写入：该格式不支持）",
                    self.agents.len()
                ));
            }
            let bad = self.count_invalid_numeric_fields();
            if bad > 0 {
                status.push_str(&format!("（已忽略 {} 个无效数字字段）", bad));
            }
        }
        // WSL 同步：仅勾选“WSL同步”且写入路径为本地时，同步到 WSL 侧默认路径；
        // 写入前检测对应 agent 是否已安装（未安装则跳过并提示）。
        if self.sync_wsl && ok && !is_wsl_path(path) {
            match backends::wsl_target(fmt) {
                Some(wsl_path) => match self.save_backend_to(fmt, &wsl_path) {
                    Ok(_) => status.push_str(&format!("; {}(WSL): 已同步", fmt.label())),
                    Err(e) => status.push_str(&format!("; {}(WSL): 同步失败({})", fmt.label(), e)),
                },
                None => status.push_str(&format!("; {}(WSL): 未安装，跳过同步", fmt.label())),
            }
        }
        status
    }

    /// 通用保存：按后端构造 root、渲染内容并写入。
    pub(super) fn save_backend_to(
        &mut self,
        fmt: ConfigFormat,
        path: &str,
    ) -> Result<Option<String>, String> {
        let backend = backends::backend(fmt);
        // 已加载的当前文件整体替换；同格式的其他路径（WSL 同步等）先读后合并；
        // 跨格式目标（来源格式不同）做「干净转换」：目标文件里由组件状态接管的
        // provider / agent 容器整体丢弃（条目与顺序都来自界面），其余顶层字段保留。
        let is_current = self.source_format == fmt && path == self.loaded_path;
        let cross_format = self.source_format != fmt;
        // 写往**别的**页面时，agent 的 model 必须先按目标页网关归一：三页共用同一份
        // agents 数据，而 `model` 的 provider 前缀必须是目标页网关认的（kilo 网关不认
        // `opencode/…`）。当前页写自己加载来的数据则原样落盘——那一份在切页时已经
        // 归一并显示给用户看过，这里再动一次反而会悄悄改掉用户没看到的东西。
        let agents = if cross_format {
            self.agents_for_page(fmt)
        } else {
            self.agents.clone()
        };
        let target_root: Option<Value> = if is_current {
            None
        } else {
            // 目标文件存在但读不出 / 解析不了 → 取消保存。静默回落空对象会把
            // 「先读后合并」变成「对空合并后整体覆写」，目标文件的顶层设置被清掉。
            let mut target = backend
                .load_target_root(path)
                .map_err(|e| format!("目标文件无法读取，已取消保存（未动原文件）: {e}"))?;
            if cross_format {
                strip_cross_format_containers(fmt, &mut target, !agents.is_empty());
            }
            Some(target)
        };
        let root = backend.serialize_root(
            &agents,
            &self.providers,
            self.extras_for(fmt),
            target_root.as_ref(),
        );
        let content = if fmt == ConfigFormat::DeepSeekHarness && is_current {
            // 未发生任何结构化修改时直接保留原始 YAML，避免无意义的
            // 缩进、引号、键顺序变化；实际修改后再使用稳定的 DSH 渲染器。
            match util::read_config_content(path) {
                Ok(original)
                    if util::parse_yaml_content(&original).ok().as_ref() == Some(&root) =>
                {
                    original
                }
                _ => backend.render(&root, self.save_format == SaveFormat::Compact)?,
            }
        } else {
            backend.render(&root, self.save_format == SaveFormat::Compact)?
        };
        let dsh_sidecar_backup = if fmt == ConfigFormat::DeepSeekHarness {
            let sidecar = credentials::sidecar_path(path);
            if util::config_exists(&sidecar) {
                Some((sidecar.clone(), util::read_config_content(&sidecar)?))
            } else {
                Some((sidecar, String::new()))
            }
        } else {
            None
        };
        // WorkBuddy 的「停用」语义是**整条不写**：同一个模型 id 只保留第一条，其余连
        // 所属厂商一起从文件里消失（包括那些厂商的 API key）。这让一次普通保存删掉的
        // 东西可能比跨格式转换还多，所以它也必须先备份，不能沿用「同格式保存不备份」。
        let wb_shrinks = fmt == ConfigFormat::WorkBuddy
            && is_current
            && util::read_config_content(path)
                .ok()
                .and_then(|old| util::parse_config_content(&old).ok())
                .map(|old_root| {
                    let before = old_root.as_array().map(Vec::len).unwrap_or(0);
                    let after = root.as_array().map(Vec::len).unwrap_or(0);
                    before > after
                })
                // 读不出 / 解析不了旧文件时按「会收缩」处理：下面的备份分支会读原文件，
                // 读失败即取消保存。绝不在不知道原文件内容的情况下收缩式覆写。
                .unwrap_or(true);
        // 跨格式转换会整体接管目标文件的 provider/agent：先把原文件滚动备份为 .bak，
        // 备份失败则取消保存（宁可不让存，也不能把旧配置静默抵掉）。原文件**读不出**
        // 同样取消保存——备份的前提是知道原文件里有什么，读失败还继续写就是蒙眼覆写。
        let backup = if (cross_format && !is_current) || wb_shrinks {
            match util::read_config_content(path) {
                Ok(old) if !old.is_empty() && old != content => {
                    let backup_path = format!("{}.bak", path);
                    backends::write_config(&backup_path, &old).map_err(|e| {
                        format!("保存前备份失败（{}），已取消保存: {}", backup_path, e)
                    })?;
                    Some(backup_path)
                }
                Ok(_) => None,
                Err(e) => return Err(format!("读取原文件失败（{path}），已取消保存: {e}")),
            }
        } else {
            None
        };
        // sidecar 一律在主配置**之前**写：WorkBuddy 的全量副本要靠读主配置继承
        // 未知字段，而主配置马上会被筛成「只剩勾选的条目」；副本先落盘才拿得到全量字段。
        // DSH 的凭据 sidecar 无此依赖，同一位置写即可。
        if matches!(fmt, ConfigFormat::DeepSeekHarness | ConfigFormat::WorkBuddy) {
            backend.save_sidecars(path, &self.providers)?;
        }
        if let Err(error) = backends::write_config(path, &content) {
            if let Some((sidecar, old_content)) = dsh_sidecar_backup {
                let restore = if old_content.is_empty() {
                    if util::config_exists(&sidecar) {
                        util::remove_config(&sidecar)
                    } else {
                        Ok(())
                    }
                } else {
                    backends::write_config(&sidecar, &old_content)
                };
                if let Err(restore_error) = restore {
                    return Err(format!("{}；凭据回滚失败: {}", error, restore_error));
                }
            }
            return Err(error);
        }
        // 当前文件保存成功后，回填 opencode 的 extras 载体（self.root）保持与磁盘一致
        if is_current && fmt.is_opencode_family() {
            self.root = root;
        }
        // 磁盘内容变了：对比视图的缓存必须作废，否则「已与磁盘一致」会被继续显示成
        // 「还有 N 处改动」。签名里带上这个计数即可，不必每帧重读文件（目标在 WSL
        // 时读一次要起进程）。计数只增不减，回绕的后果仅是偶尔多算一次差异。
        self.save_serial = self.save_serial.wrapping_add(1);
        Ok(backup)
    }

    /// 当前文件保存时使用的基底 extras（按后端取对应载体）。
    pub(super) fn extras_for(&self, fmt: ConfigFormat) -> &Value {
        match fmt {
            // opencode 系（含 kilocode / mimocode）extras 就是完整 root。
            ConfigFormat::Opencode | ConfigFormat::Kilocode | ConfigFormat::Mimocode => &self.root,
            // pi 系（pi / oh-my-pi）共用 extras 载体：providers 之外的顶层字段
            ConfigFormat::Pi | ConfigFormat::OhMyPi => &self.pi_extras,
            // ZCode / WorkBuddy 与 opencode、DSH 一样，extras 就是完整 root
            // （序列化时由各后端自行保留未接管的容器与顶层字段）。
            ConfigFormat::DeepSeekHarness | ConfigFormat::ZCode | ConfigFormat::WorkBuddy => {
                &self.root
            }
        }
    }
}
