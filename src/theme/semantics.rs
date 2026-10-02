use eframe::egui::{self, Color32};

/// 语义色（绿 = 正常 / 黄 = 注意 / 红 = 异常 / 蓝 = 信息）。
///
/// **跨主题统一**：所有主题共用同一套（黑白灰随主题变，彩色不变），
/// 取中间调色，让同一支颜色在浅底和深底上都看得清（单测锁定对比度 ≥3:1）。
/// 唯一的例外是「信息蓝」和主题强调色撞色时改用青蓝，见 [`semantics`]。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Semantics {
    pub ok: Color32,
    pub warn: Color32,
    pub err: Color32,
    pub info: Color32,
}

/// 全主题共用的一套语义色。
pub const SEMANTICS: Semantics = Semantics {
    ok: Color32::from_rgb(0x19, 0x94, 0x4D),
    warn: Color32::from_rgb(0xA9, 0x7A, 0x00),
    err: Color32::from_rgb(0xD9, 0x59, 0x50),
    info: Color32::from_rgb(0x4F, 0x84, 0xCD),
};

/// 「信息蓝」与强调色撞色时的替代色（青蓝）。
const SEMANTICS_INFO_ALT: Color32 = Color32::from_rgb(0x15, 0x90, 0x9A);

/// 和强调色多近算「撞色」（RGB 空间欧氏距离）。
const ACCENT_CLASH: f32 = 45.0;

/// 取当前界面该用的语义色。
///
/// 正常就是 [`SEMANTICS`]（所有主题一致）；只有当前主题的强调色和信息蓝太接近时，
/// 信息蓝换成青蓝 —— 否则用量文字会和按钮 / 链接糊成一片。
pub fn semantics(ui: &egui::Ui) -> Semantics {
    semantics_with_accent(ui.visuals().hyperlink_color)
}

/// 按强调色取语义色（拆出来便于单测）。
fn semantics_with_accent(accent: Color32) -> Semantics {
    let mut colors = SEMANTICS;
    if distance(accent, colors.info) < ACCENT_CLASH {
        colors.info = SEMANTICS_INFO_ALT;
    }
    colors
}

/// RGB 欧氏距离。
fn distance(a: Color32, b: Color32) -> f32 {
    let [ar, ag, ab, _] = a.to_array();
    let [br, bg, bb, _] = b.to_array();
    let d = |x: u8, y: u8| {
        let diff = f32::from(x) - f32::from(y);
        diff * diff
    };
    (d(ar, br) + d(ag, bg) + d(ab, bb)).sqrt()
}

