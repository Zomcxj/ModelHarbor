//! 响应解析：兼容账单、面板状态、账号、签到、调用日志。
use super::*;
use serde_json::Value;

/// 解析 `/api/status` 的面板名（`system_name` + 版本），失败返回空串。
pub fn parse_panel(status_json: Option<&str>) -> String {
    let Some(text) = status_json else {
        return String::new();
    };
    let Ok(root) = serde_json::from_str::<Value>(text) else {
        return String::new();
    };
    let data = root.get("data").unwrap_or(&root);
    let name = data
        .get("system_name")
        .or_else(|| data.get("systemName"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let version = data.get("version").and_then(Value::as_str).unwrap_or("");
    match (name.is_empty(), version.is_empty()) {
        (true, true) => String::new(),
        (false, true) => name.to_string(),
        (true, false) => version.to_string(),
        (false, false) => format!("{} {}", name, version),
    }
}

/// 汇总解析订阅 + 用量 + 站点信息。
pub fn parse(
    subscription_json: &str,
    usage_json: Option<&str>,
    status_json: Option<&str>,
) -> Billing {
    let mut out = Billing {
        panel: parse_panel(status_json),
        ..Default::default()
    };
    let Ok(sub) = serde_json::from_str::<Value>(subscription_json) else {
        return out;
    };

    // 非标准形状：直接展示原值，不做单位换算（避免猜错量级）。
    if sub.get("object").and_then(Value::as_str) == Some("credit_summary")
        || sub.get("total_granted").is_some()
    {
        out.shape = Shape::CreditSummary;
        out.raw_credit = Some(format!(
            "总额度 {} / 已用 {} / 可用 {}（单位未标注，按原值展示）",
            raw_num(sub.get("total_granted")),
            raw_num(sub.get("total_used")),
            raw_num(sub.get("total_available")),
        ));
        return out;
    }

    // 常规 OpenAI 兼容账单。
    // `hard_limit_usd` 为 0 / 缺失表示「无硬上限」（不少站用 0 占位），
    // 此时算出来的「余额」会是负数，误导性强 —— 直接当成没有额度信息。
    out.limit_usd = sub
        .get("hard_limit_usd")
        .and_then(Value::as_f64)
        .or_else(|| sub.get("soft_limit_usd").and_then(Value::as_f64))
        .filter(|limit| *limit > 0.0);
    out.placeholder_limit = out
        .limit_usd
        .is_some_and(|limit| limit >= PLACEHOLDER_LIMIT_USD);
    // 占位额度算出来的「余额」没有意义（1e8 − 已用）：直接不产生这个数字，
    // 免得界面上出现一个看着像真实余额的量。
    if out.placeholder_limit {
        out.limit_usd = None;
    }
    if let Some(text) = usage_json {
        // total_usage 单位是美分。
        out.used_usd = serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|usage| usage.get("total_usage").and_then(Value::as_f64))
            .map(|cents| cents / 100.0);
    }
    if let (Some(limit), Some(used)) = (out.limit_usd, out.used_usd) {
        // 额度比已用小（数据不一致或已超支）时不显示负数余额：只说清楚已用多少。
        let balance = limit - used;
        if balance >= 0.0 {
            out.balance_usd = Some(balance);
        }
    }
    out.shape = if out.limit_usd.is_some() || out.used_usd.is_some() {
        Shape::Subscription
    } else {
        Shape::Unknown
    };
    out
}

/// 解析 `/api/user/self`（**需要面板访问令牌/PAT**，普通登录用户即可）。
///
/// `data.quota` 是**剩余**额度、`used_quota` 是累计已用，两者都是 quota 点，
/// 按 `/api/status` 给出的 [`Units`] 换算成金额（与令牌额度同一套换算，不另猜汇率）。
///
/// 以下情况返回 `None`（视为「没拿到账号数据」，由调用方决定降级或提示）：
/// - 不是 JSON / 根不是对象；
/// - `success` 显式为 `false`（令牌无效、未提供令牌时站点用 200 + `success:false` 报错）；
/// - 一个可用字段都没有（只有 `group` 这类非数字字段不算数据）。
pub fn parse_account_self(json: &str, units: &Units) -> Option<AccountInfo> {
    let root = serde_json::from_str::<Value>(json).ok()?;
    if root.get("success").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    // 站点都在 `data` 下返回；少数变体把字段放在根上，一并兼容。
    let data = root
        .get("data")
        .filter(|value| value.is_object())
        .unwrap_or(&root);
    if !data.is_object() {
        return None;
    }
    let quota = data.get("quota").and_then(Value::as_f64);
    let used = data.get("used_quota").and_then(Value::as_f64);
    // 次数容忍整数 / 浮点两种写法（站点实现不完全一致）。
    let requests = data.get("request_count").and_then(value_as_u64);
    // 全是 None 说明这不是一份可用的账号数据（例如 success:true 但 data 为空）。
    if quota.is_none() && used.is_none() && requests.is_none() {
        return None;
    }
    let to_money = |points: f64| points / units.quota_per_unit;
    Some(AccountInfo {
        balance_usd: quota.map(to_money),
        used_usd: used.map(to_money),
        requests,
        group: data
            .get("group")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

/// 读一个非负整数（容忍整数 / 浮点两种写法：站点实现不完全一致）。
fn value_as_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_f64().filter(|n| *n >= 0.0).map(|n| n as u64))
}

/// 签到状态（`GET /api/user/checkin`，只需面板令牌/PAT，**只读**）。
///
/// 不是每个站点都开这个功能：未启用时站点用 `200 + success:false` 报
/// 「签到功能未启用」，[`parse_checkin_status`] 会把原话作为错误返回。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CheckinStatus {
    /// 站点是否启用签到。
    pub enabled: bool,
    /// 今天是否已签到。
    pub today: bool,
    /// **今天**签到的额度（美元）：站点只在 `records` 里逐日给，取最大日期那条。
    /// 今天没签（或站点没给明细）时为 `None`。
    pub today_usd: Option<f64>,
    /// 本月签到次数。
    pub month_count: u64,
    /// 累计签到次数。
    pub total_count: u64,
    /// 累计获得的额度（美元）；站点没给就是 `None`。
    pub total_usd: Option<f64>,
    /// 单次签到额度区间（美元）；站点未配置（0）时为 `None`。
    pub min_usd: Option<f64>,
    pub max_usd: Option<f64>,
}

/// 取站点自己给的错误消息（`message` 字段），取不到就用一句兜底。
///
/// 站点的话比我们猜的准：「签到功能未启用」「今日已签到」
/// 「Turnstile token 为空」都是它自己说的，要原样带给用户。
fn site_message(root: &Value, fallback: &str) -> String {
    root.get("message")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

/// 解析 `GET /api/user/checkin`。
///
/// 失败情形一律返回 `Err`（而不是 `None`）：未启用、被拒绝、结构不认识
/// 都需要给用户一句具体的话，不能被当成「这个站点没有签到」而静默。
pub fn parse_checkin_status(json: &str, units: &Units) -> Result<CheckinStatus, String> {
    let root = serde_json::from_str::<Value>(json).map_err(|_| "返回内容不是 JSON".to_string())?;
    if !root.is_object() {
        return Err("返回内容不是可识别的签到状态".to_string());
    }
    if root.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(site_message(&root, "站点拒绝了签到状态查询"));
    }
    let data = root
        .get("data")
        .filter(|value| value.is_object())
        .ok_or_else(|| "返回内容不是可识别的签到状态".to_string())?;
    let stats = data.get("stats").filter(|value| value.is_object());
    let count = |key: &str| {
        stats
            .and_then(|stats| stats.get(key))
            .and_then(value_as_u64)
            .unwrap_or(0)
    };
    let to_money = |points: f64| points / units.quota_per_unit;
    let today = stats
        .and_then(|stats| stats.get("checked_in_today"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // 今日金额：站点只在 `records` 里逐日给额度。取**最大日期**那条
    // （`YYYY-MM-DD` 的字典序就是时间序），不依赖站点给的先后顺序；
    // 且只在 `checked_in_today` 为真时才算「今天」——否则最大日期未必是今天，
    // 把别的日子当成今日会误导。
    let today_usd = if today {
        stats
            .and_then(|stats| stats.get("records"))
            .and_then(Value::as_array)
            .and_then(|records| {
                records
                    .iter()
                    .filter_map(|record| {
                        let date = record.get("checkin_date").and_then(Value::as_str)?;
                        let quota = record.get("quota_awarded").and_then(Value::as_f64)?;
                        Some((date, quota))
                    })
                    .max_by_key(|(date, _)| *date)
                    .map(|(_, quota)| quota)
            })
            .map(to_money)
    } else {
        None
    };
    // 0 表示站点没配置这一项：显示「额度 $0.00」只会误导。
    let positive_usd = |key: &str| {
        data.get(key)
            .and_then(Value::as_f64)
            .filter(|points| *points > 0.0)
            .map(to_money)
    };
    Ok(CheckinStatus {
        enabled: data
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        today,
        today_usd,
        month_count: count("checkin_count"),
        total_count: count("total_checkins"),
        total_usd: stats
            .and_then(|stats| stats.get("total_quota"))
            .and_then(Value::as_f64)
            .filter(|points| *points > 0.0)
            .map(to_money),
        min_usd: positive_usd("min_quota"),
        max_usd: positive_usd("max_quota"),
    })
}

/// 签到状态的**短**描述（卡片那一行里用，越短越好）。
///
/// 站点没给的数字一律不提：不编 `$0.00`、不编「本月 0 次」。
pub fn checkin_short(status: &CheckinStatus) -> String {
    if !status.enabled {
        return "未启用".to_string();
    }
    if !status.today {
        return "未签".to_string();
    }
    match status.today_usd {
        Some(today) => format!("今日已签 {}", money(today)),
        None => "今日已签".to_string(),
    }
}

/// 签到状态的**完整**描述（悬停说明里用）。
pub fn checkin_long(status: &CheckinStatus) -> String {
    if !status.enabled {
        return "该站点未启用签到".to_string();
    }
    let mut text = checkin_short(status);
    let mut extra: Vec<String> = Vec::new();
    if status.month_count > 0 {
        extra.push(format!("本月 {} 次", status.month_count));
    }
    if let Some(total) = status.total_usd {
        extra.push(format!("累计获得 {}", money(total)));
    }
    if !extra.is_empty() {
        text.push_str(&format!("（{}）", extra.join("，")));
    }
    text
}

/// 一条调用日志（只留统计要用的字段；`content` / `ip` 这类隐私字段不解析）。#[derive(Clone, Debug, PartialEq)]
pub struct LogEntry {
    /// 发生时间（秒级 Unix 时间戳）。
    pub at: i64,
    /// 消耗（quota 点）。
    pub quota: f64,
    pub model: String,
}

/// 解析 `/api/log/token` 的日志列表（`{success, data:[...]}`；`data` 缺失时为空表）。
pub fn parse_token_logs(json: &str) -> Vec<LogEntry> {
    let Ok(root) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let Some(items) = root.get("data").and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let at = item.get("created_at").and_then(Value::as_i64)?;
            Some(LogEntry {
                at,
                quota: item.get("quota").and_then(Value::as_f64).unwrap_or(0.0),
                model: item
                    .get("model_name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            })
        })
        .collect()
}

/// 日志聚合结果（`from` 为 `None` 表示全部）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LogSummary {
    pub count: usize,
    pub calls: u64,
    /// 区间内消耗（quota 点）。
    pub quota: f64,
    /// 按模型汇总（quota 降序，最多 [`MODEL_TOP`] 项）。
    pub models: Vec<(String, f64)>,
}

/// 悬停里最多列几个模型。
pub const MODEL_TOP: usize = 3;

/// 按时间下限汇总日志（时间戳全为秒）。
pub fn summarize_logs(entries: &[LogEntry], from: Option<i64>) -> LogSummary {
    let mut summary = LogSummary::default();
    let mut per_model: Vec<(String, f64)> = Vec::new();
    for entry in entries {
        if from.is_some_and(|from| entry.at < from) {
            continue;
        }
        summary.count += 1;
        summary.calls += 1;
        summary.quota += entry.quota;
        match per_model.iter_mut().find(|(name, _)| name == &entry.model) {
            Some((_, quota)) => *quota += entry.quota,
            None => per_model.push((entry.model.clone(), entry.quota)),
        }
    }
    per_model.sort_by(|a, b| b.1.total_cmp(&a.1));
    per_model.truncate(MODEL_TOP);
    summary.models = per_model;
    summary
}
