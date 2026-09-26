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
        // 本机 settings.json 可能给 pi 页留过覆盖路径（例如误填的 opencode 文件）；
        // 这条测试断言的是「不存在的路径不该记进 pi 覆盖」，与既有覆盖无关，
        // 先清掉才能独立成立。
        app.config_paths.set_local_path(
            ConfigFormat::Pi,
            &ConfigPaths::default_local_path(ConfigFormat::Pi),
        );
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
        let path =
            std::env::temp_dir().join(format!("model-harbor-pi-owner-{}.json", std::process::id()));
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
            "真实 pi 文件应记到 pi 页面"
        );
        let _ = std::fs::remove_file(app.config_path);
    }

    /// 启动方言判定必须以文件内容为准：把 opencode.json 填到 pi 页时，
    /// 按页面推断会解析出 0 条 provider（「写了路径却不自动加载」）。
    #[test]
    fn startup_format_follows_file_content_not_the_page() {
        use crate::app::startup_format;
        // opencode 方言（有 agent / provider 顶层键）
        let oc = std::env::temp_dir().join(format!(
            "model-harbor-startup-oc-{}.json",
            std::process::id()
        ));
        std::fs::write(&oc, r#"{"provider": {"a": {"models": []}}}"#)
            .expect("写 opencode 临时配置");
        let oc = oc.to_string_lossy().into_owned();

        // pi 方言（providers 复数）
        let pi = std::env::temp_dir().join(format!(
            "model-harbor-startup-pi-{}.json",
            std::process::id()
        ));
        std::fs::write(&pi, r#"{"providers": {"a": {"models": []}}}"#).expect("写 pi 临时配置");
        let pi = pi.to_string_lossy().into_owned();

        // 关键场景：pi 页填了 opencode 文件 → 必须纠正成 opencode
        assert_eq!(
            startup_format(ConfigFormat::Pi, &oc),
            ConfigFormat::Opencode,
            "opencode 文件被填到 pi 页时应按内容纠正为 opencode"
        );
        // 反向同理
        assert_eq!(
            startup_format(ConfigFormat::Opencode, &pi),
            ConfigFormat::Pi
        );
        // 路径为空 / 不存在：保持页面推断（不猜）
        assert_eq!(startup_format(ConfigFormat::Pi, "   "), ConfigFormat::Pi);
        let missing = std::env::temp_dir()
            .join("model-harbor-definitely-missing.json")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            startup_format(ConfigFormat::Pi, &missing),
            ConfigFormat::Pi,
            "文件不存在时没有格式证据，不能据此改判"
        );

        let _ = std::fs::remove_file(&oc);
        let _ = std::fs::remove_file(&pi);
    }
}
#[cfg(test)]
mod preview_find_boundary_tests {
    use crate::app::preview::{find_matches, floor_char_boundary};

    /// 预览草稿是中文为主：过期查找偏移（帧首算的、同帧草稿又被编辑）可能落在
    /// 多字节字符中间。跳转前必须向下钳回字符边界，否则切片 panic。
    #[test]
    fn floor_char_boundary_never_lands_inside_a_multibyte_char() {
        // "ab模型cd"：a=0 b=1 | 模=2..5 | 型=5..8 | c=8 d=9，len=10。
        // 字符边界：0,1,2,5,8,9,10；其余都落在「模」或「型」内部。
        let text = "ab模型cd";
        assert_eq!(floor_char_boundary(text, 0), 0);
        assert_eq!(floor_char_boundary(text, 2), 2, "字符起点不动");
        for byte in [3, 4, 6, 7] {
            let bounded = floor_char_boundary(text, byte);
            assert!(
                text.is_char_boundary(bounded),
                "byte {byte} 钳到 {bounded}，必须落在字符边界"
            );
            assert!(bounded < byte, "只向下钳，不会跳到后面");
        }
        assert_eq!(floor_char_boundary(text, 8), 8, "ASCII 段内不动");
        assert_eq!(floor_char_boundary(text, 10), 10, "恰好等于长度是合法边界");
        assert_eq!(floor_char_boundary(text, 999), text.len(), "超界钳到长度内");
    }

