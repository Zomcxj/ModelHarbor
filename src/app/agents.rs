//! Agents 区块：卡片列表、编辑表单与新增表单（仅 opencode 系页面使用）。
use super::App;
use crate::app::bars::{sticky_begin, sticky_end};
use crate::model::{AgentRow, ProviderRow};
use crate::ui::{card_frame, card_list, field_label, move_item, numeric_text_edit, DragHandle};
use eframe::egui;
use std::collections::HashSet;

/// Agents 的 `model` 下拉候选：已配置 provider 的模型 + opencode 内置 Zen 网关的
/// 免费模型（裸 id 加 `opencode/` 前缀）+ 当前值。
///
/// 写成自由函数（而不是 `&self` 方法）是为了能在表单里调用：那里
/// `self.agents[idx]` 已被可变借用，整结构借用会编译不过，只能按字段拆分。
///
/// **当前值一定保留**：远程列表随时会变，若某个已配置的模型被下架，直接从候选里
/// 抹掉会让用户看到「下拉是空的 / 选中项不见了」，误以为配置坏了。保留它并排在
/// 原位，用户能看见自己配的是什么，想换再换。
fn model_options(
    providers: &[ProviderRow],
    opencode_free: &[String],
    current: &str,
) -> Vec<String> {
    let mut options: Vec<String> = providers
        .iter()
        .flat_map(|p| p.models.iter().map(|m| format!("{}/{}", p.key, m.id)))
        .collect();
    options.extend(opencode_free.iter().map(|id| format!("opencode/{}", id)));
    let current = current.trim();
    if !current.is_empty() && !options.iter().any(|option| option == current) {
        options.push(current.to_string());
    }
    options.sort();
    options.dedup();
    options
}

