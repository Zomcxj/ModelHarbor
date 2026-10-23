//! Aggregate into [`Billing`], plus the card's main line and hover detail text.
use super::*;

/// Input of [`parse_token_billing`].
pub struct TokenInputs<'a> {
    /// The token-quota endpoint's result; `None` when the endpoint doesn't exist.
    pub usage: Option<&'a TokenUsage>,
    pub logs: &'a [LogEntry],
    pub units: &'a Units,
    pub status_json: Option<&'a str>,
    /// Current time (second-resolution Unix timestamp).
    pub now: i64,
    /// The local timezone's midnight today.
    pub today_from: i64,
    pub note: Option<String>,
}

/// Aggregate "token quota + call logs + station units" → card summary.
pub fn parse_token_billing(inputs: TokenInputs<'_>) -> Billing {
    let unit = inputs.units.quota_per_unit;
    let to_money = |points: f64| points / unit;
    let today = summarize_logs(inputs.logs, Some(inputs.today_from));
    let week = summarize_logs(inputs.logs, Some(inputs.now - 7 * 86_400));
    // Only the first page is requested, so today / last-7-days stats may undercount.
    // When the quota endpoint is missing, log stats still emit with the three quota fields left empty.
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
        // Unlimited quota has no balance or cap.
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
        // Expiry: 0 / negative (no expiry) doesn't count; labels are filled by the app layer.
        expires: inputs
            .usage
            .and_then(|usage| usage.expires_at)
            .filter(|ts| *ts > 0.0)
            .map(|ts| KeyExpiry {
                ts: ts as i64,
                ..Default::default()
            }),
        unit_assumed: inputs.units.assumed,
        note: inputs.note,
        ..Default::default()
    }
}

impl Billing {
    /// A local midnight rollover hides the "today" fields, keeping lifetime, balance, and last-7-days.
    pub fn expire_today(&mut self) {
        self.today_usd = None;
        self.today_calls = None;
        self.today_models.clear();
        self.today_stale = true;
    }

    /// Whether not a single usable number exists (no balance, no usage, no today, no raw values).
    ///
    /// The UI uses it with [`Shape::Unknown`] to hide such stations.
    pub fn is_empty(&self) -> bool {
        self.used_usd.is_none()
            && self.balance_usd.is_none()
            && self.today_usd.is_none()
            && self.raw_credit.is_none()
    }

