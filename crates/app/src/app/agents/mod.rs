//! The Agents block: card list, edit form, and add form (opencode-family pages only).

mod combo;
mod view;

// Flat facade: lets `use super::*` unit tests see cross-submodule items directly.
use combo::*;
#[cfg(test)]
use view::*;

use super::App;
use crate::app::bars::{sticky_begin, sticky_end};
use crate::model::AgentRow;
use crate::ui::{card_frame, card_list, field_label, move_item, numeric_text_edit, DragHandle};
use eframe::egui;
use std::collections::HashSet;

impl App {
    /// The Agents block: the title row sticks to the top (always visible while scrolling), content right below.
    pub(super) fn ui_agents_section(&mut self, ui: &mut egui::Ui) {
        let anchor = sticky_begin(ui, 30.0);
        let matched: Vec<usize> = (0..self.agents.len()).collect();

        if self.agents.is_empty() && !self.show_new_agent {
            self.show_new_agent = true;
        }

        let mut to_remove: Option<usize> = None;
        let mut to_copy: Option<usize> = None;
        let mut hover_target: Option<String> = None;
        let card_gap = if crate::theme::active_style(ui.ctx()).has_card_shadow() {
            crate::theme::SPACE_2
        } else {
            0.0
        };
        card_list(ui, &matched, card_gap, |ui, idx| {
            self.render_agent_card(ui, idx, &mut to_remove, &mut to_copy, &mut hover_target);
        });
        if let Some(idx) = to_remove {
            self.agents.remove(idx);
            self.status = "已删除 agent".into();
        }
        if let Some(idx) = to_copy {
            let mut a = self.agents[idx].clone();
            a.key = format!("{}_copy", a.key);
            self.agents.push(a);
            self.status = "已复制 agent".into();
        }
        if self.agent_drag_src.is_some() {
            self.agent_drag_target = hover_target;
        } else {
            self.agent_drag_target = None;
        }

        ui.add_space(crate::theme::SPACE_2);
        if ui.button("新增 Agent").clicked() {
            self.show_new_agent = !self.show_new_agent;
        }
        if self.show_new_agent {
            self.ui_new_agent_form(ui);
        }
        sticky_end(ui, anchor, |ui| {
            ui.horizontal(|ui| {
                ui.strong(egui::RichText::new("Agents").size(crate::theme::TEXT_HEADING));
                // Agents belong only to opencode pages: with a non-opencode source the UI has no agents data,
                // and cross-format saves don't manage the target file's agent container.
                if !self.source_format.is_opencode_family() {
                    let src = self.source_format.label();
                    ui.label(
                        egui::RichText::new(format!(
                            "（来源 {}：目标文件已有 agents 会保留，不被清空）",
                            src
                        ))
                        .small()
                        .weak(),
                    );
                }
                if !self.agents.is_empty() {
                    let all_open = self.agents.iter().all(|a| !self.agent_collapsed(&a.key));
                    // The same icon-button set as the Providers title row: expand / collapse all.
                    let chevrons_tint = ui.visuals().text_color();
                    if crate::app::bars::toolbar_icon_button(
                        ui,
                        self.toolbar_icons.unfold.as_ref(),
                        false,
                        chevrons_tint,
                    )
                    .on_hover_text(if all_open {
                        "收起全部卡片"
                    } else {
                        "展开全部卡片"
                    })
                    .clicked()
                    {
                        // all_open true = everything is expanded now → the button reads "collapse all"
                        self.set_all_agents_collapsed(all_open);
                        let ctx = ui.ctx().clone();
                        let open = !all_open;
                        for agent in &self.agents {
                            crate::motion::snap_collapse(
                                &ctx,
                                egui::Id::new(("agent_card", agent.key.clone())),
                                open,
                            );
                        }
                    }
                }
                // The free-model status and "refresh" both live in the title row (right of the expand button).
                // Shown only for backends with a free tier.
                let (free_prefix, free_list) =
                    current_free_models(self.current_page, &self.free_models);
                if free_prefix.is_some() {
                    let (fetching, error) =
                        current_free_status(self.current_page, &self.free_models);
                    if model_options_hint(
                        ui,
                        self.current_page.label(),
                        free_list.len(),
                        error,
                        fetching,
                    ) {
                        self.start_free_models_fetch(self.current_page);
                    }
                }
            });
        });
    }