    /// 同帧偏移过期的高发场景：查找命中中文、编辑点在命中之前。
    /// 编辑后旧偏移指向汉字中间——这正是跳转切片与 galley 高亮会踩的输入。
    #[test]
    fn find_matches_offsets_are_always_char_boundaries_of_the_text_they_matched() {
        let text = "前缀模型后缀 模型 再一个模型";
        for (start, end) in find_matches(text, "模型") {
            assert!(text.is_char_boundary(start) && text.is_char_boundary(end));
            assert_eq!(&text[start..end], "模型");
        }
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
    fn collapse_state_is_scoped_to_configuration_path() {
        let mut app = App {
            config_path: r"D:\configs\one.json".into(),
            loaded_path: r"D:\configs\one.json".into(),
            ..Default::default()
        };
        app.set_provider_collapsed("shared", true);
        assert!(app.provider_collapsed("shared"));

        app.config_path = r"D:\configs\two.json".into();
        app.loaded_path = app.config_path.clone();
        assert!(
            !app.provider_collapsed("shared"),
            "另一份配置不能继承第一份配置的折叠状态"
        );
        app.set_provider_collapsed("shared", true);

        app.config_path = r"D:\configs\one.json".into();
        app.loaded_path = app.config_path.clone();
        assert!(app.provider_collapsed("shared"));
    }

    #[test]
    fn legacy_collapse_key_migrates_to_loaded_configuration() {
        let mut app = App {
            config_path: r"D:\configs\legacy.json".into(),
            loaded_path: r"D:\configs\legacy.json".into(),
            providers: vec![crate::model::ProviderRow {
                key: "legacy-provider".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let legacy = crate::prefs::legacy_collapsed_id("providers", "legacy-provider");
        app.collapsed.insert(legacy.clone());

        app.migrate_legacy_collapsed();

        assert!(!app.collapsed.contains(&legacy));
        assert!(app.provider_collapsed("legacy-provider"));
    }

    #[test]
    fn pruning_one_configuration_keeps_other_configuration_state() {
        let mut app = App {
            config_path: r"D:\configs\one.json".into(),
            loaded_path: r"D:\configs\one.json".into(),
            providers: vec![crate::model::ProviderRow {
                key: "alive".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        app.set_provider_collapsed("alive", true);
        app.set_provider_collapsed("removed", true);
        let other = crate::prefs::collapsed_id(
            &crate::prefs::config_identity(r"D:\configs\two.json"),
            "providers",
            "other",
        );
        app.collapsed.insert(other.clone());

        app.prune_collapsed();

        assert!(app.provider_collapsed("alive"));
        assert!(!app.provider_collapsed("removed"));
        assert!(app.collapsed.contains(&other));
    }
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
mod layer_order_tests {
    use crate::app::preview::PREVIEW_RESIZER_ORDER;
    use crate::app::App;
    use eframe::egui;

    /// 预览分隔条必须低于令牌悬浮窗：否则分割线会横穿窗口。
    #[test]
    fn preview_resizer_sits_below_the_tokens_window() {
        let resizer = PREVIEW_RESIZER_ORDER;
        let window = App::TOKENS_WINDOW_ORDER;
        assert!(
            resizer < window,
            "分隔条层级 {resizer:?} 不低于令牌窗层级 {window:?}"
        );
        // 悬停提示仍要显示在窗上面。
        assert!(window < egui::Order::Tooltip);
        // 分隔条要能接住拖拽：不能掉到背景层（预览面板本身就在 background）。
        assert!(resizer > egui::Order::Background);
    }
}

#[cfg(test)]
mod syntax_highlight_tests {
    use crate::app::syntax::{
        json_tokens_with, syntax_tokens_with, yaml_tokens_with, PreviewSyntax, SyntaxPalette,
        SYN_DARK, SYN_LIGHT,
    };
    use eframe::egui;

    /// 既有断言都按深色那套写，这里固定住配色。
    fn json_tokens(text: &str) -> Vec<(usize, usize, egui::Color32)> {
        json_tokens_with(text, SYN_DARK)
    }

    fn yaml_tokens(text: &str) -> Vec<(usize, usize, egui::Color32)> {
        yaml_tokens_with(text, SYN_DARK)
    }

    fn syntax_tokens(text: &str, syntax: PreviewSyntax) -> Vec<(usize, usize, egui::Color32)> {
        syntax_tokens_with(text, syntax, SYN_DARK)
    }

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
        assert_eq!(color_of("apiKey"), SYN_DARK.key);
        assert_eq!(color_of("sk-xxx"), SYN_DARK.string);
        assert_eq!(color_of("12"), SYN_DARK.number);
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
            .find(|(s, _, c)| *s == comment_start && *c == SYN_DARK.comment);
        assert!(comment.is_some(), "未识别行注释");
        assert!(tokens
            .iter()
            .any(|(s, _, c)| *s == text.find("\"名前\"").unwrap() && *c == SYN_DARK.key));
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
        assert_eq!(color_of("baseURL"), SYN_DARK.key);
        assert_eq!(color_of("https://example.com/v1"), SYN_DARK.string);
        assert_eq!(color_of("180000"), SYN_DARK.number);
    }

    #[test]
    fn syntax_dispatch_matches_page_kind() {
        assert_eq!(syntax_tokens("{}", PreviewSyntax::Json).len(), 2);
        assert!(syntax_tokens("a: 1\n", PreviewSyntax::Yaml).len() >= 3);
    }

    /// 两套配色都必须在自己那类底色上读得清：语法色是正文，按 WCAG 正文下限
    /// 4.5:1 卡；注释最淡，按提示下限 3.0:1 卡。
    #[test]
    fn every_syntax_palette_reads_on_its_background() {
        use crate::theme::{contrast_for_tests, CONTRAST_HINT_MIN, CONTRAST_TEXT_MIN};
        let dark_bg = egui::Color32::from_rgb(0x1E, 0x1E, 0x1E);
        let light_bg = egui::Color32::from_rgb(0xF5, 0xF5, 0xF5);
        let rose_bg = egui::Color32::from_rgb(0xFB, 0xEE, 0xF0);
        let cases: [(SyntaxPalette, egui::Color32, &str); 3] = [
            (SYN_DARK, dark_bg, "深色"),
            (SYN_LIGHT, light_bg, "浅色"),
            (SYN_LIGHT, rose_bg, "玫瑰"),
        ];
        for (pal, bg, name) in cases {
            for (label, color, floor) in [
                ("键", pal.key, CONTRAST_TEXT_MIN),
                ("字符串", pal.string, CONTRAST_TEXT_MIN),
                ("数字", pal.number, CONTRAST_TEXT_MIN),
                ("字面量", pal.literal, CONTRAST_TEXT_MIN),
                ("注释", pal.comment, CONTRAST_HINT_MIN),
                ("标点", pal.punct, CONTRAST_TEXT_MIN),
            ] {
                let ratio = contrast_for_tests(color, bg);
                assert!(
                    ratio >= floor,
                    "{name}主题的{label}色太淡：{ratio:.2}:1（下限 {floor}）"
                );
            }
        }
    }

    /// 明暗两套必须真的不同，否则浅色主题还是深色配色。
    #[test]
    fn light_and_dark_palettes_differ() {
        assert_ne!(SYN_DARK.key, SYN_LIGHT.key);
        assert_ne!(SYN_DARK.string, SYN_LIGHT.string);
        assert_ne!(SYN_DARK.comment, SYN_LIGHT.comment);
        assert_ne!(SYN_DARK.error, SYN_LIGHT.error);
    }

    /// 查找命中的底色要压得住语法色：命中区换成配好的文字色，
    /// 且底色与文字色两边都够看。
    #[test]
    fn find_highlight_is_readable_on_both_palettes() {
        use crate::app::syntax::apply_find_background;
        use crate::theme::{contrast_for_tests, distance_for_tests, CONTRAST_TEXT_MIN};
        // 底色与面板的可见度用 RGB 距离判：琥珀黄和浅灰面板的**亮度**接近，
        // 对比度只有 1.5:1，但肉眼分得很清，用对比度卡会误报。
        let min_distance = 100.0;
        let cases = [
            (SYN_DARK, "深色", egui::Color32::from_rgb(0x1E, 0x1E, 0x1E)),
            (SYN_LIGHT, "浅色", egui::Color32::from_rgb(0xF5, 0xF5, 0xF5)),
            (SYN_LIGHT, "玫瑰", egui::Color32::from_rgb(0xFB, 0xEE, 0xF0)),
        ];
        for (pal, name, bg) in cases {
            for (label, fill, ink) in [
                ("当前命中", pal.find_current_bg, pal.find_current_fg),
                ("其余命中", pal.find_other_bg, pal.find_other_fg),
            ] {
                let ratio = contrast_for_tests(ink, fill);
                assert!(
                    ratio >= CONTRAST_TEXT_MIN,
                    "{name}主题的{label}文字太淡：{ratio:.2}:1（下限 {CONTRAST_TEXT_MIN}）"
                );
                let pop = distance_for_tests(fill, bg);
                assert!(
                    pop >= min_distance,
                    "{name}主题的{label}底色与背景分不开：距离 {pop:.1}（下限 {min_distance}）"
                );
            }
        }
        // 命中区的前景色真的被换掉了（不是沿用语法色）。
        let mut job = egui::text::LayoutJob::default();
        job.append(
            "hello",
            0.0,
            egui::TextFormat {
                color: SYN_LIGHT.comment,
                ..Default::default()
            },
        );
        apply_find_background(&mut job, &[(0, 5)], 0, SYN_LIGHT);
        assert_eq!(job.sections[0].format.color, SYN_LIGHT.find_current_fg);
        assert_eq!(job.sections[0].format.background, SYN_LIGHT.find_current_bg);
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
    use crate::app::preview::{
        diff_recompute_due, diff_signature, diff_step, preview_should_rebuild,
    };

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

    /// 签名必须区分「草稿变了」和「写盘了」两件事。
    ///
    /// 只看路径与草稿的话，保存之后（草稿与路径都没变、磁盘却已经追上来了）
    /// 缓存不会失效，界面会一直显示一份早就落盘的「改动」。
    #[test]
    fn diff_signature_changes_with_draft_and_with_save_count() {
        let base = diff_signature("/tmp/a.json", "{\"a\":1}", 0);
        assert_eq!(
            base,
            diff_signature("/tmp/a.json", "{\"a\":1}", 0),
            "同一输入必须得到同一签名（否则每帧都重算）"
        );
        assert_ne!(
            base,
            diff_signature("/tmp/a.json", "{\"a\":2}", 0),
            "草稿变化要换签名"
        );
        assert_ne!(
            base,
            diff_signature("/tmp/a.json", "{\"a\":1}", 1),
            "写盘次数变化要换签名（磁盘内容已变）"
        );
        assert_ne!(
            base,
            diff_signature("/tmp/b.json", "{\"a\":1}", 0),
            "目标路径变化要换签名"
        );
    }

    #[test]
    fn diff_recompute_waits_for_the_same_signature_to_settle() {
        let sig = 42;
        // 第一次见到该签名：还没计时，不算到点。
        assert!(!diff_recompute_due(None, sig, 100.0));
        assert!(!diff_recompute_due(Some((sig, 100.0)), sig, 100.0));
        // 未等够防抖时长。
        assert!(!diff_recompute_due(Some((sig, 100.0)), sig, 100.4));
        // 等够即到点。
        assert!(diff_recompute_due(Some((sig, 100.0)), sig, 100.5));
        assert!(diff_recompute_due(Some((sig, 100.0)), sig, 101.0));
    }

    #[test]
    fn a_new_signature_restarts_the_debounce_clock() {
        // 上一帧在给 sig 计时，这一帧草稿变了（新签名）：不能沿用旧时刻，
        // 否则连续输入时每一帧都会「等够时间」而逐键重算。
        assert!(!diff_recompute_due(Some((1, 100.0)), 2, 100.9));
        // 新签名自己等够后照样到点。
        assert!(diff_recompute_due(Some((2, 100.0)), 2, 100.5));
    }

    /// 复现并锁住一个真实出现过的缺陷：防抖时钟被每帧重置，永远不到点。
    ///
    /// 只测 `diff_recompute_due` 是发现不了的——那个函数本身是对的，
    /// 错在调用方每帧把 `pending` 覆盖成 `(signature, now)`。
    /// 这里驱动 `diff_step` 走多帧，断言时刻**第一次**记下后就不再变。
    #[test]
    fn the_debounce_clock_is_recorded_once_and_not_reset_every_frame() {
        let sig = 7;
        // 第一帧：没有缓存结果 → 立刻算，不进入计时。
        let (due, pending) = diff_step(None, None, sig, 100.0);
        assert!(due, "没有结果可显示时应立刻算");
        assert_eq!(pending, None);

        // 之后缓存的是旧签名（模拟「结果算出来了，但草稿又变了」）：
        // 逐帧推进，时钟必须停在第一次那一刻，而不是每帧被重置。
        let mut pending = None;
        for frame in 0..5 {
            let now = 100.1 + frame as f64 * 0.1;
            let (due, next) = diff_step(Some(999), pending, sig, now);
            assert!(!due, "第 {frame} 帧不该到点（才过了 {} 秒）", now - 100.1);
            pending = next;
        }
        assert_eq!(
            pending,
            Some((sig, 100.1)),
            "计时起点必须是第一次见到该签名的那一刻，不能被后续帧刷新"
        );
        // 终于等够：到点，并清空计时状态。
        let (due, pending) = diff_step(Some(999), pending, sig, 100.6);
        assert!(due, "等够 0.5 秒后必须到点");
        assert_eq!(pending, None);
    }

    /// 缓存已经是最新签名时不该重算，也不该留下计时状态。
    #[test]
    fn an_up_to_date_cache_never_recomputes() {
        let sig = 11;
        let (due, pending) = diff_step(Some(sig), Some((sig, 100.0)), sig, 999.0);
        assert!(!due);
        assert_eq!(pending, None, "已是最新就该清掉计时状态");
    }
}

#[cfg(test)]
mod preview_diff_tests {
    use crate::app::diff::{diff_hunks, DiffSummary, LineKind};
    use crate::app::preview::diff_signature;

    /// 对比视图的核心语义：显示的就是「按一下保存会改掉什么」。
    ///
    /// 这里用一份 opencode 配置走完「原文件 → 改过的草稿」的完整链路，
    /// 断言新增/删除的行数落在用户真正改过的那几处，而不是整份文件。
    #[test]
    fn a_one_line_change_shows_up_as_one_add_and_one_remove() {
        let original = r#"{
  "provider": {
    "p1": { "options": { "baseURL": "https://old.example/v1" } }
  }
}"#;
        let edited = original.replace("old.example", "new.example");
        let (lines, summary) = diff_hunks(original, &edited, 3);
        assert_eq!(
            summary,
            DiffSummary {
                added: 1,
                removed: 1
            },
            "只改了一行 baseURL，不该报成整份文件重写"
        );
        let removed: Vec<&str> = lines
            .iter()
            .filter(|l| l.kind == LineKind::Removed)
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(
            removed,
            vec!["    \"p1\": { \"options\": { \"baseURL\": \"https://old.example/v1\" } }"]
        );
    }

    /// 跨格式保存会把目标文件的 provider 容器整段换成界面里的那份：
    /// 对比必须把「旧容器消失、新容器出现」如实显示出来，不能只说一句「有改动」。
    #[test]
    fn replacing_a_container_shows_both_the_old_and_the_new_entries() {
        let on_disk = r#"{
  "provider": {
    "keep": { "npm": "@ai-sdk/openai" },
    "gone": { "npm": "@ai-sdk/anthropic" }
  }
}"#;
        let pending = r#"{
  "provider": {
    "keep": { "npm": "@ai-sdk/openai" },
    "added": { "npm": "@ai-sdk/google" }
  }
}"#;
        let (lines, summary) = diff_hunks(on_disk, pending, 3);
        assert_eq!(
            summary,
            DiffSummary {
                added: 1,
                removed: 1
            }
        );
        let removed: Vec<&str> = lines
            .iter()
            .filter(|l| l.kind == LineKind::Removed)
            .map(|l| l.text.as_str())
            .collect();
        let added: Vec<&str> = lines
            .iter()
            .filter(|l| l.kind == LineKind::Added)
            .map(|l| l.text.as_str())
            .collect();
        assert!(
            removed.iter().any(|l| l.contains("gone")),
            "被删的 provider 要看得见"
        );
        assert!(
            added.iter().any(|l| l.contains("added")),
            "新增的 provider 要看得见"
        );
        assert!(
            !removed.iter().any(|l| l.contains("keep")),
            "两侧都有的 provider 不该被算成改动"
        );
    }

    /// 新建文件（目标不存在 → 磁盘侧为空）时，整份文档都是新增。
    #[test]
    fn a_file_that_does_not_exist_yet_reads_as_all_new() {
        let pending = "{\n  \"provider\": {}\n}";
        let (_, summary) = diff_hunks("", pending, 3);
        assert_eq!(summary.removed, 0, "空文件没有可删的行");
        assert_eq!(summary.added, 3, "整份文档都算新增");
    }

    /// 对比缓存签名必须把路径也算进去：切页会换目标文件，不同文件的同名草稿
    /// 不能共用一份缓存结果。
    #[test]
    fn switching_pages_invalidates_the_cache_via_the_path() {
        let draft = "{\n  \"provider\": {}\n}";
        assert_ne!(
            diff_signature("/home/u/.config/opencode/opencode.json", draft, 0),
            diff_signature("/home/u/.config/kilo/kilo.json", draft, 0),
            "目标文件不同，签名必须不同"
        );
    }
}

#[cfg(test)]
mod station_token_tests {
    use crate::app::App;
    use crate::model::ProviderRow;

    /// 造一个只含指定 provider 的 App（令牌表置空，不受宿主机真实 tokens.json 影响）。
    fn app_with_providers(stations: &[(&str, &str)]) -> App {
        let providers: Vec<ProviderRow> = stations
            .iter()
            .map(|(key, base_url)| {
                let mut provider = ProviderRow::new();
                provider.key = (*key).to_string();
                provider.base_url = (*base_url).to_string();
                provider
            })
            .collect();
        App {
            providers,
            tokens: crate::tokens::StationTokens::default(),
            ..App::default()
        }
    }

    #[test]
    fn station_token_is_shared_by_every_provider_on_the_same_site() {
        let mut app = app_with_providers(&[
            ("oc_gemai", "https://gemai.huchan.cn/v1"),
            ("pi_gemai", "https://Gemai.Huchan.CN/v1/"),
            ("other", "https://other.example.com/v1"),
        ]);
        // 站点级：只在站点 origin 上存一份。
        app.tokens.set("https://gemai.huchan.cn", "pat-shared");

        assert_eq!(app.station_pat("https://gemai.huchan.cn/v1"), "pat-shared");
        assert_eq!(
            app.station_pat("https://gemai.huchan.cn"),
            "pat-shared",
            "同一站点的不同写法必须命中同一份令牌"
        );
        assert_eq!(
            app.station_pat("https://other.example.com/v1"),
            "",
            "别的站点不能被牵连"
        );
    }

    #[test]
    fn station_token_lookup_is_empty_when_never_configured() {
        let app = app_with_providers(&[("a", "https://a.example.com/v1")]);
        assert_eq!(app.station_pat("https://a.example.com/v1"), "");
        // 没有 baseUrl 的 provider 不该 panic，也不该命中任何令牌。
        assert_eq!(app.station_pat(""), "");
    }

    #[test]
    fn removing_a_station_token_clears_it_for_all_its_providers() {
        let mut app = app_with_providers(&[
            ("a", "https://a.example.com/v1"),
            ("b", "https://a.example.com/v2"),
            ("keep", "https://keep.example.com/v1"),
        ]);
        app.tokens.set("https://a.example.com", "pat-a");
        app.tokens.set("https://keep.example.com", "pat-keep");

        app.tokens.remove("https://a.example.com");

        assert_eq!(app.station_pat("https://a.example.com/v1"), "");
        assert_eq!(app.station_pat("https://a.example.com/v2"), "");
        assert_eq!(
            app.station_pat("https://keep.example.com/v1"),
            "pat-keep",
            "其他站点的令牌不受影响"
        );
    }

    #[test]
    fn station_user_id_is_station_scoped_and_optional() {
        let mut app = app_with_providers(&[
            ("oc_gemai", "https://gemai.huchan.cn/v1"),
            ("pi_gemai", "https://Gemai.Huchan.CN/v1/"),
            ("other", "https://other.example.com/v1"),
        ]);
        // 默认不填：此时不发 New-Api-User，新版站点照常可查。
        assert_eq!(app.station_user_id("https://gemai.huchan.cn/v1"), "");
        app.tokens.set("https://gemai.huchan.cn", "pat-shared");
        app.tokens.set_user_id("https://gemai.huchan.cn", "12345");
        // 同站点不同写法必须拿到同一个 ID（与令牌同样按 origin 归并）。
        assert_eq!(app.station_user_id("https://gemai.huchan.cn/v1"), "12345");
        assert_eq!(app.station_user_id("https://Gemai.Huchan.CN/v1/"), "12345");
        assert_eq!(app.station_user_id("https://other.example.com/v1"), "");
    }

    #[test]
    fn user_id_hint_follows_the_last_query_result() {
        use crate::app::balance::BalanceState;
        let mut app = app_with_providers(&[("a", "https://a.example.com/v1")]);
        let keys = vec!["a".to_string()];
        assert!(!app.station_needs_user_id(&keys), "还没查过就不该提示");

        // 令牌侧也失败 → 错误落在 Err。
        app.balance.insert(
            "a".to_string(),
            BalanceState {
                result: Some(Err(
                    "HTTP 401（New-Api-User header not provided）".to_string()
                )),
                ..Default::default()
            },
        );
        assert!(app.station_needs_user_id(&keys), "Err 里的缺头信息要认出来");

        // 令牌侧成功、只有账号部分失败 → 说明落在成功结果的 note 里。
        app.balance.insert(
            "a".to_string(),
            BalanceState {
                result: Some(Ok(crate::billing::Billing {
                    note: Some("账号令牌查询失败：该站点要求 New-Api-User".to_string()),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        assert!(
            app.station_needs_user_id(&keys),
            "note 里的缺头信息也要认出来"
        );

        // 无关失败不能触发提示（否则会误导用户去填没用的 ID）。
        app.balance.insert(
            "a".to_string(),
            BalanceState {
                result: Some(Err("网络错误：连接被重置".to_string())),
                ..Default::default()
            },
        );
        assert!(!app.station_needs_user_id(&keys));
        assert!(
            !app.station_needs_user_id(&["missing".to_string()]),
            "没有记录的 provider 不该 panic 也不该提示"
        );
    }
}

mod net_guard_override_tests {
    use crate::app::fetch::net_guard_gate;
    use crate::app::App;

    /// 守卫值换算：默认拦截，放行开关打开后不再拦截。
    #[test]
    fn override_releases_the_model_probe_gate() {
        let guard = Some("检测到系统代理".to_string());
        assert_eq!(
            net_guard_gate(&guard, false).as_deref(),
            Some("检测到系统代理"),
            "默认必须继续拦截"
        );
        assert_eq!(net_guard_gate(&guard, true), None, "放行后不该再拦");
        // 没检测到代理时，开关开或关都不产生拦截。
        assert_eq!(net_guard_gate(&None, false), None);
        assert_eq!(net_guard_gate(&None, true), None);
    }

    /// 开关确实贯通到 App：既影响门控值，也进 `current_prefs`（才能落盘）。
    #[test]
    fn app_gate_and_prefs_follow_the_switch() {
        let blocked = App {
            net_guard: Some("检测到 VPN".to_string()),
            allow_model_test_with_proxy: false,
            ..App::default()
        };
        assert!(
            net_guard_gate(&blocked.net_guard, blocked.allow_model_test_with_proxy).is_some(),
            "默认拦截"
        );
        assert!(
            !blocked.current_prefs().allow_model_test_with_proxy,
            "默认值要按拦截写出"
        );

        let allowed = App {
            allow_model_test_with_proxy: true,
            ..blocked
        };
        assert_eq!(
            net_guard_gate(&allowed.net_guard, allowed.allow_model_test_with_proxy),
            None,
            "放行后不该再拦"
        );
        assert!(
            allowed.current_prefs().allow_model_test_with_proxy,
            "放行要能写进 settings.json"
        );
    }
}

#[cfg(test)]
mod save_all_tests {
    use crate::app::save::SaveTarget;
    use crate::app::App;
    use crate::format::ConfigFormat;
    use crate::model::{ModelRow, ProviderRow};

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "model_harbor_saveall_{}_{}_{}",
            tag,
            std::process::id(),
            nonce
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn provider() -> ProviderRow {
        let mut p = ProviderRow::new();
        p.key = "p1".into();
        p.pi_api = "openai-completions".into();
        p.base_url = "https://x.example/v1".into();
        p.api_key = "sk-1".into();
        let mut m = ModelRow::new();
        m.id = "m1".into();
        p.models = vec![m];
        p
    }

    #[test]
    fn every_installed_target_is_written_to_its_own_path() {
        let dir = temp_dir("paths");
        let pi_path = dir.join("pi-models.json");
        let zcode_path = dir.join("provider_config.json");
        let skipped_start = dir.join("opencode.json");
        let targets = vec![
            SaveTarget {
                backend: ConfigFormat::Pi,
                available: true,
                path: pi_path.display().to_string(),
            },
            SaveTarget {
                backend: ConfigFormat::ZCode,
                available: true,
                path: zcode_path.display().to_string(),
            },
            // 未安装：本地与 WSL 都探测不到，一键保存不得凭空创建
            SaveTarget {
                backend: ConfigFormat::Opencode,
                available: false,
                path: skipped_start.display().to_string(),
            },
        ];
        let mut app = App {
            providers: vec![provider()],
            targets,
            config_path: pi_path.display().to_string(),
            loaded_path: pi_path.display().to_string(),
            source_format: ConfigFormat::Pi,
            current_page: ConfigFormat::Pi,
            ..App::default()
        };
        app.save_all();

        assert!(pi_path.exists(), "当前页写自己的目标: {}", app.status);
        assert!(
            zcode_path.exists(),
            "其他页写各自的目标路径，不能都塞进当前页那个文件: {}",
            app.status
        );
        assert!(!skipped_start.exists(), "未安装的后端不得被创建");
        // ZCode 那份是跨格式转换出来的：必须是 ZCode 的方言，而不是 pi 的 providers。
        let written = std::fs::read_to_string(&zcode_path).unwrap();
        assert!(written.contains("providerRules"), "{written}");
        assert!(!written.contains("\"providers\""), "{written}");
        // 状态栏逐页列出结果（写没写、写到哪）
        assert!(
            app.status.contains(ConfigFormat::Pi.label()),
            "{}",
            app.status
        );
        assert!(
            app.status.contains(ConfigFormat::ZCode.label()),
            "{}",
            app.status
        );
        assert!(app.status.starts_with("一键保存:"), "{}", app.status);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_typed_path_on_the_current_page_is_still_honored() {
        // 当前页输入框里改了路径但没回车加载：一键保存照旧按「先读后合并」写到那个路径，
        // 只有**其他**页面必须改用各自的目标路径。
        let dir = temp_dir("typed");
        let pi_path = dir.join("pi-models.json");
        let typed_path = dir.join("typed-by-hand.json");
        let zcode_path = dir.join("provider_config.json");
        let targets = vec![
            SaveTarget {
                backend: ConfigFormat::Pi,
                available: true,
                path: pi_path.display().to_string(),
            },
            SaveTarget {
                backend: ConfigFormat::ZCode,
                available: true,
                path: zcode_path.display().to_string(),
            },
        ];
        let mut app = App {
            providers: vec![provider()],
            targets,
            config_path: typed_path.display().to_string(),
            loaded_path: pi_path.display().to_string(),
            source_format: ConfigFormat::Pi,
            current_page: ConfigFormat::Pi,
            ..App::default()
        };
        app.save_all();
        assert!(
            typed_path.exists(),
            "当前页仍按输入框里的路径写: {}",
            app.status
        );
        assert!(!pi_path.exists(), "默认目标不该被写（用户已改路径）");
        assert!(zcode_path.exists(), "其他页照旧写各自的目标");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_workbuddy_save_that_drops_entries_backs_up_first() {
        // 生效清单会比界面上的条目少（同 id 只留第一条、未勾选的不写），
        // 所以这份文件确实被「删」过东西。虽然是同格式保存，也必须先备份——
        // 老规矩只在跨格式转换时备份，这种删得比跨格式还狠的情况反而没有后路。
        //
        // 注意备份和全量副本是**两件事**：备份是「上一次的 models.json 原文」，
        // 全量副本是「ModelHarbor 维护的全部条目 + 勾选状态」。两者都要有。
        let dir = temp_dir("wb_shrink");
        let wb_path = dir.join("models.json");
        let saved = serde_json::json!([
            { "id": "gpt-5.6-sol", "name": "a", "url": "https://x.example/v1",
              "apiKey": "sk-a" },
            { "id": "gpt-5.6-sol", "name": "b", "url": "https://y.example/v1",
              "apiKey": "sk-b" }
        ]);
        std::fs::write(&wb_path, serde_json::to_string_pretty(&saved).unwrap()).unwrap();
        let text = std::fs::read_to_string(&wb_path).unwrap();
        let load = crate::backends::backend(ConfigFormat::WorkBuddy)
            .parse(&text)
            .unwrap();
        let mut app = App {
            providers: load.providers,
            config_path: wb_path.display().to_string(),
            loaded_path: wb_path.display().to_string(),
            source_format: ConfigFormat::WorkBuddy,
            current_page: ConfigFormat::WorkBuddy,
            ..App::default()
        };
        let backup = app
            .save_backend_to(ConfigFormat::WorkBuddy, &wb_path.display().to_string())
            .expect("保存应当成功");
        let backup = backup.expect("删条目时必须先备份");
        let kept: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&wb_path).unwrap()).unwrap();
        assert_eq!(kept.as_array().unwrap().len(), 1, "只留启用的那条");
        let old: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&backup).unwrap()).unwrap();
        assert_eq!(
            old.as_array().unwrap().len(),
            2,
            "备份必须是删之前的两条（含被删厂商的 key）"
        );
        // 全量副本也必须在：它才是「取消勾选不会丢配置」的依托。
        let full_path = crate::backends::workbuddy::full_store_path(&wb_path.display().to_string());
        let full: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&full_path).unwrap()).unwrap();
        assert_eq!(
            full.as_array().unwrap().len(),
            2,
            "全量副本必须两条都在（含被筛掉那条的 key）: {full:#?}"
        );
        assert_eq!(
            full.as_array().unwrap()[1]["apiKey"],
            serde_json::json!("sk-b"),
            "被筛掉那条的 key 必须留在副本里"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn workbuddy_reload_restores_unchecked_entries_from_the_full_store() {
        // 端到端：保存 → 重新加载，界面上的条目数必须回到保存前。
        // 这是用户真正在意的性质——取消勾选是「不生效」，不是「删掉」。
        let dir = temp_dir("wb_restore");
        let wb_path = dir.join("models.json");
        let saved = serde_json::json!([
            { "id": "gpt-5.6-sol", "name": "a", "url": "https://x.example/v1",
              "apiKey": "sk-a", "tags": ["a-only"] },
            { "id": "gpt-5.6-sol", "name": "b", "url": "https://y.example/v1",
              "apiKey": "sk-b", "tags": ["b-only"] },
            { "id": "unique", "name": "c", "url": "https://z.example/v1",
              "apiKey": "sk-c" }
        ]);
        std::fs::write(&wb_path, serde_json::to_string_pretty(&saved).unwrap()).unwrap();
        let path = wb_path.display().to_string();
        let text = std::fs::read_to_string(&wb_path).unwrap();
        let b = crate::backends::backend(ConfigFormat::WorkBuddy);
        let load = b.parse_at(&text, &path).unwrap();
        let before: usize = load.providers.iter().map(|p| p.models.len()).sum();
        assert_eq!(before, 3, "首次加载应看到全部三条");

        let mut app = App {
            providers: load.providers,
            config_path: path.clone(),
            loaded_path: path.clone(),
            source_format: ConfigFormat::WorkBuddy,
            current_page: ConfigFormat::WorkBuddy,
            ..App::default()
        };
        app.save_backend_to(ConfigFormat::WorkBuddy, &path)
            .expect("保存应当成功");

        // 生效清单确实变少了。
        let eff: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&wb_path).unwrap()).unwrap();
        assert_eq!(eff.as_array().unwrap().len(), 2, "生效清单 = 唯一 id 数");

        // 但重新加载后条目数必须回到 3——从全量副本还原。
        let reloaded = b
            .parse_at(&std::fs::read_to_string(&wb_path).unwrap(), &path)
            .unwrap();
        let after: usize = reloaded.providers.iter().map(|p| p.models.len()).sum();
        assert_eq!(after, before, "重新加载必须从全量副本还原全部条目");
        // 被筛掉那条的字段也必须完好。
        let all: Vec<(String, String)> = reloaded
            .providers
            .iter()
            .flat_map(|p| p.models.iter().map(move |m| (p.key.clone(), m.id.clone())))
            .collect();
        assert!(all.contains(&("b".to_string(), "gpt-5.6-sol".to_string())));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_workbuddy_save_that_keeps_everything_writes_no_backup() {
        // 没删东西就别产生 .bak，否则每次保存都在磁盘上堆垃圾。
        let dir = temp_dir("wb_keep");
        let wb_path = dir.join("models.json");
        let saved = serde_json::json!([
            { "id": "gpt-5.6-sol", "name": "a", "url": "https://x.example/v1",
              "apiKey": "sk-a" },
            { "id": "claude-opus-5", "name": "b", "url": "https://y.example/v1",
              "apiKey": "sk-b" }
        ]);
        std::fs::write(&wb_path, serde_json::to_string_pretty(&saved).unwrap()).unwrap();
        let text = std::fs::read_to_string(&wb_path).unwrap();
        let load = crate::backends::backend(ConfigFormat::WorkBuddy)
            .parse(&text)
            .unwrap();
        let mut app = App {
            providers: load.providers,
            config_path: wb_path.display().to_string(),
            loaded_path: wb_path.display().to_string(),
            source_format: ConfigFormat::WorkBuddy,
            current_page: ConfigFormat::WorkBuddy,
            ..App::default()
        };
        let backup = app
            .save_backend_to(ConfigFormat::WorkBuddy, &wb_path.display().to_string())
            .expect("保存应当成功");
        assert!(backup.is_none(), "没删条目就不该产生 .bak: {backup:?}");
        assert!(!dir.join("models.json.bak").exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// 跨页写 agent：`model` 的 provider 前缀必须是**目标页网关**认的那个。
///
/// 三页（opencode / kilocode / mimocode）共用同一份 agents 数据，而 `model` 是
/// `provider/model`。把 opencode 页配好的 `opencode/…` 原样写进 kilo.json，kilo 网关
/// 不认这个前缀，agent 直接跑不起来。保存路径必须逐页归一。
#[cfg(test)]
mod cross_page_agent_model_tests {
    use crate::app::App;
    use crate::format::ConfigFormat;
    use crate::model::{AgentRow, ProviderRow};

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "model_harbor_pageagents_{}_{}_{}",
            tag,
            std::process::id(),
            nonce
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn agent(key: &str, model: &str) -> AgentRow {
        let mut row = AgentRow::new();
        row.key = key.to_string();
        row.model = model.to_string();
        row
    }

    fn configured_provider(key: &str, model: &str) -> ProviderRow {
        let mut p = ProviderRow::new();
        p.key = key.to_string();
        p.models = vec![{
            let mut m = crate::model::ModelRow::new();
            m.id = model.to_string();
            m
        }];
        p
    }

    /// 端到端：opencode 页的数据写进 kilo 文件，落盘的必须是 kilo 认的引用。
    #[test]
    fn saving_to_another_page_writes_that_pages_gateway_model() {
        let dir = temp_dir("kilo");
        let kilo_path = dir.join("kilo.json");
        let path = kilo_path.display().to_string();
        // 来源是 opencode 页，agents 里混着「自家网关」与「自配 provider」两类引用。
        let mut app = App {
            agents: vec![
                agent("writing", "sensenova/sensenova-6.8-flash-lite"),
                agent("fallback", "opencode/ling-3.0-flash-fin-free"),
            ],
            providers: vec![configured_provider("sensenova", "sensenova-6.8-flash-lite")],
            source_format: ConfigFormat::Opencode,
            current_page: ConfigFormat::Opencode,
            config_path: path.clone(),
            loaded_path: path.clone(),
            ..App::default()
        };
        app.save_backend_to(ConfigFormat::Kilocode, &path)
            .expect("保存应当成功");

        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&kilo_path).unwrap()).unwrap();
        let agents = written.get("agent").and_then(|v| v.as_object()).unwrap();
        assert_eq!(
            agents["writing"]["model"], "sensenova/sensenova-6.8-flash-lite",
            "自配 provider 的引用在 kilo 页同样有效（provider 容器一并写入），不该被改"
        );
        assert_eq!(
            agents["fallback"]["model"], "kilo/kilo-auto/free",
            "opencode 网关的引用必须换成 kilo 自家网关模型"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 同一份 agents 分别写三页，各自拿到自己网关认的值（一键保存不会串台）。
    #[test]
    fn each_page_gets_its_own_gateway_reference() {
        let dir = temp_dir("three");
        let mut app = App {
            agents: vec![agent("fallback", "opencode/ling-3.0-flash-fin-free")],
            source_format: ConfigFormat::Opencode,
            current_page: ConfigFormat::Opencode,
            ..App::default()
        };
        let mut written: Vec<(ConfigFormat, String)> = Vec::new();
        for (fmt, file) in [
            (ConfigFormat::Opencode, "opencode.json"),
            (ConfigFormat::Kilocode, "kilo.json"),
            (ConfigFormat::Mimocode, "mimocode.json"),
        ] {
            let path = dir.join(file).display().to_string();
            app.save_backend_to(fmt, &path).expect("保存应当成功");
            let root: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let model = root["agent"]["fallback"]["model"]
                .as_str()
                .unwrap()
                .to_string();
            written.push((fmt, model));
        }
        assert_eq!(written[0].1, "opencode/ling-3.0-flash-fin-free");
        assert_eq!(written[1].1, "kilo/kilo-auto/free");
        assert_eq!(written[2].1, "mimo/mimo-auto");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 界面状态本身不该被保存路径改掉：保存到别的页只是「写出去时归一」，
    /// 用户在 opencode 页看到的仍应是自己配的那串。
    #[test]
    fn saving_to_another_page_does_not_mutate_the_ui_state() {
        let dir = temp_dir("nomutate");
        let path = dir.join("kilo.json").display().to_string();
        let mut app = App {
            agents: vec![agent("fallback", "opencode/ling-3.0-flash-fin-free")],
            source_format: ConfigFormat::Opencode,
            current_page: ConfigFormat::Opencode,
            ..App::default()
        };
        app.save_backend_to(ConfigFormat::Kilocode, &path)
            .expect("保存应当成功");
        assert_eq!(
            app.agents[0].model, "opencode/ling-3.0-flash-fin-free",
            "写别的页不能顺手改掉当前页显示的配置"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 真实配置回归：把用户本机的 opencode.json 分别写向三页，逐条检查落盘的
    /// `model` 前缀是不是目标页认的。非用户机器（CI）跳过。
    ///
    /// 这条测的是**端到端**结论，而不是某个函数的中间值：用户遇到的正是
    /// 「agents 里混着 opencode/ 与自配 provider 两种引用，写进 kilo 就坏掉」。
    #[test]
    fn the_real_opencode_agents_stay_valid_on_every_page() {
        let home = std::path::PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
        let oc = home.join(".config/opencode/opencode.json");
        if !oc.exists() {
            return; // 非用户机器（CI）跳过
        }
        let dir = temp_dir("real");
        let mut app = App {
            config_path: oc.display().to_string(),
            ..App::default()
        };
        app.reload_for_page(ConfigFormat::Opencode, false);
        assert!(
            !app.agents.is_empty(),
            "真实 opencode.json 里应当有 agents 才能验证"
        );
        let configured: Vec<String> = app.providers.iter().map(|p| p.key.clone()).collect();

        for (fmt, file) in [
            (ConfigFormat::Opencode, "opencode.json"),
            (ConfigFormat::Kilocode, "kilo.json"),
            (ConfigFormat::Mimocode, "mimocode.json"),
        ] {
            let path = dir.join(file).display().to_string();
            app.save_backend_to(fmt, &path).expect("保存应当成功");
            let root: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let agents = root
                .get("agent")
                .and_then(|v| v.as_object())
                .unwrap_or_else(|| panic!("{} 应当写出 agent 容器", fmt.label()));
            for (key, value) in agents {
                let model = value.get("model").and_then(|v| v.as_str()).unwrap_or("");
                if model.trim().is_empty() {
                    continue;
                }
                assert!(
                    crate::opencode_models::model_is_valid_on(fmt, &configured, model),
                    "{} 页写出的 {}.model = {:?} 在该页无效（网关不认这个前缀）",
                    fmt.label(),
                    key,
                    model
                );
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- 切页往返：用户没改任何东西，配置不能被改掉 ----

    /// 造一个只含 agents 的 App（切页归一不碰 provider）。
    fn app_with(agents: Vec<AgentRow>, page: ConfigFormat) -> App {
        App {
            agents,
            source_format: page,
            current_page: page,
            ..App::default()
        }
    }

    /// **回归**：opencode → kilo → opencode 走一圈，原来配好的 `opencode/…` 必须还在。
    ///
    /// 这是切页归一最容易踩的坑：单向替换之后切回来，用户什么也没改却丢了配置。
    #[test]
    fn switching_pages_and_back_restores_the_original_model() {
        let mut app = app_with(
            vec![agent("fallback", "opencode/mimo-v2.6-flash-free")],
            ConfigFormat::Opencode,
        );
        // 切到 kilo 页：换成 kilo 自家网关模型
        app.normalize_agent_models_for_page(ConfigFormat::Kilocode, Some(ConfigFormat::Opencode));
        app.current_page = ConfigFormat::Kilocode;
        assert_eq!(app.agents[0].model, "kilo/kilo-auto/free");
        // 切回 opencode 页：必须还原成用户原来配的那个，而不是默认值
        app.normalize_agent_models_for_page(ConfigFormat::Opencode, Some(ConfigFormat::Kilocode));
        app.current_page = ConfigFormat::Opencode;
        assert_eq!(
            app.agents[0].model, "opencode/mimo-v2.6-flash-free",
            "切回来必须还原用户原来配的值，不能变成默认模型"
        );
    }

    /// 三页各配各的，来回切都各归各的。
    #[test]
    fn each_page_remembers_its_own_model_choice() {
        let mut app = app_with(
            vec![agent("a", "opencode/big-pickle")],
            ConfigFormat::Opencode,
        );
        // opencode → kilo，用户在 kilo 页手动挑了另一个
        app.normalize_agent_models_for_page(ConfigFormat::Kilocode, Some(ConfigFormat::Opencode));
        app.current_page = ConfigFormat::Kilocode;
        app.agents[0].model = "kilo/kilo-auto/balanced".into();
        // kilo → mimo（kilo 的选择被记住），用户在 mimo 页也挑了一个
        app.normalize_agent_models_for_page(ConfigFormat::Mimocode, Some(ConfigFormat::Kilocode));
        app.current_page = ConfigFormat::Mimocode;
        assert_eq!(app.agents[0].model, "mimo/mimo-auto");
        app.agents[0].model = "xiaomi/mimo-v2.6-pro".into();
        // mimo → kilo：回到用户在 kilo 页挑的那个
        app.normalize_agent_models_for_page(ConfigFormat::Kilocode, Some(ConfigFormat::Mimocode));
        app.current_page = ConfigFormat::Kilocode;
        assert_eq!(app.agents[0].model, "kilo/kilo-auto/balanced");
        // kilo → opencode：回到最初的
        app.normalize_agent_models_for_page(ConfigFormat::Opencode, Some(ConfigFormat::Kilocode));
        app.current_page = ConfigFormat::Opencode;
        assert_eq!(app.agents[0].model, "opencode/big-pickle");
    }

    /// 记忆只在切页时建立：第一次进某页没有记忆，按无效引用替换。
    #[test]
    fn the_first_visit_to_a_page_has_no_memory_to_restore() {
        let mut app = app_with(
            vec![agent("a", "opencode/big-pickle")],
            ConfigFormat::Opencode,
        );
        // 直接进 mimo 页（没有「离开 kilo」这一步）
        let replaced = app
            .normalize_agent_models_for_page(ConfigFormat::Mimocode, Some(ConfigFormat::Opencode));
        assert_eq!(replaced, 1);
        assert_eq!(app.agents[0].model, "mimo/mimo-auto");
    }

    /// 保存到某页时用的是**该页记忆里的值**，而不是当前页那份。
    ///
    /// 否则用户在 kilo 页选的 `kilo-auto/balanced` 会被写成默认的 `kilo-auto/free`。
    #[test]
    fn saving_to_a_page_uses_that_pages_remembered_choice() {
        let dir = temp_dir("remember");
        let mut app = app_with(
            vec![agent("a", "opencode/big-pickle")],
            ConfigFormat::Opencode,
        );
        // 去 kilo 页挑一个非默认的模型，再回到 opencode 页
        app.normalize_agent_models_for_page(ConfigFormat::Kilocode, Some(ConfigFormat::Opencode));
        app.current_page = ConfigFormat::Kilocode;
        app.agents[0].model = "kilo/kilo-auto/balanced".into();
        app.normalize_agent_models_for_page(ConfigFormat::Opencode, Some(ConfigFormat::Kilocode));
        app.current_page = ConfigFormat::Opencode;

        let path = dir.join("kilo.json").display().to_string();
        app.save_backend_to(ConfigFormat::Kilocode, &path)
            .expect("保存应当成功");
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            root["agent"]["a"]["model"], "kilo/kilo-auto/balanced",
            "写 kilo 页必须用用户在 kilo 页挑的那个，而不是当前页的引用"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 重新加载配置后，旧文件的 agent 视图必须作废（键可能已不存在）。
    #[test]
    fn reloading_discards_the_per_page_memory() {
        let mut app = app_with(
            vec![agent("old", "opencode/big-pickle")],
            ConfigFormat::Opencode,
        );
        app.normalize_agent_models_for_page(ConfigFormat::Kilocode, Some(ConfigFormat::Opencode));
        assert!(!app.agent_models_by_page.is_empty(), "切页应当留下记忆");
        app.reload_for_page(ConfigFormat::Opencode, false);
        assert!(
            app.agent_models_by_page.is_empty(),
            "重新加载后旧记忆必须清空"
        );
    }
}

#[cfg(test)]
mod real_file_grouping {
    use crate::app::App;
    use crate::format::ConfigFormat;
    use std::path::PathBuf;

    /// 走 App 的真实加载路径（`reload_for_page`）确认 WorkBuddy 的分组结果。
    #[test]
    fn workbuddy_loads_all_models_per_provider() {
        let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
        let path = home.join(".workbuddy/models.json");
        if !path.exists() {
            return; // 非用户机器（CI）跳过
        }
        let mut app = App {
            config_path: path.display().to_string(),
            ..App::default()
        };
        app.reload_for_page(ConfigFormat::WorkBuddy, false);

        let multi: Vec<(String, usize)> = app
            .providers
            .iter()
            .filter(|p| p.models.len() > 1)
            .map(|p| (p.key.clone(), p.models.len()))
            .collect();
        println!(
            "App 加载: page={:?} source={:?} providers={} models={} 多模型={:?}",
            app.current_page,
            app.source_format,
            app.providers.len(),
            app.providers.iter().map(|p| p.models.len()).sum::<usize>(),
            multi
        );
        assert!(
            app.providers.iter().any(|p| p.models.len() > 1),
            "一家提供商的多个模型必须都进同一个 provider 行"
        );
    }
}

/// 用真实配置文件确认「启用」开关只在 WorkBuddy 页出现。
///
/// 这是用户报的场景：加载 `opencode.json` 后，除 opencode 外的每一页都冒出了开关。
#[cfg(test)]
mod real_config_enable_visibility {
    use crate::app::providers::ProviderFormFlags;
    use crate::app::App;
    use crate::format::ConfigFormat;
    use std::path::PathBuf;

    #[test]
    fn only_the_workbuddy_page_offers_the_enable_toggle() {
        let home = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default());
        let opencode = home.join(".config/opencode/opencode.json");
        if !opencode.exists() {
            return; // 非用户机器（CI）跳过
        }
        // 先加载 opencode 的文件——正是触发旧 bug 的形态（source ≠ 当前页）。
        let mut app = App {
            config_path: opencode.display().to_string(),
            ..App::default()
        };
        app.reload_for_page(ConfigFormat::Opencode, false);
        assert_eq!(app.source_format, ConfigFormat::Opencode);

        for page in [
            ConfigFormat::Opencode,
            ConfigFormat::Kilocode,
            ConfigFormat::Mimocode,
            ConfigFormat::Pi,
            ConfigFormat::OhMyPi,
            ConfigFormat::DeepSeekHarness,
            ConfigFormat::ZCode,
        ] {
            app.current_page = page;
            assert!(
                !ProviderFormFlags::new(&app).show_model_disabled,
                "{} 页不该有「启用」开关",
                page.label()
            );
        }
        app.current_page = ConfigFormat::WorkBuddy;
        assert!(ProviderFormFlags::new(&app).show_model_disabled);
    }
}

/// 拖动光标：任一拖动源（含页签/后端图标）都必须点亮自定义抓取光标。
///
/// 背景：抓取态的判定此前漏了 `tab_drag_src`，于是拖卡片是抓取光标、拖后端图标
/// 却退回系统手型。自定义光标是整窗生效的，漏一个拖动源就少一处。
#[cfg(all(test, target_os = "windows"))]
mod drag_cursor_tests {
    use crate::app::App;
    use crate::format::ConfigFormat;

    #[test]
    fn every_drag_source_activates_the_grab_cursor() {
        let mut app = App::default();
        assert!(!app.is_dragging_anything(), "静止时不该点亮抓取光标");

        app.tab_drag_src = Some(ConfigFormat::ZCode);
        assert!(
            app.is_dragging_anything(),
            "拖动页签（后端图标）必须点亮抓取光标"
        );
        app.tab_drag_src = None;

        app.provider_drag_src = Some("p".to_string());
        assert!(app.is_dragging_anything(), "拖动 provider 卡片必须点亮");
        app.provider_drag_src = None;

        app.agent_drag_src = Some("a".to_string());
        assert!(app.is_dragging_anything(), "拖动 agent 卡片必须点亮");
        app.agent_drag_src = None;

        app.model_drag_src = Some("p\u{1f}m".to_string());
        assert!(app.is_dragging_anything(), "拖动 model 卡片必须点亮");
        app.model_drag_src = None;

        assert!(!app.is_dragging_anything(), "全部松开后必须熄灭抓取光标");
    }
}

/// 页签（后端图标）的状态配色：底色随状态走，图标色**不随状态走**。
///
/// 背景：高亮此前只改按钮填充，而 16px 图标几乎占满按钮，能看见的只剩一圈细边。
/// 后来改成「选中就把图标 tint 成强调色上的文字色」，深色主题下那正好是黑色，
/// 图标整个变黑（用户实测报障）。现在状态一律靠底色 + 描边表达，三态三色：
/// 选中 = **悬浮色 + 加粗描边**（悬浮的加重版，不是另起一套颜色）；
/// 拖动源 = 橙（与卡片拖动源同源 `DRAG_SOURCE_FILL`）；
/// 换位目标 = 绿（与卡片落点同源 `DROP_TARGET_COLOR`，拖动中补画在落点上）。
/// 图标 tint 只在白（已安装）/ 压淡（未安装）之间选。这里用离屏渲染把实际颜色钉住。
#[cfg(test)]
mod tab_highlight_tests {
    use crate::app::App;
    use crate::format::ConfigFormat;
    use eframe::egui;

    /// 用户实际使用的页签顺序（见其 settings.json 的 tab_order）。
    const TAB_ORDER: [&str; 6] = [
        "opencode",
        "pi",
        "zcode",
        "workbuddy",
        "deepseek-harness",
        "oh-my-pi",
    ];

    /// 页签按钮的可见状态：外框矩形、底色、描边色。
    type TabBox = (egui::Rect, egui::Color32, egui::Color32);

    /// 图标矩形：位置 + tint 色。
    type IconBox = (egui::Rect, egui::Color32);

    /// 页签条这一帧收集到的原始矩形：
    /// (矩形, 填充, 描边色, 是否贴图(brush), 描边宽度, 圆角)。
    type RawRect = (egui::Rect, egui::Color32, egui::Color32, bool, f32, u8);

    /// `tab_shapes_at` 的三段结果：按钮本体 / 图标 / 每个槽位的落点绿环。
    /// 绿环那一段带上矩形与圆角，用来断言它和按钮本体同几何（只换颜色、不改形状）。
    type TabShapes = (
        Vec<TabBox>,
        Vec<IconBox>,
        Vec<Option<(egui::Rect, egui::Color32, u8)>>,
    );

    /// 与 `tab_shapes` 同一个主题下的「悬浮底色 / 悬浮描边色」。
    ///
    /// 选中态定义为「悬浮的加重版」，所以断言必须拿这两个值来比，
    /// 而不是在测试里再写一份颜色常量——那样改了主题也照样通过。
    fn hover_visuals() -> (egui::Color32, egui::Color32) {
        let ctx = egui::Context::default();
        crate::theme::Theme::from_key("dark")
            .apply_style(&ctx, crate::theme::UiStyle::from_key("cloud"));
        let hovered = ctx.style().visuals.widgets.hovered;
        (hovered.bg_fill, hovered.bg_stroke.color)
    }

    /// 离屏跑一遍顶部栏，收集页签条区域内的**按钮底色**与**图标 tint**。
    ///
    /// egui 0.33 把图片画成带 `brush` 的 `RectShape`（贴图与 `fill` 相乘），**不是**
    /// `Shape::Mesh`，所以图标靠 `brush.is_some()` 认，tint 就是它的 `fill`。
    /// 一个页签因此贡献两个矩形：按钮底色（无 brush、约 44px 宽）与图标（有 brush、16px）。
    /// 两者都落在页签条区域内，所以先按区域筛、再按左边缘排序，下标才对得上页签序号。
    ///
    /// `pointer` 给出时，本帧把指针放在该位置（用来触发 hover，验证换位绿环）。
    fn tab_shapes_at(app: &mut App, pointer: Option<egui::Pos2>) -> TabShapes {
        tab_shapes_at_with(app, pointer, None)
    }

    /// `press_at` 给出时，第一帧在该处**按下主键并保持**，第二帧把指针移到 `pointer`。
    ///
    /// 这才是真实拖拽的形态，也是换位绿环回归的关键：egui 在
    /// 「有键按下且按下的不是本控件」时会强制清掉 HOVERED
    /// （`context.rs`：`if input.pointer.any_down() && !is_interacted_with`）。
    /// 拖动时按键落在**源**页签上，指针移到**目标**页签——目标既不是被按下的控件、
    /// 也没被点击，于是 `hovered()` 恒为 false，绿环永远画不出来（落点也算不出）。
    /// 若按键直接落在目标上，目标自己就是被按下的控件，`hovered()` 反而正常为真——
    /// 那样测等于没测，所以必须分两处。
    fn tab_shapes_at_with(
        app: &mut App,
        pointer: Option<egui::Pos2>,
        press_at: Option<egui::Pos2>,
    ) -> TabShapes {
        let ctx = egui::Context::default();
        crate::theme::Theme::from_key("dark")
            .apply_style(&ctx, crate::theme::UiStyle::from_key("cloud"));
        // 图标是 `update()` 里惰性加载的；测试直接调 `ui_top_bar` 不经过 `update`，
        // 不先加载就没有贴图网格，tint 也就无从断言。
        app.load_backend_icons(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 800.0));
        let moved = |p: egui::Pos2| vec![egui::Event::PointerMoved(p)];
        let press = |p: egui::Pos2| {
            vec![
                egui::Event::PointerMoved(p),
                egui::Event::PointerButton {
                    pos: p,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
            ]
        };
        // 帧序列：按下那一帧 → 指针移到目标那一帧。egui 的交互判定用的是**上一帧**
        // 登记的控件矩形，所以每个位置都要跑够帧数才生效。
        let frames: Vec<Vec<egui::Event>> = match (press_at, pointer) {
            (Some(from), Some(to)) => vec![press(from), press(from), moved(to), moved(to)],
            (None, Some(to)) => vec![moved(to), moved(to)],
            _ => vec![Vec::new()],
        };
        let mut last = None;
        for events in frames {
            last = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    app.ui_top_bar(ctx);
                },
            ));
        }
        let out = last.expect("至少跑一帧");
        let mut rects: Vec<RawRect> = Vec::new();
        fn collect(shape: &egui::Shape, out: &mut Vec<RawRect>) {
            match shape {
                egui::Shape::Rect(r) => out.push((
                    r.rect,
                    r.fill,
                    r.stroke.color,
                    r.brush.is_some(),
                    r.stroke.width,
                    r.corner_radius.nw,
                )),
                egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        for cs in &out.shapes {
            collect(&cs.shape, &mut rects);
        }
        // 页签条的横向范围必须**按页签数量算**，不能写死。每个页签占 48px
        // （44 宽 + 4 间距），写死 `left < 400` 时第 9 个页签（left=392，图标在 406）
        // 会被整条滤掉，表现为「少了一个图标 tint」。
        let strip_right = 48.0 * crate::backends::BACKENDS.len() as f32 + 8.0;
        let in_strip = |r: &egui::Rect| {
            r.top() < 40.0 && r.left() < strip_right && r.width() < 60.0 && r.height() < 40.0
        };
        // 悬停时同一个页签会画出**两个**矩形：egui 的 hover 框（外扩 1px、底色
        // `#383838`、描边强调色）与我们补画的落点绿环（无填充）。非悬停时只有一个本体。
        // 直接按「有没有填充」筛会在悬停那一帧错位，所以**按 x 中心聚类**：
        // 同一个页签的所有矩形共享中心，取其中填充非透明（或最宽）的那个当本体。
        let is_ring = |fill: &egui::Color32, stroke: &egui::Color32, textured: bool, w: f32| {
            !textured
                && w > 0.0
                && *fill == egui::Color32::TRANSPARENT
                && *stroke == crate::ui::DROP_TARGET_COLOR
        };
        let mut candidates: Vec<RawRect> = rects
            .iter()
            .filter(|(r, _, _, textured, w, _)| in_strip(r) && !*textured && *w > 0.0)
            .copied()
            .collect();
        candidates.sort_by(|a, b| a.0.center().x.partial_cmp(&b.0.center().x).unwrap());

        let mut bodies: Vec<TabBox> = Vec::new();
        let mut rings: Vec<Option<(egui::Rect, egui::Color32, u8)>> = Vec::new();
        for group in cluster_by_center_x(&candidates) {
            // 本体 = 该簇里填充非透明的那一个；若全透明（不该发生）取最宽的。
            let body = group
                .iter()
                .find(|(_, fill, _, _, _, _)| *fill != egui::Color32::TRANSPARENT)
                .or_else(|| {
                    group
                        .iter()
                        .max_by(|a, b| a.0.width().partial_cmp(&b.0.width()).unwrap())
                })
                .expect("簇非空");
            bodies.push((body.0, body.1, body.2));
            rings.push(
                group
                    .iter()
                    .find(|(_, fill, stroke, textured, w, _)| is_ring(fill, stroke, *textured, *w))
                    .map(|(rect, _, stroke, _, _, radius)| (*rect, *stroke, *radius)),
            );
        }

        let mut icons: Vec<IconBox> = rects
            .iter()
            .filter(|(r, _, _, textured, _, _)| in_strip(r) && *textured)
            .map(|(r, fill, _, _, _, _)| (*r, *fill))
            .collect();
        icons.sort_by(|a, b| a.0.left().partial_cmp(&b.0.left()).unwrap());
        (bodies, icons, rings)
    }

    /// 把矩形按 x 中心分组：中心相差不到 3px 的算同一个页签（hover 框与本体差 1px）。
    fn cluster_by_center_x(sorted: &[RawRect]) -> Vec<Vec<RawRect>> {
        let mut groups: Vec<Vec<RawRect>> = Vec::new();
        for item in sorted {
            match groups.last_mut() {
                Some(g) if (g[0].0.center().x - item.0.center().x).abs() < 3.0 => g.push(*item),
                _ => groups.push(vec![*item]),
            }
        }
        groups
    }

    fn tab_fills(app: &mut App) -> Vec<TabBox> {
        tab_shapes_at(app, None).0
    }

    fn tab_icon_tints(app: &mut App) -> Vec<egui::Color32> {
        tab_shapes_at(app, None)
            .1
            .into_iter()
            .map(|(_, fill)| fill)
            .collect()
    }

    /// 把指针停在第 `slot` 个页签上再跑一帧，返回每个槽位的绿环（无则 `None`）。
    fn tab_rings_with_pointer_on(
        app: &mut App,
        slot: usize,
    ) -> Vec<Option<(egui::Rect, egui::Color32, u8)>> {
        // 先跑一帧拿到稳定的几何（布局由页签数量决定，不随指针变），
        // 再按目标槽位的中心点跑第二帧，让 hover 真正命中。
        let boxes = tab_fills(app);
        let center = boxes[slot].0.center();
        tab_shapes_at(app, Some(center)).2
    }

    /// 同上，但模拟**真实拖拽**：先在第 `from` 个页签按下主键，再把指针移到第 `to` 个。
    fn tab_rings_while_dragging(
        app: &mut App,
        from: usize,
        to: usize,
    ) -> Vec<Option<(egui::Rect, egui::Color32, u8)>> {
        let boxes = tab_fills(app);
        let start = boxes[from].0.center();
        let end = boxes[to].0.center();
        tab_shapes_at_with(app, Some(end), Some(start)).2
    }

    /// 让全部后端都算「已安装」，页签顺序才与跑测试这台机器无关。
    ///
    /// 槽位由 `ConfigPaths::validate_target`（配置文件**是否真实存在**）决定：
    /// 已安装的排在前面、顺序取 `tab_order`，未安装的按名字排在后面。在开发机上
    /// 恰好装了那几个后端，写死的槽位下标（第 3 个是 ZCode）就对得上；CI 是干净机器、
    /// 一个都没装，顺序退化成全字母序，下标全部错位，整批测试在 CI 上挂掉而本地一直绿。
    /// 所以这里给每个后端在临时目录里造一份真实存在的配置文件，把顺序钉死：
    /// 全部「已安装」后，顺序只由 `TAB_ORDER` 决定，未列进去的几家按字母序补在其后。
    fn install_every_backend(app: &mut App) {
        // 全进程共用一份：路径只要存在即可，各测试各建一份只会往临时目录里堆垃圾。
        static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
        let root = DIR.get_or_init(|| {
            let root =
                std::env::temp_dir().join(format!("model-harbor-tabs-{}", std::process::id()));
            std::fs::create_dir_all(&root).expect("建临时配置目录");
            root
        });
        for backend in crate::backends::BACKENDS {
            let id = backend.id();
            let file = root.join(format!("{}.json", id.label()));
            if !file.exists() {
                std::fs::write(&file, "{}").expect("写临时配置");
            }
            app.config_paths.set_local_path(id, &file.to_string_lossy());
        }
    }

    fn app_with_tabs() -> App {
        let mut app = App {
            tab_order: TAB_ORDER.iter().map(|s| (*s).to_string()).collect(),
            ..Default::default()
        };
        install_every_backend(&mut app);
        app
    }

    /// 选中页签的底色 = 悬浮色（`widgets.hovered.bg_fill`），不是另起一套颜色。
    #[test]
    fn the_selected_tab_uses_the_hover_fill_with_a_bold_ring() {
        let mut app = app_with_tabs();
        app.current_page = ConfigFormat::Opencode;
        let tabs = tab_fills(&mut app);
        assert_eq!(
            tabs.len(),
            crate::backends::BACKENDS.len(),
            "每个后端图标各一个按钮"
        );
        assert_eq!(
            tabs[0].1,
            hover_visuals().0,
            "选中页签的底色必须是悬浮色（选中 = 悬浮的加重版）"
        );
        assert_eq!(tabs[0].2, hover_visuals().1, "选中页签的描边用悬浮描边色");
        // 其余未选中：既不是悬浮底色，也不带 2px 描边。
        for (i, (_, fill, stroke)) in tabs.iter().enumerate().skip(1) {
            assert_ne!(*fill, hover_visuals().0, "第 {i} 个未选中，不该用悬浮底色");
            assert_ne!(
                *stroke,
                hover_visuals().1,
                "第 {i} 个未选中，不该有选中描边"
            );
        }
    }

    #[test]
    fn highlight_follows_the_current_page() {
        let mut app = app_with_tabs();
        app.current_page = ConfigFormat::ZCode;
        let tabs = tab_fills(&mut app);
        assert_eq!(
            tabs[2].1,
            hover_visuals().0,
            "ZCode 在第 3 个槽位，必须是选中底色"
        );
        assert_eq!(
            tabs.iter()
                .filter(|(_, f, _)| *f == hover_visuals().0)
                .count(),
            1,
            "同一时刻只该有一个选中页签"
        );
    }

    #[test]
    fn the_grabbed_tab_is_orange_and_differs_from_the_selected_one() {
        // 「我正抓着这页」（橙，拖动源色）与「我在这页」（悬浮加重）刻意不同色，
        // 两者才不会看混。
        let mut app = app_with_tabs();
        app.current_page = ConfigFormat::Opencode;
        app.tab_drag_src = Some(ConfigFormat::ZCode);
        let tabs = tab_fills(&mut app);
        assert_eq!(tabs[0].1, hover_visuals().0, "当前页面保持选中态");
        assert_eq!(
            tabs[2].1,
            crate::ui::DRAG_SOURCE_FILL,
            "被抓住的页签用橙色（拖动源色）"
        );
        assert_ne!(
            crate::ui::DRAG_SOURCE_FILL,
            hover_visuals().0,
            "拖动源色与选中色必须是两个不同的颜色"
        );
    }

    #[test]
    fn the_icon_tint_does_not_depend_on_which_tab_is_selected() {
        // 曾经把选中页签的图标 tint 成 `selection.stroke.color`，深色主题下那正好是
        // 黑色，图标整个变黑。状态只能靠底色/描边表达，图标 tint 只能是白（已安装）
        // 或压淡色（未安装）——**逐槽位比对**才真的锁住这一点：换个选中页，同一个槽位
        // 的 tint 必须一模一样。只断言「不是黑色」会漏掉「换成任意别的颜色」的回归。
        let tints_for = |page| {
            let mut app = app_with_tabs();
            app.current_page = page;
            let tints = tab_icon_tints(&mut app);
            assert_eq!(
                tints.len(),
                crate::backends::BACKENDS.len(),
                "每个页签各有一个图标 tint"
            );
            tints
        };
        let baseline = tints_for(ConfigFormat::Opencode);
        for (i, tint) in baseline.iter().enumerate() {
            let is_black = tint.r() == 0 && tint.g() == 0 && tint.b() == 0;
            assert!(!is_black, "第 {i} 个页签的图标被 tint 成了黑色");
        }
        for page in [
            ConfigFormat::Pi,
            ConfigFormat::ZCode,
            ConfigFormat::WorkBuddy,
            ConfigFormat::DeepSeekHarness,
            ConfigFormat::OhMyPi,
        ] {
            assert_eq!(
                tints_for(page),
                baseline,
                "{page:?} 被选中后图标 tint 变了；tint 不该随选中态变化"
            );
        }
    }

    #[test]
    fn the_swap_target_is_marked_green_and_the_source_is_not() {
        // 拖动中：指针所在的**别的**页签画绿环（换位目标），被拖的那个自己用橙。
        // 绿色只给落点——拖动源不能同时是绿色，否则「要换到哪」看不出来。
        let mut app = app_with_tabs();
        app.current_page = ConfigFormat::Opencode;
        app.tab_drag_src = Some(ConfigFormat::Opencode);
        // 指针落在第 3 个槽位（ZCode）上。
        let hovered = 2usize;
        let rings = tab_rings_with_pointer_on(&mut app, hovered);
        assert_eq!(
            rings[hovered].map(|(_, c, _)| c),
            Some(crate::ui::DROP_TARGET_COLOR),
            "指针所在的槽位必须是绿色换位目标"
        );
        for (i, ring) in rings.iter().enumerate() {
            if i == hovered {
                continue;
            }
            assert_ne!(
                ring.map(|(_, c, _)| c),
                Some(crate::ui::DROP_TARGET_COLOR),
                "第 {i} 个不是落点，不该有绿色环"
            );
        }
        // 拖动源自己不能是绿色落点：源用橙色底色，绿环只属于落点。
        let tabs = tab_fills(&mut app);
        assert_eq!(
            tabs[0].1,
            crate::ui::DRAG_SOURCE_FILL,
            "拖动源用橙色，不是绿色"
        );
    }

    #[test]
    fn no_green_ring_appears_when_nothing_is_being_dragged() {
        // 绿环是「拖动中的落点」专用提示；不在拖动时不该出现，
        // 否则悬停就变绿，和「选中」的观感混在一起。
        let mut app = app_with_tabs();
        app.current_page = ConfigFormat::Opencode;
        let rings = tab_rings_with_pointer_on(&mut app, 2);
        for (i, ring) in rings.iter().enumerate() {
            assert_ne!(
                ring.map(|(_, c, _)| c),
                Some(crate::ui::DROP_TARGET_COLOR),
                "第 {i} 个：没在拖动却画了绿色落点环"
            );
        }
    }

    #[test]
    fn the_green_ring_shows_up_while_the_button_is_actually_held() {
        // 回归：真实拖拽时键按在**源**页签上、指针移到**目标**页签上，而 egui 会在
        // 「有键按下且按下的不是本控件」时强制清掉 HOVERED，所以用 `hovered()` 判断落点
        // 的话绿环永远画不出来（`drop_on` 也永远是 None，换位功能整个是坏的）。
        // 上面那个测试没按键，`hovered()` 正常为真，于是「看起来是对的」——
        // 正是这个假象让 bug 一直没被测出来。
        let mut app = app_with_tabs();
        app.current_page = ConfigFormat::Opencode;
        app.tab_drag_src = Some(ConfigFormat::Opencode);
        // 从第 1 个槽位（源）拖到第 3 个（目标）。
        let (from, to) = (0usize, 2usize);
        let rings = tab_rings_while_dragging(&mut app, from, to);
        assert_eq!(
            rings[to].map(|(_, c, _)| c),
            Some(crate::ui::DROP_TARGET_COLOR),
            "按住键拖动时，指针所在的槽位仍然必须是绿色换位目标"
        );
        for (i, ring) in rings.iter().enumerate() {
            if i == to {
                continue;
            }
            assert_ne!(
                ring.map(|(_, c, _)| c),
                Some(crate::ui::DROP_TARGET_COLOR),
                "第 {i} 个不是落点，不该有绿色环"
            );
        }
    }

    /// 换位绿环必须与按钮**同圆角**，只换颜色、不改形状。
    ///
    /// 用户报的「被选中图标按钮绿色对了，不要变圆角啊，我只是让你改边框颜色」：
    /// 绿环曾经写死圆角 3.0，而云朵档的按钮圆角是 16——于是绿环成了套在圆角按钮上的
    /// 一个方框，看着像另画了个矩形。这里钉住两者取同一个值。
    #[test]
    fn the_green_ring_shares_the_buttons_corner_radius() {
        let mut app = app_with_tabs();
        app.current_page = ConfigFormat::Opencode;
        app.tab_drag_src = Some(ConfigFormat::Opencode);
        let (from, to) = (0usize, 2usize);
        let shapes = {
            let boxes = tab_fills(&mut app);
            let start = boxes[from].0.center();
            let end = boxes[to].0.center();
            tab_shapes_at_with(&mut app, Some(end), Some(start))
        };
        let (bodies, _, rings) = shapes;
        let (_, _, radius) = rings[to].expect("目标槽位必须有绿环");
        // 按钮本体的圆角：取本体矩形的圆角（`TabBox` 只带颜色，所以用主题值核对）。
        let expected = crate::theme::UiStyle::from_key("cloud").radius();
        assert_eq!(
            radius, expected,
            "绿环圆角必须等于按钮圆角（云朵档 = {expected}），不能写死"
        );
        assert_ne!(radius, 3, "曾经写死 3.0，正是「变圆角了」的根因");
        // 绿环与按钮本体几何一致（矩形完全相同），说明只换了描边颜色、没改形状。
        assert_eq!(
            rings[to].map(|(r, _, _)| r),
            Some(bodies[to].0),
            "绿环矩形必须与按钮本体完全重合"
        );
    }
}

/// 进 WorkBuddy 页时，同一 id 多条启用必须收敛成「只启用第一条」。
///
/// 症状来自跨方言共享数据：opencode 等格式没有 `disabled` 概念（`convert` 里一律
/// 读成启用），所以切到 WorkBuddy 页时每个模型都是勾选态——而 WorkBuddy 按裸 id
/// 全局去重，多开的根本不生效，界面显示成「全部启用」是在骗人。
#[cfg(test)]
mod workbuddy_enable_normalization_tests {
    use crate::app::App;
    use crate::format::ConfigFormat;
    use crate::model::{ModelRow, ProviderRow};

    fn provider(key: &str, ids: &[&str]) -> ProviderRow {
        let mut p = ProviderRow::new();
        p.key = key.to_string();
        p.models = ids
            .iter()
            .map(|id| {
                let mut m = ModelRow::new();
                m.id = (*id).to_string();
                m
            })
            .collect();
        p
    }

    #[test]
    fn duplicates_all_enabled_are_reduced_to_the_first() {
        let mut app = App {
            providers: vec![
                provider("a", &["gpt-5.6-sol", "other"]),
                provider("b", &["gpt-5.6-sol"]),
                provider("c", &["gpt-5.6-sol"]),
            ],
            source_format: ConfigFormat::WorkBuddy,
            current_page: ConfigFormat::WorkBuddy,
            ..App::default()
        };
        app.normalize_workbuddy_enable_flags();
        let flags: Vec<(&str, &str, bool)> = app
            .providers
            .iter()
            .flat_map(|p| {
                p.models
                    .iter()
                    .map(|m| (p.key.as_str(), m.id.as_str(), m.disabled))
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(
            flags,
            vec![
                ("a", "gpt-5.6-sol", false),
                ("a", "other", false),
                ("b", "gpt-5.6-sol", true),
                ("c", "gpt-5.6-sol", true),
            ],
            "同一 id 只留第一条启用，不同 id 不受影响"
        );
    }

    #[test]
    fn normalization_is_idempotent_and_keeps_a_users_choice() {
        let mut app = App {
            providers: vec![provider("a", &["m1"]), provider("b", &["m1"])],
            source_format: ConfigFormat::WorkBuddy,
            current_page: ConfigFormat::WorkBuddy,
            ..App::default()
        };
        // 用户明确关掉第一条、启用第二条（与「第一条生效」相反）。
        app.providers[0].models[0].disabled = true;
        app.providers[1].models[0].disabled = false;
        app.normalize_workbuddy_enable_flags();
        assert!(
            app.providers[0].models[0].disabled,
            "用户关掉的那条不能被重新打开"
        );
        assert!(
            !app.providers[1].models[0].disabled,
            "用户勾的那条必须保持启用"
        );
        // 幂等：再跑一次不变。
        app.normalize_workbuddy_enable_flags();
        assert!(app.providers[0].models[0].disabled);
        assert!(!app.providers[1].models[0].disabled);
    }
}

/// 「启用」开关只属于 WorkBuddy 一页。
///
/// 曾经的 bug：开关可见性取自 `page_has_model_field("disabled")`，而那个函数在
/// 「已加载的文件格式 ≠ 当前页」时**一律返回 true**（为了让切到的新页面能填所有
/// 字段）。用户加载的是 `opencode.json`，于是切到 pi / omp / DSH / ZCode 每一页都
/// 冒出了「启用」开关——只有 opencode 自己那页（格式相同）没中招，正是用户报的
/// 「除了 opencode 都加了」。
#[cfg(test)]
mod model_enable_visibility_tests {
    use crate::app::providers::ProviderFormFlags;
    use crate::app::App;
    use crate::format::ConfigFormat;
    use crate::model::{ModelRow, ProviderRow};

    /// 造一个「文件来自 opencode、当前页是 `page`」的 App——正是会触发旧 bug 的形态。
    fn app_loaded_from_opencode(page: ConfigFormat) -> App {
        let mut model = ModelRow::new();
        model.id = "m1".into();
        model.source_format = Some(ConfigFormat::Opencode);
        model.raw = serde_json::json!({ "name": "m1" });
        let mut provider = ProviderRow::new();
        provider.key = "p1".into();
        provider.models = vec![model];
        App {
            providers: vec![provider],
            source_format: ConfigFormat::Opencode,
            current_page: page,
            ..App::default()
        }
    }

    #[test]
    fn only_workbuddy_shows_the_enable_toggle() {
        for page in [
            ConfigFormat::Opencode,
            ConfigFormat::Kilocode,
            ConfigFormat::Mimocode,
            ConfigFormat::Pi,
            ConfigFormat::OhMyPi,
            ConfigFormat::DeepSeekHarness,
            ConfigFormat::ZCode,
        ] {
            let app = app_loaded_from_opencode(page);
            assert!(
                !ProviderFormFlags::new(&app).show_model_disabled,
                "{} 页不该有「启用」开关",
                page.label()
            );
        }
        let app = app_loaded_from_opencode(ConfigFormat::WorkBuddy);
        assert!(
            ProviderFormFlags::new(&app).show_model_disabled,
            "WorkBuddy 页必须有「启用」开关"
        );
    }

    /// 即使加载的就是 WorkBuddy 自己的文件，其他页也不该跟着显示开关。
    #[test]
    fn a_workbuddy_file_does_not_leak_the_toggle_onto_other_pages() {
        let mut app = app_loaded_from_opencode(ConfigFormat::ZCode);
        app.source_format = ConfigFormat::WorkBuddy;
        assert!(
            !ProviderFormFlags::new(&app).show_model_disabled,
            "文件来自 WorkBuddy，但当前页是 ZCode，不该显示开关"
        );
    }

    /// 谓词本身：只有 WorkBuddy 为真。
    #[test]
    fn has_model_enable_is_workbuddy_only() {
        for format in [
            ConfigFormat::Opencode,
            ConfigFormat::Kilocode,
            ConfigFormat::Mimocode,
            ConfigFormat::Pi,
            ConfigFormat::OhMyPi,
            ConfigFormat::DeepSeekHarness,
            ConfigFormat::ZCode,
        ] {
            assert!(
                !format.has_model_enable(),
                "{} 不该有启用语义",
                format.label()
            );
        }
        assert!(ConfigFormat::WorkBuddy.has_model_enable());
    }
}
