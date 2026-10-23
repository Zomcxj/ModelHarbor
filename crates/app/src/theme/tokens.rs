use eframe::egui::Color32;

/// The alpha used when dimming disabled widgets (matching egui's `Visuals::disabled_alpha` default).
pub(crate) const DISABLED_ALPHA: f32 = 0.5;

/// The acrylic tier: the panel fill's retained opacity (the rest reveals DWM acrylic blur).
///
/// Cards / inputs are more solid than it — see [`GLASS_SURFACE_ALPHA`]; the two keep their gap, and cards
/// are always **more solid** than the panel.
pub const GLASS_PANEL_ALPHA: f32 = 0.50;

/// The acrylic tier: cards' / inputs' / widgets' fill opacity, higher than [`GLASS_PANEL_ALPHA`].
pub const GLASS_SURFACE_ALPHA: f32 = 0.55;

// ── Spacing scale (multiples of 4) ────────────────────────────
/// Between inline elements.
pub const SPACE_1: f32 = 4.0;
/// Within one group.
pub const SPACE_2: f32 = 8.0;
/// Between a widget and its label.
pub const SPACE_3: f32 = 12.0;
/// Card padding.
pub const SPACE_4: f32 = 16.0;
/// Between cards.
pub const SPACE_5: f32 = 24.0;
/// Between blocks.
pub const SPACE_6: f32 = 32.0;
/// Page-level whitespace.
pub const SPACE_7: f32 = 48.0;

// ── Font-size scale ──────────────────────────────────────────
/// Badges / very short marks, 11px (egui's default `Small` is 9px).
pub const TEXT_CAPTION: f32 = 11.0;
/// Secondary info: sources, request counts, station names.
pub const TEXT_SMALL: f32 = 12.0;
/// Card titles (the provider key).
pub const TEXT_TITLE: f32 = 16.0;
/// Body / widget text, 13px (egui's default).
pub const TEXT_BODY: f32 = 13.0;
/// Block titles.
pub const TEXT_HEADING: f32 = 18.0;

/// The compile-time assertion that the type scale strictly increases (caption < small < body < title < heading).
const _: () = {
    assert!(TEXT_CAPTION < TEXT_SMALL, "字阶错位：caption 不小于 small");
    assert!(TEXT_SMALL < TEXT_BODY, "字阶错位：small 不小于 body");
    assert!(TEXT_BODY < TEXT_TITLE, "字阶错位：body 不小于 title");
    assert!(TEXT_TITLE < TEXT_HEADING, "字阶错位：title 不小于 heading");
};

// ── Corner radius ────────────────────────────────────────────
/// The radius slider's cap (the appearance panel's continuous range is 0..=this).
pub const RADIUS_SLIDER_MAX: u8 = 20;
/// Badges / small buttons.
pub const RADIUS_SM: u8 = 4;
/// Cards and widgets.
pub const RADIUS_MD: u8 = 10;
/// Popovers.
pub const RADIUS_LG: u8 = 14;

/// The minimum height of a clickable target (the floor under touch and high DPI).
pub const TAP_TARGET_MIN: f32 = 24.0;

/// The minimum contrast body / semantic colors require against fills (WCAG AA).
pub const CONTRAST_TEXT_MIN: f32 = 4.5;

/// Hints' / placeholder text's minimum contrast, one tier below body.
pub const CONTRAST_HINT_MIN: f32 = 3.0;

/// Non-text UI components' (widget borders, frames) minimum contrast.
pub const CONTRAST_UI_MIN: f32 = 3.0;

/// Composite a **premultiplied-alpha** color onto an opaque base: `result = src + base × (1 − α)`.
pub(crate) fn over(dst: Color32, src: Color32) -> Color32 {
    let keep = 1.0 - f32::from(src.a()) / 255.0;
    let mix = |s: u8, d: u8| {
        (f32::from(s) + f32::from(d) * keep)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(
        mix(src.r(), dst.r()),
        mix(src.g(), dst.g()),
        mix(src.b(), dst.b()),
    )
}
