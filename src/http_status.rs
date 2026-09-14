//! HTTP 状态码的「人话解释」：报错时在状态码后面补上原因和处理建议。
//!
//! 用途：
//! - 延迟 / 连通性探测：`HTTP 503 服务临时不可用（1234 ms）`；
//! - 拉取模型列表失败：`HTTP 404 接口或模型不存在：Not Found` + 换行给出处理建议；
//! - 卡片 / 列表里的单行错误摘要：`HTTP 503 服务临时不可用`。
//!
//! 判定口径面向「中转站 / API Gateway」场景：4xx 通常是请求侧（参数、鉴权、额度、
//! 模型名、上下文），5xx 通常是中转站、网关或上游服务侧。5xx 与 429 可以重试；
//! 400 / 401 / 403 / 404 / 413 / 422 重试没用，要先改配置或请求内容。
//!
//! 未收录的状态码只显示数字（`HTTP 599`），不猜原因。

/// 一条状态码解释：`reason` 用于单行摘要，`hint` 用于悬停提示 / 弹层里的建议。
struct Entry {
    code: u16,
    reason: &'static str,
    hint: &'static str,
}

/// 常见状态码表（按码值升序；单测会校验有序、无重复、文案非空）。
const ENTRIES: &[Entry] = &[
    Entry {
        code: 400,
        reason: "请求格式错误",
        hint: "参数、请求体或模型名不符合接口要求；检查模型名与 Base URL",
    },
    Entry {
        code: 401,
        reason: "认证失败",
        hint: "API Key 缺失、写错或已失效；重新复制 Key，确认前后没有空格",
    },
    Entry {
        code: 402,
        reason: "余额或额度不足",
        hint: "账户余额或套餐额度用尽；充值，或换一个有额度的 Key",
    },
    Entry {
        code: 403,
        reason: "无权限",
        hint: "Key 无权调用该模型，或被地区 / 账号限制；换模型或检查 Key 权限",
    },
    Entry {
        code: 404,
        reason: "接口或模型不存在",
        hint: "Base URL 路径或模型名写错；检查接口地址与模型 id",
    },
    Entry {
        code: 405,
        reason: "请求方法不允许",
        hint: "该端点不接受这个 HTTP 方法；多为地址写错，或中转站适配不全",
    },
    Entry {
        code: 406,
        reason: "响应格式不被接受",
        hint: "服务端无法给出请求要求的格式；检查 Accept 头与流式设置",
    },
    Entry {
        code: 408,
        reason: "请求超时",
        hint: "服务端等待客户端超时，或请求排队过久；重试，或缩短上下文",
    },
    Entry {
        code: 409,
        reason: "请求冲突",
        hint: "并发或状态冲突，多见于会话类接口；稍后重试",
    },
    Entry {
        code: 410,
        reason: "接口或模型已下线",
        hint: "服务端已移除该接口或模型；换模型，或更新 Base URL",
    },
    Entry {
        code: 413,
        reason: "请求体过大",
        hint: "上下文、文件或图片超过服务端上限；减少内容，或重新开会话",
    },
    Entry {
        code: 415,
        reason: "媒体类型不支持",
        hint: "Content-Type 或上传格式不被支持；检查请求头与上传格式",
    },
    Entry {
        code: 422,
        reason: "参数无法处理",
        hint: "参数格式正确但内容不被支持（如不支持某些工具调用）；换模型，或关掉该功能",
    },
    Entry {
        code: 429,
        reason: "请求过多（限流）",
        hint: "RPM / TPM 或并发超限、上游限流；等一会儿、换线路，或提高限额",
    },
    Entry {
        code: 500,
        reason: "服务端内部错误",
        hint: "中转站或上游内部异常；重试，或查中转站后台日志",
    },
    Entry {
        code: 501,
        reason: "接口未实现",
        hint: "请求的端点或功能不被支持（如某些 /responses 子路径）；换接口，或让中转站适配",
    },
    Entry {
        code: 502,
        reason: "网关错误",
        hint: "中转站连不上上游、上游挂了或线路异常；换线路 / 换模型 / 稍后重试",
    },
    Entry {
        code: 503,
        reason: "服务临时不可用",
        hint: "上游繁忙、维护或过载；稍后重试，或切到备用线路",
    },
    Entry {
        code: 504,
        reason: "网关超时",
        hint: "上游长时间未返回、流式输出中断；重试、缩短上下文，或换线路",
    },
    Entry {
        code: 505,
        reason: "HTTP 版本不支持",
        hint: "服务端不支持请求使用的 HTTP 版本；多为网关配置问题",
    },
    Entry {
        code: 507,
        reason: "存储配额不足",
        hint: "账户存储或配额用尽；检查账户额度",
    },
    Entry {
        code: 520,
        reason: "未知错误（CF）",
        hint: "Cloudflare 与源站之间返回了无法识别的内容；稍后重试，或联系中转站",
    },
    Entry {
        code: 521,
        reason: "源站不可用（CF）",
        hint: "Cloudflare 找不到源站，或源站拒绝连接；稍后重试，或换线路",
    },
    Entry {
        code: 522,
        reason: "连接超时（CF）",
        hint: "Cloudflare 连不上源站；稍后重试，或换线路",
    },
    Entry {
        code: 523,
        reason: "源站不可达（CF）",
        hint: "源站地址或 DNS 解析有问题；联系中转站",
    },
    Entry {
        code: 524,
        reason: "响应超时（CF）",
        hint: "源站处理超过 Cloudflare 网关上限（默认 100 秒）；缩短上下文，或换模型",
    },
    Entry {
        code: 525,
        reason: "SSL 握手失败（CF）",
        hint: "Cloudflare 与源站的 TLS 握手失败；多为源站证书或配置问题",
    },
    Entry {
        code: 526,
        reason: "证书无效（CF）",
        hint: "源站证书无效或不受信任；联系中转站",
    },
    Entry {
        code: 529,
        reason: "站点过载",
        hint: "上游整体过载（Anthropic 常见）；稍后重试，或换线路",
    },
];

