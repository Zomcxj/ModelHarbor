//! 顶栏 / 状态栏 / 页头等外围栏位，以及吸顶标题与错误文本处理小工具。
use super::{App, SaveFormat};
use crate::app::preview::PREVIEW_EDITOR_ID;
use crate::app::save::PageTarget;
use crate::backends;
use crate::format::ConfigFormat;
use crate::theme::Theme;
use crate::ui::move_item;
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

/// 外观面板里的形状预览按钮：填强调色、按预设圆角，选中画 2px 高亮描边。
///
/// 选中描边**统一 2px**、与形状自身的 `border_width` 解耦——云朵这类
/// 「卡片不描边」的预设若沿用自身宽度，选中后就没有和其他形状一样的
/// 高亮圈。浅底用深描边、深底用白描边，保证任何主题下都看得清。
fn shape_button(
    ui: &mut egui::Ui,
    style: crate::theme::UiStyle,
    is_current: bool,
    current_accent: egui::Color32,
    dark: bool,
) -> bool {
    let bright =
        current_accent.r() as u32 + current_accent.g() as u32 + current_accent.b() as u32 > 384;
    let text_color = if bright {
        egui::Color32::BLACK
    } else {
        egui::Color32::WHITE
    };
    let stroke = if is_current {
        egui::Stroke::new(
            2.0f32,
            if dark {
                egui::Color32::WHITE
            } else {
                egui::Color32::from_gray(40)
            },
        )
    } else {
        egui::Stroke::new(style.border_width(), current_accent)
    };
    let btn = egui::Button::new(egui::RichText::new(style.label()).color(text_color))
        .fill(current_accent)
        .stroke(stroke)
        .min_size(egui::vec2(52.0, 0.0))
        .corner_radius(style.radius() as f32);
    ui.add(btn).clicked()
}

/// 吸顶条的 Y：还没滚过标题时钉在内容流位置，滚过之后钉在滚动区**可视顶**。
///
/// egui 会把滚动区裁剪顶向上扩 `clip_rect_margin`（默认 3px，
/// `content_clip_rect.min.y = inner_rect.min.y - margin`）。如果直接用
/// `clip_rect().top()`，标题要多滚 3px 才停住——看起来整行先往上抬一下。
/// 可视顶 = clip_top + margin，吸顶钉在它上面就纹丝不动。
/// 抽成纯函数是为了能直接断言这两条性质；`round` 收掉亚像素差。
pub(super) fn sticky_y(content_top: f32, clip_top: f32, clip_margin: f32) -> f32 {
    content_top.max(clip_top + clip_margin).round()
}

