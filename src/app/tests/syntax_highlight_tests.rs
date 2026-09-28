use crate::app::syntax::{
    json_tokens_with, syntax_tokens_with, yaml_tokens_with, PreviewSyntax, SyntaxPalette, SYN_DARK,
    SYN_LIGHT,
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
    let text = "{\n  // 中文注释 \"引号\"\n  \"名前\": \"值\",\n  /* 块注释 */\n  \"n\": 1.5e3\n}";
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