/// 对比度（WCAG，1.0 ~ 21.0）。只在单测里用来锁定「每个主题都看得清」。
#[cfg(test)]
fn contrast(a: Color32, b: Color32) -> f32 {
    let channel = |value: u8| {
        let v = f32::from(value) / 255.0;
        if v <= 0.039_28 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let rel = |color: Color32| {
        let [r, g, b, _] = color.to_array();
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    };
    let (la, lb) = (rel(a), rel(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// 单测用：其他模块锁定配色对比度时复用同一套公式。
#[cfg(test)]
pub(crate) fn contrast_for_tests(a: Color32, b: Color32) -> f32 {
    contrast(a, b)
}

/// 单测用：RGB 欧氏距离，判断两块颜色是否看得出区别。
#[cfg(test)]
pub(crate) fn distance_for_tests(a: Color32, b: Color32) -> f32 {
    distance(a, b)
}

#[cfg(test)]
mod semantics_tests {
    use super::*;
    use crate::theme::tokens::{over, DISABLED_ALPHA};
    use crate::theme::{
        needs_apply, Theme, CONTRAST_HINT_MIN, CONTRAST_TEXT_MIN, CONTRAST_UI_MIN, SPACE_1,
        SPACE_2, SPACE_4, TAP_TARGET_MIN, TEXT_BODY, TEXT_CAPTION, TEXT_HEADING,
    };

    #[test]
    fn every_semantic_color_reads_on_every_theme() {
        // 语义文字会同时出现在面板、展开卡片和折叠卡片上，三种真实底色都必须清楚。
        for theme in Theme::ALL {
            let palette = theme.palette();
            for (background_name, background) in [
                ("panel", palette.panel),
                ("faint", palette.faint),
                ("extreme", palette.extreme),
            ] {
                for (color_name, color) in [
                    ("ok", SEMANTICS.ok),
                    ("warn", SEMANTICS.warn),
                    ("err", SEMANTICS.err),
                    ("info", SEMANTICS.info),
                    ("info_alt", SEMANTICS_INFO_ALT),
                ] {
                    let ratio = contrast(color, background);
                    assert!(
                        ratio >= 3.0,
                        "{} / {background_name} 的 {color_name} 对比度只有 {ratio:.2}（底色 {background:?}）",
                        theme.key()
                    );
                }
            }
        }
    }

    #[test]
    fn info_only_shifts_when_it_clashes_with_the_accent() {
        for theme in Theme::ALL {
            let colors = semantics_with_accent(theme.palette().accent);
            let expected = if distance(theme.palette().accent, SEMANTICS.info) < ACCENT_CLASH {
                SEMANTICS_INFO_ALT
            } else {
                SEMANTICS.info
            };
            assert_eq!(colors.info, expected, "{}", theme.key());
            assert_eq!(colors.ok, SEMANTICS.ok, "绿黄红不随主题变：{}", theme.key());
            assert_eq!(colors.warn, SEMANTICS.warn, "{}", theme.key());
            assert_eq!(colors.err, SEMANTICS.err, "{}", theme.key());
            // 换过之后必须真的拉开距离
            assert!(
                distance(colors.info, theme.palette().accent) >= ACCENT_CLASH,
                "{} 的信息色仍与强调色撞色",
                theme.key()
            );
        }
    }

    #[test]
    fn apply_switches_the_active_palette_and_pins_egui_theme() {
        let ctx = egui::Context::default();
        let mut applied: Option<Theme> = None;
        for theme in Theme::ALL {
            if needs_apply(&mut applied, theme) {
                theme.apply(&ctx);
            }
            let palette = theme.palette();
            let expected_egui_theme = egui::Theme::from_dark_mode(palette.dark);
            let expected_preference = egui::ThemePreference::from(expected_egui_theme);
            let visuals = ctx.style().visuals.clone();
            assert_eq!(visuals.panel_fill, palette.panel, "{}", theme.key());
            assert_eq!(visuals.dark_mode, palette.dark, "{}", theme.key());
            assert_eq!(ctx.theme(), expected_egui_theme, "{}", theme.key());
            assert_eq!(
                ctx.options(|options| options.theme_preference),
                expected_preference,
                "{} 不应继续跟随系统主题",
                theme.key()
            );
        }
        // 同一主题不重复应用（每帧重设会白白丢掉 egui 的样式缓存）。
        // 用 `ALL` 的末项而不是写死某个主题：主题表增删时这里不该跟着改。
        let last = *Theme::ALL.last().expect("主题表非空");
        assert!(!needs_apply(&mut applied, last));
        assert!(needs_apply(&mut applied, Theme::Dark));
    }

    #[test]
    fn hint_text_is_a_real_grey_not_a_dimmed_body_color() {
        // 要保证的性质不是「提示色比正文暗」——浅色主题的正文本来就是深色，
        // 提示比它**浅**才对——而是「提示对背景的对比度明显低于正文」，
        // 同时不能淡到读不清。下面按 WCAG 对比度查。
        let ctx = egui::Context::default();
        let mut applied: Option<Theme> = None;
        for theme in Theme::ALL {
            if needs_apply(&mut applied, theme) {
                theme.apply(&ctx);
            }
            let visuals = ctx.style().visuals.clone();
            let hint_color = theme.palette().hint_color();
            assert_eq!(visuals.weak_text_color(), hint_color, "{}", theme.key());

            let panel = visuals.panel_fill;
            let body = contrast(visuals.text_color(), panel);
            let hint = contrast(hint_color, panel);
            assert!(
                hint < body * 0.6,
                "{} 的提示色不够淡：正文 {body:.1}:1 vs 提示 {hint:.1}:1",
                theme.key()
            );
            assert!(
                hint >= CONTRAST_HINT_MIN,
                "{} 的提示色太淡，读不清：{hint:.1}:1（下限 {CONTRAST_HINT_MIN}）",
                theme.key()
            );
        }
    }

    #[test]
    fn every_theme_meets_the_text_and_border_contrast_floor() {
        // 设计系统的硬指标：正文与强调色是文字（4.5:1），描边是非文字 UI（3:1）。
        for theme in Theme::ALL {
            let palette = theme.palette();
            for (what, color, floor) in [
                ("正文", palette.text, CONTRAST_TEXT_MIN),
                ("强调色", palette.accent, CONTRAST_TEXT_MIN),
                ("描边", palette.border, CONTRAST_UI_MIN),
            ] {
                for (background_name, background) in [
                    ("panel", palette.panel),
                    ("faint", palette.faint),
                    ("extreme", palette.extreme),
                ] {
                    let ratio = contrast(color, background);
                    assert!(
                        ratio >= floor,
                        "{} 的{what} / {background_name} 对比度 {ratio:.2}:1 低于 {floor}:1",
                        theme.key()
                    );
                }
            }
        }
    }

    #[test]
    fn accent_text_reads_on_the_accent_fill() {
        // 强调色是选中态行与主按钮的**底色**，所以它上面的文字要单独校验。
        for theme in Theme::ALL {
            let palette = theme.palette();
            let ratio = contrast(palette.accent_text, palette.accent);
            assert!(
                ratio >= CONTRAST_TEXT_MIN,
                "{} 的强调色文字对比度只有 {ratio:.2}:1（下限 {CONTRAST_TEXT_MIN}）",
                theme.key()
            );
        }
    }

    #[test]
    fn tap_targets_are_tall_enough_to_hit() {
        let ctx = egui::Context::default();
        Theme::Dark.apply(&ctx);
        let spacing = ctx.style().spacing.clone();
        assert!(
            spacing.interact_size.y >= TAP_TARGET_MIN,
            "点击区高度 {} 低于 {TAP_TARGET_MIN}",
            spacing.interact_size.y
        );
    }

    #[test]
    fn spacing_scale_is_used_by_the_style() {
        // 刻度常量必须真的进了 Style，否则改 token 不影响界面。
        let ctx = egui::Context::default();
        Theme::Dark.apply(&ctx);
        let spacing = ctx.style().spacing.clone();
        assert_eq!(spacing.item_spacing.y, SPACE_2);
        assert_eq!(spacing.button_padding.x, SPACE_4 - 2.0);
        assert_eq!(spacing.button_padding.y, SPACE_1 - 1.0);
    }

    #[test]
    fn text_scale_is_used_by_the_style() {
        // egui 默认 Small 是 9px，中文读不清；这条钉住它走我们的刻度。
        let ctx = egui::Context::default();
        Theme::Dark.apply(&ctx);
        let styles = ctx.style().text_styles.clone();
        assert_eq!(styles[&egui::TextStyle::Small].size, TEXT_CAPTION);
        assert_eq!(styles[&egui::TextStyle::Body].size, TEXT_BODY);
        assert_eq!(styles[&egui::TextStyle::Button].size, TEXT_BODY);
        assert_eq!(styles[&egui::TextStyle::Heading].size, TEXT_HEADING);
        // 副信息字号必须明显高于 egui 默认的 9px，否则中文读不清。
        let default_small = egui::TextStyle::Small.resolve(&egui::Style::default()).size;
        assert!(
            TEXT_CAPTION > default_small,
            "副信息字号 {TEXT_CAPTION} 不应退回 egui 默认的 {default_small}"
        );
        // 字阶的严格递增由模块顶部的 `const { assert!() }` 编译期保证。
    }

    /// 一次丢弃式渲染：取回本帧所有顶点色（含数量）。
    ///
    /// 画三帧：egui 第一帧还没有字体，文字要等下一帧才画得出来。
    fn render_colors(theme: Theme, draw: impl Fn(&mut egui::Ui)) -> Vec<(Color32, usize)> {
        let ctx = egui::Context::default();
        theme.apply(&ctx);
        let mut out = None;
        for _ in 0..3 {
            out = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(320.0, 120.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| draw(ui));
                },
            ));
        }
        // 用 `[r,g,b,a]` 当键：`Color32` 自己没有 `Ord`。
        let mut counts: std::collections::BTreeMap<[u8; 4], usize> = Default::default();
        for prim in ctx.tessellate(out.expect("应有一帧").shapes, 1.0) {
            if let egui::epaint::Primitive::Mesh(mesh) = prim.primitive {
                for v in mesh.vertices {
                    *counts
                        .entry([v.color.r(), v.color.g(), v.color.b(), v.color.a()])
                        .or_default() += 1;
                }
            }
        }
        counts
            .into_iter()
            .map(|(k, n)| (Color32::from_rgba_premultiplied(k[0], k[1], k[2], k[3]), n))
            .collect()
    }

    #[test]
    fn hint_color_matches_a_disabled_widget_as_rendered() {
        // 提示色不是拍出来的常数，而是「禁用控件文字叠出来的观感」。
        // 那就不能只验算式：要拿 egui **真实画出来的顶点色**核对
        // 算式踩的是不是那两层色（按钮底、文字），否则 egui 换了
        // 禁用按钮用哪个 WidgetVisuals，算式再对也是错的。
        for theme in Theme::ALL {
            let palette = theme.palette();
            let faded = |color: Color32| color.gamma_multiply(DISABLED_ALPHA);
            let colors: Vec<Color32> = render_colors(theme, |ui| {
                ui.add_enabled(false, egui::Button::new("删除"));
            })
            .into_iter()
            .map(|(color, _)| color)
            .collect();
            assert!(
                colors.contains(&faded(palette.widget)),
                "{} 的禁用按钮底应是控件底色淡化",
                theme.key()
            );
            assert!(
                colors.contains(&faded(palette.text)),
                "{} 的禁用按钮文字应是正文色淡化",
                theme.key()
            );
            assert_eq!(
                over(
                    over(palette.panel, faded(palette.widget)),
                    faded(palette.text)
                ),
                palette.hint_color(),
                "{} 的提示色与禁用按钮文字不一致",
                theme.key()
            );
        }
    }

    #[test]
    fn a_text_edit_hint_actually_renders_in_the_hint_color() {
        // 钉住渲染结果：占位提示画出来的必须是提示色，不能是正文色。
        // （纯文本取色是 `override_text_color.unwrap_or(PLACEHOLDER)`，
        // 正文色若走 override，就会连 `hint_text` 的兜底色一起挡掉。）
        for theme in Theme::ALL {
            let palette = theme.palette();
            let colors = render_colors(theme, |ui| {
                let mut text = String::new();
                ui.add(egui::TextEdit::singleline(&mut text).hint_text("粘贴面板访问令牌"));
            });
            assert!(
                colors
                    .iter()
                    .any(|(color, _)| *color == palette.hint_color()),
                "{} 的占位提示没有用提示色",
                theme.key()
            );
            assert!(
                !colors.iter().any(|(color, _)| *color == palette.text),
                "{} 的占位提示被正文色盖住了",
                theme.key()
            );
        }
    }

    #[test]
    fn system_theme_change_cannot_replace_the_applied_palette() {
        let ctx = egui::Context::default();
        Theme::Nord.apply(&ctx);
        let expected_panel = Theme::Nord.palette().panel;

        let _ = ctx.run(
            egui::RawInput {
                system_theme: Some(egui::Theme::Light),
                ..Default::default()
            },
            |_| {},
        );

        assert_eq!(ctx.theme(), egui::Theme::Dark);
        assert_eq!(ctx.style().visuals.panel_fill, expected_panel);
    }
}
