use super::flags::CardActions;
use crate::app::balance;
use crate::app::bars::{sticky_begin, sticky_end};
use crate::app::App;
use crate::credentials;
use crate::ui::card_list;
use eframe::egui;

impl App {
    /// Providers 区块：标题行吸顶（滚动时始终显示在顶部），内容紧跟其下。
    pub(in crate::app) fn ui_providers_section(&mut self, ui: &mut egui::Ui) {
        // 吸顶区是**固定高度矩形**（sticky_end 用 max_rect 建子 ui）：
        // 里面所有内容都必须装在这一个高度里，多出来的行不会撑开矩形，
        // 而是直接画到下面的卡片区上。所以「代理支持」是并回标题行、
        // 而不是另起一行。
        let anchor = sticky_begin(ui, 30.0);
        let matched: Vec<usize> = (0..self.providers.len()).collect();

        if self.providers.is_empty() && !self.show_new_provider {
            self.show_new_provider = true;
        }

        self.ui_first_run_guide(ui);

        // 拖拽落点必须在**所有卡片渲染完之后**统一聚合再写入 self：
        // 卡片各自赋值会被后渲染的卡片用 None 覆盖（模型卡片曾因此丢失绿色落点边框）。
        let mut actions = CardActions::default();
        let card_gap = if crate::theme::active_style(ui.ctx()).has_card_shadow() {
            crate::theme::SPACE_2
        } else {
            0.0
        };
        card_list(ui, &matched, card_gap, |ui, idx| {
            self.render_provider_card(ui, idx, &mut actions);
        });
        // 同一模型 id 全局只能开一个（WorkBuddy 按裸 id 去重，同名的只有第一条生效）。
        // 必须在**所有卡片渲染完之后**统一处理：卡片各自改会漏掉别的卡片里的同名条目。
        if let Some(picked) = actions.model_enable.take() {
            self.enable_model_exclusively(picked);
        }
        if let Some(idx) = actions.remove {
            self.providers.remove(idx);
            self.status = "已删除 provider".into();
        }
        if let Some(idx) = actions.copy {
            let mut p = self.providers[idx].clone();
            p.key = format!("{}_copy", p.key);
            self.providers.push(p);
            self.status = "已复制 provider".into();
        }
        if self.provider_drag_src.is_some() {
            self.provider_drag_target = actions.hover;
        } else {
            self.provider_drag_target = None;
        }
        // 模型拖拽落点：与 provider 同样在外层聚合，保证任意展开顺序下被拖到的
        // 模型卡片都能拿到绿色边框（见 render_provider_form 里的说明）。
        if self.model_drag_src.is_some() {
            self.model_drag_target = actions.model_hover;
        } else {
            self.model_drag_target = None;
        }

        ui.add_space(crate::theme::SPACE_3);
        if ui.button("新增 Provider").clicked() {
            self.show_new_provider = !self.show_new_provider;
        }
        if self.show_new_provider {
            self.ui_new_provider_form(ui);
        }
        sticky_end(ui, anchor, |ui| {
            ui.horizontal(|ui| {
                ui.strong(egui::RichText::new("Providers").size(crate::theme::TEXT_HEADING));
                // 三个动作钮保留在标题行左侧（与标题同排、随内容左对齐）：
                // 展开/收起全部、连通性测试、查询用户数据，统一图标样式；
                // 「代理支持」已改为页头右上角的 globe 图标开关（见 top_bar）。
                // 展开/收起全部（chevrons 图标）：收起全部卡片时也始终可见。
                if !self.providers.is_empty() {
                    let all_open = self
                        .providers
                        .iter()
                        .all(|p| !self.provider_collapsed(&p.key));
                    let chevrons_tint = ui.visuals().text_color();
                    if crate::app::bars::toolbar_icon_button(
                        ui,
                        self.toolbar_icons.chevrons_down_up.as_ref(),
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
                        // all_open 为真 = 现在全部展开 → 按钮是「收起全部」
                        self.set_all_providers_collapsed(all_open);
                        // 批量不走高度补间：25 张卡同时把整份表单画进裁剪区会卡。
                        let ctx = ui.ctx().clone();
                        let open = !all_open;
                        for provider in &self.providers {
                            crate::motion::snap_collapse(
                                &ctx,
                                egui::Id::new(("provider_card", provider.key.clone())),
                                open,
                            );
                        }
                    }
                }

                // 连通性测试（zap 图标）：批量测所有厂商连通性；
                // 文字回退见 toolbar_icon_button 的 None 分支。
                let zap_tint = ui.visuals().text_color();
                if crate::app::bars::toolbar_icon_button(
                    ui,
                    self.toolbar_icons.zap.as_ref(),
                    false,
                    zap_tint,
                )
                .on_hover_text("连通性测试：批量测全部厂商的连通性")
                .clicked()
                {
                    let targets: Vec<(String, String, String, String)> = self
                        .providers
                        .iter()
                        .map(|p| {
                            let api = p.effective_api();
                            (
                                p.key.clone(),
                                p.base_url.clone(),
                                credentials::effective_secret(p),
                                api,
                            )
                        })
                        .collect();
                    let count = targets.len();
                    for (key, base, secret, api) in targets {
                        Self::start_provider_latency(&mut self.latency, &key, &base, &secret, &api);
                    }
                    self.status = format!("已开始连通性测试（{} 个厂商）", count);
                }

                // 查询用户数据（database 图标，最贴行尾）：一次查完当前页面全部厂商
                // （只读管理接口，直连不走代理）。一份结果里能有什么就显示什么：
                // 余额 / 已用 / 今日 / 近 7 天 / 签到状态。查不到的站点不在卡片上
                // 显示，只在状态栏汇总（避免一堆红字噪音）。
                let database_tint = ui.visuals().text_color();
                if crate::app::bars::toolbar_icon_button(
                    ui,
                    self.toolbar_icons.database.as_ref(),
                    false,
                    database_tint,
                )
                .on_hover_text("查询用户数据：批量查全部厂商的余额/用量/签到")
                .clicked()
                {
                    let targets: Vec<balance::Query> = self
                        .providers
                        .iter()
                        .map(|p| balance::Query {
                            key: p.key.clone(),
                            base_url: p.base_url.clone(),
                            secret: credentials::effective_secret(p),
                            // 站点级面板令牌：没设置就是空串（只查 sk- 那两个接口）。
                            pat: self.station_pat(&p.base_url),
                            // 旧版 new-api 要的用户 ID：没填就是空串（不发该头）。
                            user_id: self.station_user_id(&p.base_url),
                        })
                        .collect();
                    let now = ui.input(|i| i.time);
                    let mut started = 0usize;
                    for query in targets {
                        if Self::start_balance_query(&mut self.balance, query, now).is_none() {
                            started += 1;
                        }
                    }
                    self.balance_batch = started > 0;
                    self.status = if started == 0 {
                        "所有厂商都还在冷却中，请几秒后再试".to_string()
                    } else {
                        format!("已开始查询 {} 个厂商的用户数据…", started)
                    };
                }
            });
        });
    }
}
