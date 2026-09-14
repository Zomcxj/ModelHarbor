//! 官方 provider 预设：新增 provider 时一键填充 key / baseUrl / 协议。
//!
//! 数据来源：本地 `@earendil-works/pi-ai` v0.85.1 的 `dist/providers/data/*.json`
//! （38 个官方 provider 的权威 baseUrl 与 api 枚举）。
//!
//! **预设只是可选的起点**：
//! - 下拉首项是「(自定义 / 不套用)」，不选预设就能像以前一样全部手填；
//! - 套用之后所有字段依旧可改，第三方、中转站、自建端点不受任何限制；
//! - 预设只写 key / baseUrl / 协议（opencode 页写 npm），
//!   **绝不写入密钥**、也不改动已有的模型列表、超时等字段。
//!
//! 少数 provider 故意不收录：`azure-openai-responses` 的端点形如
//! `https://<资源名>.openai.azure.com/...`，每个账号都不同，须手动填写。

use crate::model::ProviderRow;

/// 预设所属页面的方言，决定协议落在哪个字段。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PresetDialect {
    /// opencode 页：协议用 npm 包表达
    Opencode,
    /// pi / oh-my-pi / DSH 页：协议落在 api 字段
    PiLike,
}

/// 一个官方 provider 的预设值。
pub struct ProviderPreset {
    /// provider key（写进配置里的对象键）
    pub key: &'static str,
    /// 下拉里的显示名
    pub label: &'static str,
    /// 官方 baseUrl；极少数是需用户补占位符的模板（Cloudflare / Vertex）
    pub base_url: &'static str,
    /// pi 方言 api；opencode 页由 `convert::api_to_npm` 推导出 npm
    pub api: &'static str,
}

