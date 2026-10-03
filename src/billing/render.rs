//! 汇总成 [`Billing`]，以及卡片主行与悬停详情的文案。
use super::*;

/// [`parse_token_billing`] 的输入。
pub struct TokenInputs<'a> {
    /// 令牌额度接口的结果；该接口不存在时为 `None`。
    pub usage: Option<&'a TokenUsage>,
    pub logs: &'a [LogEntry],
    pub units: &'a Units,
    pub status_json: Option<&'a str>,
    /// 当前时间（秒级 Unix 时间戳）。
    pub now: i64,
    /// 本地时区今天 0 点。
    pub today_from: i64,
    pub note: Option<String>,
}

/// 汇总「令牌额度 + 调用日志 + 站点单位」→ 卡片摘要。
pub fn parse_token_billing(inputs: TokenInputs<'_>) -> Billing {
    let unit = inputs.units.quota_per_unit;
    let to_money = |points: f64| points / unit;
    let today = summarize_logs(inputs.logs, Some(inputs.today_from));
    let week = summarize_logs(inputs.logs, Some(inputs.now - 7 * 86_400));
    // 只请求首个分页，今日 / 近 7 天统计可能偏小。
    // 额度接口缺失时日志统计照常出，额度三项留空。
    let unlimited = inputs.usage.is_some_and(|usage| usage.unlimited);
    Billing {
        panel: parse_panel(inputs.status_json),
        source: Source::Token,
        shape: match inputs.usage {
            None => Shape::TokenLogsOnly,
            Some(usage) if usage.unlimited => Shape::TokenUnlimited,
            Some(_) => Shape::TokenQuota,
        },
        used_usd: inputs.usage.and_then(|usage| usage.used).map(to_money),
        // 不限额度时没有余额与上限。
        balance_usd: inputs
            .usage
            .filter(|usage| !usage.unlimited)
            .and_then(|usage| usage.available)
            .map(to_money),
        limit_usd: inputs
            .usage
            .filter(|usage| !usage.unlimited)
            .and_then(|usage| usage.granted)
            .filter(|value| *value > 0.0)
            .map(to_money),
        unlimited,
        today_usd: (!today.models.is_empty() || today.count > 0).then(|| to_money(today.quota)),
        today_calls: (today.count > 0).then_some(today.calls),
        today_models: today
            .models
            .iter()
            .map(|(name, quota)| (name.clone(), to_money(*quota)))
            .collect(),
        week_usd: (week.count > 0).then(|| to_money(week.quota)),
        unit_assumed: inputs.units.assumed,
        note: inputs.note,
        ..Default::default()
    }
}

impl Billing {
    /// 跨日本地快照隐藏“今日”字段，保留累计、余额与近 7 天。
    pub fn expire_today(&mut self) {
        self.today_usd = None;
        self.today_calls = None;
        self.today_models.clear();
        self.today_stale = true;
    }

    /// 是否一个可用数字都没有（没余额、没已用、没今日、没原值）。
    ///
    /// 界面用它配合 [`Shape::Unknown`] 把这类站点隐掉。
    pub fn is_empty(&self) -> bool {
        self.used_usd.is_none()
            && self.balance_usd.is_none()
            && self.today_usd.is_none()
            && self.raw_credit.is_none()
    }

    /// 卡片上的一行摘要。
    pub fn inline(&self) -> String {
        // 账号级数据优先。
        if let Some(account) = &self.account {
            let parts = self.account_parts(account);
            return if parts.is_empty() {
                "未返回额度信息".to_string()
            } else {
                parts.join(" · ")
            };
        }
        self.inline_token_or_compat()
    }

    /// 把签到段接到摘要行末尾；没数据则不接。
    fn push_checkin(&self, parts: &mut Vec<String>) {
        if let Some(status) = &self.checkin {
            parts.push(format!("签到 {}", checkin_short(status)));
        }
    }

    /// 有账号数据时的摘要各段：账号余额 + 已用（没余额时单说已用 / 请求数），其次今日。
    fn account_parts(&self, account: &AccountInfo) -> Vec<String> {
        let mut parts: Vec<String> = Vec::new();
        if let Some(balance) = account.balance_usd {
            parts.push(format!("账号余额 {}", money(balance)));
            // 已用紧跟余额。
            if let Some(used) = account.used_usd {
                parts.push(format!("已用 {}", money(used)));
            }
        } else if let Some(used) = account.used_usd {
            parts.push(format!("账号已用 {}", money(used)));
        } else if let Some(requests) = account.requests {
            parts.push(format!("账号请求 {} 次", requests));
        }
        if let Some(today) = self.today_usd {
            parts.push(match self.today_calls {
                Some(calls) => format!("今日 {}（{} 次）", money(today), calls),
                None => format!("今日 {}", money(today)),
            });
        }
        if let Some(week) = self.week_usd {
            parts.push(format!("近 7 天 {}", money(week)));
        }
        self.push_checkin(&mut parts);
        parts
    }

