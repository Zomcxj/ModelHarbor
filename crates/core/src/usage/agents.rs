//! The [`ConfigFormat`] ↔ tokscale client id mapping.
//!
//! **The two sides' naming disagrees on 6 of 10** — explicit mapping is mandatory; string-assembly inference
//! would guarantee mistakes:
//!
//! | ModelHarbor | tokscale | Difference |
//! |---|---|---|
//! | `Kilocode` | `kilo` | tokscale also has a `kilocode` (the VS Code extension);
//! |            |         | ModelHarbor manages the Kilo **CLI** |
//! | `Mimocode` | `micode` | Different spelling |
//! | `OhMyPi` | `omp` | Abbreviation |
//! | `DeepSeekHarness` | `dsh` | Abbreviation |
//! | `QwenCode` | `qwen` | `-code` dropped |
//! | `KimiCode` | `kimi` | `-code` dropped |

use crate::format::ConfigFormat;

/// ModelHarbor's config format → tokscale's client id.
pub fn tokscale_client_for(format: ConfigFormat) -> Option<&'static str> {
    Some(match format {
        ConfigFormat::Opencode => "opencode",
        // ⚠️ Not `kilocode`: that's the VS Code extension (`ui_messages.json`);
        // ModelHarbor's kilocode backend manages the Kilo CLI's `kilo.json`.
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

/// tokscale's client id → ModelHarbor's config format.
///
/// `None` means the client isn't among ModelHarbor's 10 agents — tokscale supports 56, and scans may
/// surface claude / codex / cursor etc.; callers should drop them.
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

    /// Every backend maps to a tokscale id, without repeats.
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

    /// Round-trip consistency: format → client → format returns the original.
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

    /// The error-prone ones pinned one by one: these 6 can't be inferred from strings.
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

    /// `kilocode` is a different client (the VS Code extension) — never maps to ModelHarbor's kilocode.
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

    /// Clients outside the 10 return None (tokscale has 56).
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

    /// The scan's input is exactly 10 clients.
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
