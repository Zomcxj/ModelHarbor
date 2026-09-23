//! Agents 区块：卡片列表、编辑表单与新增表单（仅 opencode 系页面使用）。
use super::fetch::FreeModelsState;
use super::App;
use crate::app::bars::{sticky_begin, sticky_end};
use crate::format::ConfigFormat;
use crate::model::{AgentRow, ProviderRow};
use crate::ui::{card_frame, card_list, field_label, move_item, numeric_text_edit, DragHandle};
use eframe::egui;
use std::collections::{HashMap, HashSet};

/// Agents 的 `model` 下拉候选，按「自家网关 → 自配 provider → 当前值」排序。
///
/// 写成自由函数（而不是 `&self` 方法）是为了能在表单里调用：那里
/// `self.agents[idx]` 已被可变借用，整结构借用会编译不过，只能按字段拆分。
///
/// **顺序就是需求**：用户要的是「自家网关排最前」，所以**不能再整体 `sort()`**——
/// 那会把网关模型和自配 provider 混在一起按字典序排，`kilo-auto/free` 这种
/// 网关里的关键项会被 `kilo/~anthropic/...` 挤到后面。分三段拼接：
///
/// 1. `gateway`：本页网关的模型（首选在最前，见 [`crate::opencode_models::gateway_models`]）；
/// 2. 自配 provider 的模型（按 provider 配置顺序、模型配置顺序，保持用户自己的排列）；
/// 3. 当前值（若前两段都没有它）。
///
/// **当前值一定保留**：远程列表随时会变，若某个已配置的模型被下架，直接从候选里
/// 抹掉会让用户看到「下拉是空的 / 选中项不见了」，误以为配置坏了。保留它并排在
/// 最后，用户能看见自己配的是什么，想换再换。
fn model_options(providers: &[ProviderRow], gateway: &[String], current: &str) -> Vec<String> {
    let mut options: Vec<String> = Vec::new();
    options.extend(gateway.iter().cloned());
    options.extend(
        providers
            .iter()
            .flat_map(|p| p.models.iter().map(|m| format!("{}/{}", p.key, m.id))),
    );
    let current = current.trim();
    if !current.is_empty() && !options.iter().any(|option| option == current) {
        options.push(current.to_string());
    }
    // 只去重、**不排序**：顺序承载「自家网关排最前」的语义。
    let mut seen = HashSet::new();
    options.retain(|option| seen.insert(option.clone()));
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
    backend: &str,
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
                egui::RichText::new(format!("未获取到 {} 免费模型", backend))
                    .small()
                    .weak(),
            );
        } else {
            ui.label(
                egui::RichText::new(format!("{} 免费模型 {} 个", backend, free_count))
                    .small()
                    .weak(),
            );
        }
        clicked = ui
            .button("刷新")
            .on_hover_text(format!(
                "重新拉取 {} 的免费模型列表\n（默认每 {} 小时自动更新一次）",
                backend,
                crate::opencode_models::CACHE_TTL_SECS / 3600
            ))
            .clicked();
    }
    clicked
}

/// 当前页面后端的免费模型 `(引用前缀, 裸 id 列表)`；没有免费层时前缀为 `None`。
///
/// 前缀来自 [`crate::opencode_models::source_for`]，与拉取时用的 provider id 同源，
/// 避免两处各写一份字符串而漂移。
///
/// 只用于**免费层的提示与刷新按钮**（mimocode 没有免费层，不显示这些）。
/// 下拉候选请用 [`current_gateway_options`]——那才是「自家网关模型」的完整口径。
///
/// 写成自由函数（不是 `&self` 方法）：表单里 `self.agents[idx]` 已被可变借用，
/// 只能按字段拆分借用，整结构借用会编译不过。
fn current_free_models(
    page: ConfigFormat,
    free_models: &HashMap<ConfigFormat, FreeModelsState>,
) -> (Option<&'static str>, &[String]) {
    let Some(source) = crate::opencode_models::source_for(page) else {
        return (None, &[]);
    };
    let models = free_models
        .get(&page)
        .map(|state| state.models.as_slice())
        .unwrap_or(&[]);
    (Some(source.provider_id), models)
}

/// 当前页面**自家网关**的模型候选（已带前缀，首选在最前），供下拉排序与替换使用。
///
/// 与 [`current_free_models`] 的区别：这里覆盖**所有** opencode 系页面，包括没有免费层
/// 的 mimocode（它的兜底清单见 [`crate::opencode_models::gateway_models`]）。下拉的
/// 「自家网关排最前」与「切页即替换」都以此为准。
fn current_gateway_options(
    page: ConfigFormat,
    free_models: &HashMap<ConfigFormat, FreeModelsState>,
) -> Vec<String> {
    let free = free_models
        .get(&page)
        .map(|state| state.models.as_slice())
        .unwrap_or(&[]);
    crate::opencode_models::gateway_models(page, free)
}

