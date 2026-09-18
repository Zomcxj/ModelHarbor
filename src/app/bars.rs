//! 顶栏 / 状态栏 / 页头等外围栏位，以及吸顶标题与错误文本处理小工具。
use super::{App, SaveFormat};
use crate::app::preview::PREVIEW_EDITOR_ID;
use crate::app::save::PageTarget;
use crate::backends;
use crate::format::ConfigFormat;
use crate::theme::Theme;
use crate::util::show_file_dialog;
use eframe::egui;

/// 吸顶标题占位：在内容流中预留标题行高度，返回绘制锚点。
/// 必须与 [`sticky_end`] 配对，并在 section 内容渲染完成后调用 sticky_end，
/// 以保证标题最后绘制（否则会被下方滚动内容覆盖）。
pub(super) fn sticky_begin(ui: &mut egui::Ui, height: f32) -> (f32, f32, f32, f32) {
    let avail = ui.available_rect_before_wrap();
    ui.allocate_exact_size(egui::vec2(avail.width(), height), egui::Sense::hover());
    (avail.top(), avail.left(), avail.right(), height)
}

/// 绘制吸顶标题：未滚过时留在内容流中；滚动越过视口顶部后吸附在滚动区顶部。
pub(super) fn sticky_end(
    ui: &mut egui::Ui,
    anchor: (f32, f32, f32, f32),
    paint: impl FnOnce(&mut egui::Ui),
) {
    let (top, left, right, height) = anchor;
    let clip_top = ui.clip_rect().top();
    let y = top.max(clip_top);
    let target = egui::Rect::from_min_max(egui::pos2(left, y), egui::pos2(right, y + height));
    if !ui.clip_rect().intersects(target) {
        return;
    }
    // 用 new_child 而非 scope_builder：后者会推进父 cursor 到吸顶位置，
    // 破坏内容流导致滚动区滚轮失效。
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(target));
    // 拦截层：吸顶条空白区域（标题文字间隙等）的点击/拖拽先被此层消费，
    // 不再穿透到其正下方的卡片控件（误删/误展开/误拖放目标）。
    // 先注册，后续的按钮仍在其上层优先响应。
    child.interact(
        target,
        child.id().with("sticky-block"),
        egui::Sense::click_and_drag(),
    );
    child
        .painter()
        .rect_filled(target, 0.0, ui.visuals().panel_fill);
    paint(&mut child);
    // 标题下边线：吸顶时也能与内容分隔
    child.painter().hline(
        target.x_range(),
        target.bottom() - 1.0,
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
}

/// 错误摘要：HTTP 状态码 + 人话原因（如 `HTTP 503 服务临时不可用`）；
/// 其他错误截断为短文本。
///
/// 卡片 / 列表里只显示这一行，完整说明（含处理建议）挂在悬停提示上。
pub(super) fn short_err(err: &str) -> String {
    if let Some(code) = http_status_code(err) {
        return crate::http_status::label(code);
    }
    let t = err.trim();
    if t.chars().count() > 96 {
        let mut s: String = t.chars().take(96).collect();
        s.push('…');
        s
    } else {
        t.to_string()
    }
}