    /// 没有账号数据时的短行（令牌额度 / 兼容账单）。
    fn inline_token_or_compat(&self) -> String {
        // 面板账户 / 令牌额度：最多两段，有余额先显余额，其次今日用量。
        if self.source == Source::Token {
            let mut parts: Vec<String> = Vec::new();
            match (self.balance_usd, self.used_usd) {
                (Some(balance), _) => parts.push(format!("余额 {}", money(balance))),
                // 不限额度站只显已用。
                (None, Some(used)) => parts.push(format!("已用 {}", money(used))),
                (None, None) => {}
            }
            match (self.today_usd, self.used_usd) {
                (Some(today), _) => parts.push(format!("今日 {}", money(today))),
                // 拿不到今日时用累计，只在已有余额时补。
                (None, Some(used)) if self.balance_usd.is_some() => {
                    parts.push(format!("累计 {}", money(used)))
                }
                (None, _) => {}
            }
            self.push_checkin(&mut parts);
            if parts.is_empty() {
                return "未返回额度信息".to_string();
            }
            return parts.join(" · ");
        }
        if self.raw_credit.is_some() {
            return "额度单位未标注（悬停看原值）".to_string();
        }
        match (self.used_usd, self.balance_usd) {
            (Some(used), Some(balance)) => {
                format!("已用 {} · 余额 {}", money(used), money(balance))
            }
            (Some(used), None) => format!("已用 {}", money(used)),
            (None, Some(balance)) => format!("余额 {}", money(balance)),
            (None, None) => "未返回额度信息".to_string(),
        }
    }

    /// 是否有可展示的用量结果（Unknown / 空数据隐藏）。
    ///
    /// 账号数据单独就能让卡片显示。
    pub fn is_displayable(&self) -> bool {
        // 签到状态也算可展示数据。
        self.has_token_side() || self.account.is_some() || self.checkin.is_some()
    }

    /// 令牌侧（令牌额度 / 兼容账单）是否真有可展示数据。
    fn has_token_side(&self) -> bool {
        self.shape != Shape::Unknown && !self.is_empty()
    }

    /// 卡片上的一行完整用量，列出能拿到的字段（余额 / 累计 / 今日 + 请求数 / 近 7 天）。
    pub fn inline_full(&self) -> String {
        let parts = self.summary_parts();
        if parts.is_empty() {
            return self.inline();
        }
        parts.join(" · ")
    }

    /// 卡片主行要突出的那一个数字（余额优先，其次已用）。
    ///
    /// 取不到时返回 `None`。
    pub fn headline(&self) -> Option<String> {
        self.summary_parts().into_iter().next()
    }

    /// 主行除 [`Billing::headline`] 之外的其余数字（字号降一档显示）。
    pub fn inline_rest(&self) -> String {
        let parts = self.summary_parts();
        if parts.len() <= 1 {
            return String::new();
        }
        parts[1..].join(" · ")
    }

    /// 摘要各段，顺序即优先级（第一段作为主数字）。
    fn summary_parts(&self) -> Vec<String> {
        // 账号级余额优先。
        if let Some(account) = &self.account {
            return Self::account_parts(self, account);
        }
        let mut parts: Vec<String> = Vec::new();
        match (self.balance_usd, self.used_usd) {
            (Some(balance), _) => {
                parts.push(format!("余额 {}", money(balance)));
                if let Some(used) = self.used_usd {
                    parts.push(format!("累计 {}", money(used)));
                }
            }
            (None, Some(used)) => parts.push(format!("已用 {}", money(used))),
            (None, None) => {}
        }
        if let Some(today) = self.today_usd {
            parts.push(match self.today_calls {
                Some(calls) => format!("今日 {}（{} 次）", money(today), calls),
                None => format!("今日 {}", money(today)),
            });
        }
        if let Some(week) = self.week_usd {
            parts.push(format!("近 7 天 {}", money(week)));
        }
        if let Some(raw) = &self.raw_credit {
            parts.push(raw.clone());
        }
        self.push_checkin(&mut parts);
        parts
    }

