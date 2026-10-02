use super::toolbar_icon_button;
use crate::app::health;
use crate::app::preview::PREVIEW_EDITOR_ID;
use crate::app::App;
use crate::backends;
use crate::format::ConfigFormat;
use crate::theme::Theme;
use crate::ui::move_item;
use crate::util::show_file_dialog;
use eframe::egui;

/// 外观面板里的形状预览按钮：填强调色、按预设圆角，选中画 2px 高亮描边。
///
/// 选中描边**统一 2px**、与形状自身的 `border_width` 解耦——云朵这类
/// 外观弹层里的一行主题按钮：按钮用主题强调色填充、文字按底色取黑/白，
/// 当前主题加一圈描边（颜色由调用方按面板底色定）。返回本帧被点中的主题。
fn theme_button_row(
    ui: &mut egui::Ui,
    themes: &[Theme],
    current: Theme,
    radius: f32,
    ring: egui::Color32,
) -> Option<Theme> {
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for t in themes {
            let accent = t.accent_color();
            // 文字颜色：根据背景亮暗自动选择黑/白
            let text_color = if accent.r() as u32 + accent.g() as u32 + accent.b() as u32 > 384 {
                egui::Color32::BLACK
            } else {
                egui::Color32::WHITE
            };
            let stroke = if *t == current {
                egui::Stroke::new(2.0f32, ring)
            } else {
                egui::Stroke::NONE
            };
            let btn = egui::Button::new(egui::RichText::new(t.label()).color(text_color))
                .fill(accent)
                .stroke(stroke)
                .min_size(egui::vec2(52.0, 0.0))
                .corner_radius(radius);
            if ui.add(btn).clicked() {
                clicked = Some(*t);
            }
        }
    });
    clicked
}

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