    /// One summary line on the card.
    pub fn inline(&self) -> String {
        // Account-level data wins.
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

    /// Append the check-in segment to the summary line's end; skipped when there's no data.
    fn push_checkin(&self, parts: &mut Vec<String>) {
        if let Some(status) = &self.checkin {
            parts.push(format!("签到 {}", checkin_short(status)));
        }
    }

    /// Append expiry urgency to the summary line's end; takes a slot only when near (≤7 days) or expired.
    fn push_expiry(&self, parts: &mut Vec<String>) {
        if let Some(expiry) = &self.expires {
            match expiry.days_left {
                Some(n) if n < 0 => parts.push("已过期".to_string()),
                Some(n) if (0..=7).contains(&n) => parts.push(format!("{n} 天后到期")),
                _ => {}
            }
        }
    }

    /// Summary segments when account data exists: account balance + used (usage / request count alone when there's no balance), then today.
    fn account_parts(&self, account: &AccountInfo) -> Vec<String> {
        let mut parts: Vec<String> = Vec::new();
        if let Some(balance) = account.balance_usd {
            parts.push(format!("账号余额 {}", money(balance)));
            // Used follows the balance.
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
        self.push_expiry(&mut parts);
        // Expiry urgency: takes a slot only when near (≤7 days) or expired.
        parts
    }

    /// The short line without account data (token quota / compatible billing).
    fn inline_token_or_compat(&self) -> String {
        // Panel account / token quota: at most two segments — balance first when present, then today's usage.
        if matches!(self.source, Source::Token | Source::Sub2api) {
            let mut parts: Vec<String> = Vec::new();
            match (self.balance_usd, self.used_usd) {
                (Some(balance), _) => parts.push(format!("余额 {}", money(balance))),
                // Unlimited stations show usage only.
                (None, Some(used)) => parts.push(format!("已用 {}", money(used))),
                (None, None) => {}
            }
            match (self.today_usd, self.used_usd) {
                (Some(today), _) => parts.push(format!("今日 {}", money(today))),
                // Without today's data, lifetime substitutes — appended only when a balance already shows.
                (None, Some(used)) if self.balance_usd.is_some() => {
                    parts.push(format!("累计 {}", money(used)))
                }
                (None, _) => {}
            }
            self.push_checkin(&mut parts);
            self.push_expiry(&mut parts);
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

    /// Whether there's a presentable usage result (Unknown / empty data hides).
    ///
    /// Account data alone can make the card display.
    pub fn is_displayable(&self) -> bool {
        // Check-in status also counts as presentable data.
        self.has_token_side() || self.account.is_some() || self.checkin.is_some()
    }

    /// Whether the token side (token quota / compatible billing) truly has presentable data.
    fn has_token_side(&self) -> bool {
        self.shape != Shape::Unknown && !self.is_empty()
    }

    /// One full usage line on the card, listing every obtainable field (balance / lifetime / today + requests / last 7 days).
    pub fn inline_full(&self) -> String {
        let parts = self.summary_parts();
        if parts.is_empty() {
            return self.inline();
        }
        parts.join(" · ")
    }

    /// The number the card's main line highlights (balance first, then usage).
    ///
    /// Returns `None` when nothing is available.
    pub fn headline(&self) -> Option<String> {
        self.summary_parts().into_iter().next()
    }

    /// The main line's remaining numbers besides [`Billing::headline`] (displayed one font size smaller).
    pub fn inline_rest(&self) -> String {
        let parts = self.summary_parts();
        if parts.len() <= 1 {
            return String::new();
        }
        parts[1..].join(" · ")
    }

    /// The summary's segments, in priority order (the first is the headline number).
    fn summary_parts(&self) -> Vec<String> {
        // Account-level balance wins.
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

    /// The hover tip: full explanation and the data's provenance.
    pub fn detail(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        if !self.panel.is_empty() {
            lines.push(format!("面板：{}", self.panel));
        }
        // Account and token data are two accounting systems; listed in sections when both exist.
        if let Some(account) = &self.account {
            lines.extend(Self::account_lines(account));
        }
        // Check-in is also account-level info, grouped with the account section.
        if let Some(status) = &self.checkin {
            lines.push(format!("签到：{}", checkin_long(status)));
        }
        // Panel per-day usage (/api/data/self): includes non-local usage — the accounting is noted.
        if !self.panel_days.is_empty() {
            lines.push("面板按天（站点统计，含非本机用量）".to_string());
            for day in &self.panel_days {
                let label = if day.date.is_empty() {
                    format!("ts {}", day.day_ts)
                } else {
                    day.date.clone()
                };
                lines.push(format!("  {label}：{}（{} 次）", money(day.usd), day.calls));
            }
        }
        let has_account_side =
            self.account.is_some() || self.checkin.is_some() || !self.panel_days.is_empty();
        if has_account_side && !self.has_token_side() {
            // Account / check-in data only: don't write an empty "this token" section.
            return lines.join("\n");
        }
        if has_account_side {
            lines.push("—— 本令牌 ——".to_string());
        }
        if matches!(self.source, Source::Token | Source::Sub2api) {
            return self.detail_token(lines);
        }
        self.detail_compat(lines)
    }

    /// The hover note for account-level quota (`/api/user/self`).
    ///
    /// Items merge into "quota on one line, counts and group on another"; "shared per station" is flagged in the title.
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

    /// The hover note for compatible billing (`/dashboard/billing/*`).
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

    /// The hover details for token quota (`/api/usage/token/` + `/api/log/token`).
    fn detail_token(&self, mut lines: Vec<String>) -> String {
        // With the quota endpoint missing (`Shape::TokenLogsOnly`), no quota line is written.
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
        if let Some(expiry) = &self.expires {
            if !expiry.label.is_empty() {
                let tail = match expiry.days_left {
                    Some(n) if n < 0 => "（已过期）".to_string(),
                    Some(n) if (0..=7).contains(&n) => format!("（{n} 天后到期）"),
                    _ => String::new(),
                };
                lines.push(format!("到期：{}{tail}", expiry.label));
            }
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
