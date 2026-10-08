use super::*;

impl super::App {
    pub(in crate::app) fn render_provider_form(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        model_hover_target: &mut Option<String>,
        model_enable_target: &mut Option<(usize, usize)>,
    ) {
        let prev_key = self.providers[idx].key.clone();
        let other_keys: HashSet<String> = self
            .providers
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != idx)
            .map(|(_, p)| p.key.trim().to_string())
            .collect();
        let (variants_label, variant_names) = self.dialect_variants();
        let flags = ProviderFormFlags::new(self);
        let show_model_disabled = flags.show_model_disabled;
        // 全局重复 id 集合：跨所有 provider 按裸 id 去重。
        // 必须在取 `p = &mut self.providers[idx]` 之前算好。
        let global_dup_ids: HashSet<String> = if show_model_disabled {
            let mut seen: HashSet<String> = HashSet::new();
            let mut dup: HashSet<String> = HashSet::new();
            for pv in self.providers.iter() {
                for m in pv.models.iter() {
                    let id = m.id.trim().to_string();
                    if !id.is_empty() && !seen.insert(id.clone()) {
                        dup.insert(id);
                    }
                }
            }
            dup
        } else {
            HashSet::new()
        };
        let p = &mut self.providers[idx];
        let header_ctx = ProviderHeaderCtx {
            page: self.current_page,
            npm_salt: format!("provider_npm_{}", p.key),
            api_salt: format!("provider_api_{}", p.key),
            relaxed: false,
            npm_empty_label: true,
            show_api_keys: self.show_api_keys,
            other_keys: Some(&other_keys),
        };
        provider_header_fields(ui, p, &flags, &header_ctx);

