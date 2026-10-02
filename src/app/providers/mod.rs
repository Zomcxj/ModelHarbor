//! Providers 区块：厂商卡片列表、拖拽落点聚合、表单字段可见性标志与思考档位方言。
//!
//! 按关注点拆分：`flags` 表单字段可见性标志与卡片动作聚合、`section`
//! 区块 glue（吸顶标题行 + 卡片列表 + 批量按钮）、`card` 单张厂商卡片、
//! `tokens` 令牌/用户数据面板。跨切方法留在本文件。

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
    /// 当前页面的思考档位标签。
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
            // QwenCode 的档位是 capabilities.reasoning.efforts；官方词表就是
            // low/medium/high/xhigh/max（"replaces the supported subset of
            // low/medium/high/xhigh/max"），没有 opencode 那些 none/ultra。
            ConfigFormat::QwenCode => ("efforts", &["low", "medium", "high", "xhigh", "max"]),
            // KimiCode 的档位是 `support_efforts` 数组。官方没有固定词表（各模型自带，
            // 本机文件里是 low/high/max），所以这里只给一组**常见值**供快速点选，
            // 用户填任意字符串都会被原样写入——不能把词表当成校验。
            ConfigFormat::KimiCode => ("support_efforts", &["low", "medium", "high", "max"]),
        }
    }

    /// 首次使用引导条：三步上手 + 一句查余额前提，可关闭（状态存 settings.json）。
    ///
    /// 只在用户没关过时出现；关掉后不再打扰。
    /// 首次使用引导条：三步上手 + 一句查余额前提，可关闭（状态存 settings.json）。
    ///
    /// 只在用户没关过时出现；关掉后不再打扰。
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
    /// WorkBuddy 的选择器按**裸 model id 全局去重**：同一个模型名无论挂在哪个厂商下，
    /// 都只列出一行、只认第一条。所以「同一个模型开两个」没有意义——第二个根本不会生效，
    /// 用户却以为配了两份。这里勾上一个就把同 id 的其他条目全部关掉，让界面与
    /// WorkBuddy 的实际行为一致；想换厂商就直接勾那一家。
    /// 让 `(provider 下标, 模型下标)` 成为其模型 id 下唯一启用的条目。
    ///
    /// WorkBuddy 的选择器按**裸 model id 全局去重**：同一个模型名无论挂在哪个厂商下，
    /// 都只列出一行、只认第一条。所以「同一个模型开两个」没有意义——第二个根本不会生效，
    /// 用户却以为配了两份。这里勾上一个就把同 id 的其他条目全部关掉，让界面与
    /// WorkBuddy 的实际行为一致；想换厂商就直接勾那一家。
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
                    // 按下标比对：同一张卡片里也可能有两条同 id 的条目
                    // （用户文件里就有这种形态），只比 id 会让它们同时保持启用。
                    m.disabled = (i, j) != picked;
                }
            }
        }
    }

    /// 进入 WorkBuddy 页时，把「同一 id 多条都启用」收敛成「只留第一条启用」。
    ///
    /// WorkBuddy 的选择器按**裸 id 全局去重**，同一个模型名多开根本不会生效——界面显示成
    /// 全部启用就是在骗人。而数据可能来自别的方言：那些格式没有 `disabled` 概念
    /// （`convert` 里一律读成启用），所以「各页共享同一份数据」这件事会让 WorkBuddy 页
    /// 一开始就全是勾选状态。收敛必须在这里做，**不能只在写 WorkBuddy 的生效清单时做**：
    /// 界面显示的状态本身就得是真实的。
    ///
    /// 幂等：收敛后每个 id 至多一条启用，再跑一次不改动任何东西。
    ///
    /// 这里「保留第一条」是安全的，不会顶掉用户的选择：界面的启用开关是**全局互斥**的
    /// （`enable_model_exclusively`，勾一个就把同 id 的其余全关掉），所以「同一 id 多条
    /// 同时启用」根本不可能由用户的显式操作产生——它只可能来自跨方言的数据，或用户手动
    /// 把两行的 id 改成了同一个。这两种情况下都没有「用户选中的那一条」可尊重，
    /// 按位置取第一条正是与 WorkBuddy 实际行为一致的做法。
    ///
    /// 文件里带的显式勾选记录由后端处理（`build_load` 让显式标记优先于位置推导），
    /// 这里的兜底只负责跨方言共享数据这一种来源。
    /// 进入 WorkBuddy 页时，把「同一 id 多条都启用」收敛成「只留第一条启用」。
    ///
    /// WorkBuddy 的选择器按**裸 id 全局去重**，同一个模型名多开根本不会生效——界面显示成
    /// 全部启用就是在骗人。而数据可能来自别的方言：那些格式没有 `disabled` 概念
    /// （`convert` 里一律读成启用），所以「各页共享同一份数据」这件事会让 WorkBuddy 页
    /// 一开始就全是勾选状态。收敛必须在这里做，**不能只在写 WorkBuddy 的生效清单时做**：
    /// 界面显示的状态本身就得是真实的。
    ///
    /// 幂等：收敛后每个 id 至多一条启用，再跑一次不改动任何东西。
    ///
    /// 这里「保留第一条」是安全的，不会顶掉用户的选择：界面的启用开关是**全局互斥**的
    /// （`enable_model_exclusively`，勾一个就把同 id 的其余全关掉），所以「同一 id 多条
    /// 同时启用」根本不可能由用户的显式操作产生——它只可能来自跨方言的数据，或用户手动
    /// 把两行的 id 改成了同一个。这两种情况下都没有「用户选中的那一条」可尊重，
    /// 按位置取第一条正是与 WorkBuddy 实际行为一致的做法。
    ///
    /// 文件里带的显式勾选记录由后端处理（`build_load` 让显式标记优先于位置推导），
    /// 这里的兜底只负责跨方言共享数据这一种来源。
    pub(super) fn normalize_workbuddy_enable_flags(&mut self) {
        let mut seen: HashSet<String> = HashSet::new();
        for p in self.providers.iter_mut() {
            for m in p.models.iter_mut() {
                let id = m.id.trim().to_string();
                // 空 id 的行保存时回落到 provider key，谈不上「同一个模型」，跳过。
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
        // 下标型弹窗键直接关闭（避免前缀歧义），需要时重新打开即可
        let variant_prefix = format!("variant_open_{}_", old);
        let show_key = format!("show_new_model_{}", old);
        let new_variant_key = format!("new_model_variant_{}", old);
        self.variant_open
            .retain(|k| !k.starts_with(&variant_prefix) && k != &show_key && k != &new_variant_key);
    }
}