    pub(super) fn render_agent_card(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        to_remove: &mut Option<usize>,
        to_copy: &mut Option<usize>,
        hover_target: &mut Option<String>,
    ) {
        let key = self.agents[idx].key.clone();
        let open = !self.agent_collapsed(&key);
        let highlight = if self.agent_drag_target.as_deref() == Some(key.as_str()) {
            2
        } else if self.agent_drag_src.as_deref() == Some(key.as_str()) {
            1
        } else {
            0
        };
        let card_id = egui::Id::new(("agent_card", key.clone()));
        let resp = card_frame(ui, open, highlight, card_id, |ui| {
            // Same provider card head: the left group's content is variable-length, so `Sides::shrink_left().truncate()`
            // measures the right group first, keeping "delete / duplicate" glued to the card's right edge.
            let _ = egui::Sides::new().shrink_left().truncate().show(
                ui,
                |ui| {
                    let h = ui.add(DragHandle);
                    if h.drag_started() {
                        self.agent_drag_src = Some(key.clone());
                        self.agent_drag_target = None;
                    }
                    if h.drag_stopped() {
                        if self.agent_drag_src == Some(key.clone()) {
                            if let Some(dst) = self.agent_drag_target.clone() {
                                let s = self.agents.iter().position(|a| a.key == key);
                                let d = self.agents.iter().position(|a| a.key == dst);
                                if let (Some(s), Some(d)) = (s, d) {
                                    move_item(&mut self.agents, s, d);
                                }
                            }
                        }
                        self.agent_drag_src = None;
                        self.agent_drag_target = None;
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(if open { "▼" } else { "▶" }).size(14.0),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        self.set_agent_collapsed(&key, open);
                    }
                    ui.strong(&self.agents[idx].key);
                },
                |ui| {
                    if ui.button("删除").clicked() {
                        *to_remove = Some(idx);
                    }
                    if ui.button("复制").clicked() {
                        *to_copy = Some(idx);
                    }
                },
            );
            // Collapse / expand with height animation; the animation id derives from the key — a rename changes the id.
            crate::motion::animated_collapse(ui, card_id, open, |ui| {
                self.render_agent_form(ui, idx);
            });
        });
        if let Some(src_key) = &self.agent_drag_src {
            if src_key != &key && resp.contains_pointer() && hover_target.is_none() {
                *hover_target = Some(key.clone());
            }
        }
    }