        ui.add_space(crate::theme::SPACE_2);
        ui.add_space(crate::theme::SPACE_2);
        // 本帧用户点下的探测请求（provider key, model id），UI 循环外统一发起。
        let mut probe_request: Option<(String, String)> = None;
        let fetch_api = p.effective_api();
        let fetch_secret = credentials::effective_secret(p);
        let mut fetch_ctx = FetchSectionCtx {
            model_fetch: &mut self.model_fetch,
            model_fetch_open: &mut self.model_fetch_open,
            latency: &self.latency,
            current_page: self.current_page,
            fetch_key: &p.key,
            popup_salt: "model_fetch",
            scroll_salt: egui::Id::new(("model_fetch_scroll", p.key.clone())),
        };
        models_fetch_section(
            ui,
            &mut fetch_ctx,
            &p.base_url,
            &fetch_secret,
            &fetch_api,
            &mut p.models,
        );
        let mut rm: Option<usize> = None;
        let mut model_hover_here: Option<String> = None;
        let mut model_drag_stopped = false;
        // 本帧被勾上的启用开关，用 `(provider 下标, 模型下标)` 记录。
        // 互斥在全部卡片渲染完后统一处理（见 `ui_providers_section`）。
        let mut enable_request: Option<(usize, usize)> = None;
        for j in 0..p.models.len() {
            let model_key = format!("{}\u{1f}{}", p.key, p.models[j].id);
            let model_highlight = if self.model_drag_target.as_deref() == Some(model_key.as_str()) {
                2
            } else if self.model_drag_src.as_deref() == Some(model_key.as_str()) {
                1
            } else {
                0
            };
            let other_ids: HashSet<String> = p
                .models
                .iter()
                .enumerate()
                .filter(|(j2, _)| *j2 != j)
                .map(|(_, m)| m.id.trim().to_string())
                .collect();
            let model_response = card_frame(
                ui,
                true,
                model_highlight,
                egui::Id::new(("model_card", model_key.clone())),
                |ui| {
                    // 停用的模型整行压淡。
                    if show_model_disabled && p.models[j].disabled {
                        ui.set_opacity(0.45);
                    }
                    ui.horizontal(|ui| {
                        let handle = ui.add(DragHandle);
                        if handle.drag_started() {
                            self.model_drag_src = Some(model_key.clone());
                            self.model_drag_target = None;
                        }
                        if handle.drag_stopped() {
                            model_drag_stopped = true;
                        }
                        // 单模型延迟测试：按钮在拖动按钮右侧，结果显示在按钮右侧。
                        let model_id = p.models[j].id.trim().to_string();
                        let gate =
                            net_guard_gate(&self.net_guard, self.allow_model_test_with_proxy);
                        if model_probe_button(
                            ui,
                            &self.probe,
                            &p.key,
                            ui.input(|i| i.time),
                            gate.as_deref(),
                        ) {
                            probe_request = Some((p.key.clone(), model_id.clone()));
                        }
                        let latency = self.latency.get(&p.key);
                        model_latency_label(ui, latency, &model_id);
                        // 右对齐区放在行尾；先加的靠最右（「删」在右、「启用」紧贴其左）。
                        let enabled_now = !p.models[j].disabled;
                        let mut enable_clicked: Option<bool> = None;
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("删").clicked() {
                                rm = Some(j);
                            }
                            if show_model_disabled {
                                // 先加开关、后加文字，右对齐布局下读作「启用 [开关] 删」。
                                // id 用行的稳定键（`model_key` = provider 键 + 模型 id），
                                // 不用 egui 自动 id。
                                let mut enabled = enabled_now;
                                let toggle = crate::ui::toggle_switch(
                                    ui,
                                    egui::Id::new(("model_enable", model_key.as_str())),
                                    &mut enabled,
                                )
                                .on_hover_text(
                                    "WorkBuddy 的选择器**按模型 id 全局去重**：同一个模型名\
                                     无论挂在哪个厂商下，都只会列出一行、只有第一条生效。\n\
                                     所以同一 id 全局只能开一个——打开这个，同名的其他条目\
                                     会自动关闭。\n\
                                     只有开启的会写进 WorkBuddy 的 models.json；关闭**不会\
                                     删除配置**，条目仍保存在同目录的 models.full.json 里，\n\
                                     开回来即可恢复。想换一家厂商的同一个模型，直接开它即可。\n\
                                     ⚠ 不要为了区分同名模型去改 id —— id 同时就是发给上游的\
                                     模型名，改了会直接请求失败。",
                                );
                                ui.label("启用");
                                if toggle.clicked() {
                                    enable_clicked = Some(enabled);
                                }
                            }
                        });
                        if let Some(on) = enable_clicked {
                            p.models[j].disabled = !on;
                            if on {
                                enable_request = Some((idx, j));
                            }
                        }
                    });
                    // 用 `(provider key, 模型下标)` 当下拉 id 盐：模型 id 允许为空或重复。
                    model_core_fields_row(
                        ui,
                        &mut p.models[j],
                        &flags,
                        false,
                        Some((&other_ids, &global_dup_ids)),
                        &format!("{}#{}", p.key, j),
                    );
                    let mut variants = VariantsCtx {
                        label: variants_label,
                        names: variant_names,
                        open_set: &mut self.variant_open,
                        open_key: format!("variant_open_{}_{}", p.key, j),
                        normalize: true,
                    };
                    model_modalities_row(ui, &mut p.models[j], &flags, &mut variants, false);
                },
            );
            if let Some(src) = &self.model_drag_src {
                if src != &model_key
                    && model_response.contains_pointer()
                    && model_hover_here.is_none()
                {
                    model_hover_here = Some(model_key.clone());
                }
            }
        }
        if let Some((provider_key, model_id)) = probe_request {
            let now = ui.input(|i| i.time);
            let base_url = p.base_url.clone();
            let secret = credentials::effective_secret(p);
            let api = p.effective_api();
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
        // 只登记本 provider 内被拖到的模型，跨卡片聚合由调用方统一写入
        // `self.model_drag_target`。
        merge_drag_target(model_hover_target, model_hover_here);
        if let Some(picked) = enable_request {
            model_enable_target.get_or_insert(picked);
        }
        if model_drag_stopped {
            if let Some(src) = self.model_drag_src.take() {
                let target = self.model_drag_target.take();
                if let Some(dst) = target {
                    let source = p
                        .models
                        .iter()
                        .position(|m| format!("{}\u{1f}{}", p.key, m.id) == src);
                    let destination = p
                        .models
                        .iter()
                        .position(|m| format!("{}\u{1f}{}", p.key, m.id) == dst);
                    if let (Some(source), Some(destination)) = (source, destination) {
                        move_item(&mut p.models, source, destination);
                    }
                }
            }
        }
        if let Some(j) = rm {
            p.models.remove(j);
            // 删除后关闭该 provider 的档位弹窗。
            let prefix = format!("variant_open_{}_", p.key);
            self.variant_open.retain(|k| !k.starts_with(&prefix));
        }
        ui.add_space(crate::theme::SPACE_2);
        let show_new_model_key = format!("show_new_model_{}", p.key);
        let show_new_model = self.variant_open.contains(&show_new_model_key);
        let btn_text = if show_new_model {
            "收起"
        } else {
            "添加 Model"
        };
        if ui
            .add_sized([120.0, 20.0], egui::Button::new(btn_text))
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
                open_key: format!("new_model_variant_{}", p.key),
                normalize: false,
            };
            new_model_subform(ui, p, &flags, &mut variants, &show_new_model_key);
        }
        // key 重命名后同步展开状态与弹窗键
        let new_key = self.providers[idx].key.clone();
        if new_key != prev_key {
            self.sync_provider_rename(&prev_key, &new_key);
        }
    }
}
