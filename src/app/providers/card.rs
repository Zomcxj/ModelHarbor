use super::flags::CardActions;
use crate::app::bars::short_err;
use crate::app::fetch::{latency_color, matrix_label};
use crate::app::App;
use crate::ui::{card_frame, move_item, DragHandle};
use eframe::egui;

/// 卡片头右组的余额展示，供 `Sides` 的右闭包使用。
///
/// 右组数据先算成自有值：`Sides::show` 的两个闭包同时存活，左闭包要可变借用 `self`。
enum BalanceDisplay {
    /// 正在查询（转圈 + 「查询中…」）。
    Loading,
    /// 查询完成且有可展示数据（主数字 / 其余数字 / 悬停详情）。
    ///
    /// `headline` 非可选：没有主数字时构造处直接返回 `None`。
    Info {
        headline: String,
        rest: String,
        detail: String,
    },
}

impl App {
    pub(super) fn render_provider_card(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        actions: &mut CardActions,
    ) {
        let key = self.providers[idx].key.clone();
        let open = !self.provider_collapsed(&key);
        let highlight = if self.provider_drag_target.as_deref() == Some(key.as_str()) {
            2
        } else if self.provider_drag_src.as_deref() == Some(key.as_str()) {
            1
        } else {
            0
        };
        let card_id = egui::Id::new(("provider_card", key.clone()));
        let resp = card_frame(ui, open, highlight, card_id, |ui| {
            // 头部一行分左右两组：左组（拖柄 / 折叠 / 厂商名 / baseUrl 提示 / 连通性
            // 结果）内容不定长，右组（删除 / 复制 / 余额）贴在卡片右缘。
            //
            // 用 `Sides::shrink_left().truncate()`：先量右组，再把左组限制在剩余宽度内
            // 并按需截断。
            let balance_display = self.balance.get(&key).and_then(|state| {
                let display_result =
                    state.display_result(crate::app::balance::local_midnight_unix());
                match (&state.rx, display_result.as_ref()) {
                    (Some(_), _) => Some(BalanceDisplay::Loading),
                    (None, Some(Ok(info))) if info.is_displayable() => Some(BalanceDisplay::Info {
                        // 没有主数字就不显示。
                        headline: info.headline()?,
                        rest: info.inline_rest(),
                        detail: info.detail(),
                    }),
                    _ => None,
                }
            });
            let _ = egui::Sides::new().shrink_left().truncate().show(
                ui,
                |ui| {
                    let h = ui.add(DragHandle);
                    if h.drag_started() {
                        self.provider_drag_src = Some(key.clone());
                        self.provider_drag_target = None;
                    }
                    if h.drag_stopped() {
                        if self.provider_drag_src == Some(key.clone()) {
                            if let Some(dst) = self.provider_drag_target.clone() {
                                let s = self.providers.iter().position(|p| p.key == key);
                                let d = self.providers.iter().position(|p| p.key == dst);
                                if let (Some(s), Some(d)) = (s, d) {
                                    move_item(&mut self.providers, s, d);
                                }
                            }
                        }
                        self.provider_drag_src = None;
                        self.provider_drag_target = None;
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
                        self.set_provider_collapsed(&key, open);
                    }
                    ui.strong(&self.providers[idx].key);
                    // baseUrl 体检提示：`//v1` 这类笔误在卡片上可见（只提示，不自动改写）。
                    let suspicions = crate::util::url_suspicions(&self.providers[idx].base_url);
                    if !suspicions.is_empty() {
                        ui.label(
                            egui::RichText::new(format!("⚠ baseUrl: {}", suspicions.join("、")))
                                .small()
                                .color(crate::theme::semantics(ui).err),
                        )
                        .on_hover_text(format!(
                            "当前 baseUrl
{}
只提示，不自动改写配置",
                            self.providers[idx].base_url
                        ));
                    }
                    // 连通性测试结果：显示在厂商名字右侧，卡片收起时也可见。
                    if let Some(state) = self.latency.get(&key) {
                        if state.provider_rx.is_some() {
                            matrix_label(ui, &key);
                        } else if let Some(res) = &state.provider {
                            match res {
                                Ok(ms) => {
                                    ui.label(
                                        egui::RichText::new(format!("{}ms", ms))
                                            .color(latency_color(*ms, crate::theme::semantics(ui))),
                                    );
                                }
                                Err(err) => {
                                    ui.label(
                                        egui::RichText::new(short_err(err))
                                            .color(crate::theme::semantics(ui).err),
                                    )
                                    .on_hover_text(err);
                                }
                            }
                        }
                    }
                },
                |ui| {
                    if ui.button("删除").clicked() {
                        actions.remove = Some(idx);
                    }
                    if ui.button("复制").clicked() {
                        actions.copy = Some(idx);
                    }
                    // 查询结果紧挨「复制」左侧：余额 / 已用 / 今日 / 签到状态都在这行里。
                    // 查不到的站点不显示，也不显示占位余额。
                    match &balance_display {
                        Some(BalanceDisplay::Loading) => {
                            ui.add(egui::Spinner::new().size(14.0));
                            // 字号与连通性结果（`123ms`）一致。
                            ui.label(egui::RichText::new("查询中…").weak());
                        }
                        Some(BalanceDisplay::Info {
                            headline,
                            rest,
                            detail,
                        }) => {
                            // 主数字加粗，其余数字为淡色小字；两段仍在同一行。
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(headline)
                                        .strong()
                                        .color(crate::theme::semantics(ui).info),
                                )
                                .truncate(),
                            )
                            .on_hover_text(detail);
                            if !rest.is_empty() {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(rest)
                                            .size(crate::theme::TEXT_SMALL)
                                            .color(ui.visuals().weak_text_color()),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(detail);
                            }
                        }
                        None => {}
                    }
                },
            );
            // 折叠 / 展开带高度动画；动画 id 按 key 派生。
            crate::motion::animated_collapse(ui, card_id, open, |ui| {
                self.render_provider_form(
                    ui,
                    idx,
                    &mut actions.model_hover,
                    &mut actions.model_enable,
                );
            });
        });
        if let Some(src_key) = &self.provider_drag_src {
            if src_key != &key && resp.contains_pointer() && actions.hover.is_none() {
                actions.hover = Some(key.clone());
            }
        }
    }
}
