//! Provider 编辑 / 新增表单、模型获取弹层与表单字段控件。
use super::App;
use crate::app::fetch::{
    cancel_discovery, discovery_panel, model_latency_label, model_probe_button, net_guard_gate,
    probe_candidates_label, start_discovery, DiscoveryState, LatencyState, NEW_PROVIDER_FETCH_KEY,
};
use crate::app::providers::ProviderFormFlags;
use crate::convert;
use crate::credentials;
use crate::format::ConfigFormat;
use crate::model::{ModelRow, ProviderRow};
use crate::ui::{
    card_frame, field_label, merge_drag_target, move_item, numeric_text_edit, secret_text_edit,
    DragHandle,
};
use eframe::egui;
use model_harbor_core::discovery::DiscoveryCache;
use std::collections::{HashMap, HashSet};

/// 「获取模型」面板的分栏参数：列间水平间距、单列最小宽度、最大列数。
mod combos;
mod model_row;
mod presets;
mod sections;
use combos::*;
use model_row::*;
use presets::*;
use sections::*;

mod popup;
mod render;
#[cfg(test)]
mod tests;
pub(super) use popup::*;

impl App {
    pub(super) fn ui_new_provider_form(&mut self, ui: &mut egui::Ui) {
        let (variants_label, variant_names) = self.dialect_variants();
        let flags = ProviderFormFlags::new(self);
        ui.group(|ui| {
            // 官方预设（可选）：一键填 key / baseUrl / 协议，不套用则手填。
            ui.horizontal_wrapped(|ui| {
                let dialect = if flags.show_oc {
                    crate::presets::PresetDialect::Opencode
                } else {
                    crate::presets::PresetDialect::PiLike
                };
                provider_preset_combo(ui, &mut self.new_provider, dialect, "new_provider_preset");
            });
            let header_ctx = ProviderHeaderCtx {
                page: self.current_page,
                npm_salt: "new_provider_npm".to_string(),
                api_salt: "new_provider_api".to_string(),
                relaxed: true,
                npm_empty_label: false,
                show_api_keys: self.show_api_keys,
                other_keys: None,
            };
            provider_header_fields(ui, &mut self.new_provider, &flags, &header_ctx);
            ui.add_space(crate::theme::SPACE_2);
            let mut fetch_ctx = FetchSectionCtx {
                latency: &self.latency,
                current_page: self.current_page,
                fetch_key: NEW_PROVIDER_FETCH_KEY,
                discovery: &mut self.discovery,
                discovery_cache: &mut self.discovery_cache,
            };
            let fetch_secret = credentials::effective_secret(&self.new_provider);
            if let Some(msg) = models_fetch_section(
                ui,
                &mut fetch_ctx,
                &self.new_provider.base_url,
                &fetch_secret,
                &mut self.new_provider.models,
            ) {
                self.status = msg;
            }
            let mut rm_new: Option<usize> = None;
            let mut move_new_request: Option<(usize, usize)> = None;
            // 本帧用户点下的探测请求。
            let mut probe_request: Option<(String, String)> = None;
            for j in 0..self.new_provider.models.len() {
                let model_count = self.new_provider.models.len();
                card_frame(
                    ui,
                    true,
                    0,
                    egui::Id::new(("new_provider_model", j)),
                    |ui| {
                        ui.horizontal(|ui| {
                            if j > 0 && ui.button("↑").clicked() {
                                move_new_request = Some((j, j - 1));
                            }
                            if j + 1 < model_count && ui.button("↓").clicked() {
                                move_new_request = Some((j, j + 1));
                            }
                            // 单模型延迟测试：按钮在调整按钮右侧，结果显示在按钮右侧。
                            let model_id = self.new_provider.models[j].id.trim().to_string();
                            let gate =
                                net_guard_gate(&self.net_guard, self.allow_model_test_with_proxy);
                            if model_probe_button(
                                ui,
                                &self.probe,
                                NEW_PROVIDER_FETCH_KEY,
                                ui.input(|i| i.time),
                                gate.as_deref(),
                            ) {
                                probe_request =
                                    Some((NEW_PROVIDER_FETCH_KEY.to_string(), model_id.clone()));
                            }
                            let latency = self.latency.get(NEW_PROVIDER_FETCH_KEY);
                            model_latency_label(ui, latency, &model_id);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("删").clicked() {
                                        rm_new = Some(j);
                                    }
                                },
                            );
                        });
                        model_core_fields_row(
                            ui,
                            &mut self.new_provider.models[j],
                            &flags,
                            true,
                            None,
                            &format!("new_provider#{}", j),
                        );
                    },
                );
            }
            if let Some((provider_key, model_id)) = probe_request {
                let now = ui.input(|i| i.time);
                let base_url = self.new_provider.base_url.clone();
                let secret = credentials::effective_secret(&self.new_provider);
                let api = self.new_provider.effective_api();
                let gate = net_guard_gate(&self.net_guard, self.allow_model_test_with_proxy);
                self.status = Self::run_model_probe(
                    &mut self.probe,
                    &mut self.latency,
                    gate.as_deref(),
                    &provider_key,
                    &model_id,
                    now,
                    &base_url,
                    &secret,
                    &api,
                );
            }
            if let Some((from, to)) = move_new_request {
                move_item(&mut self.new_provider.models, from, to);
            }
            if let Some(j) = rm_new {
                self.new_provider.models.remove(j);
            }
            ui.add_space(crate::theme::SPACE_2);
            let show_new_model_key = format!("new_provider_show_model_{}", self.new_provider.key);
            let show_new_model = self.variant_open.contains(&show_new_model_key);
            if ui
                .button(if show_new_model {
                    "收起"
                } else {
                    "添加 Model"
                })
                .clicked()
            {
                if show_new_model {
                    self.variant_open.remove(&show_new_model_key);
                } else {
                    self.variant_open.insert(show_new_model_key.clone());
                }
            }
            if show_new_model {
                let mut variants = VariantsCtx {
                    label: variants_label,
                    names: variant_names,
                    open_set: &mut self.variant_open,
                    open_key: "new_provider_new_model_variant".to_string(),
                    normalize: false,
                };
                new_model_subform(
                    ui,
                    &mut self.new_provider,
                    &flags,
                    &mut variants,
                    &show_new_model_key,
                );
            }
            ui.horizontal(|ui| {
                ui.add_space(crate::theme::SPACE_7);
                if ui.button("确认").clicked() {
                    let key = self.new_provider.key.trim().to_string();
                    if key.is_empty() {
                        self.status = "请填写 provider key".into();
                    } else if self.providers.iter().any(|p| p.key.trim() == key) {
                        self.status = format!("provider key \"{}\" 已存在", key);
                    } else {
                        let np = self.new_provider.clone();
                        self.providers.push(np);
                        self.new_provider = ProviderRow::new();
                        self.show_new_provider = false;
                        self.clear_new_provider_state();
                        self.status = "已添加 provider".into();
                    }
                }
                if ui.button("取消").clicked() {
                    self.new_provider = ProviderRow::new();
                    self.show_new_provider = false;
                    self.clear_new_provider_state();
                }
            });
        });
    }

    /// 关闭新增 provider 表单时清理其测试/获取状态。
    pub(super) fn clear_new_provider_state(&mut self) {
        self.latency.remove(NEW_PROVIDER_FETCH_KEY);
        // 探测状态一并清掉（在飞线程的回包因接收端被丢而作废）。
        self.discovery.remove(NEW_PROVIDER_FETCH_KEY);
        // 释放探测的串行位。
        self.probe.release(Some(NEW_PROVIDER_FETCH_KEY));
    }
}