/// 当前页面后端的免费模型拉取状态：`(是否在飞, 失败原因)`。
fn current_free_status(
    page: ConfigFormat,
    free_models: &HashMap<ConfigFormat, FreeModelsState>,
) -> (bool, Option<&str>) {
    match free_models.get(&page) {
        Some(state) => (state.fetching(), state.error.as_deref()),
        None => (false, None),
    }
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
            let (free_prefix, free_list) =
                current_free_models(self.current_page, &self.free_models);
            let gateway = current_gateway_options(self.current_page, &self.free_models);
            let options = model_options(&self.providers, &gateway, &current);
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
            // 只对确实有免费层的后端显示提示与刷新按钮（mimocode 没有，不占位置）。
            if free_prefix.is_some() {
                let (fetching, error) = current_free_status(self.current_page, &self.free_models);
                let backend = self.current_page.label();
                if model_options_hint(ui, backend, free_list.len(), error, fetching) {
                    refresh_free = true;
                }
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
            self.start_free_models_fetch(self.current_page);
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
                let (free_prefix, free_list) =
                    current_free_models(self.current_page, &self.free_models);
                let gateway = current_gateway_options(self.current_page, &self.free_models);
                let options = model_options(&self.providers, &gateway, &current);
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
                // 只对确实有免费层的后端显示提示与刷新按钮（mimocode 没有，不占位置）。
                if free_prefix.is_some() {
                    let (fetching, error) =
                        current_free_status(self.current_page, &self.free_models);
                    let backend = self.current_page.label();
                    if model_options_hint(ui, backend, free_list.len(), error, fetching) {
                        self.start_free_models_fetch(self.current_page);
                    }
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

    /// 某个 opencode 系页面的 agent model 视图：该页上用户真正配过的那份值。
    ///
    /// - 目标页就是**当前页**：以 `agents` 为准（用户可能正在编辑），记忆已过期。
    /// - 别的页面：以离开该页时存下的记忆覆盖对应 key，其余沿用当前值。
    ///
    /// 这样「一键保存」写到各页的才是**用户在该页配过的值**，而不是当前页那份被
    /// 归一过的值（否则用户在 kilo 页选的 `kilo-auto/balanced` 会被写成默认的
    /// `kilo-auto/free`，因为当前页看到的是 opencode 的引用）。
    fn page_agent_view(&self, page: ConfigFormat) -> Vec<AgentRow> {
        let mut view = self.agents.clone();
        if page == self.current_page {
            return view;
        }
        self.overlay_remembered_models(page, &mut view);
        view
    }

    /// 用某页的记忆覆盖 `view` 里对应 agent 的 model（key 已被删的条目自然跳过）。
    fn overlay_remembered_models(&self, page: ConfigFormat, view: &mut [AgentRow]) {
        let Some(saved) = self.agent_models_by_page.get(&page) else {
            return;
        };
        for agent in view.iter_mut() {
            if let Some(model) = saved.get(agent.key.trim()) {
                agent.model = model.clone();
            }
        }
    }

    /// 切页时的 agent model 处理：先还原该页自己的视图，再替换仍然无效的引用。
    ///
    /// ## 为什么需要「记忆」
    ///
    /// 三页共用同一份 `agents`，但每页网关不同，`model` 的前缀必须换成本页认的。
    /// 只在切页时单向替换是不够的——用户从 opencode 页切到 kilo 页（`opencode/…`
    /// 被换成 `kilo/kilo-auto/free`），再切回 opencode 页，**原来的 `opencode/…`
    /// 已经没了**，用户什么也没改却丢了配置。
    ///
    /// 所以离开一页时把该页的 model 视图存进 [`Self::agent_models_by_page`]，
    /// 回到该页先还原再归一：用户在各页配的、各自有效的那份值都留得住。
    ///
    /// `leaving` 是**刚刚离开**的页面（没有则传 `None`），它的当前 `agents` 值需要
    /// 先记下来——那是用户在该页真正编辑过的内容。
    ///
    /// 还原时**不看 `current_page`**：调用方通常已经把 `current_page` 设成了新页
    /// （见 `ui_top_bar`），若沿用 [`Self::page_agent_view`] 的当前页判定，记忆会被
    /// 当成过期而跳过，还原就失效了。
    ///
    /// 返回被替换的条数（供调用方在状态栏提示），0 表示无需改动。
    pub(super) fn normalize_agent_models_for_page(
        &mut self,
        page: ConfigFormat,
        leaving: Option<ConfigFormat>,
    ) -> usize {
        if let Some(prev) = leaving {
            self.remember_agent_models(prev);
        }
        if !page.is_opencode_family() {
            return 0;
        }
        // 还原：把该页上次离开时的视图取回来（key 已被删的条目自然跳过）。
        let mut restored = self.agents.clone();
        self.overlay_remembered_models(page, &mut restored);
        let keys: Vec<String> = self.providers.iter().map(|p| p.key.clone()).collect();
        let free = self.page_gateway_free_models(page);
        let (agents, replaced) = agents_for_page(&restored, page, &keys, &free);
        // 无论有没有发生替换都要写回：还原本身就是要让界面显示该页自己的值。
        self.agents = agents;
        replaced
    }

    /// 记下某页当前的 agent model 视图（页面 → agent key → model）。
    ///
    /// 用 `key.trim()` 作键：与保存时的重复判定同一口径，改名前后不会错位。
    fn remember_agent_models(&mut self, page: ConfigFormat) {
        if !page.is_opencode_family() {
            return;
        }
        let view: HashMap<String, String> = self
            .agents
            .iter()
            .map(|a| (a.key.trim().to_string(), a.model.clone()))
            .collect();
        self.agent_models_by_page.insert(page, view);
    }

    /// 本页动态拉到的网关免费模型裸 id；没有免费层（mimocode）时为空。
    fn page_gateway_free_models(&self, page: ConfigFormat) -> Vec<String> {
        self.free_models
            .get(&page)
            .map(|state| state.models.clone())
            .unwrap_or_default()
    }

    /// 写入某一页时要落盘的 agents：先取该页自己的视图，再替换其中无效的引用。
    ///
    /// **保存路径也必须按目标页归一**，不能只靠切页时改内存值。三页共用同一份
    /// agents 数据，而「一键保存」会把这同一份数据分别写进三个文件——若不按目标页
    /// 分别归一，最后访问过的那一页的模型会被写进所有文件，其余文件里就是无效引用
    /// （kilo 网关不认 `opencode/…`）。这里逐页归一，每个文件都拿到自己网关认的值。
    pub(super) fn agents_for_page(&self, page: ConfigFormat) -> Vec<AgentRow> {
        let keys: Vec<String> = self.providers.iter().map(|p| p.key.clone()).collect();
        let free = self.page_gateway_free_models(page);
        agents_for_page(&self.page_agent_view(page), page, &keys, &free).0
    }
}

/// 把 `agents` 里指向别家网关的 `model` 换成 `page` 自家网关的首选模型。
///
/// 返回 `(归一后的列表, 替换条数)`；无需改动时原样克隆返回、条数为 0。
///
/// ## 为什么需要
///
/// opencode 系三页共用同一份 agent 数据，但每页的**网关不同**：`model` 是
/// `provider/model`，前半段必须是本页网关认的 provider id。把 opencode 页配好的
/// `opencode/ling-3.0-flash-fin-free` 带到 kilo 页，kilo 网关不认这个前缀，agent
/// 直接跑不起来——而且界面看不出问题（下拉里就显示着那串字）。
///
/// ## 什么算「无效」
///
/// 判据见 [`crate::opencode_models::model_is_valid_on`]：前缀要么是本页网关，要么是
/// 用户自己配的 provider key（保存时 provider 容器一并写入，任何一页都有效）。
/// 只有**别家网关**的前缀才需要替换。
///
/// 空 model 不动：那是「还没选」，不是「选错了」，替换成默认值反而像擅自替用户做了决定。
/// 拿不到任何自家模型时也一律不动：宁可不替换，也不能把用户的配置清成空串。
fn agents_for_page(
    agents: &[AgentRow],
    page: ConfigFormat,
    configured_keys: &[String],
    free: &[String],
) -> (Vec<AgentRow>, usize) {
    let mut out = agents.to_vec();
    if !page.is_opencode_family() {
        return (out, 0);
    }
    let invalid: Vec<usize> = out
        .iter()
        .enumerate()
        .filter(|(_, a)| {
            let model = a.model.trim();
            !model.is_empty()
                && !crate::opencode_models::model_is_valid_on(page, configured_keys, model)
        })
        .map(|(idx, _)| idx)
        .collect();
    if invalid.is_empty() {
        return (out, 0);
    }
    let Some(replacement) = crate::opencode_models::default_gateway_model(page, free) else {
        return (out, 0);
    };
    for idx in &invalid {
        out[*idx].model = replacement.clone();
    }
    let replaced = invalid.len();
    (out, replaced)
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
