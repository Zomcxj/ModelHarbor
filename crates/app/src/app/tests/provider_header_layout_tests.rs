//! A card head's "delete / duplicate" buttons must always be visible.
//!
//! The card head is a `ui.horizontal` (left group: grab handle, collapse button, vendor name, baseUrl note,
//! connectivity result) nesting a `right_to_left` button group, laid out with
//! `egui::Sides::new().shrink_left().truncate()`: measure the right group first, then constrain the left
//! group to the remaining width, truncating as needed. Rendered offscreen with real structure (preview
//! sidebar + scroll area + the Providers block), asserting every provider's "delete" button lands inside the component area.

use crate::app::App;
use crate::format::ConfigFormat;
use crate::model::{ModelRow, ProviderRow};
use eframe::egui;

/// Provider names and order from a real user config.
const PROVIDERS: [&str; 15] = [
    "sensenova",
    "openai_apizh",
    "openai_leyi",
    "openai_247kan",
    "grok_247kan",
    "claude_justwoker",
    "claude_agentrouter",
    "openai_agentrouter",
    "claude_linxi",
    "claude_100x",
    "openai_she",
    "openai_xxs",
    "openai_hajimi",
    "gm_huige",
    "workbuddy",
];

fn provider(key: &str) -> ProviderRow {
    let mut p = ProviderRow::new();
    p.key = key.to_string();
    let mut m = ModelRow::new();
    m.id = "m1".to_string();
    p.models = vec![m];
    p
}

struct TextAt {
    text: String,
    rect: egui::Rect,
}

/// Collect all text shapes (including nested).
fn texts_of(out: &egui::FullOutput) -> Vec<TextAt> {
    let mut texts = Vec::new();
    fn collect(shape: &egui::Shape, out: &mut Vec<TextAt>) {
        match shape {
            egui::Shape::Text(t) => out.push(TextAt {
                text: t.galley.text().to_string(),
                rect: egui::Rect::from_min_size(t.pos, t.galley.size()),
            }),
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    for cs in &out.shapes {
        collect(&cs.shape, &mut texts);
    }
    texts
}

/// Scenario parameters: window width, whether the preview is open, the first provider's connectivity error text length.
struct Scene {
    width: f32,
    preview_open: bool,
    err_len: usize,
}

/// Offscreen-render the Providers block with real structure: the preview sidebar (ratio-sized) + the central scroll area.
///
/// Returns (component-area rect, text shapes).
fn render(scene: &Scene) -> (egui::Rect, Vec<TextAt>) {
    let mut app = App {
        providers: PROVIDERS.iter().map(|k| provider(k)).collect(),
        source_format: ConfigFormat::Opencode,
        current_page: ConfigFormat::Opencode,
        // Turn off the first-use guidance: it would push every card down, disturbing the y-based pairing assertions.
        guide_dismissed: true,
        show_preview: scene.preview_open,
        ..App::default()
    };
    // Collapse everything, leaving only card heads (this test targets head layout), while also avoiding the
    // user prefs' collapse state — expanded cards would grow too tall and tail cards would clip vertically in the ScrollArea.
    app.set_all_providers_collapsed(true);
    if scene.err_len > 0 {
        let state = app.latency.entry(PROVIDERS[0].to_string()).or_default();
        state.provider = Some(Err("e".repeat(scene.err_len)));
    }
    let ctx = egui::Context::default();
    crate::theme::Theme::from_key("dark").apply_style(
        &ctx,
        crate::theme::UiStyle::from_key("cloud"),
        false,
    );
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(scene.width, 820.0));
    let mut area = egui::Rect::NOTHING;
    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        },
        |ctx| {
            // Isomorphic to App::update: the preview sidebar first (ratio-sized), then the central panel.
            let max_w = (scene.width - 320.0).max(220.0);
            let preview_w = (scene.width * 0.38).clamp(220.0, max_w);
            egui::SidePanel::right("preview_panel")
                .exact_width(preview_w)
                .show_animated(ctx, app.show_preview, |ui| {
                    ui.label("预览面板占位");
                });
            egui::CentralPanel::default().show(ctx, |ui| {
                area = ui.available_rect_before_wrap();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        app.ui_providers_section(ui);
                    });
            });
        },
    );
    (area, texts_of(&out))
}

