//! Providers 区块：厂商卡片列表、拖拽落点聚合、表单字段可见性标志与思考档位方言。
//!
//! 子模块：`flags` 表单字段可见性标志与卡片动作聚合、`section` 区块 glue（吸顶标题行
//! + 卡片列表 + 批量按钮）、`card` 单张厂商卡片、`tokens` 令牌 / 用户数据面板。

mod card;
mod flags;
mod section;
mod tokens;

pub(in crate::app) use flags::ProviderFormFlags;

use super::App;
use crate::format::ConfigFormat;
use eframe::egui;
use std::collections::HashSet;

impl App {
    /// 当前页面的思考档位标签：字段名 + 可选值。
    pub(super) fn dialect_variants(&self) -> (&'static str, &'static [&'static str]) {
        match self.current_page {
            // opencode 系三者档位字段名相同。
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
            // ZCode 的档位存在 optionSpecs.reasoningLevel.values，词表与内置库一致。
            ConfigFormat::ZCode => (
                "optionSpecs.reasoningLevel.values",
                &["minimal", "low", "medium", "high", "xhigh", "max"],
            ),
            // WorkBuddy 没有档位清单，只有 supportsReasoning 布尔。
            ConfigFormat::WorkBuddy => ("supportsReasoning", &[]),
            // QwenCode 的档位是 capabilities.reasoning.efforts。
            ConfigFormat::QwenCode => ("efforts", &["low", "medium", "high", "xhigh", "max"]),
            // KimiCode 的档位是 `support_efforts` 数组，官方无固定词表：这里只是
            // 一组常见值供快速点选，用户填任意字符串都会原样写入，不做校验。
            ConfigFormat::KimiCode => ("support_efforts", &["low", "medium", "high", "max"]),
        }
    }

    /// 首次使用引导条：三步上手 + 一句查余额前提，可关闭（状态存 settings.json）。
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

    /// 让 `(provider 下标, 模型下标)` 成为其模型 id 下唯一启用的条目。
    ///
    /// WorkBuddy 的选择器按裸 model id 全局去重，同 id 只认第一条。
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
                    // 按下标比对：同一张卡片里也可能有两条同 id 的条目。
                    m.disabled = (i, j) != picked;
                }
            }
        }
    }

    /// 进入 WorkBuddy 页时，把「同一 id 多条都启用」收敛成「只留第一条启用」。
    ///
    /// WorkBuddy 的选择器按裸 id 全局去重，同一 id 至多一条启用；收敛幂等，
    /// 再跑一次不改动任何东西。
    pub(super) fn normalize_workbuddy_enable_flags(&mut self) {
        let mut seen: HashSet<String> = HashSet::new();
        for p in self.providers.iter_mut() {
            for m in p.models.iter_mut() {
                let id = m.id.trim().to_string();
                // 空 id 的行保存时回落到 provider key，跳过。
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
    /// provider key 重命名后同步 UI 状态（卡片折叠集合 + 弹窗键）。
    pub(super) fn sync_provider_rename(&mut self, old: &str, new: &str) {
        if old == new || new.is_empty() {
            return;
        }
        self.rename_collapsed_card("providers", old, new);
        // 下标型弹窗键直接关闭，需要时重新打开即可
        let variant_prefix = format!("variant_open_{}_", old);
        let show_key = format!("show_new_model_{}", old);
        let new_variant_key = format!("new_model_variant_{}", old);
        self.variant_open
            .retain(|k| !k.starts_with(&variant_prefix) && k != &show_key && k != &new_variant_key);
    }
}