    /// 悬停提示：完整说明与口径来源。
    pub fn detail(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        if !self.panel.is_empty() {
            lines.push(format!("面板：{}", self.panel));
        }
        // 账号数据与令牌数据是两套口径，同时存在时分节列出。
        if let Some(account) = &self.account {
            lines.extend(Self::account_lines(account));
        }
        // 签到也是账号级信息，跟账号那节排在一起。
        if let Some(status) = &self.checkin {
            lines.push(format!("签到：{}", checkin_long(status)));
        }
        let has_account_side = self.account.is_some() || self.checkin.is_some();
        if has_account_side && !self.has_token_side() {
            // 只有账号 / 签到数据：不写空的「本令牌」分节。
            return lines.join("\n");
        }
        if has_account_side {
            lines.push("—— 本令牌 ——".to_string());
        }
        if self.source == Source::Token {
            return self.detail_token(lines);
        }
        self.detail_compat(lines)
    }

    /// 账号级额度（`/api/user/self`）的悬停说明。
    ///
    /// 每项合并成「额度一行、计数与分组一行」，「同站点共用」在标题里标出。
    fn account_lines(account: &AccountInfo) -> Vec<String> {
        let mut lines = vec!["账号级额度（面板访问令牌，同站点共用）".to_string()];
        let mut amounts: Vec<String> = Vec::new();
        if let Some(balance) = account.balance_usd {
            amounts.push(format!("余额 {}", money(balance)));
        }
        if let Some(used) = account.used_usd {
            amounts.push(format!("已用 {}", money(used)));
        }
        if !amounts.is_empty() {
            lines.push(amounts.join(" · "));
        }
        let mut meta: Vec<String> = Vec::new();
        if let Some(requests) = account.requests {
            meta.push(format!("请求 {} 次", requests));
        }
        if !account.group.is_empty() {
            meta.push(format!("分组 {}", account.group));
        }
        if !meta.is_empty() {
            lines.push(meta.join(" · "));
        }
        lines
    }

    /// 兼容账单（`/dashboard/billing/*`）的悬停说明。
    fn detail_compat(&self, mut lines: Vec<String>) -> String {
        if let Some(used) = self.used_usd {
            lines.push(format!("已用：{}", money(used)));
        }
        if let Some(limit) = self.limit_usd {
            lines.push(format!("额度：{}", money(limit)));
        }
        if let Some(balance) = self.balance_usd {
            lines.push(format!("余额：{}", money(balance)));
        }
        if let Some(raw) = &self.raw_credit {
            lines.push(format!("账单：{}", raw));
        }
        if lines.is_empty() {
            lines.push("该站没有返回可识别的账单信息".to_string());
        }
        lines.join("\n")
    }

    /// 令牌额度（`/api/usage/token/` + `/api/log/token`）的悬停详情。
    fn detail_token(&self, mut lines: Vec<String>) -> String {
        // 额度接口缺失（`Shape::TokenLogsOnly`）时不写额度行。
        if self.unlimited {
            lines.push("额度：不限".to_string());
        }
        if let Some(used) = self.used_usd {
            lines.push(format!("累计已用：{}", money(used)));
        }
        if let Some(balance) = self.balance_usd {
            lines.push(format!("余额：{}", money(balance)));
        }
        if let Some(limit) = self.limit_usd {
            lines.push(format!("额度：{}", money(limit)));
        }
        match (self.today_usd, self.today_calls) {
            (Some(today), Some(calls)) => {
                lines.push(format!("今日已用：{}（{} 次请求）", money(today), calls));
            }
            (Some(today), None) => lines.push(format!("今日已用：{}", money(today))),
            (None, Some(calls)) => lines.push(format!("今日请求：{} 次", calls)),
            (None, None) => {}
        }
        if !self.today_models.is_empty() {
            let models: Vec<String> = self
                .today_models
                .iter()
                .map(|(name, usd)| format!("{} {}", name, money(*usd)))
                .collect();
            lines.push(format!("今日模型：{}", models.join("、")));
        }
        if let Some(week) = self.week_usd {
            lines.push(format!("近 7 天：{}", money(week)));
        }
        if self.today_stale {
            lines.push("今日数据已跨日，请重新查询".to_string());
        }
        if self.unit_assumed {
            lines.push(
                "换算：站点没给 quota_per_unit，按默认 1 美元 = 500,000 quota 估算".to_string(),
            );
        }
        lines.join("\n")
    }
}