fn entry(code: u16) -> Option<&'static Entry> {
    ENTRIES.iter().find(|e| e.code == code)
}

/// 单行摘要：`HTTP 503 服务临时不可用`；未收录的状态码只给数字。
pub fn label(code: u16) -> String {
    match entry(code) {
        Some(e) => format!("HTTP {} {}", code, e.reason),
        None => format!("HTTP {}", code),
    }
}

/// 悬停提示用的完整说明：`HTTP 503 服务临时不可用：上游繁忙、维护或过载；稍后重试…`。
pub fn detail(code: u16) -> String {
    match entry(code) {
        Some(e) => format!("{}：{}", label(code), e.hint),
        None => label(code),
    }
}

/// 处理建议；未收录的状态码返回 `None`。
pub fn hint(code: u16) -> Option<&'static str> {
    entry(code).map(|e| e.hint)
}

/// 成因归类：4xx = 请求侧，5xx = 中转站 / 网关 / 上游侧。
pub fn side(code: u16) -> &'static str {
    if (400..500).contains(&code) {
        "请求侧"
    } else if (500..600).contains(&code) {
        "中转站 / 网关 / 上游侧"
    } else {
        "未知"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_sorted_and_unique() {
        for pair in ENTRIES.windows(2) {
            assert!(
                pair[0].code < pair[1].code,
                "状态码表必须严格升序且不重复：{} / {}",
                pair[0].code,
                pair[1].code
            );
        }
    }

    #[test]
    fn entries_have_reason_and_hint() {
        for e in ENTRIES {
            assert!(!e.reason.is_empty(), "{} 缺 reason", e.code);
            assert!(!e.hint.is_empty(), "{} 缺 hint", e.code);
            // 单行摘要要短：reason 不超过 16 个字符
            assert!(
                e.reason.chars().count() <= 16,
                "{} 的 reason 太长（{} 字符），卡片上会挤",
                e.code,
                e.reason.chars().count()
            );
            // 只收错误码
            assert!((400..600).contains(&e.code), "{} 不是错误码", e.code);
            // 建议里不再嵌套冒号，避免 detail 出现两个冒号
            assert!(
                !e.hint.contains('：'),
                "{} 的 hint 不要用全角冒号，改用分号：{}",
                e.code,
                e.hint
            );
        }
    }

    #[test]
    fn label_appends_reason_after_code() {
        assert_eq!(label(503), "HTTP 503 服务临时不可用");
        assert_eq!(label(401), "HTTP 401 认证失败");
        assert_eq!(label(429), "HTTP 429 请求过多（限流）");
    }

    #[test]
    fn unknown_code_has_no_guessed_reason() {
        assert_eq!(label(599), "HTTP 599");
        assert_eq!(detail(599), "HTTP 599");
        assert!(hint(599).is_none());
        assert_eq!(label(200), "HTTP 200", "成功码不带原因");
    }

    #[test]
    fn detail_contains_label_and_hint() {
        let d = detail(503);
        assert!(d.starts_with(&label(503)), "detail 必须以 label 开头：{d}");
        assert!(d.contains(hint(503).unwrap()), "detail 必须含处理建议：{d}");
        assert_eq!(d.matches('：').count(), 1, "detail 只应有一个冒号：{d}");
    }

    #[test]
    fn side_splits_request_from_upstream() {
        assert_eq!(side(404), "请求侧");
        assert_eq!(side(503), "中转站 / 网关 / 上游侧");
        assert_eq!(side(200), "未知");
    }

    /// 速查表里最常被误判的一组：429 / 5xx 可重试，401 / 404 重试无用。
    #[test]
    fn retry_semantics_are_documented() {
        for code in [500, 502, 503, 504, 529, 429] {
            let h = hint(code).unwrap();
            assert!(
                h.contains("重试") || h.contains("等一会儿"),
                "{code} 属于可重试状态，建议里应提到重试：{h}"
            );
        }
        for code in [401, 404] {
            let h = hint(code).unwrap();
            assert!(
                h.contains("检查") || h.contains("Key"),
                "{code} 重试无用，建议应先改配置：{h}"
            );
        }
    }
}