/// 预设表，常用的排在前面（顺序即下拉顺序）。
pub const PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        key: "openai",
        label: "OpenAI",
        base_url: "https://api.openai.com/v1",
        api: "openai-responses",
    },
    ProviderPreset {
        key: "anthropic",
        label: "Anthropic",
        base_url: "https://api.anthropic.com",
        api: "anthropic-messages",
    },
    ProviderPreset {
        key: "google",
        label: "Google Gemini",
        base_url: "https://generativelanguage.googleapis.com/v1beta",
        api: "google-generative-ai",
    },
    ProviderPreset {
        key: "deepseek",
        label: "DeepSeek",
        base_url: "https://api.deepseek.com",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "openrouter",
        label: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "xai",
        label: "xAI（Grok）",
        base_url: "https://api.x.ai/v1",
        api: "openai-responses",
    },
    ProviderPreset {
        key: "groq",
        label: "Groq",
        base_url: "https://api.groq.com/openai/v1",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "mistral",
        label: "Mistral",
        base_url: "https://api.mistral.ai",
        api: "mistral-conversations",
    },
    ProviderPreset {
        key: "moonshotai",
        label: "Moonshot（Kimi 国际）",
        base_url: "https://api.moonshot.ai/v1",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "moonshotai-cn",
        label: "Moonshot（Kimi 中国）",
        base_url: "https://api.moonshot.cn/v1",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "kimi-coding",
        label: "Kimi Coding",
        base_url: "https://api.kimi.com/coding",
        api: "anthropic-messages",
    },
    ProviderPreset {
        key: "minimax",
        label: "MiniMax",
        base_url: "https://api.minimax.io/anthropic",
        api: "anthropic-messages",
    },
    ProviderPreset {
        key: "minimax-cn",
        label: "MiniMax（中国）",
        base_url: "https://api.minimaxi.com/anthropic",
        api: "anthropic-messages",
    },
    ProviderPreset {
        key: "zai",
        label: "Z.ai（GLM Coding）",
        base_url: "https://api.z.ai/api/coding/paas/v4",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "zai-coding-cn",
        label: "智谱 GLM Coding（中国）",
        base_url: "https://open.bigmodel.cn/api/coding/paas/v4",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "nvidia",
        label: "NVIDIA NIM",
        base_url: "https://integrate.api.nvidia.com/v1",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "cerebras",
        label: "Cerebras",
        base_url: "https://api.cerebras.ai/v1",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "together",
        label: "Together AI",
        base_url: "https://api.together.ai/v1",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "fireworks",
        label: "Fireworks AI",
        base_url: "https://api.fireworks.ai/inference/v1",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "huggingface",
        label: "Hugging Face Router",
        base_url: "https://router.huggingface.co/v1",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "baseten",
        label: "Baseten",
        base_url: "https://inference.baseten.co/v1",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "vercel-ai-gateway",
        label: "Vercel AI Gateway",
        base_url: "https://ai-gateway.vercel.sh",
        api: "anthropic-messages",
    },
    ProviderPreset {
        key: "opencode",
        label: "opencode zen",
        base_url: "https://opencode.ai/zen/v1",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "opencode-go",
        label: "opencode zen（Go）",
        base_url: "https://opencode.ai/zen/go/v1",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "github-copilot",
        label: "GitHub Copilot",
        base_url: "https://api.individual.githubcopilot.com",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "openai-codex",
        label: "OpenAI Codex（ChatGPT 订阅）",
        base_url: "https://chatgpt.com/backend-api",
        api: "openai-codex-responses",
    },
    ProviderPreset {
        key: "ant-ling",
        label: "Ant Ling（蚂蚁百灵）",
        base_url: "https://api.ant-ling.com/v1",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "xiaomi",
        label: "小米 MiMo",
        base_url: "https://api.xiaomimimo.com/v1",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "xiaomi-token-plan-cn",
        label: "小米 Token Plan（中国）",
        base_url: "https://token-plan-cn.xiaomimimo.com/v1",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "xiaomi-token-plan-sgp",
        label: "小米 Token Plan（新加坡）",
        base_url: "https://token-plan-sgp.xiaomimimo.com/v1",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "xiaomi-token-plan-ams",
        label: "小米 Token Plan（阿姆斯特丹）",
        base_url: "https://token-plan-ams.xiaomimimo.com/v1",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "qwen-token-plan",
        label: "通义 Token Plan（新加坡）",
        base_url: "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "qwen-token-plan-cn",
        label: "通义 Token Plan（北京）",
        base_url: "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "qwen-token-plan-individual",
        label: "通义 Token Plan（个人版）",
        base_url: "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "cloudflare-ai-gateway",
        label: "Cloudflare AI Gateway（需补账号占位符）",
        base_url: "https://gateway.ai.cloudflare.com/v1/{CLOUDFLARE_ACCOUNT_ID}/{CLOUDFLARE_GATEWAY_ID}/compat",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "cloudflare-workers-ai",
        label: "Cloudflare Workers AI（需补账号占位符）",
        base_url: "https://api.cloudflare.com/client/v4/accounts/{CLOUDFLARE_ACCOUNT_ID}/ai/v1",
        api: "openai-completions",
    },
    ProviderPreset {
        key: "amazon-bedrock",
        label: "Amazon Bedrock（区域端点）",
        base_url: "https://bedrock-runtime.us-east-1.amazonaws.com",
        api: "bedrock-converse-stream",
    },
    ProviderPreset {
        key: "google-vertex",
        label: "Google Vertex AI（需补区域占位符）",
        base_url: "https://{location}-aiplatform.googleapis.com",
        api: "google-vertex",
    },
];

/// 按 key 精确查找预设（大小写敏感，与配置里的 key 一致）。
pub fn find(key: &str) -> Option<&'static ProviderPreset> {
    PRESETS.iter().find(|p| p.key == key)
}

