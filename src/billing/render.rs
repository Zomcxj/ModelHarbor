//! 汇总成 [`Billing`]，以及卡片主行与悬停详情的文案。
use super::*;

/// [`parse_token_billing`] 的输入（字段多，打包成一个结构）。
pub struct TokenInputs<'a> {
    /// 令牌额度接口的结果；该接口整个不存在时为 `None`（日志统计照常出）。
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

/// 汇总「令牌额度 + 调用日志 + 站点单位」→ 卡片摘要（全程无需面板 token）。
pub fn parse_token_billing(inputs: TokenInputs<'_>) -> Billing {
    let unit = inputs.units.quota_per_unit;
    let to_money = |points: f64| points / unit;
    let today = summarize_logs(inputs.logs, Some(inputs.today_from));
    let week = summarize_logs(inputs.logs, Some(inputs.now - 7 * 86_400));
    // 只请求首个分页，无法证明服务端没有后续页（今日 / 近 7 天统计可能偏小）。
    // 额度接口可能整个不存在（把 baseUrl 指向中转域名的站点就没有这个面板路由）：
    // 此时日志统计照常出，额度三项留空，绝不编数字。
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
        // 不限额度时没有「余额 / 上限」可言，不编数字。
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
    /// 界面用它配合 [`Shape::Unknown`] 把「查不到」的站点隐掉。
    pub fn is_empty(&self) -> bool {
        self.used_usd.is_none()
            && self.balance_usd.is_none()
            && self.today_usd.is_none()
            && self.raw_credit.is_none()
    }

    /// 卡片上的一行摘要（尽量短，卡片收起时也显示）。
    pub fn inline(&self) -> String {
        // 账号级数据优先：它才是「我还剩多少钱」，令牌级降为补充。
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

    /// 把签到段接到摘要行末尾（没数据就不接，绝不写占位）。
    fn push_checkin(&self, parts: &mut Vec<String>) {
        if let Some(status) = &self.checkin {
            parts.push(format!("签到 {}", checkin_short(status)));
        }
    }

    /// 有账号数据时的摘要各段：账号余额 + 已用（没余额时才退而单说已用 / 请求数），其次今日。
    fn account_parts(&self, account: &AccountInfo) -> Vec<String> {
        let mut parts: Vec<String> = Vec::new();
        if let Some(balance) = account.balance_usd {
            parts.push(format!("账号余额 {}", money(balance)));
            // 已用紧跟余额：只看余额看不出「用掉多少」，而对那些令牌额度接口缺失、
            // 调用日志又为空的中转站（如 ps.air-outer.com）来说，
            // 这个数字是唯一拿得到的用量。
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
        // 面板账户 / 令牌额度：数字是真实的（不是占位额度），最多显示两段：
        // 有余额就先显余额，其次今日用量（没有今日就用累计已用）。
        if self.source == Source::Token {
            let mut parts: Vec<String> = Vec::new();
            match (self.balance_usd, self.used_usd) {
                (Some(balance), _) => parts.push(format!("余额 {}", money(balance))),
                // 不限额度站：「已用」才是重点（写「余额」会让人以为里面有钱）。
                (None, Some(used)) => parts.push(format!("已用 {}", money(used))),
                (None, None) => {}
            }
            match (self.today_usd, self.used_usd) {
                (Some(today), _) => parts.push(format!("今日 {}", money(today))),
                // 拿不到今日时用累计兼顾信息量（只在已有余额时补，避免重复）
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

    /// 卡片上的一行完整用量（字段多，单独占一行；悬停看详情）。
    ///
    /// 与 [`Billing::inline`] 的区别：这里把能拿到的字段都列出来
    ///（余额 / 累计 / 今日 + 请求数 / 近 7 天），用于卡片正文那一行。
    /// 是否有可展示的用量结果（Unknown / 空数据隐藏）。
    ///
    /// 账号数据（面板令牌）单独就能让卡片显示：站点未开 `/api/usage/token/` 时，
    /// 只要拿到账号额度就不该整块隐掉。
    pub fn is_displayable(&self) -> bool {
        // 签到状态也算可展示数据：有些站点只有签到读得到（有就输出，没有就不输出）。
        self.has_token_side() || self.account.is_some() || self.checkin.is_some()
    }

    /// 令牌侧（令牌额度 / 兼容账单）是否真有可展示数据。
    fn has_token_side(&self) -> bool {
        self.shape != Shape::Unknown && !self.is_empty()
    }

    pub fn inline_full(&self) -> String {
        let parts = self.summary_parts();
        if parts.is_empty() {
            return self.inline();
        }
        parts.join(" · ")
    }

    /// 卡片主行要突出的那一个数字（余额优先，其次已用）。
    ///
    /// 卡片上只能有一个「第一眼看到」的数：多个数字等权并排时，
    /// 反而哪个都记不住。取不到就返回 `None`（副行照常出）。
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

    /// 摘要各段（顺序即优先级：第一段会被当作主数字）。
    fn summary_parts(&self) -> Vec<String> {
        // 账号级余额优先，且不再把令牌级累计挤在同一行（降到 `detail`）。
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
        // 账号数据（面板令牌）与令牌数据是两套口径：同时存在时分节列出，不混在一起。
        if let Some(account) = &self.account {
            lines.extend(Self::account_lines(account));
        }
        // 签到也是账号级信息（靠同一个面板令牌读），就跟账号那节排在一起。
        if let Some(status) = &self.checkin {
            lines.push(format!("签到：{}", checkin_long(status)));
        }
        let has_account_side = self.account.is_some() || self.checkin.is_some();
        if has_account_side && !self.has_token_side() {
            // 只有账号 / 签到数据：不写一个空的「本令牌」分节。
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
    /// 悬停小窗是「扫一眼」的地方：每项占一行、外加一句解释，会把数字挤到
    /// 视线之外。这里按「额度一行、计数与分组一行」合并；但「同站点共用」
    /// 这层含义要留个短标记——否则读者会以为这是某个 sk- 令牌的余额。
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
        // 额度接口缺失（`Shape::TokenLogsOnly`）时不写任何一行：
        // 缺数据本身不产生数字，写一句解释只是噪音。
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
