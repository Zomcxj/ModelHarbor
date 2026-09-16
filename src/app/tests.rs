//! `app` 模块的单元测试：序列化排版、预览查找/重建、语法高亮、
//! 模型获取与延迟探测（协议分类、SSE 首字、节流门控）。

#[cfg(test)]
mod path_reload_tests {
    use crate::app::App;
    use crate::format::{ConfigFormat, ConfigPaths};

    fn missing_json_path() -> String {
        let path = std::env::temp_dir().join(format!(
            "model-harbor-missing-pi-{}-models.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn missing_json_on_pi_page_does_not_overwrite_opencode_path() {
        let mut app = App::default();
        let opencode_path = r"D:\existing-opencode\opencode.json";
        app.config_paths
            .set_local_path(ConfigFormat::Opencode, opencode_path);
        app.current_page = ConfigFormat::Pi;
        app.config_path = missing_json_path();
        app.loaded_path.clear();

        app.reload();

        assert_eq!(
            app.config_paths.local_path(ConfigFormat::Opencode),
            opencode_path,
            "不存在的 .json 不能因探测回落污染 opencode 覆盖"
        );
        assert_eq!(app.current_page, ConfigFormat::Pi, "加载失败时应留在原页面");
        assert_eq!(
            app.config_paths.local_path(ConfigFormat::Pi),
            ConfigPaths::default_local_path(ConfigFormat::Pi),
            "加载失败也不能把不存在路径记到 pi 覆盖"
        );
    }

    #[test]
    fn existing_pi_file_is_remembered_only_by_detected_page() {
        let path = std::env::temp_dir().join(format!(
            "model-harbor-pi-owner-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, r#"{"providers": {}}"#).expect("写 pi 临时配置");
        let path = path.to_string_lossy().into_owned();
        let mut app = App::default();
        let opencode_path = r"D:\existing-opencode\opencode.json";
        app.config_paths
            .set_local_path(ConfigFormat::Opencode, opencode_path);
        app.current_page = ConfigFormat::Opencode;
        app.config_path = path.clone();
        app.loaded_path.clear();

        app.reload();

        assert_eq!(app.source_format, ConfigFormat::Pi);
        assert_eq!(app.current_page, ConfigFormat::Pi);
        assert_eq!(app.config_paths.local_path(ConfigFormat::Pi), path);
        assert_eq!(
            app.config_paths.local_path(ConfigFormat::Opencode),
            opencode_path,
            "真实 pi 文件不应记到原来的 opencode 页面"
        );
        let _ = std::fs::remove_file(app.config_path);
    }
}
#[cfg(test)]
mod preview_collapse_tests {
    use crate::app::App;
    use crate::format::ConfigFormat;

    #[test]
    fn preview_edits_preserve_surviving_collapsed_cards_and_prune_removed_ones() {
        let mut app = App {
            current_page: ConfigFormat::Opencode,
            source_format: ConfigFormat::Opencode,
            ..Default::default()
        };
        app.set_provider_collapsed("a", true);
        app.set_provider_collapsed("removed", true);
        app.preview_draft = r#"{
            "provider": {
                "a": {"description": "edited"},
                "c": {}
            }
        }"#
        .into();

        assert!(app.apply_preview_draft());
        assert!(app.provider_collapsed("a"), "存活卡片应保留折叠状态");
        assert!(!app.provider_collapsed("c"), "新增卡片天然展开");
        assert!(
            !app.provider_collapsed("removed"),
            "草稿删除的卡片应清理失效折叠键"
        );
    }

    #[test]
    fn invalid_preview_does_not_change_cards_or_collapse_state() {
        let mut app = App {
            current_page: ConfigFormat::Opencode,
            source_format: ConfigFormat::Opencode,
            preview_draft: r#"{"provider":{"a":{}}}"#.into(),
            ..Default::default()
        };
        assert!(app.apply_preview_draft());
        app.set_provider_collapsed("a", true);
        let before_root = app.root.clone();
        let before_keys: Vec<String> = app.providers.iter().map(|p| p.key.clone()).collect();
        let before_collapsed = app.collapsed.clone();

        app.preview_draft = "{ invalid".into();
        assert!(!app.apply_preview_draft());

        assert_eq!(app.root, before_root);
        assert_eq!(
            app.providers
                .iter()
                .map(|p| p.key.clone())
                .collect::<Vec<_>>(),
            before_keys
        );
        assert_eq!(app.collapsed, before_collapsed);
    }
}

#[cfg(test)]
mod prefs_snapshot_tests {
    use crate::app::App;

    #[test]
    fn current_prefs_sorts_collapsed_cards_before_comparison() {
        let mut app = App::default();
        app.collapsed.clear();
        app.collapsed.insert("providers/z".into());
        app.collapsed.insert("agents/a".into());

        assert_eq!(
            app.current_prefs().collapsed,
            vec!["agents/a".to_string(), "providers/z".to_string()]
        );
    }
}

#[cfg(test)]
mod compact_tests {
    use crate::app::serialize::{compact_json, pretty_json, serialize_object, CompactRole};

    #[test]
    fn pretty_agent_fields_combine_on_same_line() {
        let root = serde_json::json!({
            "agent": {
                "writing": {
                    "mode": "subagent",
                    "description": "写技术文档",
                    "model": "sensenova/sensenova-6.8-flash-lite",
                    "variant": "high",
                    "temperature": 0.3,
                    "color": "info",
                    "system": "负责文档"
                }
            }
        });
        let output = pretty_json(&root);
        println!("=== PRETTY OUTPUT ===\n{}", output);
        // model, variant, temperature, color should be on the same line
        assert!(
            output.contains(
                "\"model\": \"sensenova/sensenova-6.8-flash-lite\", \"variant\": \"high\","
            ),
            "agent model+variant should combine:\n{}",
            output
        );
        assert!(
            output.contains("\"temperature\": 0.3, \"color\": \"info\", \"system\": \"负责文档\""),
            "agent temperature+color+system should combine:\n{}",
            output
        );
    }

    #[test]
    fn compact_variants_preserve_values() {
        let value = serde_json::json!({
            "variants": {
                "medium": { "reasoningEffort": "medium" },
                "high": { "reasoningEffort": "high" }
            }
        });
        let output = serialize_object(value.as_object().unwrap(), 1, CompactRole::Target);
        assert!(output.contains(
            "\"variants\": { \"medium\": {\"reasoningEffort\":\"medium\"}, \"high\": {\"reasoningEffort\":\"high\"} }"
        ));
    }

    #[test]
    fn pi_models_array_expands_to_multiple_lines() {
        let root = serde_json::json!({
            "providers": {
                "openai": {
                    "api": "openai-completions",
                    "models": [
                        { "id": "gpt-4o", "name": "GPT-4o" },
                        { "id": "gpt-4o-mini", "name": "GPT-4o mini" }
                    ]
                }
            }
        });
        let pretty = pretty_json(&root);
        assert!(pretty.contains("\"models\": [\n"));
        assert!(pretty.contains("\"id\": \"gpt-4o\", \"name\": \"GPT-4o\""));
        assert!(pretty.contains("\"id\": \"gpt-4o-mini\", \"name\": \"GPT-4o mini\""));

        let compact = compact_json(&root);
        assert!(compact.contains("\"models\": [\n"));
        assert!(compact.contains("\"id\": \"gpt-4o\", \"name\": \"GPT-4o\""));
        assert!(compact.contains("\"id\": \"gpt-4o-mini\", \"name\": \"GPT-4o mini\""));
    }

    #[test]
    fn compact_targets_agent_fields_only() {
        let root = serde_json::json!({
            "agent": { "writer": { "mode": "subagent", "description": "text", "model": "p/m" } },
            "other": { "a": 1, "b": 2 }
        });
        let output = compact_json(&root);
        assert!(output.contains("\"writer\": {"));
        assert!(output.contains("\"mode\": \"subagent\", \"description\": \"text\""));
        assert!(output.contains("\"other\": {\n    \"a\": 1,"));
    }

    #[test]
    fn compact_targets_provider_model_fields() {
        let root = serde_json::json!({
            "provider": {
                "p": {
                    "models": {
                        "m": {
                            "name": "Model",
                            "reasoning": true,
                            "variants": { "medium": {}, "high": {} }
                        }
                    }
                }
            }
        });
        let output = compact_json(&root);
        assert!(output.contains("\"models\": {\"m\":{"));
        assert!(output.contains("\"name\":\"Model\",\"reasoning\":true"));
        assert!(output.contains("\"variants\":{\"medium\":{},\"high\":{}}"));
    }

    #[test]
    fn compact_wraps_mcp_and_options_from_second_level() {
        let root = serde_json::json!({
            "mcp": {
                "server": {
                    "command": "node",
                    "args": ["server.js"],
                    "enabled": true
                }
            },
            "provider": {
                "p": {
                    "options": {
                        "baseURL": "https://example.com/v1",
                        "apiKey": "sk-test",
                        "timeout": 30000
                    }
                }
            }
        });
        let output = compact_json(&root);

        assert!(output.contains(
            "\"server\": {\"command\":\"node\",\"args\":[\"server.js\"],\"enabled\":true"
        ));
        assert!(output.contains("\"options\": {\"baseURL\":\"https://example.com/v1\",\"apiKey\":\"sk-test\",\"timeout\":30000"));
    }

    #[test]
    fn default_format_keeps_nested_leaf_objects_on_one_line() {
        let root = serde_json::json!({
            "provider": {
                "p": {
                    "options": { "baseURL": "https://example.com/v1", "timeout": 30000 }
                }
            }
        });

        let output = compact_json(&root);

        assert!(output
            .contains("\"options\": {\"baseURL\":\"https://example.com/v1\",\"timeout\":30000}"));
        assert!(!output.contains("\"options\": {\n"));
    }

    #[test]
    fn save_serializers_preserve_variant_settings() {
        let root = serde_json::json!({
            "provider": {
                "p": {
                    "models": {
                        "m": {
                            "variants": { "high": { "reasoningEffort": "high" } }
                        }
                    }
                }
            }
        });

        assert!(compact_json(&root).contains("reasoningEffort"));
        assert!(compact_json(&root).contains("\"high\":{\"reasoningEffort\":\"high\"}"));
    }
}

#[cfg(test)]
mod model_fetch_tests {
    use crate::app::bars::{sanitize_network_error, short_err};
    use crate::app::fetch::{chat_url, parse_models_response};
    use crate::app::providers_form::{fetch_grid_columns, FETCH_GRID_GAP_X};
    use crate::app::App;

    /// 卡片 / 列表里的单行摘要：状态码后面保留人话原因，丢掉耗时与处理建议。
    #[test]
    fn short_err_keeps_status_code_with_reason() {
        assert_eq!(
            short_err("HTTP 503 服务临时不可用：上游繁忙、维护或过载；稍后重试，或切到备用线路（1234 ms）"),
            "HTTP 503 服务临时不可用"
        );
        // 拉取模型失败那种多行错误：只取第一行的状态码 + 原因
        assert_eq!(
            short_err("HTTP 404 接口或模型不存在：Not Found\nBase URL 路径或模型名写错；检查接口地址与模型 id"),
            "HTTP 404 接口或模型不存在"
        );
        // 未收录的状态码：只给数字，不猜原因
        assert_eq!(short_err("HTTP 599（200 ms）"), "HTTP 599");
        // 非 HTTP 错误：原样短文本（超长才截断）
        assert_eq!(
            short_err("网络错误：Connection refused"),
            "网络错误：Connection refused"
        );
        assert_eq!(short_err("  "), "");
    }

    #[test]
    fn fetch_grid_columns_never_exceed_available_width() {
        // 总宽 = 列数 * 列宽 + 间距 * (列数 - 1)，任何宽度下都不得超过可用宽度。
        let total = |(cols, col_w): (usize, f32)| {
            cols as f32 * col_w + FETCH_GRID_GAP_X * (cols - 1) as f32
        };
        // 宽窗口：取满 5 列并把宽度均分（5 * 217.6 + 4 * 28 = 1200）。
        let wide = fetch_grid_columns(50, 1200.0);
        assert_eq!(wide.0, 5);
        assert!((wide.1 - 217.6).abs() < 0.01);
        assert!(total(wide) <= 1200.5);
        // 中等宽度：列数随可用宽度下降，仍恰好铺满。
        let mid = fetch_grid_columns(50, 400.0);
        assert_eq!(mid.0, 2);
        assert!(total(mid) <= 400.5);
        // 极窄窗口：退化为单列，宽度不超过可用宽度。
        let narrow = fetch_grid_columns(50, 120.0);
        assert_eq!(narrow.0, 1);
        assert!(narrow.1 <= 120.0);
        // 模型很少时不空出多余列。
        assert_eq!(fetch_grid_columns(3, 1200.0).0, 3);
        // 空列表（防御性）不 panic，也不返回 0 列。
        assert_eq!(fetch_grid_columns(0, 300.0).0, 1);
    }

    #[test]
    fn parse_openai_style_models() {
        let text = r#"{"object":"list","data":[{"id":"gpt-4o","object":"model"},{"id":"gpt-4o-mini","object":"model"}]}"#;
        let ids = parse_models_response(text).unwrap();
        assert_eq!(ids, vec!["gpt-4o", "gpt-4o-mini"]);
    }

    #[test]
    fn parse_anthropic_style_models() {
        let text = r#"{"data":[{"type":"model","id":"claude-3-7-sonnet-20250219"},{"type":"model","id":"claude-sonnet-4-20250514"}]}"#;
        let ids = parse_models_response(text).unwrap();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"claude-sonnet-4-20250514".to_string()));
    }

    #[test]
    fn parse_gemini_style_models() {
        let text =
            r#"{"models":[{"name":"models/gemini-2.0-flash"},{"name":"models/gemini-2.5-pro"}]}"#;
        let ids = parse_models_response(text).unwrap();
        assert_eq!(ids, vec!["gemini-2.0-flash", "gemini-2.5-pro"]);
    }

    #[test]
    fn parse_error_message() {
        let text = r#"{"error":{"message":"Invalid API key"}}"#;
        let err = parse_models_response(text).unwrap_err();
        assert!(err.contains("Invalid API key"));
    }

    #[test]
    fn parse_dedupes_ids_and_ignores_missing() {
        let text = r#"{"data":[{"id":"a"},{"id":"a"},{"name":"b"},{"foo":"c"}]}"#;
        let ids = parse_models_response(text).unwrap();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn models_url_openai_variants() {
        assert_eq!(
            App::models_url("https://api.openai.com/v1", "openai-completions"),
            "https://api.openai.com/v1/models"
        );
        assert_eq!(
            App::models_url("https://api.openai.com/v1/", "openai-completions"),
            "https://api.openai.com/v1/models"
        );
        assert_eq!(
            App::models_url("https://gw.example.com", "openai-completions"),
            "https://gw.example.com/models"
        );
    }

    #[test]
    fn models_url_anthropic_uses_v1() {
        assert_eq!(
            App::models_url("https://api.anthropic.com", "anthropic-messages"),
            "https://api.anthropic.com/v1/models"
        );
        assert_eq!(
            App::models_url("https://api.anthropic.com/v1", "anthropic-messages"),
            "https://api.anthropic.com/v1/models"
        );
    }

    #[test]
    fn chat_url_openai_and_anthropic() {
        assert_eq!(
            chat_url(
                "https://api.openai.com/v1",
                "openai-completions",
                "gpt-4o",
                true
            ),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            chat_url(
                "https://api.anthropic.com",
                "anthropic-messages",
                "claude-sonnet-4",
                true
            ),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn chat_url_streaming_only_changes_google() {
        // 除 Google 系外，流式与非流式是同一个端点（靠请求体里的 stream 字段区分）
        assert_eq!(
            chat_url(
                "https://gw.example.com/v1",
                "openai-completions",
                "gpt-4o",
                false
            ),
            chat_url(
                "https://gw.example.com/v1",
                "openai-completions",
                "gpt-4o",
                true
            )
        );
        // Google 系要换动词（streamGenerateContent）并加 SSE 查询参数
        assert_eq!(
            chat_url(
                "https://generativelanguage.googleapis.com/v1beta",
                "google-generative-ai",
                "gemini-2.5-pro",
                true
            ),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:streamGenerateContent?alt=sse"
        );
        assert_eq!(
            chat_url(
                "https://us-central1-aiplatform.googleapis.com/v1",
                "google-vertex",
                "gemini-2.5-pro",
                true
            ),
            "https://us-central1-aiplatform.googleapis.com/v1/publishers/google/models/gemini-2.5-pro:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn query_key_appends_after_existing_query() {
        use crate::app::fetch::{with_query_key, AuthKind};
        assert_eq!(
            with_query_key("https://gw.example.com/models", AuthKind::QueryKey, "sk-1"),
            "https://gw.example.com/models?key=sk-1"
        );
        // Google 流式端点已经带了 ?alt=sse，密钥要用 & 接着拼
        assert_eq!(
            with_query_key(
                "https://gw.example.com/models/m:streamGenerateContent?alt=sse",
                AuthKind::QueryKey,
                "sk-1"
            ),
            "https://gw.example.com/models/m:streamGenerateContent?alt=sse&key=sk-1"
        );
    }

    #[test]
    fn chat_url_follows_selected_protocol() {
        let base = "https://gw.example.com/v1";
        // Responses 系走 /responses（含 Azure）
        assert_eq!(
            chat_url(base, "openai-responses", "gpt-4o", false),
            "https://gw.example.com/v1/responses"
        );
        assert_eq!(
            chat_url(base, "azure-openai-responses", "gpt-4o", false),
            "https://gw.example.com/v1/responses"
        );
        // Mistral 会话协议仍是 chat/completions
        assert_eq!(
            chat_url(base, "mistral-conversations", "mistral-large", false),
            "https://gw.example.com/v1/chat/completions"
        );
        // pi 自己的协议是 /messages（不带 v1 前缀时也直接用 base）
        assert_eq!(
            chat_url(base, "pi-messages", "some-model", false),
            "https://gw.example.com/v1/messages"
        );
        // Google 系需要模型名参与路径
        assert_eq!(
            chat_url(
                "https://generativelanguage.googleapis.com/v1beta",
                "google-generative-ai",
                "gemini-2.5-pro",
                false
            ),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:generateContent"
        );
        assert_eq!(
            chat_url(
                "https://us-central1-aiplatform.googleapis.com/v1",
                "google-vertex",
                "gemini-2.5-pro",
                false
            ),
            "https://us-central1-aiplatform.googleapis.com/v1/publishers/google/models/gemini-2.5-pro:generateContent"
        );
    }

    #[test]
    fn api_wire_classifies_and_flags_unsupported() {
        use crate::app::fetch::{api_wire, auth_kind, unsupported_reason, ApiWire, AuthKind};
        // 未知值与空值都归到兼容层，不会漏掉协议分支
        assert_eq!(api_wire("openai-completions"), ApiWire::ChatCompletions);
        assert_eq!(api_wire("unknown-api"), ApiWire::ChatCompletions);
        assert_eq!(api_wire(""), ApiWire::ChatCompletions);
        assert_eq!(api_wire("openai-codex-responses"), ApiWire::Responses);
        assert_eq!(api_wire("anthropic-messages"), ApiWire::AnthropicMessages);
        assert_eq!(api_wire("pi-messages"), ApiWire::PiMessages);
        // 鉴权方式随协议变化
        assert_eq!(
            auth_kind(api_wire("anthropic-messages")),
            AuthKind::AnthropicKey
        );
        assert_eq!(
            auth_kind(api_wire("azure-openai-responses")),
            AuthKind::AzureKey
        );
        assert_eq!(
            auth_kind(api_wire("google-generative-ai")),
            AuthKind::QueryKey
        );
        assert_eq!(auth_kind(api_wire("openai-responses")), AuthKind::Bearer);
        // 需要专有鉴权的协议提前报错，不发无意义的请求
        assert!(unsupported_reason("google-gemini-cli").is_some());
        assert!(unsupported_reason("bedrock-converse-stream").is_some());
        assert!(unsupported_reason("openai-completions").is_none());
    }

    #[test]
    fn minimal_body_matches_protocol() {
        use crate::app::fetch::{api_wire, minimal_body};
        let question = "世界上最长的河流是哪条？只回答河名";
        let chat = minimal_body(api_wire("openai-completions"), "m", question);
        assert_eq!(chat["messages"][0]["content"], question);
        assert_eq!(chat["max_tokens"], 16);
        let resp = minimal_body(api_wire("openai-responses"), "m", question);
        assert_eq!(resp["input"], question);
        assert_eq!(resp["max_output_tokens"], 16);
        let anthropic = minimal_body(api_wire("anthropic-messages"), "m", question);
        assert_eq!(anthropic["messages"][0]["content"], question);
        let google = minimal_body(api_wire("google-generative-ai"), "m", question);
        assert_eq!(google["contents"][0]["parts"][0]["text"], question);
        assert_eq!(google["generationConfig"]["maxOutputTokens"], 16);
        // 全部走流式（Google 除外：它的流式靠换端点，请求体里没有 stream 字段）
        assert_eq!(chat["stream"], true);
        assert_eq!(resp["stream"], true);
        assert_eq!(anthropic["stream"], true);
        assert!(google.get("stream").is_none());
        // 字段名不得互相串用（Responses 没有 messages，Google 没有 model）
        assert!(resp.get("messages").is_none());
        assert!(google.get("model").is_none());
    }

    #[test]
    fn probe_user_agent_matches_whitelisted_clients() {
        use crate::app::fetch::{api_wire, probe_user_agent};
        // Anthropic 系用 Claude Code 的身份，其余用 opencode 的身份
        // （中转站只放行这两种；ureq 默认 UA / 无 UA / pi 的 UA 都会 401）
        assert!(probe_user_agent(api_wire("anthropic-messages")).starts_with("claude-cli/"));
        assert!(probe_user_agent(api_wire("pi-messages")).starts_with("claude-cli/"));
        assert!(probe_user_agent(api_wire("openai-completions")).starts_with("opencode/"));
        assert!(probe_user_agent(api_wire("openai-responses")).starts_with("opencode/"));
        assert!(probe_user_agent(api_wire("google-generative-ai")).starts_with("opencode/"));
    }

    #[test]
    fn chunk_content_detection_per_protocol() {
        use crate::app::fetch::{api_wire, chunk_has_content};
        let parse = |text: &str| match serde_json::from_str::<serde_json::Value>(text) {
            Ok(value) => value,
            Err(err) => panic!("测试用例不是合法 JSON：{err}"),
        };
        // Chat Completions：role 块不算出字，content / reasoning_content 才算
        let chat = api_wire("openai-completions");
        assert!(!chunk_has_content(
            chat,
            &parse(r#"{"choices":[{"delta":{"role":"assistant"}}]}"#)
        ));
        assert!(chunk_has_content(
            chat,
            &parse(r#"{"choices":[{"delta":{"content":"尼罗河"}}]}"#)
        ));
        assert!(chunk_has_content(
            chat,
            &parse(r#"{"choices":[{"delta":{"reasoning_content":"嗯"}}]}"#)
        ));
        // 内容块数组形态也要认（部分中转站回 parts）
        assert!(chunk_has_content(
            chat,
            &parse(r#"{"choices":[{"delta":{"content":[{"text":"你"}]}}]}"#)
        ));
        // Responses：只有 *.delta 事件且带 delta 文本才算
        let responses = api_wire("openai-responses");
        assert!(!chunk_has_content(
            responses,
            &parse(r#"{"type":"response.created"}"#)
        ));
        assert!(chunk_has_content(
            responses,
            &parse(r#"{"type":"response.output_text.delta","delta":"你"}"#)
        ));
        // Anthropic：content_block_delta 里的 text / thinking
        let anthropic = api_wire("anthropic-messages");
        assert!(!chunk_has_content(
            anthropic,
            &parse(r#"{"type":"message_start","message":{"id":"x"}}"#)
        ));
        assert!(chunk_has_content(
            anthropic,
            &parse(r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"你"}}"#)
        ));
        // Google：candidates[0].content.parts[*].text
        let google = api_wire("google-generative-ai");
        assert!(!chunk_has_content(
            google,
            &parse(r#"{"candidates":[{"content":{"parts":[{"text":""}]}}]}"#)
        ));
        assert!(chunk_has_content(
            google,
            &parse(r#"{"candidates":[{"content":{"parts":[{"text":"尼罗河"}]}}]}"#)
        ));
    }

    #[test]
    fn stream_ttft_reads_first_content_and_drains() {
        use crate::app::fetch::{api_wire, read_stream_ttft};
        let started = std::time::Instant::now();
        // role 块在前：首字必须落在 content 块上，且读完流后不能报错
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"尼罗河\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let ttft = read_stream_ttft(
            std::io::Cursor::new(sse.as_bytes()),
            api_wire("openai-completions"),
            started,
        );
        assert!(matches!(ttft, Some(ms) if ms < 1_000));
        // 只有元事件（没有任何内容）：退回第一个数据包的时刻，不返回 None
        let meta = "data: {\"type\":\"response.created\"}\n\n";
        let fallback = read_stream_ttft(
            std::io::Cursor::new(meta.as_bytes()),
            api_wire("openai-responses"),
            started,
        );
        assert!(fallback.is_some());
        // 空流（连数据包都没有）才算失败
        let empty: &[u8] = b"";
        assert!(read_stream_ttft(
            std::io::Cursor::new(empty),
            api_wire("openai-completions"),
            started
        )
        .is_none());
    }

    #[test]
    fn stream_end_markers_are_recognized() {
        use crate::app::fetch::is_stream_end;
        assert!(is_stream_end("data: [DONE]"));
        assert!(is_stream_end("event: message_stop"));
        assert!(is_stream_end("data: {\"type\":\"response.completed\"}"));
        // 普通内容块不能被误判成结束
        assert!(!is_stream_end(
            "data: {\"choices\":[{\"delta\":{\"content\":\"尼罗河\"}}]}"
        ));
        // content_block_stop 里含 stop，但不是结束事件
        assert!(!is_stream_end(
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\"}"
        ));
    }

    #[test]
    fn probe_gate_throttles_per_provider() {
        use crate::app::fetch::{ProbeGate, ProbeGateState, PROBE_PROVIDER_GAP_S};
        let mut gate = ProbeGate::default();
        assert_eq!(gate.state("a", 100.0, None), ProbeGateState::Ready);
        gate.start("a", 100.0);
        // 同一 provider 在飞期间 Busy（探测最长 10s，可能超过 5s 间隔，
        // 否则会同时向同一中转站发两个请求）
        assert_eq!(gate.state("a", 101.0, None), ProbeGateState::Busy);
        // 不同 provider 互不牵连：另一个厂商立即可测（可并行）
        assert_eq!(gate.state("b", 100.0, None), ProbeGateState::Ready);
        gate.start("b", 100.0);
        gate.finish("a");
        // 同一 provider 的任意两次探测（同模型 / 不同模型都算）统一间隔 5 秒
        match gate.state("a", 102.0, None) {
            ProbeGateState::Cooling(left) => {
                assert!(left > 0.0 && left <= PROBE_PROVIDER_GAP_S)
            }
            other => panic!("应为 Cooling，实际 {other:?}"),
        }
        assert_eq!(gate.state("a", 105.0, None), ProbeGateState::Ready);
        // b 仍在飞，不影响 a
        assert_eq!(gate.state("b", 106.0, None), ProbeGateState::Busy);
        gate.finish("b");
        // 网络守卫优先级最高：即使冷却结束也不允许探测
        assert!(matches!(
            gate.state("a", 200.0, Some("检测到系统代理")),
            ProbeGateState::NetBlocked(_)
        ));
        // 释放：重载 / 关闭表单时不能把某个 provider 永久卡在 Busy
        gate.start("c", 300.0);
        gate.release(Some("a"));
        assert_eq!(gate.state("c", 400.0, None), ProbeGateState::Busy);
        gate.release(None);
        assert_eq!(gate.state("c", 400.0, None), ProbeGateState::Ready);
    }

    #[test]
    fn probe_questions_rotate_per_provider() {
        use crate::app::fetch::{probe_question, ProbeGate, PROBE_QUESTIONS};
        // 同一 provider 连续取题不重复（避免每次都问同一句）
        let mut gate = ProbeGate::default();
        let first = gate.next_question("relay-a");
        let second = gate.next_question("relay-a");
        assert_ne!(first, second);
        // 不同 provider 的起始偏移不同（同一游标下题目不同）
        let other = probe_question("relay-b", 0);
        assert_ne!(probe_question("relay-a", 0), other);
        assert!(PROBE_QUESTIONS.contains(&other));
    }

    #[test]
    fn models_url_google_vertex_uses_publishers_path() {
        assert_eq!(
            App::models_url(
                "https://us-central1-aiplatform.googleapis.com/v1",
                "google-vertex"
            ),
            "https://us-central1-aiplatform.googleapis.com/v1/publishers/google/models"
        );
        assert_eq!(
            App::models_url(
                "https://generativelanguage.googleapis.com/v1beta",
                "google-generative-ai"
            ),
            "https://generativelanguage.googleapis.com/v1beta/models"
        );
    }

    #[test]
    fn sanitize_network_error_strips_url_query() {
        let msg = r#"connection failed: Connection refused (os error 10061) for URL "https://example.com/v1?key=sk-super-secret#frag""#;
        let out = sanitize_network_error(msg);
        assert!(!out.contains("sk-super-secret"), "泄露了 query: {}", out);
        assert!(!out.contains('#'), "泄露了 fragment: {}", out);
        assert!(out.contains("https://example.com/v1"));
    }

    #[test]
    fn sanitize_network_error_passthrough_without_url() {
        let msg = "dns error: failed to lookup address";
        assert_eq!(sanitize_network_error(msg), msg);
    }

    #[test]
    fn sanitize_network_error_url_no_query_untouched() {
        let msg = r#"connection failed for URL "https://api.example.com/v1/models""#;
        assert_eq!(sanitize_network_error(msg), msg);
    }
}

#[cfg(test)]
mod syntax_highlight_tests {
    use crate::app::syntax::{
        json_tokens, syntax_tokens, yaml_tokens, PreviewSyntax, SYN_COMMENT, SYN_KEY, SYN_NUMBER,
        SYN_STRING,
    };
    use eframe::egui;

    /// 段落必须落在字符边界上，否则 LayoutJob 切片会 panic。
    fn assert_boundaries(text: &str, tokens: &[(usize, usize, egui::Color32)]) {
        for &(s, e, _) in tokens {
            assert!(
                text.is_char_boundary(s),
                "start {} 不在字符边界: {:?}",
                s,
                text
            );
            assert!(
                text.is_char_boundary(e),
                "end {} 不在字符边界: {:?}",
                e,
                text
            );
            assert!(s <= e);
        }
    }

    #[test]
    fn json_distinguishes_key_and_value_strings() {
        let text = r#"{"apiKey": "sk-xxx", "count": 12, "on": true}"#;
        let tokens = json_tokens(text);
        assert_boundaries(text, &tokens);
        let color_of = |needle: &str| {
            let start = text.find(needle).unwrap();
            tokens
                .iter()
                .find(|(s, e, _)| *s <= start && start < *e)
                .map(|(_, _, c)| *c)
                .unwrap()
        };
        assert_eq!(color_of("apiKey"), SYN_KEY);
        assert_eq!(color_of("sk-xxx"), SYN_STRING);
        assert_eq!(color_of("12"), SYN_NUMBER);
    }

    #[test]
    fn json_handles_comments_and_non_ascii() {
        let text =
            "{\n  // 中文注释 \"引号\"\n  \"名前\": \"值\",\n  /* 块注释 */\n  \"n\": 1.5e3\n}";
        let tokens = json_tokens(text);
        assert_boundaries(text, &tokens);
        let comment_start = text.find("//").unwrap();
        let comment = tokens
            .iter()
            .find(|(s, _, c)| *s == comment_start && *c == SYN_COMMENT);
        assert!(comment.is_some(), "未识别行注释");
        assert!(tokens
            .iter()
            .any(|(s, _, c)| *s == text.find("\"名前\"").unwrap() && *c == SYN_KEY));
    }

    #[test]
    fn yaml_colors_keys_comments_and_literals() {
        let text = "# 顶部注释\nbaseURL: \"https://example.com/v1\"\ntimeout: 180000\nenabled: true\n# 中文注释\n";
        let tokens = yaml_tokens(text);
        assert_boundaries(text, &tokens);
        let color_of = |needle: &str| {
            let start = text.find(needle).unwrap();
            tokens
                .iter()
                .find(|(s, e, _)| *s <= start && start < *e)
                .map(|(_, _, c)| *c)
                .unwrap()
        };
        assert_eq!(color_of("baseURL"), SYN_KEY);
        assert_eq!(color_of("https://example.com/v1"), SYN_STRING);
        assert_eq!(color_of("180000"), SYN_NUMBER);
    }

    #[test]
    fn syntax_dispatch_matches_page_kind() {
        assert_eq!(syntax_tokens("{}", PreviewSyntax::Json).len(), 2);
        assert!(syntax_tokens("a: 1\n", PreviewSyntax::Yaml).len() >= 3);
    }
}

#[cfg(test)]
mod latency_tests {
    use crate::app::fetch::{
        latency_color, matrix_glyphs, LATENCY_GOOD_MS, LATENCY_SLOW_MS, MATRIX_CHARS, MATRIX_LEN,
    };
    use crate::theme::SEMANTICS;

    #[test]
    fn latency_color_thresholds() {
        let colors = SEMANTICS;
        assert_eq!(latency_color(0, colors), colors.ok);
        assert_eq!(latency_color(LATENCY_GOOD_MS - 1, colors), colors.ok);
        assert_eq!(latency_color(LATENCY_GOOD_MS, colors), colors.warn);
        assert_eq!(latency_color(LATENCY_SLOW_MS - 1, colors), colors.warn);
        assert_eq!(latency_color(LATENCY_SLOW_MS, colors), colors.err);
    }

    #[test]
    fn matrix_glyphs_shape_and_variation() {
        let frame = matrix_glyphs(7, "provider/model", MATRIX_LEN);
        assert_eq!(frame.chars().count(), MATRIX_LEN);
        assert!(frame.chars().all(|c| MATRIX_CHARS.contains(c)));
        // 同一帧 + 同一 salt 稳定（不依赖保存的随机状态）
        assert_eq!(frame, matrix_glyphs(7, "provider/model", MATRIX_LEN));
        // 换行（salt 不同）或换帧都会刷新字符
        assert_ne!(frame, matrix_glyphs(7, "provider/other", MATRIX_LEN));
        assert!((8..16).any(|f| frame != matrix_glyphs(f, "provider/model", MATRIX_LEN)));
    }
}

#[cfg(test)]
mod preview_sync_tests {
    use crate::app::preview::preview_should_rebuild;

    #[test]
    fn rebuild_gate_ignores_focus_but_keeps_user_text() {
        // 没在预览里手改：始终按组件状态重建（与焦点无关）
        assert!(preview_should_rebuild(false, None, 100.0));
        // 刚在预览里输入（2 秒内）：保留用户文本，避免打断手改
        assert!(!preview_should_rebuild(false, Some(99.5), 100.0));
        assert!(!preview_should_rebuild(false, Some(100.0), 100.0));
        // 停止输入超过 2 秒：回到组件状态（预览不会一直停在旧内容上）
        assert!(preview_should_rebuild(false, Some(97.9), 100.0));
        // 上次解析失败：保留用户文本，等用户修正或点「重新生成」
        assert!(!preview_should_rebuild(true, None, 100.0));
        assert!(!preview_should_rebuild(true, Some(1.0), 100.0));
    }
}