/// 从 `HTTP 503 …` 形式的错误文本里取出状态码；取不到返回 `None`。
///
/// 供错误文本的消费方使用（避免从展示文本里再手写一遍解析）。
pub(super) fn http_status_code(err: &str) -> Option<u16> {
    let rest = err.strip_prefix("HTTP ")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// 网络错误文本脱敏：ureq 的 Transport Display 会包含目标 URL，
/// 若用户把凭据放进 URL query（如 `?key=sk-…`）会随错误泄漏到
/// 状态栏/悬停提示；剥离 URL 的 query/fragment 后返回。
pub(super) fn sanitize_network_error(text: &str) -> String {
    const MARK: &str = "for URL \"";
    if let Some(idx) = text.find(MARK) {
        let head = &text[..idx + MARK.len()];
        let rest = &text[idx + MARK.len()..];
        let url = rest.split('"').next().unwrap_or(rest);
        let cut = url.find(['?', '#']).unwrap_or(url.len());
        format!("{}{}\"", head, &url[..cut])
    } else {
        text.to_string()
    }
}

impl App {
    pub(super) fn ui_top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.style_mut().spacing.interact_size.y = 18.0;
            // 第一行：页面切换 / 来源 / 右侧 WSL 同步 + 主题
            ui.horizontal(|ui| {
                let icons: Vec<Option<egui::TextureHandle>> = self.backend_icons.clone();
                for (i, b) in backends::BACKENDS.iter().enumerate() {
                    let id = b.id();
                    let btn = match icons.get(i).and_then(|o| o.as_ref()) {
                        Some(tex) => egui::Button::image(
                            egui::Image::from_texture(tex)
                                .fit_to_exact_size(egui::vec2(16.0, 16.0)),
                        ),
                        None => egui::Button::new(""),
                    };
                    let is_selected = self.current_page == id;
                    let btn = if is_selected {
                        // 选中态：填充 + 描边，与未选中图标拉开视觉层级
                        btn.fill(ui.visuals().selection.bg_fill).stroke(
                            egui::Stroke::new(1.0, ui.visuals().selection.stroke.color),
                        )
                    } else {
                        btn
                    };
                    // 只显示图标，鼠标悬停提示名称；加大点击区便于操作
                    let btn_resp = ui
                        .add(btn.min_size(egui::vec2(24.0, 22.0)))
                        .on_hover_text(id.label())
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if btn_resp.clicked() {
                        if id == ConfigFormat::DeepSeekHarness
                            && self.current_page != ConfigFormat::DeepSeekHarness
                        {
                            self.project_dsh_credentials();
                        }
                        self.sync_provider_secrets(id);
                        // 对应 agent 未在 WSL 安装的页面：关闭并禁用 WSL 同步
                        if backends::wsl_target(id).is_none() {
                            self.sync_wsl = false;
                        }
                        self.current_page = id;
                        // 切换页面后必须重建预览草稿：草稿只在「预览未聚焦且上次解析成功」时才
                        // 跟随组件状态，否则会停留在上一页的内容上（预览框仍有焦点或上次解析失败）。
                        self.reset_preview_draft();
                        ctx.memory_mut(|m| m.surrender_focus(egui::Id::new(PREVIEW_EDITOR_ID)));
                    }
                }
                ui.separator();
                // 右侧：WSL 同步 + 主题
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // 按当前页面检测对应 agent 是否已在 WSL 安装
                    let current = self.current_page;
                    let wsl_installed = backends::wsl_target(current).is_some();
                    let wsl_tip = if wsl_installed {
                        format!("保存时同步写入 WSL 侧 {} 的配置", current.label())
                    } else {
                        format!(
                            "WSL 中未检测到 {} 安装（配置文件或其目录均不存在），保存仅写 Windows 本地",
                            current.label()
                        )
                    };
                    ui.add_enabled(
                        wsl_installed,
                        egui::Checkbox::new(&mut self.sync_wsl, "WSL同步"),
                    )
                    .on_hover_text(wsl_tip);
                    ui.separator();
                    let look_btn = ui
                        .button("外观")
                        .on_hover_text("主题、形状与圆角")
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    egui::Popup::menu(&look_btn)
                        // 菜单默认是 `top_down_justified`：每一项的填充会撑满整个
                        // 弹出宽度，文字只占左边一小段，看着像「填充与文字没对齐」。
                        // 改成左对齐的普通纵向布局，填充就贴着文字宽度。
                        .layout(egui::Layout::top_down(egui::Align::LEFT))
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            ui.set_min_width(240.0);
                            ui.label(egui::RichText::new("主题").small().weak());
                            // 当前选中的形状圆角，所有按钮统一使用
                            let current_radius = self.ui_style.radius() as f32;
                            // 当前主题的强调色（用于形状按钮填充）
                            let current_accent = self.theme.accent_color();
                            // 深色主题一行
                            ui.label(egui::RichText::new("深色").size(10.0).weak());
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                for t in &Theme::ALL[0..4] {
                                    let is_current = self.theme == *t;
                                    let accent = t.accent_color();
                                    // 按钮用主题色填充
                                    let fill = accent;
                                    // 文字颜色：根据背景亮暗自动选择黑/白
                                    let text_color = if accent.r() as u32 + accent.g() as u32 + accent.b() as u32 > 384 {
                                        egui::Color32::BLACK
                                    } else {
                                        egui::Color32::WHITE
                                    };
                                    // 当前主题：加一圈对比色边框
                                    let stroke = if is_current {
                                        egui::Stroke::new(2.0, egui::Color32::WHITE)
                                    } else {
                                        egui::Stroke::NONE
                                    };
                                    let btn = egui::Button::new(egui::RichText::new(t.label()).color(text_color))
                                        .fill(fill)
                                        .stroke(stroke)
                                        .min_size(egui::vec2(52.0, 0.0))
                                        .corner_radius(current_radius);
                                    if ui.add(btn).clicked() {
                                        self.theme = *t;
                                    }
                                }
                            });
                            // 亮色主题一行
                            ui.label(egui::RichText::new("亮色").size(10.0).weak());
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                for t in &Theme::ALL[4..8] {
                                    let is_current = self.theme == *t;
                                    let accent = t.accent_color();
                                    let fill = accent;
                                    let text_color = if accent.r() as u32 + accent.g() as u32 + accent.b() as u32 > 384 {
                                        egui::Color32::BLACK
                                    } else {
                                        egui::Color32::WHITE
                                    };
                                    let stroke = if is_current {
                                        egui::Stroke::new(2.0, egui::Color32::from_gray(40))
                                    } else {
                                        egui::Stroke::NONE
                                    };
                                    let btn = egui::Button::new(egui::RichText::new(t.label()).color(text_color))
                                        .fill(fill)
                                        .stroke(stroke)
                                        .min_size(egui::vec2(52.0, 0.0))
                                        .corner_radius(current_radius);
                                    if ui.add(btn).clicked() {
                                        self.theme = *t;
                                    }
                                }
                            });
                            ui.add_space(crate::theme::SPACE_2);
                            ui.label(egui::RichText::new("形状").small().weak());
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                for style in crate::theme::UiStyle::ALL {
                                    let is_current = self.ui_style == style;
                                    // 按实际圆角和边框宽度绘制预览按钮
                                    let radius = style.radius() as f32;
                                    let border = style.border_width();
                                    // 填充当前主题的强调色
                                    let fill = current_accent;
                                    // 文字颜色：根据当前主题色亮暗自动选择
                                    let text_color = if current_accent.r() as u32 + current_accent.g() as u32 + current_accent.b() as u32 > 384 {
                                        egui::Color32::BLACK
                                    } else {
                                        egui::Color32::WHITE
                                    };
                                    // 当前形状：加边框
                                    let stroke_color = if is_current {
                                        if current_accent.r() as u32 + current_accent.g() as u32 + current_accent.b() as u32 > 384 {
                                            egui::Color32::from_gray(40)
                                        } else {
                                            egui::Color32::WHITE
                                        }
                                    } else {
                                        fill
                                    };
                                    let btn = egui::Button::new(egui::RichText::new(style.label()).color(text_color))
                                        .fill(fill)
                                        .stroke(egui::Stroke::new(border, stroke_color))
                                        .min_size(egui::vec2(52.0, 0.0))
                                        .corner_radius(radius);
                                    if ui.add(btn).clicked() {
                                        self.ui_style = style;
                                    }
                                }
                            });
                        });
                });
            });
            // 第二行：配置文件 / 保存格式
            ui.horizontal(|ui| {
                ui.label("配置文件:");
                let path_resp = ui
                    .add(egui::TextEdit::singleline(&mut self.config_path).desired_width(420.0))
                    .on_hover_text(
                        "回车加载该路径；留空后回车 = 清除本页路径覆盖，回到自动探测到的默认路径。\n手动指定过的路径按页面记住（存在 .modelharbor/settings.json）。",
                    );
                // 回车确认：按当前输入路径重新加载（egui 单行编辑回车即失焦）
                if path_resp.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && self.config_path != self.loaded_path
                {
                    if self.config_path.trim().is_empty() {
                        // 留空 + 回车 = 清除本页路径覆盖，回到默认路径
                        let page = self.current_page;
                        self.reset_page_path(page);
                    } else {
                        self.reload();
                    }
                }
                // 文件来源显示在原本“加载”按钮的位置；加载改为回车或“浏览”。
                // 只显示“来源：”+ 各后端官方图标（名称见悬停提示）。
                ui.label(egui::RichText::new("来源:").weak());
                if let Some(icon) = self.icon_for(self.source_format) {
                    ui.add(
                        egui::Image::from_texture(icon)
                            .fit_to_exact_size(egui::vec2(14.0, 14.0)),
                    )
                    .on_hover_text(self.source_format.label());
                } else {
                    ui.label(egui::RichText::new(self.source_format.label()).weak());
                }
                if ui.button("浏览").clicked() {
                    if let Some(p) = show_file_dialog() {
                        self.config_path = p;
                        self.reload();
                    }
                }
                if !self.config_path.is_empty() && self.config_path != self.loaded_path {
                    ui.label(egui::RichText::new("未加载").small().color(crate::theme::semantics(ui).warn))
                        .on_hover_text("路径已修改但未加载：保存时将按“先读后合并”写入该路径（不破坏目标文件已有配置）。\n在此按回车可切换到该文件。");
                }
                ui.separator();
                ui.label("保存格式:");
                let format_btn = ui.button(self.save_format.label());
                if format_btn.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let hovering = format_btn.hovered();
                let scroll = ui
                    .input(|i| i.events.iter().any(|e| matches!(e, egui::Event::MouseWheel { .. })));
                // 滚轮切换：一次连续滚动手势只切换一次，避免快速滚动时来回翻转
                if hovering && scroll && !self.save_format_wheel_latch {
                    self.save_format = match self.save_format {
                        SaveFormat::Current => SaveFormat::Compact,
                        SaveFormat::Compact => SaveFormat::Current,
                    };
                    self.save_format_wheel_latch = true;
                }
                if !scroll || !hovering {
                    self.save_format_wheel_latch = false;
                }
                if format_btn.clicked() {
                    self.save_format = match self.save_format {
                        SaveFormat::Current => SaveFormat::Compact,
                        SaveFormat::Compact => SaveFormat::Current,
                    };
                }
            });
        });
    }

    pub(super) fn ui_status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("bottom")
            .exact_height(32.0)
            .show(ctx, |ui| {
                // 底部文字：靠下（不垂直居中）且左对齐，右侧统计仍靠右。
                ui.with_layout(egui::Layout::left_to_right(egui::Align::BOTTOM), |ui| {
                    if let Some(err) = &self.load_error {
                        ui.label(
                            egui::RichText::new(format!("⚠ 加载失败: {}", err))
                                .color(crate::theme::semantics(ui).err),
                        );
                    }
                    ui.label(egui::RichText::new(&self.status).weak());
                    // 右侧：当前页 + 数量统计，随时可见页面身份
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::BOTTOM), |ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "agents: {} | providers: {}",
                                self.agents.len(),
                                self.providers.len()
                            ))
                            .weak(),
                        );
                        ui.label(
                            egui::RichText::new(format!("当前页: {}", self.current_page.label()))
                                .weak(),
                        );
                    });
                });
            });
    }

    /// 页头：本页保存按钮 + 写入路径。
    pub(super) fn ui_page_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let fmt = self.current_page;
            let target = self.page_save_path(fmt);
            let (path, kind, can_save) = match &target {
                PageTarget::Current(p) => (p.clone(), "当前文件（agent/provider 整体替换）", true),
                PageTarget::Modified(p) => (p.clone(), "路径已修改未加载：先读后合并写入", true),
                PageTarget::Default(p) => {
                    let ok = self.targets.iter().any(|t| t.backend == fmt && t.available);
                    (p.clone(), "默认目标（Windows 本地；WSL 仅勾选后写入）", ok)
                }
            };
            if ui
                .add_enabled(
                    can_save,
                    egui::Button::new(egui::RichText::new("保存").strong())
                        .fill(ui.visuals().selection.bg_fill),
                )
                .clicked()
            {
                self.save_page(fmt);
            }
            if let Some(icon) = self.icon_for(fmt) {
                ui.add(egui::Image::from_texture(icon).fit_to_exact_size(egui::vec2(12.0, 12.0)));
            }
            // 路径可能很长（吸顶区是固定高度，不能换行）：截断显示，全文放悬停。
            ui.add(
                egui::Label::new(egui::RichText::new(format!("写入: {path}")).weak()).truncate(),
            )
            .on_hover_text(format!("{kind}\n{path}"));
            // 该格式不支持的区块提前提示，避免保存后才发现数据没写入
            if fmt != ConfigFormat::Opencode && !self.agents.is_empty() {
                ui.label(
                    egui::RichText::new(format!(
                        "⚠ {} 个 agents 不会写入该格式",
                        self.agents.len()
                    ))
                    .small()
                    .color(crate::theme::semantics(ui).warn),
                )
                .on_hover_text("该格式不支持 agent 定义，保存时将忽略");
            }
            // 右端按钮组：「令牌 · 显示密钥 · 预览」（右对齐布局里越晚添加越靠左，
            // 所以按倒序加）。三个都跟「保存 / 看」同一个动作有关，放在保存这一行；
            // 嵌套的右对齐布局会占掉本行剩余宽度，所以它们紧贴右边缘，
            // 写入路径太长把本行占满时会落到下一行并同样靠右。
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // 预览：右侧面板实时展示当前页面的序列化内容，可编辑并应用回组件。
                if ui
                    .button(if self.show_preview {
                        "关闭预览"
                    } else {
                        "预览"
                    })
                    .on_hover_text(
                        "在右侧打开当前页面「待保存文档」预览；可直接编辑，改动实时应用并自动保存",
                    )
                    .clicked()
                {
                    self.show_preview = !self.show_preview;
                    if self.show_preview {
                        // 打开时以组件状态重建待保存文档
                        self.reset_preview_draft();
                    }
                }
                // 全局密钥显隐：一键切换全部 API Key 的明文 / 掩码。
                // 文案带「密钥」二字，与区块「隐藏/展开」、卡片 ▼/▶ 折叠按钮明确区分。
                if ui
                    .button(if self.show_api_keys {
                        "隐藏密钥"
                    } else {
                        "显示密钥"
                    })
                    .on_hover_text(if self.show_api_keys {
                        "点击掩码全部 API Key（默认状态）"
                    } else {
                        "点击显示全部 API Key 明文（注意防窥）"
                    })
                    .clicked()
                {
                    self.show_api_keys = !self.show_api_keys;
                }
                // 令牌：站点面板访问令牌（PAT）管理；填了才能查账号级真实余额。
                if ui
                    .button("令牌")
                    .on_hover_text(
                        "管理站点的面板访问令牌（PAT）：同一站点的 provider 共用一份\n\
                         只用于只读查询账号数据，不参与配置保存；存在 .modelharbor/tokens.json",
                    )
                    .clicked()
                {
                    self.show_tokens = !self.show_tokens;
                    if self.show_tokens {
                        // 重新打开时按已保存的值重填草稿（避免残留上次未保存的改动）。
                        self.token_draft.clear();
                        self.token_uid_draft.clear();
                    }
                }
            });
        });
        ui.separator();
    }
}
