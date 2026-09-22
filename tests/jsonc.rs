use model_harbor::app::parse_config_content;
use model_harbor::format::{ConfigFormat, ConfigPaths};
use model_harbor::util::strip_jsonc_comments;

#[test]
fn strip_line_comments() {
    let src = "{\n  // comment\n  \"a\": 1 }\n";
    assert_eq!(strip_jsonc_comments(src), "{\n  \n  \"a\": 1 }\n");
}

#[test]
fn strip_block_comments() {
    let src = "{ /* block\ncomment */ \"a\": 1 }";
    let out = strip_jsonc_comments(src);
    assert!(out.contains("\"a\": 1"));
    assert!(!out.contains("block"));
}

#[test]
fn string_with_slashes_not_stripped() {
    let src = "{\"url\": \"http://example.com\"}";
    assert_eq!(strip_jsonc_comments(src), src);
}

#[test]
fn string_with_escaped_quote_then_slash() {
    let src = "{\"a\": \"x\\\"\", \"b\": 1}";
    assert_eq!(strip_jsonc_comments(src), src);
}

#[test]
fn trailing_commas_removed() {
    let src = "{\"a\": 1, \"b\": [1, 2, ], }";
    assert_eq!(strip_jsonc_comments(src), "{\"a\": 1, \"b\": [1, 2 ] }");
}

#[test]
fn parse_jsonc_content() {
    let src = "{\n  // opencode config\n  \"agent\": {},\n  \"provider\": {},\n}";
    let v = parse_config_content(src).unwrap();
    assert!(v.get("agent").is_some());
    assert!(v.get("provider").is_some());
}

#[test]
fn parse_invalid_content_is_err() {
    assert!(parse_config_content("{ broken").is_err());
}

#[test]
fn parse_empty_content_is_ok() {
    let v = parse_config_content("   ").unwrap();
    assert!(v.as_object().unwrap().is_empty());
}

#[test]
fn detect_from_content_distinguishes_formats() {
    assert_eq!(
        ConfigPaths::detect_from_content("{\"providers\": {}}"),
        ConfigFormat::Pi
    );
    assert_eq!(
        ConfigPaths::detect_from_content("{\"provider\": {}}"),
        ConfigFormat::Opencode
    );
    assert_eq!(
        ConfigPaths::detect_from_content("{}"),
        ConfigFormat::Opencode
    );
    assert_eq!(
        ConfigPaths::detect_from_content("{\"providers\": {}, \"provider\": {}}"),
        ConfigFormat::Opencode,
        "both keys present -> opencode wins"
    );
}

#[test]
fn target_path_prefers_local_when_exists() {
    let mut p = std::env::temp_dir();
    p.push("opencode_test_local_target.json");
    std::fs::write(&p, "{}").unwrap();
    let paths = ConfigPaths {
        opencode: p.to_string_lossy().to_string(),
        kilocode: p.to_string_lossy().to_string(),
        mimocode: p.to_string_lossy().to_string(),
        pi: p.to_string_lossy().to_string(),
        oh_my_pi: p.to_string_lossy().to_string(),
        deepseek_harness: p.to_string_lossy().to_string(),
        zcode: p.to_string_lossy().to_string(),
        workbuddy: p.to_string_lossy().to_string(),
    };
    assert_eq!(
        paths.target_path(ConfigFormat::Opencode),
        p.to_string_lossy().to_string(),
        "local file must take priority over WSL target"
    );
    std::fs::remove_file(&p).ok();
}