    pub(super) fn render_agent_form(&mut self, ui: &mut egui::Ui, idx: usize) {
        // The dialect label (v2's temperature lives under request.body).
        let form_flags = crate::app::providers::ProviderFormFlags::new(self);
        let prev_key = self.agents[idx].key.clone();
        let other_keys: HashSet<String> = self
            .agents
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != idx)
            .map(|(_, a)| a.key.trim().to_string())
            .collect();
        let a = &mut self.agents[idx];
        ui.horizontal_wrapped(|ui| {
            field_label(ui, 120.0, "key");
            let key_resp = crate::ui::text_edit(
                ui,
                egui::TextEdit::singleline(&mut a.key).desired_width(120.0),
            );
            if !a.key.trim().is_empty() && other_keys.contains(a.key.trim()) {
                key_resp.on_hover_text("key 与其他 agent 重复，保存将被阻止");
                ui.label(
                    egui::RichText::new("⚠ 重复")
                        .small()
                        .color(crate::theme::semantics(ui).err),
                );
            }
            field_label(ui, 120.0, "mode");
            crate::ui::text_edit(
                ui,
                egui::TextEdit::singleline(&mut a.mode).desired_width(120.0),
            );
            field_label(ui, 120.0, "description");
            crate::ui::text_edit(
                ui,
                egui::TextEdit::singleline(&mut a.description).desired_width(450.0),
            );
        });
        ui.horizontal_wrapped(|ui| {
            field_label(ui, 120.0, "model");
            let current = a.model.clone();
            let ctx = AgentComboCtx {
                page: self.current_page,
                providers: &self.providers,
                free_models: &self.free_models,
            };
            a.model = ctx.model_combo(ui, format!("agent_model_{}", a.key), &current);
            field_label(ui, 120.0, "variant");
            let current_variant = a.variant.clone();
            a.variant =
                agent_variant_combo(ui, format!("agent_variant_{}", a.key), &current_variant);
        });
        ui.horizontal_wrapped(|ui| {
            field_label(ui, 120.0, form_flags.agent_temperature_label);
            numeric_text_edit(ui, &mut a.temperature, 120.0, "");
            field_label(ui, 120.0, "color");
            crate::ui::text_edit(
                ui,
                egui::TextEdit::singleline(&mut a.color).desired_width(120.0),
            );
            field_label(ui, 120.0, "system");
            crate::ui::text_edit(
                ui,
                egui::TextEdit::singleline(&mut a.system).desired_width(450.0),
            );
        });
        // Sync the expanded state after a key rename.
        let new_key = self.agents[idx].key.clone();
        if new_key != prev_key {
            self.sync_agent_rename(&prev_key, &new_key);
        }
    }

    pub(super) fn ui_new_agent_form(&mut self, ui: &mut egui::Ui) {
        // The dialect label (v2's temperature lives under request.body).
        let form_flags = crate::app::providers::ProviderFormFlags::new(self);
        ui.group(|ui| {
            ui.horizontal_wrapped(|ui| {
                field_label(ui, 120.0, "key");
                crate::ui::text_edit(
                    ui,
                    egui::TextEdit::singleline(&mut self.new_agent.key)
                        .hint_text("coding-assistant")
                        .desired_width(120.0),
                );
                field_label(ui, 120.0, "mode");
                crate::ui::text_edit(
                    ui,
                    egui::TextEdit::singleline(&mut self.new_agent.mode)
                        .hint_text("subagent")
                        .desired_width(120.0),
                );
                field_label(ui, 120.0, "description");
                crate::ui::text_edit(
                    ui,
                    egui::TextEdit::singleline(&mut self.new_agent.description)
                        .hint_text("简要描述此 agent 的用途")
                        .desired_width(450.0),
                );
            });
            ui.horizontal_wrapped(|ui| {
                field_label(ui, 120.0, "model");
                let current = self.new_agent.model.clone();
                let ctx = AgentComboCtx {
                    page: self.current_page,
                    providers: &self.providers,
                    free_models: &self.free_models,
                };
                self.new_agent.model = ctx.model_combo(ui, "new_agent_model".to_string(), &current);
                field_label(ui, 120.0, "variant");
                let current_variant = self.new_agent.variant.clone();
                self.new_agent.variant =
                    agent_variant_combo(ui, "new_agent_variant".to_string(), &current_variant);
            });
            ui.horizontal_wrapped(|ui| {
                field_label(ui, 120.0, form_flags.agent_temperature_label);
                numeric_text_edit(ui, &mut self.new_agent.temperature, 120.0, "0.7");
                field_label(ui, 120.0, "color");
                crate::ui::text_edit(
                    ui,
                    egui::TextEdit::singleline(&mut self.new_agent.color)
                        .hint_text("#00ccff")
                        .desired_width(120.0),
                );
                field_label(ui, 120.0, "system");
                crate::ui::text_edit(
                    ui,
                    egui::TextEdit::singleline(&mut self.new_agent.system)
                        .hint_text("系统提示词")
                        .desired_width(450.0),
                );
            });
            ui.horizontal(|ui| {
                ui.add_space(crate::theme::SPACE_7);
                if ui.button("确认").clicked() {
                    let key = self.new_agent.key.trim().to_string();
                    if key.is_empty() {
                        self.status = "请填写 agent key".into();
                    } else if self.agents.iter().any(|a| a.key.trim() == key) {
                        self.status = format!("agent key \"{}\" 已存在", key);
                    } else {
                        let na = self.new_agent.clone();
                        self.agents.push(na);
                        self.new_agent = AgentRow::new();
                        self.show_new_agent = false;
                        self.status = "已添加 agent".into();
                    }
                }
                if ui.button("取消").clicked() {
                    self.new_agent = AgentRow::new();
                    self.show_new_agent = false;
                }
            });
        });
    }

    /// Sync UI state (the card-collapse set) after an agent key rename.
    pub(super) fn sync_agent_rename(&mut self, old: &str, new: &str) {
        if old == new || new.is_empty() {
            return;
        }
        self.rename_collapsed_card("agents", old, new);
    }
}