/// 套用预设：只写 key / baseUrl / 协议，其余字段一律保持原样。
///
/// - opencode 页：协议有对应 npm 时写 npm 并清掉可能残留的 pi api（避免同一 provider 出现
///   两套协议写法）；没有对应 npm 的协议（如 `openai-codex-responses`）则写回 api 字段，
///   既不丢协议信息，也不动用户已填的 npm；
/// - pi / oh-my-pi / DSH 页：写 api。
///
/// 密钥、超时、模型列表、用户手填的任何内容都不会被触碰。
pub fn apply(row: &mut ProviderRow, preset: &ProviderPreset, dialect: PresetDialect) {
    row.key = preset.key.to_string();
    row.base_url = preset.base_url.to_string();
    match dialect {
        PresetDialect::Opencode => {
            let npm = crate::convert::api_to_npm(preset.api);
            if npm.is_empty() {
                row.pi_api = preset.api.to_string();
            } else {
                row.npm = npm;
                row.pi_api = String::new();
            }
        }
        PresetDialect::PiLike => {
            row.pi_api = preset.api.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::{OMP_APIS, PI_APIS};
    use crate::model::ModelRow;

    fn preset(key: &str) -> &'static ProviderPreset {
        find(key).unwrap_or_else(|| panic!("缺少预设：{key}"))
    }

    #[test]
    fn presets_are_well_formed() {
        for p in PRESETS {
            assert!(!p.key.is_empty(), "key 不能为空");
            assert!(!p.label.is_empty(), "{} 缺少显示名", p.key);
            assert!(
                p.base_url.starts_with("https://"),
                "{} 的 baseUrl 必须是 https:// 开头，实际：{}",
                p.key,
                p.base_url
            );
            assert!(!p.api.is_empty(), "{} 缺少 api", p.key);
            // key 里不应有空格（配置里的对象键）
            assert!(!p.key.contains(' '), "{} 的 key 含空格", p.key);
        }
    }

    #[test]
    fn preset_keys_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for p in PRESETS {
            assert!(seen.insert(p.key), "预设 key 重复：{}", p.key);
        }
    }

    /// 预设的 api 必须是界面下拉里真实存在的协议，否则套用后会显示成野生值。
    #[test]
    fn preset_apis_exist_in_dialect_lists() {
        for p in PRESETS {
            assert!(
                PI_APIS.contains(&p.api) || OMP_APIS.contains(&p.api),
                "{} 的 api「{}」不在 pi/omp 协议列表里",
                p.key,
                p.api
            );
        }
    }

    /// 套用预设只动身份三件套；手填的密钥、超时、模型列表必须原样保留。
    #[test]
    fn apply_only_touches_identity_fields() {
        let mut row = ProviderRow::new();
        row.key = "my-relay".to_string();
        row.api_key = "手填的密钥".to_string();
        row.timeout = "60000".to_string();
        row.models.push(ModelRow::new());
        row.models.push(ModelRow::new());
        let models_before = row.models.len();

        apply(&mut row, preset("deepseek"), PresetDialect::PiLike);

        assert_eq!(row.key, "deepseek");
        assert_eq!(row.base_url, "https://api.deepseek.com");
        assert_eq!(row.pi_api, "openai-completions");
        // 手工内容不受影响
        assert_eq!(row.api_key, "手填的密钥");
        assert_eq!(row.timeout, "60000");
        assert_eq!(row.models.len(), models_before);
    }

    #[test]
    fn apply_opencode_writes_npm_and_clears_pi_api() {
        let mut row = ProviderRow::new();
        row.pi_api = "openai-responses".to_string();
        row.api_key = "sk-keep".to_string();

        apply(&mut row, preset("anthropic"), PresetDialect::Opencode);

        assert_eq!(row.npm, "@ai-sdk/anthropic");
        assert!(row.pi_api.is_empty(), "opencode 页不应残留 pi api");
        assert_eq!(row.api_key, "sk-keep");
    }

    /// opencode 页遇到没有对应 npm 的协议（如 Codex）：保留 api，不动用户已填的 npm。
    #[test]
    fn apply_opencode_keeps_api_without_npm_equivalent() {
        let mut row = ProviderRow::new();
        row.npm = "@ai-sdk/openai-compatible".to_string();

        apply(&mut row, preset("openai-codex"), PresetDialect::Opencode);

        assert_eq!(row.pi_api, "openai-codex-responses");
        assert_eq!(row.npm, "@ai-sdk/openai-compatible");
    }

    /// 第三方 / 中转站 / 自建端点：不套预设时没有任何校验会拦着手填。
    #[test]
    fn custom_provider_is_not_restricted() {
        let mut row = ProviderRow::new();
        row.key = "my-relay".to_string();
        row.base_url = "https://relay.example.com/v1".to_string();
        row.pi_api = "openai-completions".to_string();

        assert_eq!(row.key, "my-relay");
        assert_eq!(row.base_url, "https://relay.example.com/v1");
        assert!(find("my-relay").is_none(), "自定义 key 不应命中预设");
    }

    #[test]
    fn find_is_exact_and_case_sensitive() {
        assert!(find("deepseek").is_some());
        assert!(find("DeepSeek").is_none());
        assert!(find("").is_none());
    }
}
