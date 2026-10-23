//! The Providers block: vendor card list, drag-drop target aggregation, form field visibility flags, and reasoning-tier dialects.
//!
//! Submodules: `flags` for form field visibility and card action aggregation, `section` for the block glue
//! (sticky title row + card list + batch buttons), `card` for one vendor card, `tokens` for the token / user-data panel.

mod card;
mod flags;
mod section;
mod tokens;

pub(in crate::app) use flags::ProviderFormFlags;
pub(in crate::app) use flags::SyncField;

use super::App;
use crate::format::ConfigFormat;
use eframe::egui;
use std::collections::HashSet;

impl App {
    /// The current opencode page's schema version: v2 when the loaded content holds v2 entries, else v1.
    ///
    /// New provider / model drafts take their version from it, keeping v1-shaped new entries (protocol
    /// packages assembled as `@ai-sdk/*`, v1-only controls) out of v2 files. kilo / mimo pages always return v1.
    pub(in crate::app) fn current_opencode_schema(&self) -> crate::model::OpencodeSchemaVersion {
        let v2 = self.current_page == ConfigFormat::Opencode
            && self
                .providers
                .iter()
                .any(|p| p.schema == crate::model::OpencodeSchemaVersion::V2);
        if v2 {
            crate::model::OpencodeSchemaVersion::V2
        } else {
            crate::model::OpencodeSchemaVersion::V1
        }
    }

    /// The current page's reasoning-tier label: field name + allowed values.
    pub(super) fn dialect_variants(&self) -> (&'static str, &'static [&'static str]) {
        match self.current_page {
            // The opencode family's three share the tier field name.
            ConfigFormat::Opencode | ConfigFormat::Kilocode | ConfigFormat::Mimocode => (
                "variants",
                &["none", "low", "medium", "high", "xhigh", "max", "ultra"],
            ),
            ConfigFormat::Pi => (
                "thinkingLevelMap",
                &[
                    "off", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
                ],
            ),
            ConfigFormat::OhMyPi => (
                "thinking.efforts",
                &["minimal", "low", "medium", "high", "xhigh", "max", "ultra"],
            ),
            ConfigFormat::DeepSeekHarness => (
                "reasoningEfforts",
                &["minimal", "low", "medium", "high", "xhigh", "max", "ultra"],
            ),
            // ZCode's tiers live in optionSpecs.reasoningLevel.values, with the same vocabulary as the built-in library.
            ConfigFormat::ZCode => (
                "optionSpecs.reasoningLevel.values",
                &["minimal", "low", "medium", "high", "xhigh", "max"],
            ),
            // WorkBuddy has no tier list, only the supportsReasoning boolean.
            ConfigFormat::WorkBuddy => ("supportsReasoning", &[]),
            // QwenCode's tiers are capabilities.reasoning.efforts.
            ConfigFormat::QwenCode => ("efforts", &["low", "medium", "high", "xhigh", "max"]),
            // KimiCode's tiers are the `support_efforts` array with no official fixed vocabulary: these are just
            // common values for quick picking — the user may enter any string, written verbatim, unvalidated.
            ConfigFormat::KimiCode => ("support_efforts", &["low", "medium", "high", "max"]),
        }
    }

    /// The first-use guidance bar: three steps + one balance-query prerequisite, dismissible (state in settings.json).
    fn ui_first_run_guide(&mut self, ui: &mut egui::Ui) {
        if self.guide_dismissed {
            return;
        }
        let semantics = crate::theme::semantics(ui);
        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::same(crate::theme::SPACE_3 as i8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("开始使用")
                            .strong()
                            .color(semantics.info),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("知道了").clicked() {
                            self.guide_dismissed = true;
                        }
                    });
                });
                ui.label(
                    egui::RichText::new(
                        "1. 顶栏「配置文件」填路径或点「浏览」加载　→　\
                         2. 展开卡片填 API Key　→　3. 点「保存」写入",
                    )
                    .small(),
                );
                ui.label(
                    egui::RichText::new(
                        "想查账号余额 / 已用 / 今日用量：先在页头「令牌」里填该站点的面板访问令牌。",
                    )
                    .small()
                    .color(ui.visuals().weak_text_color()),
                );
            });
        ui.add_space(crate::theme::SPACE_2);
    }

    /// Make `(provider index, model index)` the only enabled entry for its model id.
    ///
    /// WorkBuddy's selector dedupes globally by bare model id, honoring only the first per id.
    fn enable_model_exclusively(&mut self, picked: (usize, usize)) {
        let (pi, mi) = picked;
        let Some(target) = self
            .providers
            .get(pi)
            .and_then(|p| p.models.get(mi))
            .map(|m| m.id.trim().to_string())
        else {
            return;
        };
        if target.is_empty() {
            return;
        }
        for (i, p) in self.providers.iter_mut().enumerate() {
            for (j, m) in p.models.iter_mut().enumerate() {
                if m.id.trim() == target {
                    // Compare by index: one card may hold two entries with the same id.
                    m.disabled = (i, j) != picked;
                }
            }
        }
    }

    /// Entering the WorkBuddy page: converge "multiple enabled per id" to "only the first enabled".
    ///
    /// WorkBuddy's selector dedupes globally by bare id — at most one enabled per id. Convergence is idempotent;
    /// running it again changes nothing.
    pub(super) fn normalize_workbuddy_enable_flags(&mut self) {
        let mut seen: HashSet<String> = HashSet::new();
        for p in self.providers.iter_mut() {
            for m in p.models.iter_mut() {
                let id = m.id.trim().to_string();
                // Rows with empty ids fall back to the provider key at save; skip them.
                if id.is_empty() || m.disabled {
                    continue;
                }
                if !seen.insert(id) {
                    m.disabled = true;
                }
            }
        }
    }
}

impl App {
    /// Sync UI state (the card-collapse set + popover keys) after a provider key rename.
    pub(super) fn sync_provider_rename(&mut self, old: &str, new: &str) {
        if old == new || new.is_empty() {
            return;
        }
        self.rename_collapsed_card("providers", old, new);
        // Index-keyed popovers just close; reopen when needed
        let variant_prefix = format!("variant_open_{}_", old);
        let show_key = format!("show_new_model_{}", old);
        let new_variant_key = format!("new_model_variant_{}", old);
        self.variant_open
            .retain(|k| !k.starts_with(&variant_prefix) && k != &show_key && k != &new_variant_key);
    }
}