#[cfg(test)]
mod model_options_tests {
    use super::{agents_for_page, model_options};
    use crate::format::ConfigFormat;
    use crate::model::{AgentRow, ModelRow, ProviderRow};

    /// Build a provider with several models.
    fn provider(key: &str, models: &[&str]) -> ProviderRow {
        let mut row = ProviderRow::new();
        row.key = key.to_string();
        row.models = models
            .iter()
            .map(|id| {
                let mut model = ModelRow::new();
                model.id = id.to_string();
                model
            })
            .collect();
        row
    }

    /// Build an agent with a model.
    fn agent(key: &str, model: &str) -> AgentRow {
        let mut row = AgentRow::new();
        row.key = key.to_string();
        row.model = model.to_string();
        row
    }

    #[test]
    fn includes_provider_models_with_key_prefix() {
        let providers = vec![provider("sensenova", &["deepseek-v4-flash"])];
        let options = model_options(&providers, &[], "");
        assert_eq!(options, vec!["sensenova/deepseek-v4-flash"]);
    }

    /// **The core requirement**: own-gateway models sort first, user-configured providers after.
    #[test]
    fn gateway_models_come_before_configured_providers() {
        let providers = vec![provider("aaa-first-alphabetically", &["m"])];
        let gateway = vec!["kilo/kilo-auto/free".to_string()];
        let options = model_options(&providers, &gateway, "");
        assert_eq!(
            options,
            vec!["kilo/kilo-auto/free", "aaa-first-alphabetically/m"]
        );
    }

    /// The gateway segment's internal order is the caller's (preferred first), never disturbed by lexicographic order.
    #[test]
    fn the_gateway_segment_keeps_its_given_order() {
        let gateway = vec![
            "kilo/kilo-auto/free".to_string(),
            "kilo/~anthropic/claude-sonnet-latest".to_string(),
            "kilo/z-ai/glm-5.2:free".to_string(),
        ];
        let options = model_options(&[], &gateway, "");
        assert_eq!(options, gateway, "网关段必须原样保留给定顺序");
    }

    /// The user-configured segment keeps the user's own config order (provider order → model order).
    #[test]
    fn the_provider_segment_keeps_configuration_order() {
        let providers = vec![provider("zeta", &["m2", "m1"]), provider("alpha", &["m"])];
        let options = model_options(&providers, &[], "");
        assert_eq!(options, vec!["zeta/m2", "zeta/m1", "alpha/m"]);
    }

    /// The key behavior: a changed remote list must never lose the configured current value.
    #[test]
    fn keeps_current_value_even_when_absent_from_the_list() {
        let gateway = vec!["opencode/big-pickle".to_string()];
        let options = model_options(&[], &gateway, "opencode/mimo-v2.5-free");
        assert_eq!(
            options,
            vec!["opencode/big-pickle", "opencode/mimo-v2.5-free"],
            "当前值被下架后仍须留在候选里，且排在最后"
        );
    }

    /// Pages without a dynamic list (mimocode) keep the current value all the same.
    #[test]
    fn keeps_current_value_without_any_gateway_list() {
        let options = model_options(&[], &[], "xiaomi/mimo-v2.5-pro");
        assert_eq!(options, vec!["xiaomi/mimo-v2.5-pro"]);
    }

    #[test]
    fn does_not_duplicate_current_value_already_present() {
        let gateway = vec!["opencode/big-pickle".to_string()];
        let options = model_options(&[], &gateway, "opencode/big-pickle");
        assert_eq!(options, vec!["opencode/big-pickle"]);
    }

    /// A user-configured model sharing a gateway model's name keeps one copy (dedup still applies).
    #[test]
    fn duplicates_between_the_two_segments_are_removed() {
        let providers = vec![provider("kilo", &["kilo-auto/free"])];
        let gateway = vec!["kilo/kilo-auto/free".to_string()];
        let options = model_options(&providers, &gateway, "");
        assert_eq!(options, vec!["kilo/kilo-auto/free"]);
    }