/// 模型下拉右侧的免费模型状态提示与「刷新」按钮。
///
/// 返回 `true` 表示用户点了刷新；调用方在表单渲染结束后再发起后台拉取
/// （这里不能直接调 `&mut self` 方法：此时 `self.agents[idx]` 正被可变借用）。
///
/// 写成自由函数还有一个好处：拉取中 / 失败 / 空列表三种状态各有文案，
/// 集中在视觉上贴着下拉的位置，用户不必去状态栏找原因。
fn model_options_hint(
    ui: &mut egui::Ui,
    free_count: usize,
    error: Option<&str>,
    fetching: bool,
) -> bool {
    let mut clicked = false;
    if fetching {
        ui.add(egui::Spinner::new().size(14.0));
        ui.label(egui::RichText::new("正在获取免费模型…").small().weak());
    } else {
        if let Some(err) = error {
            ui.label(
                egui::RichText::new("免费模型获取失败")
                    .small()
                    .color(crate::theme::semantics(ui).err),
            )
            .on_hover_text(err);
        } else if free_count == 0 {
            ui.label(
                egui::RichText::new("未获取到 opencode 免费模型")
                    .small()
                    .weak(),
            );
        } else {
            ui.label(
                egui::RichText::new(format!("opencode 免费模型 {} 个", free_count))
                    .small()
                    .weak(),
            );
        }
        clicked = ui
            .button("刷新")
            .on_hover_text(format!(
                "重新从 {} 拉取 opencode 免费模型列表\n（默认每 {} 小时自动更新一次）",
                crate::opencode_models::SOURCE_URL,
                crate::opencode_models::CACHE_TTL_SECS / 3600
            ))
            .clicked();
    }
    clicked
}

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
        // 表单里点了「刷新免费模型」：记下来，等 `a` 的可变借用结束后再发起后台拉取。
        let mut refresh_free = false;
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
            let options = model_options(&self.providers, &self.opencode_free, &current);
            let mut selected_idx = options.iter().position(|m| m == &current);
            egui::ComboBox::from_id_salt(format!("agent_model_{}", a.key))
                .selected_text(if current.is_empty() {
                    "选择模型..."
                } else {
                    &current
                })
                .width(180.0)
                .show_ui(ui, |ui| {
                    for (i, model) in options.iter().enumerate() {
                        let is_selected = selected_idx == Some(i);
                        if ui.selectable_label(is_selected, model.as_str()).clicked() {
                            selected_idx = Some(i);
                        }
                    }
                });
            if let Some(idx) = selected_idx {
                a.model = options[idx].clone();
            }
            let free_count = self.opencode_free.len();
            let fetching = self.opencode_free_rx.is_some();
            if model_options_hint(
                ui,
                free_count,
                self.opencode_free_error.as_deref(),
                fetching,
            ) {
                refresh_free = true;
            }
            field_label(ui, 120.0, "variant");
            let variant_options = ["", "low", "medium", "high", "xhigh", "max", "ultra"];
            let current_variant = a.variant.clone();
            let mut selected_variant = variant_options
                .iter()
                .position(|v| *v == current_variant.as_str());
            egui::ComboBox::from_id_salt(format!("agent_variant_{}", a.key))
                .selected_text(if current_variant.is_empty() {
                    "选择..."
                } else {
                    &current_variant
                })
                .width(100.0)
                .show_ui(ui, |ui| {
                    for (i, v) in variant_options.iter().enumerate() {
                        let label = if v.is_empty() { "(空)" } else { v };
                        let is_selected = selected_variant == Some(i);
                        if ui.selectable_label(is_selected, label).clicked() {
                            selected_variant = Some(i);
                        }
                    }
                });
            if let Some(idx) = selected_variant {
                a.variant = variant_options[idx].to_string();
            }
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
        if refresh_free {
            self.start_opencode_free_fetch();
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
                let options = model_options(&self.providers, &self.opencode_free, &current);
                let mut selected_idx = options.iter().position(|m| m == &current);
                let _response = egui::ComboBox::from_id_salt("new_agent_model")
                    .selected_text(if current.is_empty() {
                        "选择模型..."
                    } else {
                        &current
                    })
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        for (i, model) in options.iter().enumerate() {
                            let is_selected = selected_idx == Some(i);
                            if ui.selectable_label(is_selected, model.as_str()).clicked() {
                                selected_idx = Some(i);
                            }
                        }
                    });
                if let Some(idx) = selected_idx {
                    self.new_agent.model = options[idx].clone();
                }
                let free_count = self.opencode_free.len();
                let fetching = self.opencode_free_rx.is_some();
                if model_options_hint(
                    ui,
                    free_count,
                    self.opencode_free_error.as_deref(),
                    fetching,
                ) {
                    self.start_opencode_free_fetch();
                }
                field_label(ui, 120.0, "variant");
                let variant_options = ["", "low", "medium", "high", "xhigh", "max", "ultra"];
                let current_variant = self.new_agent.variant.clone();
                let mut selected_variant = variant_options
                    .iter()
                    .position(|v| *v == current_variant.as_str());
                egui::ComboBox::from_id_salt("new_agent_variant")
                    .selected_text(if current_variant.is_empty() {
                        "选择..."
                    } else {
                        &current_variant
                    })
                    .width(100.0)
                    .show_ui(ui, |ui| {
                        for (i, v) in variant_options.iter().enumerate() {
                            let label = if v.is_empty() { "(空)" } else { v };
                            let is_selected = selected_variant == Some(i);
                            if ui.selectable_label(is_selected, label).clicked() {
                                selected_variant = Some(i);
                            }
                        }
                    });
                if let Some(idx) = selected_variant {
                    self.new_agent.variant = variant_options[idx].to_string();
                }
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
    use super::model_options;
    use crate::model::{ModelRow, ProviderRow};

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

    #[test]
    fn includes_provider_models_with_key_prefix() {
        let providers = vec![provider("sensenova", &["deepseek-v4-flash"])];
        let options = model_options(&providers, &[], "");
        assert_eq!(options, vec!["sensenova/deepseek-v4-flash"]);
    }

    #[test]
    fn includes_free_models_with_opencode_prefix() {
        let free = vec!["big-pickle".to_string(), "some-free".to_string()];
        let options = model_options(&[], &free, "");
        assert_eq!(options, vec!["opencode/big-pickle", "opencode/some-free"]);
    }

    /// 关键行为：远程列表变了也不能把已配置的当前值弄丢，否则用户会以为配置坏了。
    #[test]
    fn keeps_current_value_even_when_absent_from_remote_list() {
        let free = vec!["big-pickle".to_string()];
        let options = model_options(&[], &free, "opencode/mimo-v2.5-free");
        assert!(
            options.contains(&"opencode/mimo-v2.5-free".to_string()),
            "当前值被下架后仍须留在候选里：{:?}",
            options
        );
    }

    #[test]
    fn does_not_duplicate_current_value_already_present() {
        let free = vec!["big-pickle".to_string()];
        let options = model_options(&[], &free, "opencode/big-pickle");
        assert_eq!(options, vec!["opencode/big-pickle"]);
    }

    #[test]
    fn empty_current_value_adds_nothing() {
        let options = model_options(&[], &[], "");
        assert!(options.is_empty());
        // 只有空白的当前值同样不占位
        assert!(model_options(&[], &[], "   ").is_empty());
    }

    #[test]
    fn output_is_sorted_and_deduplicated() {
        let providers = vec![provider("a", &["m2", "m1"]), provider("b", &["m1"])];
        let free = vec!["big-pickle".to_string()];
        let options = model_options(&providers, &free, "");
        let mut expected = options.clone();
        expected.sort();
        expected.dedup();
        assert_eq!(options, expected);
    }
}
