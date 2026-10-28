use eframe::egui::{self, Color32};

/// Semantic colors (green = normal / yellow = caution / red = error / blue = info).
///
/// All themes share one set (black/white/gray follow the theme; the colors don't), tuned to mid-tones.
/// Exception: when "info blue" collides with the theme accent, a cyan-blue substitutes — see [`semantics`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Semantics {
    pub ok: Color32,
    pub warn: Color32,
    pub err: Color32,
    pub info: Color32,
}

/// The one semantic set shared by all themes.
pub const SEMANTICS: Semantics = Semantics {
    ok: Color32::from_rgb(0x19, 0x94, 0x4D),
    warn: Color32::from_rgb(0xA9, 0x7A, 0x00),
    err: Color32::from_rgb(0xD9, 0x59, 0x50),
    info: Color32::from_rgb(0x4F, 0x84, 0xCD),
};

/// The substitute when "info blue" collides with the accent (cyan-blue).
const SEMANTICS_INFO_ALT: Color32 = Color32::from_rgb(0x15, 0x90, 0x9A);

/// How close to the accent counts as a "collision" (RGB-space Euclidean distance).
const ACCENT_CLASH: f32 = 45.0;

/// Take the semantic colors the UI should use now: [`SEMANTICS`] normally; cyan-blue when info blue collides with the accent.
pub fn semantics(ui: &egui::Ui) -> Semantics {
    semantics_with_accent(ui.visuals().hyperlink_color)
}

/// Take semantic colors by accent (split out for unit testing).
fn semantics_with_accent(accent: Color32) -> Semantics {
    let mut colors = SEMANTICS;
    if distance(accent, colors.info) < ACCENT_CLASH {
        colors.info = SEMANTICS_INFO_ALT;
    }
    colors
}

/// RGB Euclidean distance between two colors (hue-space distance; unit-test / series-separation use).
pub(crate) fn distance(a: Color32, b: Color32) -> f32 {
    let [ar, ag, ab, _] = a.to_array();
    let [br, bg, bb, _] = b.to_array();
    let d = |x: u8, y: u8| {
        let diff = f32::from(x) - f32::from(y);
        diff * diff
    };
    (d(ar, br) + d(ag, bg) + d(ab, bb)).sqrt()
}

/// Relative luminance (WCAG) of a color, from its sRGB channels.
///
/// Shared with the usage views' generated series colors: whether a color reads against its background is a
/// luminance question, not a hue or saturation one.
pub(crate) fn relative_luminance(color: Color32) -> f32 {
    let channel = |value: u8| {
        let v = f32::from(value) / 255.0;
        if v <= 0.039_28 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let [r, g, b, _] = color.to_array();
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

/// Contrast ratio (WCAG, 1.0–21.0). Unit tests only.
#[cfg(test)]
fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// For tests: other modules locking color contrast reuse the same formula.
#[cfg(test)]
pub(crate) fn contrast_for_tests(a: Color32, b: Color32) -> f32 {
    contrast(a, b)
}

/// For tests: RGB Euclidean distance, judging whether two colors are visibly distinct.
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
        // Semantic text is contrast-checked on three fills: the panel, an expanded card, and a collapsed card.
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
            // After the swap, the distance must genuinely open up
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
        // The same theme isn't applied twice.
        // Use `ALL`'s last entry rather than hard-coding one: theme-table changes won't require edits here.
        let last = *Theme::ALL.last().expect("主题表非空");
        assert!(!needs_apply(&mut applied, last));
        assert!(needs_apply(&mut applied, Theme::Dark));
    }

    #[test]
    fn hint_text_is_a_real_grey_not_a_dimmed_body_color() {
        // The property under test: "the hint's contrast against the background is clearly lower than body text",
        // while never fading below legibility. Checked by WCAG contrast below.
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
        // Hard targets: body text and accents are text (4.5:1); borders are non-text UI (3:1).
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
        // The accent is the selected row's and the primary button's fill; its text is checked separately.
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
        // Scale constants must genuinely land in the Style.
        let ctx = egui::Context::default();
        Theme::Dark.apply(&ctx);
        let spacing = ctx.style().spacing.clone();
        assert_eq!(spacing.item_spacing.y, SPACE_2);
        assert_eq!(spacing.button_padding.x, SPACE_4 - 2.0);
        assert_eq!(spacing.button_padding.y, SPACE_1 - 1.0);
    }

    #[test]
    fn text_scale_is_used_by_the_style() {
        // egui's default Small is 9px — this pins it to this repo's scale.
        let ctx = egui::Context::default();
        Theme::Dark.apply(&ctx);
        let styles = ctx.style().text_styles.clone();
        assert_eq!(styles[&egui::TextStyle::Small].size, TEXT_CAPTION);
        assert_eq!(styles[&egui::TextStyle::Body].size, TEXT_BODY);
        assert_eq!(styles[&egui::TextStyle::Button].size, TEXT_BODY);
        assert_eq!(styles[&egui::TextStyle::Heading].size, TEXT_HEADING);
        // The secondary-info font size must clearly exceed egui's default 9px.
        let default_small = egui::TextStyle::Small.resolve(&egui::Style::default()).size;
        assert!(
            TEXT_CAPTION > default_small,
            "副信息字号 {TEXT_CAPTION} 不应退回 egui 默认的 {default_small}"
        );
        // The type scale's strict increase is guaranteed at compile time by the module-top `const { assert!() }`.
    }

    /// One throwaway render: collects this frame's vertex colors (with counts).
    ///
    /// Paints three frames: egui's first frame has no fonts yet — text only appears the frame after.
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
        // `[r,g,b,a]` as the key: `Color32` itself has no `Ord`.
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
        // The hint color is what disabled widget text stacks into, so verify the formula against egui's actually-painted vertex colors.
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
        // Pin the render: the placeholder hint must paint in the hint color, never the body color.
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