    #[test]
    fn empty_current_value_adds_nothing() {
        assert!(model_options(&[], &[], "").is_empty());
        // A whitespace-only current value doesn't take a slot either
        assert!(model_options(&[], &[], "   ").is_empty());
    }

    // ---- Model normalization on page switches / saves ----

    /// **The core requirement**: a model pointing at another page's gateway becomes the target page's own preferred model.
    #[test]
    fn a_foreign_gateway_model_is_replaced_with_the_pages_own() {
        let agents = vec![agent("fallback", "opencode/ling-3.0-flash-fin-free")];
        let (out, replaced) = agents_for_page(&agents, ConfigFormat::Kilocode, &[], &[]);
        assert_eq!(replaced, 1);
        assert_eq!(out[0].model, "kilo/kilo-auto/free");
    }

    /// Own-gateway and user-configured references are never touched.
    #[test]
    fn valid_references_are_left_alone() {
        let agents = vec![
            agent("own", "kilo/kilo-auto/free"),
            agent("configured", "sensenova/deepseek-v4-flash"),
        ];
        let keys = vec!["sensenova".to_string()];
        let (out, replaced) = agents_for_page(&agents, ConfigFormat::Kilocode, &keys, &[]);
        assert_eq!(replaced, 0);
        assert_eq!(out[0].model, "kilo/kilo-auto/free");
        assert_eq!(out[1].model, "sensenova/deepseek-v4-flash");
    }

    /// An empty model means "not chosen yet", not "chosen wrong" — no default may be filled in behind the user's back.
    #[test]
    fn an_empty_model_is_not_replaced() {
        let agents = vec![agent("blank", ""), agent("spaces", "   ")];
        let (out, replaced) = agents_for_page(&agents, ConfigFormat::Mimocode, &[], &[]);
        assert_eq!(replaced, 0);
        assert!(out[0].model.is_empty());
        assert_eq!(out[1].model, "   ");
    }

    /// The same agents data normalizes to different values per page.
    #[test]
    fn the_same_agents_normalize_differently_per_page() {
        let agents = vec![agent("a", "opencode/ling-3.0-flash-fin-free")];
        let kilo = agents_for_page(&agents, ConfigFormat::Kilocode, &[], &[]).0;
        let mimo = agents_for_page(&agents, ConfigFormat::Mimocode, &[], &[]).0;
        let oc = agents_for_page(&agents, ConfigFormat::Opencode, &[], &[]).0;
        assert_eq!(kilo[0].model, "kilo/kilo-auto/free");
        assert_eq!(mimo[0].model, "mimo/mimo-auto");
        // The opencode page: already its own gateway — untouched as-is
        assert_eq!(oc[0].model, "opencode/ling-3.0-flash-fin-free");
    }

    /// Once the dynamic list arrives, its first entry is used (not the hard-coded fallback).
    #[test]
    fn the_live_list_decides_the_replacement_target() {
        // Only references from another gateway get replaced.
        let agents = vec![agent("a", "kilo/kilo-auto/free")];
        let live = vec!["big-pickle".to_string(), "zzz-free".to_string()];
        let (out, replaced) = agents_for_page(&agents, ConfigFormat::Opencode, &[], &live);
        assert_eq!(replaced, 1);
        // big-pickle is the preferred model, promoted to first
        assert_eq!(out[0].model, "opencode/big-pickle");
    }

    /// Non-opencode-family pages have no agent concept — never touched.
    #[test]
    fn non_opencode_pages_are_untouched() {
        let agents = vec![agent("a", "opencode/x")];
        let (out, replaced) = agents_for_page(&agents, ConfigFormat::WorkBuddy, &[], &[]);
        assert_eq!(replaced, 0);
        assert_eq!(out[0].model, "opencode/x");
    }

    /// A reference without a slash is unparseable on every page; it becomes the own-gateway model.
    #[test]
    fn malformed_references_are_replaced() {
        let agents = vec![agent("a", "no-slash")];
        let (out, replaced) = agents_for_page(&agents, ConfigFormat::Kilocode, &[], &[]);
        assert_eq!(replaced, 1);
        assert_eq!(out[0].model, "kilo/kilo-auto/free");
    }
}