/// 绘制吸顶标题：未滚过时留在内容流中；滚动越过视口顶部后吸附在滚动区顶部。
pub(super) fn sticky_end(
    ui: &mut egui::Ui,
    anchor: (f32, f32, f32, f32),
    paint: impl FnOnce(&mut egui::Ui),
) {
    let (top, left, right, height) = anchor;
    let clip_top = ui.clip_rect().top();
    let y = sticky_y(top, clip_top, ui.visuals().clip_rect_margin);
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
    // 背景填充要从**裁剪顶**开始，不能只填 target：吸附后 target.top() =
    // clip_top + clip_margin，与裁剪顶还差那 3px；只填 target 会在顶上留一条缝，
    // 滚动内容从缝里透出来，看起来像整行透明。未吸附时（target.top() 还在
    // 内容流里）不需要补——缝里本来就是空白。
    let fill_top = if y > top { clip_top } else { target.top() };
    child.painter().rect_filled(
        egui::Rect::from_min_max(egui::pos2(target.left(), fill_top), target.max),
        0.0,
        ui.visuals().panel_fill,
    );
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
                // 已安装的排在前（可拖动换位），未安装的按名字首字母排在后面。
                let installed = |id: ConfigFormat| self.config_paths.validate_target(id);
                let order = self
                    .config_paths
                    .tab_order(&self.tab_order, |id| self.config_paths.validate_target(id));
                // 已安装的那一段：拖动只在它内部换位。
                let installed_count = order.iter().filter(|id| installed(**id)).count();
                let mut drop_on: Option<usize> = None;
                let mut released = false;
                let mut clicked_page: Option<ConfigFormat> = None;

                for (slot, id) in order.iter().copied().enumerate() {
                    let i = backends::BACKENDS
                        .iter()
                        .position(|b| b.id() == id)
                        .unwrap_or(0);
                    let is_installed = installed(id);
                    // 未安装的页面把图标调淡：egui 的 Button 没有 weak()，
                    // 用 Image 的 tint 压暗（保持同一个按钮形状，只改观感）。
                    // 已安装：WHITE = 乘以白 = 不改色（原图）。绝不能用
                    // Color32::PLACEHOLDER——它是魔法值 rgba(0,255,183,4)，
                    // 直接当 tint 会把图标乘成绿色（红通道归零）。
                    let is_selected = self.current_page == id;
                    // 正被抓着的页签（拖动源）。三种状态刻意三色，互不看混：
                    //   选中（我在这页）= 悬浮色 + 加粗描边，即「悬浮的加重版」；
                    //   拖动源（我抓着这页）= 橙色，与卡片拖动源同源；
                    //   换位目标（要换到这页）= 绿色，与卡片落点同源（在下面用画笔叠）。
                    let is_dragging = self.tab_drag_src == Some(id);
                    // 图标保持原色（已安装）或压淡（未安装）。**不能按选中态改 tint**：
                    // 先前改成「强调色上的文字色」，深色主题下那正好是黑色，图标直接变黑。
                    // 状态一律靠底色 + 描边表达，图标本身不参与。
                    let icon_tint = if is_installed {
                        egui::Color32::WHITE
                    } else {
                        ui.visuals().weak_text_color()
                    };
                    let btn = match icons.get(i).and_then(|o| o.as_ref()) {
                        Some(tex) => egui::Button::image(
                            egui::Image::from_texture(tex)
                                .fit_to_exact_size(egui::vec2(16.0, 16.0))
                                .tint(icon_tint),
                        ),
                        None => egui::Button::new(""),
                    };
                    // 选中态 = 「悬浮的加重版」：底色就用悬浮色（`widgets.hovered.bg_fill`，
                    // 未选中的页签悬停时也是这块底色），再把描边加粗到 2px 拉开层级。
                    // 不另起一套颜色——用户要求选中沿用悬浮色系，只加重。
                    let selected_fill = ui.visuals().widgets.hovered.bg_fill;
                    let selected_ring = ui.visuals().widgets.hovered.bg_stroke.color;
                    // 拖动源用橙色（与卡片拖动源同源）；换位目标的绿环在按钮画完后补画。
                    let btn = match (is_dragging, is_selected) {
                        (true, _) => btn
                            .fill(crate::ui::DRAG_SOURCE_FILL)
                            .stroke(egui::Stroke::new(2.0f32, crate::ui::DRAG_SOURCE_COLOR)),
                        (false, true) => {
                            btn.fill(selected_fill)
                                .stroke(egui::Stroke::new(2.0f32, selected_ring))
                        }
                        (false, false) => btn,
                    };
                    // 未安装的页面画淡一点，与已安装的区分开。
                    let tip = if is_installed {
                        format!("{}（已安装，可拖动换位）", id.label())
                    } else {
                        format!("{}（未安装）", id.label())
                    };
                    // click_and_drag：普通 Button 只感应点击，drag_started/stopped
                    // 永不触发，已安装页就拖不动。补上拖拽感应，点击仍照常工作。
                    let btn_resp = ui
                        .add(
                            btn.min_size(egui::vec2(24.0, 22.0))
                                .sense(egui::Sense::click_and_drag()),
                        )
                        .on_hover_text(tip);
                    // 光标：悬停张开手掌、按住握成拳头（与拖动把手同一套，见 crate::ui）。
                    // 不用 egui 的 Grab/Grabbing：Windows 上它们被 winit 映射成
                    // IDC_SIZEALL 四向箭头，看着像「可移动」而非抓取。
                    let want = crate::ui::grab_cursor_for(&btn_resp);
                    if want != crate::ui::GrabCursor::None {
                        crate::ui::request_grab_cursor(ui.ctx(), want);
                    }
                    let btn_resp = if want != crate::ui::GrabCursor::None {
                        btn_resp.on_hover_cursor(egui::CursorIcon::PointingHand)
                    } else {
                        btn_resp
                    };
                    if btn_resp.clicked() {
                        clicked_page = Some(id);
                    }
                    // 拖动换位：只认已安装段内的落点（未安装的位置是推导出来的，
                    // 允许拖进去会和「未安装按字母序」的规则打架）。
                    if is_installed && slot < installed_count {
                        if btn_resp.drag_started() {
                            self.tab_drag_src = Some(id);
                        }
                        // 换位目标：拖动中指针所在的槽位画绿环（与卡片落点同色）。
                        // 必须在按钮画完后补画——指针是否在本控件上要等 allocate 之后才定，
                        // 建按钮时还不知道会不会成为落点。
                        //
                        // **这里必须用 `contains_pointer()`，不能用 `hovered()`。** egui 在
                        // 「有指针按键按下且按下的不是本控件」时会**强制清掉** HOVERED
                        // （context.rs: `if input.pointer.any_down() && !is_interacted_with`），
                        // 而拖拽全程按着键、落点又不是被按下的那个控件——于是 `hovered()`
                        // 在拖动中恒为 false，绿环一次都画不出来，`drop_on` 也永远是 None，
                        // **换位整个功能是坏的**。`contains_pointer()` 正是 egui 为拖放落点
                        // 提供的判定（文档原话：即使别的控件正被拖动也可能为 true）。
                        // 用 `Inside` 与按钮自身的描边同几何（egui 的 `Frame` 就是 Inside），
                        // 换位目标与选中态的环粗细/位置才完全一致；`Outside` 会多出一圈。
                        if self.tab_drag_src.is_some() && btn_resp.contains_pointer() {
                            if self.tab_drag_src != Some(id) {
                                ui.painter().rect_stroke(
                                    btn_resp.rect,
                                    3.0,
                                    egui::Stroke::new(2.0f32, crate::ui::DROP_TARGET_COLOR),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            drop_on = Some(slot);
                        }
                        if btn_resp.drag_stopped() {
                            released = true;
                        }
                    }
                }

                // 拖动中指针往往已经离开被拖的那个页签（拖到别的槽位上方），
                // 光标不能退回默认箭头——整段拖动期间保持拳头，直到松手。
                if self.tab_drag_src.is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    crate::ui::request_grab_cursor(ui.ctx(), crate::ui::GrabCursor::Fist);
                }

                // 松手：把被拖的页面移到落点位置，落点即用户看到的那个槽位。
                if released {
                    if let (Some(src), Some(dst)) = (self.tab_drag_src, drop_on) {
                        let mut installed_ids: Vec<ConfigFormat> = order
                            .iter()
                            .copied()
                            .filter(|id| installed(*id))
                            .collect();
                        if let (Some(from), true) =
                            (installed_ids.iter().position(|id| *id == src), dst < installed_ids.len())
                        {
                            if from != dst {
                                move_item(&mut installed_ids, from, dst);
                                self.tab_order = installed_ids
                                    .iter()
                                    .map(|id| id.label().to_string())
                                    .collect();
                            }
                        }
                    }
                    self.tab_drag_src = None;
                }

                if let Some(id) = clicked_page {
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
                                        egui::Stroke::new(2.0f32, egui::Color32::WHITE)
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
                                        egui::Stroke::new(2.0f32, egui::Color32::from_gray(40))
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
                            let dark = ui.visuals().dark_mode;
                            // 第一行：前 4 个形状
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                for style in &crate::theme::UiStyle::ALL[0..4] {
                                    if shape_button(ui, *style, self.ui_style == *style, current_accent, dark) {
                                        self.ui_style = *style;
                                    }
                                }
                            });
                            // 第二行：后 4 个形状
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                for style in &crate::theme::UiStyle::ALL[4..8] {
                                    if shape_button(ui, *style, self.ui_style == *style, current_accent, dark) {
                                        self.ui_style = *style;
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
                    ui.label(
                        egui::RichText::new(&self.status)
                            .size(crate::theme::TEXT_SMALL)
                            .weak(),
                    );
                    // 右侧：当前页 + 数量统计，随时可见页面身份
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::BOTTOM), |ui| {
                        ui.label(
                            egui::RichText::new(format!(
                                "agents: {} | providers: {}",
                                self.agents.len(),
                                self.providers.len()
                            ))
                            .size(crate::theme::TEXT_SMALL)
                            .weak(),
                        );
                        ui.label(
                            egui::RichText::new(format!("当前页: {}", self.current_page.label()))
                                .size(crate::theme::TEXT_SMALL)
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
            // 一键保存：把同一份界面状态写到每个已安装后端的目标路径，
            // 免去逐页切换逐个点保存。放在「保存」左侧（先全局后本页）。
            // 按钮直接带目标数量：一次会写几个文件必须点之前就看得见——
            // 本页的 provider 集合与别的 agent 不一致时，这一下会把它们一起改掉。
            let installed: Vec<(String, String)> = self
                .targets
                .iter()
                .filter(|t| t.available && !t.path.trim().is_empty())
                .map(|t| (t.backend.label().to_string(), t.path.clone()))
                .collect();
            let tip = if installed.is_empty() {
                "本地与 WSL 都没探测到已安装的配置，没有可写目标".to_string()
            } else {
                let mut s = format!(
                    "一次写完以下 {} 个已安装后端（各自的目标路径）：\n",
                    installed.len()
                );
                for (label, path) in &installed {
                    s.push_str(&format!("· {label}: {path}\n"));
                }
                s.push_str(
                    "未安装的跳过；跨格式转换前会把目标文件原样备份为 .bak；\n结果逐页列在状态栏。",
                );
                s
            };
            if ui
                .button(format!("一键保存 ({})", installed.len()))
                .on_hover_text(tip)
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                self.save_all();
            }
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

#[cfg(test)]
mod tests {
    use super::sticky_y;

    #[test]
    fn sticky_stays_put_until_it_hits_the_clip() {
        let margin = 3.0; // egui 默认 clip_rect_margin
                          // 滚动为零时 clip_top = 内容顶 - margin：标题钉在内容流位置。
        assert_eq!(sticky_y(40.0, 40.0 - margin, margin), 40.0);
        // 刚滚过可视顶：钉在可视顶，一格都不多滚。
        assert_eq!(sticky_y(39.0, 40.0 - margin, margin), 40.0);
        assert_eq!(sticky_y(40.0, 52.0, margin), 55.0);
    }

    #[test]
    fn sticky_does_not_jump_for_a_subpixel_gap() {
        // 滚动区顶部曾经多留 4px，标题会从 4 跳到 0。
        // 现在内容顶就是可视顶，亚像素差也要收成同一格。
        assert_eq!(sticky_y(0.4, -3.0, 3.0), 0.0);
        assert_eq!(sticky_y(0.0, -2.6, 3.0), 0.0);
    }
}
