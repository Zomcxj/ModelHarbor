//! [`ConfigFormat`] ↔ tokscale client id 的映射。
//!
//! **两边命名有 6/10 不一致**，必须显式映射，不能靠字符串拼接推断：
//!
//! | ModelHarbor | tokscale | 差异 |
//! |---|---|---|
//! | `Kilocode` | `kilo` | tokscale 另有一个 `kilocode`（VS Code 扩展），
//! |            |         | ModelHarbor 管的是 Kilo **CLI** |
//! | `Mimocode` | `micode` | 拼写不同 |
//! | `OhMyPi` | `omp` | 缩写 |
//! | `DeepSeekHarness` | `dsh` | 缩写 |
//! | `QwenCode` | `qwen` | 去掉 `-code` |
//! | `KimiCode` | `kimi` | 去掉 `-code` |

use crate::format::ConfigFormat;

/// ModelHarbor 的配置格式 → tokscale 的 client id。
pub fn tokscale_client_for(format: ConfigFormat) -> Option<&'static str> {
    Some(match format {
        ConfigFormat::Opencode => "opencode",
        // ⚠️ 不是 `kilocode`：那是 VS Code 扩展（`ui_messages.json`），
        // ModelHarbor 的 kilocode 后端管的是 Kilo CLI 的 `kilo.json`。
        ConfigFormat::Kilocode => "kilo",
        ConfigFormat::Mimocode => "micode",
        ConfigFormat::Pi => "pi",
        ConfigFormat::OhMyPi => "omp",
        ConfigFormat::DeepSeekHarness => "dsh",
        ConfigFormat::ZCode => "zcode",
        ConfigFormat::WorkBuddy => "workbuddy",
        ConfigFormat::QwenCode => "qwen",
        ConfigFormat::KimiCode => "kimi",
    })
}

/// tokscale 的 client id → ModelHarbor 的配置格式。
///
/// 返回 `None` 表示该 client 不在 ModelHarbor 的 10 个 agent 里 —— tokscale
/// 支持 56 个，扫描结果里可能出现 claude / codex / cursor 等，调用方应丢弃。
pub fn config_format_for_client(client: &str) -> Option<ConfigFormat> {
    Some(match client {
        "opencode" => ConfigFormat::Opencode,
        "kilo" => ConfigFormat::Kilocode,
        "micode" => ConfigFormat::Mimocode,
        "pi" => ConfigFormat::Pi,
        "omp" => ConfigFormat::OhMyPi,
        "dsh" => ConfigFormat::DeepSeekHarness,
        "zcode" => ConfigFormat::ZCode,
        "workbuddy" => ConfigFormat::WorkBuddy,
        "qwen" => ConfigFormat::QwenCode,
        "kimi" => ConfigFormat::KimiCode,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个后端都能映射到 tokscale id，且不重复。
    #[test]
    fn every_backend_maps_to_a_distinct_client() {
        let mut seen = Vec::new();
        for backend in crate::backends::BACKENDS {
            let client = tokscale_client_for(backend.id())
                .unwrap_or_else(|| panic!("{} 缺少 tokscale 映射", backend.id().label()));
            assert!(
                !seen.contains(&client),
                "{} 与前面的后端映射到了同一个 client {client}",
                backend.id().label()
            );
            seen.push(client);
        }
        assert_eq!(seen.len(), crate::backends::BACKENDS.len());
    }

    /// 往返一致：format → client → format 回到原值。
    #[test]
    fn round_trip_is_stable() {
        for backend in crate::backends::BACKENDS {
            let format = backend.id();
            let client = tokscale_client_for(format).expect("应有映射");
            assert_eq!(
                config_format_for_client(client),
                Some(format),
                "{} 的往返映射不一致",
                format.label()
            );
        }
    }

    /// 易错项逐个钉住：这 6 个不能靠字符串推断。
    #[test]
    fn pinned_spellings_match_tokscale() {
        assert_eq!(tokscale_client_for(ConfigFormat::Kilocode), Some("kilo"));
        assert_eq!(tokscale_client_for(ConfigFormat::Mimocode), Some("micode"));
        assert_eq!(tokscale_client_for(ConfigFormat::OhMyPi), Some("omp"));
        assert_eq!(
            tokscale_client_for(ConfigFormat::DeepSeekHarness),
            Some("dsh")
        );
        assert_eq!(tokscale_client_for(ConfigFormat::QwenCode), Some("qwen"));
        assert_eq!(tokscale_client_for(ConfigFormat::KimiCode), Some("kimi"));
    }

    /// `kilocode` 是另一个 client（VS Code 扩展），不能映射到 ModelHarbor 的 kilocode。
    #[test]
    fn vs_code_kilocode_extension_is_not_model_harbors_kilocode() {
        assert_eq!(
            config_format_for_client("kilocode"),
            None,
            "tokscale 的 `kilocode` 是 VS Code 扩展，不属于 ModelHarbor 的 10 个 agent"
        );
        assert_eq!(
            config_format_for_client("kilo"),
            Some(ConfigFormat::Kilocode),
            "Kilo CLI 才是 ModelHarbor 的 kilocode"
        );
    }

    /// 不在 10 个之内的 client 返回 None（tokscale 有 56 个）。
    #[test]
    fn unknown_clients_are_rejected() {
        for client in [
            "claude", "codex", "cursor", "gemini", "copilot", "", "PI", "Pi",
        ] {
            assert_eq!(
                config_format_for_client(client),
                None,
                "{client} 不应映射到任何后端"
            );
        }
    }

    /// 扫描入参正好是 10 个 client。
    #[test]
    fn scan_clients_cover_every_backend_once() {
        let clients = super::super::tokscale_clients();
        assert_eq!(clients.len(), crate::backends::BACKENDS.len());
        let mut sorted = clients.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), clients.len(), "不得重复");
    }
}
