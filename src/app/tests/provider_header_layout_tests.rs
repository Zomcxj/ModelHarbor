//! 卡片头部的「删除 / 复制」按钮必须始终可见。
//!
//! 曾经的 bug（用户报告：预览面板打开后，第 12 个 provider 起「删除」按钮消失）：
//! 卡片头是 `ui.horizontal`（左组：拖柄、折叠钮、厂商名、baseUrl 提示、连通性结果）
//! 里再嵌一个 `right_to_left` 放按钮组。左组内容不定长——连通性错误文本最长保留
//! 96 字符（`short_err` 上限，约 670px）——左组一长就把卡片撑得**比组件区还宽**，
//! 右组（贴在卡片右缘）随之落到视口之外被 ScrollArea 裁掉，整颗按钮消失。
//!
//! 这解释了「第 N 个起」：那些 provider 是连通性测试失败、错误文本变长之后才触发，
//! 与名字长度无关（用户配置里第 12 个 `openai_xxs` 的名字并不突出）。
//!
//! 修法：`egui::Sides::new().shrink_left().truncate()`——先量右组，再把左组限制在
//! 剩余宽度内并按需截断。这里按真实结构离屏渲染（预览侧栏 + 滚动区 + Providers 区），
//! 断言每个 provider 的「删除」按钮都渲染出来且落在组件区内。

use crate::app::App;
use crate::format::ConfigFormat;
use crate::model::{ModelRow, ProviderRow};
use eframe::egui;

/// 用户实际配置里的 provider 名与顺序。
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

/// 收集全部文本形状（含嵌套）。
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

/// 场景参数：窗口宽度、是否打开预览、首个 provider 的连通性错误文本长度。
struct Scene {
    width: f32,
    preview_open: bool,
    err_len: usize,
}

/// 按真实结构离屏渲染 Providers 区：预览侧栏（按比例定宽）+ 中央滚动区。
///
/// 返回 (组件区矩形, 文本形状)。
fn render(scene: &Scene) -> (egui::Rect, Vec<TextAt>) {
    let mut app = App {
        providers: PROVIDERS.iter().map(|k| provider(k)).collect(),
        source_format: ConfigFormat::Opencode,
        current_page: ConfigFormat::Opencode,
        // 关掉首次引导条：它会让卡片整体下移，干扰按 y 配对的断言。
        guide_dismissed: true,
        show_preview: scene.preview_open,
        ..App::default()
    };
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
            // 与 App::update 同构：先预览侧栏（按比例定宽），再中央面板。
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

/// 取与 `name` 同一行的某按钮文本（y 中心相差 8px 以内）。
fn button_on_row<'a>(texts: &'a [TextAt], name: &TextAt, label: &str) -> Option<&'a TextAt> {
    texts
        .iter()
        .filter(|t| t.text == label)
        .find(|t| (t.rect.center().y - name.rect.center().y).abs() < 8.0)
}

/// 核心回归：各种宽度 / 预览开关 / 长错误文本下，「删除」按钮都必须在组件区内。
///
/// 场景里的 96 字符正是修复前唯一能把按钮整个挤出去的条件。
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
        // 修复前：这个组合下第 1 个 provider 的「删除」按钮完全未渲染。
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
        // 极窄（下限 970 窗口 - 38% 预览后更窄）也要保住右组。
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
            // 「复制」与「删除」同组，一并钉住。
            assert!(
                button_on_row(&texts, name, "复制").is_some(),
                "{ctx_desc}：{key} 的「复制」按钮没渲染出来"
            );
        }
    }
}

/// 打印每个 provider 的「删除」按钮横向范围，便于人工核对（诊断辅助）。
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