/// Take a button's text on the same line as `name` (y centers within 8px).
fn button_on_row<'a>(texts: &'a [TextAt], name: &TextAt, label: &str) -> Option<&'a TextAt> {
    texts
        .iter()
        .filter(|t| t.text == label)
        .find(|t| (t.rect.center().y - name.rect.center().y).abs() < 8.0)
}

/// The core regression: across widths / preview toggles / long error texts, "delete" buttons must stay inside the component area.
#[test]
fn delete_button_stays_visible_in_every_layout() {
    let scenes = [
        Scene {
            width: 1250.0,
            preview_open: false,
            err_len: 0,
        },
        Scene {
            width: 970.0,
            preview_open: true,
            err_len: 0,
        },
        // Before the fix: under this combination, the 1st provider's "delete" button rendered not at all.
        Scene {
            width: 970.0,
            preview_open: true,
            err_len: 96,
        },
        Scene {
            width: 970.0,
            preview_open: false,
            err_len: 96,
        },
        // Extremely narrow (the 970 floor, narrower still after a 38% preview) must keep the right group too.
        Scene {
            width: 600.0,
            preview_open: true,
            err_len: 96,
        },
    ];
    for scene in &scenes {
        let (area, texts) = render(scene);
        let ctx_desc = format!(
            "窗口 {:.0} 预览={} 错误文本 {} 字符",
            scene.width, scene.preview_open, scene.err_len
        );
        for key in PROVIDERS {
            let name = texts
                .iter()
                .find(|t| t.text == key)
                .unwrap_or_else(|| panic!("{ctx_desc}：{key} 的名称没渲染出来"));
            let del = button_on_row(&texts, name, "删除").unwrap_or_else(|| {
                panic!("{ctx_desc}：{key} 的「删除」按钮没渲染出来（被挤出视口）")
            });
            assert!(
                del.rect.right() <= area.right() && del.rect.left() >= area.left(),
                "{ctx_desc}：{key} 的「删除」按钮跑到组件区外（{:?} 不在 {:?} 内）",
                del.rect,
                area
            );
            // "Duplicate" shares the group with "delete" — pinned together.
            assert!(
                button_on_row(&texts, name, "复制").is_some(),
                "{ctx_desc}：{key} 的「复制」按钮没渲染出来"
            );
        }
    }
}

/// Print each provider's "delete" button's horizontal range (a diagnostic aid).
#[test]
fn dump_delete_button_positions() {
    let scenes = [
        Scene {
            width: 970.0,
            preview_open: true,
            err_len: 0,
        },
        Scene {
            width: 970.0,
            preview_open: true,
            err_len: 96,
        },
    ];
    for scene in &scenes {
        let (area, texts) = render(scene);
        println!(
            "\n═══ 窗口 {:.0} 预览={} 错误文本 {} 组件区 [{:.1},{:.1}] ═══",
            scene.width,
            scene.preview_open,
            scene.err_len,
            area.left(),
            area.right()
        );
        for (i, key) in PROVIDERS.iter().enumerate() {
            let Some(name) = texts.iter().find(|t| t.text == *key) else {
                println!("  {:2}. {key:22} 名称未渲染", i + 1);
                continue;
            };
            match button_on_row(&texts, name, "删除") {
                Some(t) => println!(
                    "  {:2}. {key:22} 名 {:7.1}  删除 [{:7.1},{:7.1}] 余量 {:+.1}",
                    i + 1,
                    name.rect.left(),
                    t.rect.left(),
                    t.rect.right(),
                    area.right() - t.rect.right()
                ),
                None => println!(
                    "  {:2}. {key:22} 名 {:7.1}  删除 —— 未渲染 ——",
                    i + 1,
                    name.rect.left()
                ),
            }
        }
    }
}