impl App {
    pub(in crate::app) fn ui_top_bar(&mut self, ctx: &egui::Context) {
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
                    // 悬停提示只报页面名（未安装的补一句状态）。不写「可拖动换位」：
                    // 未安装的页签本来就是灰的、拖不动，写了反而要读者自己去对号。
                    let tip = if is_installed {
                        id.label().to_string()
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
                        //
                        // 圆角必须取**按钮自己的** `corner_radius`，不能写死。写死 3.0 时
                        // 云朵档的按钮圆角是 16，绿环就成了套在圆角按钮上的一个方框，
                        // 看着像另画了个矩形而不是「把边框换成绿色」。取同一个值，
                        // 绿环就精确压在按钮原有描边上，只换颜色、不改形状。
                        if self.tab_drag_src.is_some() && btn_resp.contains_pointer() {
                            if self.tab_drag_src != Some(id) {
                                ui.painter().rect_stroke(
                                    btn_resp.rect,
                                    ui.visuals().widgets.inactive.corner_radius,
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
                    // 离开的是哪一页：切页归一要先把它当前的 agent model 视图存下来，
                    // 否则切走再切回会把用户原来配好的值弄丢（见 normalize_agent_models_for_page）。
                    let previous_page = self.current_page;
                    if id == ConfigFormat::DeepSeekHarness
                        && self.current_page != ConfigFormat::DeepSeekHarness
                    {
                        self.project_dsh_credentials();
                    }
                    self.sync_provider_secrets(id);
                    // 对应 agent 未在 WSL 安装的页面：关闭并禁用 WSL 同步。
                    // 仅在同步已开启时才探测：wsl_target 首次调用会拉起 wsl 进程
                    // （启动 WSL 虚拟机），未开同步就探测 = 软件一开就占内存。
                    if self.sync_wsl && backends::wsl_target(id).is_none() {
                        self.sync_wsl = false;
                        crate::util::wsl_set_enabled(false);
                    }
                    self.current_page = id;
                    // WorkBuddy 页的数据可能来自别的方言（没有 `disabled` 概念，一律读成
                    // 启用）；进页时收敛成「每个 id 只启用第一条」（为什么见
                    // `normalize_workbuddy_enable_flags`）。
                    if id == ConfigFormat::WorkBuddy {
                        self.normalize_workbuddy_enable_flags();
                    }
                    // opencode 系三页共用同一份 agent 数据，但每页网关不同：agent 的
                    // `model` 前缀若指向别家网关（如把 opencode/… 带到 kilo 页），
                    // 目标页的网关根本不认，agent 跑不起来。进页即换成目标页自家网关的
                    // 首选模型——界面立即可见，保存时自然写入正确的值。
                    // 传入「刚离开的页面」：先把它的 model 视图存下来，否则切走再切回
                    // 会把用户原来配好的值弄丢（详见 `normalize_agent_models_for_page`）。
                    if id.is_opencode_family() {
                        let leaving = (previous_page != id).then_some(previous_page);
                        let replaced = self.normalize_agent_models_for_page(id, leaving);
                        if replaced > 0 {
                            self.status = format!(
                                "已把 {} 个指向其他网关的 agent model 换为 {} 自家模型",
                                replaced,
                                id.label()
                            );
                        }
                    }
                    // 切换页面后必须重建预览草稿：草稿只在「预览未聚焦且上次解析成功」时才
                    // 跟随组件状态，否则会停留在上一页的内容上（预览框仍有焦点或上次解析失败）。
                    self.reset_preview_draft();
                    ctx.memory_mut(|m| m.surrender_focus(egui::Id::new(PREVIEW_EDITOR_ID)));
                }
                ui.separator();
                // 右侧：WSL 同步 + 主题
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // WSL 探测**按需**：只在「同步已勾选」时才检测安装状态——
                    // wsl_target 首次调用会拉起 wsl 进程（启动 WSL 虚拟机），
                    // 未开启同步的启动 / 切页不应付出这份内存与启动开销。
                    let current = self.current_page;
                    let (wsl_installed, wsl_tip) = if self.sync_wsl {
                        let installed = backends::wsl_target(current).is_some();
                        let tip = if installed {
                            format!("保存时同步写入 WSL 侧 {} 的配置", current.label())
                        } else {
                            format!(
                                "WSL 中未检测到 {} 安装（配置文件或其目录均不存在），保存仅写 Windows 本地",
                                current.label()
                            )
                        };
                        (installed, tip)
                    } else {
                        (
                            true,
                            format!(
                                "勾选后保存时同步写入 WSL 侧 {} 的配置（首次勾选会检测 WSL 安装状态，会拉起 WSL）",
                                current.label()
                            ),
                        )
                    };
                    let wsl_cb = ui
                        .add_enabled(
                            wsl_installed,
                            egui::Checkbox::new(&mut self.sync_wsl, "WSL同步"),
                        )
                        .on_hover_text(wsl_tip);
                    // 勾选状态变化 → 同步 WSL 总闸（是否允许拉起 wsl 进程探测 / 读写）。
                    if wsl_cb.changed() {
                        crate::util::wsl_set_enabled(self.sync_wsl);
                    }
                    // 勾选后探测发现未安装：自动收回勾选并提示，避免复选框停在
                    // 「勾着但灰掉」的矛盾状态。
                    if self.sync_wsl && !wsl_installed {
                        self.sync_wsl = false;
                        crate::util::wsl_set_enabled(false);
                        self.status = format!(
                            "WSL 中未检测到 {} 安装，已关闭 WSL 同步",
                            current.label()
                        );
                    }
                    ui.separator();
                    let look_tint = ui.visuals().text_color();
                    let look_btn = toolbar_icon_button(
                        ui,
                        self.toolbar_icons.palette.as_ref(),
                        false,
                        look_tint,
                    )
                    .on_hover_text("外观")
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
                            // 两行按钮共用同一个绘制函数（见 theme_button_row）；
                            // 当前主题的描边色随行不同：深色行用白、亮色行用深灰，
                            // 各自与所在面板底色保持对比。
                            ui.label(egui::RichText::new("深色").size(10.0).weak());
                            if let Some(t) = theme_button_row(
                                ui,
                                &Theme::ALL[0..4],
                                self.theme,
                                current_radius,
                                egui::Color32::WHITE,
                            ) {
                                self.theme = t;
                            }
                            ui.label(egui::RichText::new("亮色").size(10.0).weak());
                            if let Some(t) = theme_button_row(
                                ui,
                                &Theme::ALL[4..8],
                                self.theme,
                                current_radius,
                                egui::Color32::from_gray(40),
                            ) {
                                self.theme = t;
                            }
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
                // 行内混排了 14px 小图标、较高的输入框与 24px 图标按钮。
                // egui 是即时模式：先放的矮控件按当时行高居中，后面高控件
                // 把行撞高后不会回溯重新居中，就会“靠上”。先把行高钉到 24，
                // 所有控件（含第一个小图标）就都基于同一高度垂直居中。
                ui.set_min_height(24.0);
                // 来源（后端 / agent）图标移到路径框前面（原「配置文件」标记位置）。
                // 只显示各后端官方图标（名称见悬停提示）。
                if let Some(icon) = self.icon_for(self.source_format) {
                    ui.add(
                        egui::Image::from_texture(icon)
                            .fit_to_exact_size(egui::vec2(14.0, 14.0)),
                    )
                    .on_hover_text(format!("来源：{}", self.source_format.label()));
                } else {
                    ui.label(egui::RichText::new(self.source_format.label()).weak());
                }
                let path_resp =
                    ui.add(egui::TextEdit::singleline(&mut self.config_path).desired_width(420.0));
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
                // 「浏览」按钮放回路径框右侧，点击弹出选文件对话框。
                let browse_tint = ui.visuals().text_color();
                if toolbar_icon_button(
                    ui,
                    self.toolbar_icons.folder_open.as_ref(),
                    false,
                    browse_tint,
                )
                .on_hover_text("浏览…选择配置文件")
                .clicked()
                {
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
                let fmt_wk = ui.visuals().weak_text_color();
                match self.toolbar_icons.file_output.as_ref() {
                    Some(tex) => {
                        ui.add(
                            egui::Image::from_texture(tex)
                                .fit_to_exact_size(egui::vec2(14.0, 14.0))
                                .tint(fmt_wk),
                        )
                        .on_hover_text("保存格式");
                    }
                    None => {
                        ui.label("保存格式:");
                    }
                }
                let format_btn = ui.button(self.save_format.label());
                if format_btn.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let hovering = format_btn.hovered();
                let scroll = ui
                    .input(|i| i.events.iter().any(|e| matches!(e, egui::Event::MouseWheel { .. })));
                // 滚轮切换：一次连续滚动手势只切换一次，避免快速滚动时来回翻转
                if hovering && scroll && !self.save_format_wheel_latch {
                    self.save_format = self.save_format.toggled();
                    self.save_format_wheel_latch = true;
                }
                if !scroll || !hovering {
                    self.save_format_wheel_latch = false;
                }
                if format_btn.clicked() {
                    self.save_format = self.save_format.toggled();
                }

                // 顶部工具组：体检、令牌、密钥显隐和右侧预览面板开关。
                // 右对齐后越晚添加的控件越靠左，保持侧边栏图标贴近工具组外侧。
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let sidebar = toolbar_icon_button(
                        ui,
                        self.toolbar_icons.panel_right.as_ref(),
                        self.show_preview,
                        ui.visuals().text_color(),
                    )
                    .on_hover_text(if self.show_preview {
                        "关闭侧边栏"
                    } else {
                        "打开侧边栏"
                    });
                    if sidebar.clicked() {
                        self.show_preview = !self.show_preview;
                        if self.show_preview {
                            // 打开时以组件状态重建待保存文档。
                            self.reset_preview_draft();
                        }
                    }

                    // 代理支持：放行 / 禁用「模型延迟测试」走系统代理；亮起 = 已放行。
                    // 检测详情折进悬停提示（原 Providers 标题行里的开关 + 独立文字标签
                    // 随之取消，见 providers/section.rs）。
                    let mut proxy_tip = if self.allow_model_test_with_proxy {
                        "代理支持：已放行「模型延迟测试」。".to_string()
                    } else {
                        "代理支持：放行后即使检测到代理 / VPN 也允许「模型延迟测试」。默认关闭。".to_string()
                    };
                    if let Some(reason) = &self.net_guard {
                        proxy_tip.push_str(&format!("\n当前检测：{reason}"));
                    }
                    proxy_tip.push_str(
                        "\n中转站普遍有多 IP 检测 / 测活风控，经代理做推理探测可能被封号；\n\
                         「连通性测试」不做推理，不受影响。",
                    );
                    let proxy = toolbar_icon_button(
                        ui,
                        self.toolbar_icons.globe.as_ref(),
                        self.allow_model_test_with_proxy,
                        ui.visuals().text_color(),
                    )
                    .on_hover_text(proxy_tip);
                    if proxy.clicked() {
                        self.allow_model_test_with_proxy = !self.allow_model_test_with_proxy;
                        // 状态栏即时回声（旧标题行开关有文字标签，搬进顶栏后动作要有反馈）。
                        self.status = if self.allow_model_test_with_proxy {
                            "代理支持：已放行「模型延迟测试」".to_string()
                        } else {
                            "代理支持：已恢复拦截「模型延迟测试」".to_string()
                        };
                    }

                    // 全局密钥显隐：一键切换全部 API Key 的明文 / 掩码。
                    let api_keys = toolbar_icon_button(
                        ui,
                        if self.show_api_keys {
                            self.toolbar_icons.eye.as_ref()
                        } else {
                            self.toolbar_icons.eye_off.as_ref()
                        },
                        self.show_api_keys,
                        ui.visuals().text_color(),
                    )
                    .on_hover_text(if self.show_api_keys {
                        "隐藏密钥"
                    } else {
                        "显示密钥"
                    });
                    if api_keys.clicked() {
                        self.show_api_keys = !self.show_api_keys;
                    }

                    // 令牌：站点面板访问令牌（PAT）管理；填了才能查账号级真实余额。
                    let tokens = toolbar_icon_button(
                        ui,
                        self.toolbar_icons.key_round.as_ref(),
                        self.show_tokens,
                        ui.visuals().text_color(),
                    )
                    .on_hover_text("令牌");
                    if tokens.clicked() {
                        self.show_tokens = !self.show_tokens;
                        if self.show_tokens {
                            // 重新打开时按已保存的值重填草稿，避免残留上次未保存的改动。
                            self.token_draft.clear();
                            self.token_uid_draft.clear();
                        }
                    }

                    // 体检：把重名 / 非法数字 / 可疑 URL / 跨网关引用等检查汇总成一张清单。
                    let (blockers, warnings) =
                        health::count_by_severity(&health::collect(&health::HealthInput {
                            page: self.current_page,
                            providers: &self.providers,
                            agents: &self.agents,
                            source_is_opencode: self.source_format.is_opencode_family(),
                        }));
                    let semantics = crate::theme::semantics(ui);
                    let (count, color) = if blockers > 0 {
                        (blockers, semantics.err)
                    } else if warnings > 0 {
                        (warnings, semantics.warn)
                    } else {
                        (0, ui.visuals().text_color())
                    };
                    let health_button = toolbar_icon_button(
                        ui,
                        self.toolbar_icons.activity.as_ref(),
                        count > 0,
                        color,
                    )
                    .on_hover_text("体检");
                    if health_button.clicked() {
                        self.show_health = !self.show_health;
                    }
                });
            });
        });
    }
}
