//! Agents 区块：卡片列表、编辑表单与新增表单（仅 opencode 系页面使用）。

mod combo;
mod view;

// 扁平门面（仅测试）：让 `use super::*` 的单测直接看到跨子模块项。
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
    /// Agents 区块：标题行吸顶（滚动时始终显示在顶部），内容紧跟其下。
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
                // agents 只属于 opencode 页：来源不是 opencode 时界面没有 agents 数据，
                // 跨格式保存不会接管目标文件的 agent 容器（避免静默清空）。
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
                    if ui
                        .push_id("agents_toggle_all", |ui| {
                            ui.button(if all_open {
                                "收起全部卡片"
                            } else {
                                "展开全部卡片"
                            })
                        })
                        .inner
                        .clicked()
                    {
                        // all_open 为真 = 现在全部展开 → 按钮是「收起全部」
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
                // 免费模型的状态与「刷新」**统一放在标题行**（展开按钮右侧），
                // 不再每个 agent 的 model 下拉旁各放一份：刷新的是本页后端的内置
                // 免费模型清单，与具体哪个 agent 无关。只对有免费层的后端显示
                // （mimocode 没有，不占位置）。
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
            ui.horizontal(|ui| {
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
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("删除").clicked() {
                        *to_remove = Some(idx);
                    }
                    if ui.button("复制").clicked() {
                        *to_copy = Some(idx);
                    }
                });
            });
            // 折叠 / 展开带高度动画；动画 id 按 key 派生，改名即换 id（状态不串卡）。
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
            let key_resp = ui.add(egui::TextEdit::singleline(&mut a.key).desired_width(120.0));
            if !a.key.trim().is_empty() && other_keys.contains(a.key.trim()) {
                key_resp.on_hover_text("key 与其他 agent 重复，保存将被阻止");
                ui.label(
                    egui::RichText::new("⚠ 重复")
                        .small()
                        .color(crate::theme::semantics(ui).err),
                );
            }
            field_label(ui, 120.0, "mode");
            ui.add(egui::TextEdit::singleline(&mut a.mode).desired_width(120.0));
            field_label(ui, 120.0, "description");
            ui.add(egui::TextEdit::singleline(&mut a.description).desired_width(450.0));
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
            field_label(ui, 120.0, "temperature");
            numeric_text_edit(ui, &mut a.temperature, 120.0, "");
            field_label(ui, 120.0, "color");
            ui.add(egui::TextEdit::singleline(&mut a.color).desired_width(120.0));
            field_label(ui, 120.0, "system");
            ui.add(egui::TextEdit::singleline(&mut a.system).desired_width(450.0));
        });
        // key 重命名后同步展开状态（避免改名导致卡片收起）
        let new_key = self.agents[idx].key.clone();
        if new_key != prev_key {
            self.sync_agent_rename(&prev_key, &new_key);
        }
    }

    pub(super) fn ui_new_agent_form(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.horizontal_wrapped(|ui| {
                field_label(ui, 120.0, "key");
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_agent.key)
                        .hint_text("coding-assistant")
                        .desired_width(120.0),
                );
                field_label(ui, 120.0, "mode");
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_agent.mode)
                        .hint_text("subagent")
                        .desired_width(120.0),
                );
                field_label(ui, 120.0, "description");
                ui.add(
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
                field_label(ui, 120.0, "temperature");
                numeric_text_edit(ui, &mut self.new_agent.temperature, 120.0, "0.7");
                field_label(ui, 120.0, "color");
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_agent.color)
                        .hint_text("#00ccff")
                        .desired_width(120.0),
                );
                field_label(ui, 120.0, "system");
                ui.add(
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

    /// agent key 重命名后同步 UI 状态（卡片折叠集合），避免改名后卡片收起。
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

    /// 造一个带若干模型的 provider。
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

    /// 造一个带 model 的 agent。
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

    /// **核心需求**：自家网关的模型排在最前，自配 provider 的排在其后。
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

    /// 网关段内部的顺序由调用方给定（首选在最前），**不能再被字典序打乱**。
    /// 旧实现整体 `sort()`，`kilo/kilo-auto/free` 会被 `kilo/~anthropic/…` 挤到后面。
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

    /// 自配 provider 段保持用户自己的配置顺序（provider 顺序 → 模型顺序）。
    #[test]
    fn the_provider_segment_keeps_configuration_order() {
        let providers = vec![provider("zeta", &["m2", "m1"]), provider("alpha", &["m"])];
        let options = model_options(&providers, &[], "");
        assert_eq!(options, vec!["zeta/m2", "zeta/m1", "alpha/m"]);
    }

    /// 关键行为：远程列表变了也不能把已配置的当前值弄丢，否则用户会以为配置坏了。
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

    /// 没有动态列表的页面（mimocode）同样要保留当前值。
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

    /// 自配 provider 的模型与网关模型同名时只留一份（去重仍然生效）。
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
        // 只有空白的当前值同样不占位
        assert!(model_options(&[], &[], "   ").is_empty());
    }

    // ---- 切页 / 保存时的 model 归一 ----

    /// **核心需求**：指向别家网关的 model 换成目标页自家网关的首选模型。
    #[test]
    fn a_foreign_gateway_model_is_replaced_with_the_pages_own() {
        let agents = vec![agent("fallback", "opencode/ling-3.0-flash-fin-free")];
        let (out, replaced) = agents_for_page(&agents, ConfigFormat::Kilocode, &[], &[]);
        assert_eq!(replaced, 1);
        assert_eq!(out[0].model, "kilo/kilo-auto/free");
    }

    /// 自家网关与自配 provider 的引用都不动。
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

    /// 空 model 是「还没选」，不是「选错了」——不能擅自填默认值。
    #[test]
    fn an_empty_model_is_not_replaced() {
        let agents = vec![agent("blank", ""), agent("spaces", "   ")];
        let (out, replaced) = agents_for_page(&agents, ConfigFormat::Mimocode, &[], &[]);
        assert_eq!(replaced, 0);
        assert!(out[0].model.is_empty());
        assert_eq!(out[1].model, "   ");
    }

    /// 同一份 agents 数据在三个页面上各自归一成**不同**的值——这是「一键保存
    /// 不会把同一串引用写进所有文件」的保证。
    #[test]
    fn the_same_agents_normalize_differently_per_page() {
        let agents = vec![agent("a", "opencode/ling-3.0-flash-fin-free")];
        let kilo = agents_for_page(&agents, ConfigFormat::Kilocode, &[], &[]).0;
        let mimo = agents_for_page(&agents, ConfigFormat::Mimocode, &[], &[]).0;
        let oc = agents_for_page(&agents, ConfigFormat::Opencode, &[], &[]).0;
        assert_eq!(kilo[0].model, "kilo/kilo-auto/free");
        assert_eq!(mimo[0].model, "mimo/mimo-auto");
        // opencode 页：本来就是自家网关，原样不动
        assert_eq!(oc[0].model, "opencode/ling-3.0-flash-fin-free");
    }

    /// 动态列表拿到后用它的第一个（而不是写死的兜底）。
    #[test]
    fn the_live_list_decides_the_replacement_target() {
        // 引用必须来自**别家**网关才会被替换，否则测不出替换目标
        let agents = vec![agent("a", "kilo/kilo-auto/free")];
        let live = vec!["big-pickle".to_string(), "zzz-free".to_string()];
        let (out, replaced) = agents_for_page(&agents, ConfigFormat::Opencode, &[], &live);
        assert_eq!(replaced, 1);
        // big-pickle 是首选，被提到第一位
        assert_eq!(out[0].model, "opencode/big-pickle");
    }

    /// 非 opencode 系页面没有 agent 概念，一律不动。
    #[test]
    fn non_opencode_pages_are_untouched() {
        let agents = vec![agent("a", "opencode/x")];
        let (out, replaced) = agents_for_page(&agents, ConfigFormat::WorkBuddy, &[], &[]);
        assert_eq!(replaced, 0);
        assert_eq!(out[0].model, "opencode/x");
    }

    /// 没斜杠的引用在任何页都解析不了，换成自家网关模型是修正。
    #[test]
    fn malformed_references_are_replaced() {
        let agents = vec![agent("a", "no-slash")];
        let (out, replaced) = agents_for_page(&agents, ConfigFormat::Kilocode, &[], &[]);
        assert_eq!(replaced, 1);
        assert_eq!(out[0].model, "kilo/kilo-auto/free");
    }
}
