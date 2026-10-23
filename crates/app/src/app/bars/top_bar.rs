use super::toolbar_icon_button;
use crate::app::{health, preview::PREVIEW_EDITOR_ID, show_file_dialog, App, MainView};
use crate::backends;
use crate::format::ConfigFormat;
use crate::theme::Theme;
use crate::ui::move_item;
use eframe::egui;

/// One theme button row in the appearance popover: filled with the theme's accent, text picks black/white by background,
/// and the current theme gets a `ring` border. Returns the theme clicked this frame.
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
            // Text color: black/white chosen automatically by background lightness
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

/// The "no card borders" preset button: drawn with the preset corner radius and border width.
/// When selected the border is fixed at 2px: dark border on light fills, white on dark.
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
    /// Switch to an agent's config page.
    ///
    /// Extracted from `ui_top_bar`'s tab clicks: that spot was one big inline block, and after adding the
    /// "tabs act as filters under the usage view" branch, the two paths tangled into unreadability.
    fn switch_to_page(&mut self, id: ConfigFormat, ctx: &egui::Context) {
        // Which page is being left: page-switch normalization needs the page just left.
        let previous_page = self.current_page;
        if id == ConfigFormat::DeepSeekHarness && self.current_page != ConfigFormat::DeepSeekHarness
        {
            self.project_dsh_credentials();
        }
        self.sync_provider_secrets(id);
        // Pages whose agent isn't installed on WSL: turn WSL sync off.
        // The checkbox is withdrawn only on a **confirmed** not-installed verdict; `Unknown` doesn't count.
        if self.sync_wsl && backends::wsl_target_state(id) == backends::WslTargetState::NotInstalled
        {
            self.sync_wsl = false;
            crate::util::wsl_set_enabled(false);
        }
        self.current_page = id;
        // The WorkBuddy page: on entry, converge to "only the first entry per id enabled".
        if id == ConfigFormat::WorkBuddy {
            self.normalize_workbuddy_enable_flags();
        }
        // The three opencode-family pages share one agent dataset but have different gateways: on entry, swap the
        // agent's `model` to the target page's own gateway preferred model. Pass the page just left to save its
        // model view first.
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
        // Rebuild the preview draft after the page switch.
        self.reset_preview_draft();
        ctx.memory_mut(|m| m.surrender_focus(egui::Id::new(PREVIEW_EDITOR_ID)));
    }

    pub(in crate::app) fn ui_top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.style_mut().spacing.interact_size.y = 18.0;
            // First row: page switch / source / right-side WSL sync + theme
            ui.horizontal(|ui| {
                let icons: Vec<Option<egui::TextureHandle>> = self.backend_icons.clone();
                // Installed pages first (drag to reorder); uninstalled ones trail alphabetically.
                let installed = |id: ConfigFormat| self.config_paths.validate_target(id);
                let order = self
                    .config_paths
                    .tab_order(&self.tab_order, |id| self.config_paths.validate_target(id));
                // The installed segment: dragging reorders only within it.
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
                    // Uninstalled pages dim their icons: darkened via `Image`'s tint.
                    // Installed ones use `WHITE` (multiplying by white = the original image).
                    let is_selected = self.current_page == id;
                    // The tab being grabbed (drag source). One color per state:
                    //   selected (I'm on this page) = hover color + thicker border;
                    //   drag source (I'm holding this page) = orange;
                    //   reorder target (moving onto this page) = green (painted over below).
                    let is_dragging = self.tab_drag_src == Some(id);
                    // Icons keep their original color (installed) or stay dimmed (not); the tint doesn't follow selection.
                    // State is expressed by fill + border.
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
                    // Selected = hover-tinted fill + 2px border.
                    let selected_fill = ui.visuals().widgets.hovered.bg_fill;
                    let selected_ring = ui.visuals().widgets.hovered.bg_stroke.color;
                    // The drag source turns orange; the reorder target's green ring paints after the button.
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
                    // The hover tip reports only the page name (uninstalled ones add a status note).
                    let tip = if is_installed {
                        id.label().to_string()
                    } else {
                        format!("{}（未安装）", id.label())
                    };
                    // click_and_drag: a plain Button senses clicks only — add drag sensing.
                    let btn_resp = ui
                        .add(
                            btn.min_size(egui::vec2(24.0, 22.0))
                                .sense(egui::Sense::click_and_drag()),
                        )
                        .on_hover_text(tip);
                    // Cursor: open palm on hover, closed fist while held (same set as drag handles — see crate::ui).
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
                    // Drag reorder: only drop points inside the installed segment count.
                    if is_installed && slot < installed_count {
                        if btn_resp.drag_started() {
                            self.tab_drag_src = Some(id);
                        }
                        // Reorder target: the slot under the pointer during a drag gets a green ring (same color as card drop targets).
                        // Painted after the button.
                        //
                        // Uses `contains_pointer()`, not `hovered()`. `Inside` shares geometry with the button's own border;
                        // the corner radius is the **button's own** `corner_radius`.
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

                // During a drag the pointer has usually left the grabbed tab; keep the fist cursor for the whole drag
                // until release.
                if self.tab_drag_src.is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    crate::ui::request_grab_cursor(ui.ctx(), crate::ui::GrabCursor::Fist);
                }

                // Release: move the dragged page to the drop position.
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
                    // Under the usage view, tabs are **filters**, not page switches: clicking an agent views that agent's
                    // usage without touching the config page's state (returning to provider management shows the original page).
                    if self.main_view == MainView::Usage {
                        self.usage_filter = Some(id);
                    } else {
                        self.switch_to_page(id, ctx);
                    }
                }
                // The view-switch strip: immediately right of the agent tabs (the user-requested position);
                // "Provider Management" first (the two buttons swapped positions).
                ui.separator();
                self.ui_view_switch(ui);
                // "All": returns the usage view's filter to unfiltered.
                // Shown only under the usage view — provider management has no "all" concept.
                // Grouped with the agent tabs: both are "which slice of data" filters.
                if self.main_view == MainView::Usage {
                    ui.separator();
                    if crate::app::usage::chip(ui, self.usage_filter.is_none(), "全部", 48.0) {
                        self.usage_filter = None;
                    }
                }
                ui.separator();

                // Right side: WSL sync + theme
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // The WSL probe runs on a **background thread** (started only after checking the box); the UI doesn't wait.
                    let current = self.current_page;
                    let probe_pending = self.sync_wsl && backends::wsl_probe_pending();
                    let (wsl_installed, wsl_tip) = if self.sync_wsl {
                        match backends::wsl_target_state(current) {
                            backends::WslTargetState::Installed(_) => (
                                true,
                                format!("保存时同步写入 WSL 侧 {} 的配置", current.label()),
                            ),
                            backends::WslTargetState::NotInstalled => (
                                false,
                                format!(
                                    "WSL 中未检测到 {} 安装（配置文件或其目录均不存在），保存仅写 Windows 本地",
                                    current.label()
                                ),
                            ),
                            // While probing: the checkbox stays usable and updates itself when the result arrives.
                            backends::WslTargetState::Unknown => (
                                true,
                                format!(
                                    "正在检测 WSL 侧的 {} 安装状态…（检测完成前保存只写 Windows 本地）",
                                    current.label()
                                ),
                            ),
                        }
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
                    // Checkbox change → sync the WSL master switch (whether wsl processes may spawn for probing / IO).
                    if wsl_cb.changed() {
                        crate::util::wsl_set_enabled(self.sync_wsl);
                    }
                    // Probe in flight: spinner note + repaint request.
                    if probe_pending {
                        ui.add(egui::Spinner::new().size(12.0));
                        ctx.request_repaint_after(std::time::Duration::from_millis(120));
                    }
                    // After checking, the probe **confirms** not installed: withdraw the check automatically with a note.
                    if self.sync_wsl && !wsl_installed {
                        self.sync_wsl = false;
                        crate::util::wsl_set_enabled(false);
                        self.status = format!(
                            "WSL 中未检测到 {} 安装，已关闭 WSL 同步",
                            current.label()
                        );
                    }
                    ui.separator();
                    // The resident-taskbar toggle (left of the theme): lit = clicking X only tucks into the tray.
                    // Resident-by-default, lit (user request).
                    let resident_tint = ui.visuals().text_color();
                    let resident_btn = toolbar_icon_button(
                        ui,
                        self.toolbar_icons.pin.as_ref(),
                        self.tray_resident,
                        resident_tint,
                    )
                    .on_hover_text(if self.tray_resident {
                        "最小化到任务栏"
                    } else {
                        "退出程序"
                    })
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                    if resident_btn.clicked() {
                        self.tray_resident = !self.tray_resident;
                        #[cfg(target_os = "windows")]
                        if self.tray_resident {
                            crate::tray::ensure();
                        } else {
                            crate::tray::teardown();
                        }
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
                        // The menu becomes a left-aligned ordinary vertical layout, each item's fill hugging its text width.
                        .layout(egui::Layout::top_down(egui::Align::LEFT))
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            ui.set_min_width(240.0);
                            ui.label(egui::RichText::new("主题").small().weak());
                            // The current selection's shape corner radius, shared by all buttons
                            let current_radius = self.ui_style.radius() as f32;
                            // The current theme's accent (for shape-button fills)
                            let current_accent = self.theme.accent_color();
                            // Both button rows share one drawing function (see theme_button_row);
                            // the current theme's border color differs per row: dark rows use white, light rows dark gray.
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
                            // First row: the first 4 shapes
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                for style in &crate::theme::UiStyle::ALL[0..4] {
                                    if shape_button(ui, *style, self.ui_style == *style, current_accent, dark) {
                                        self.ui_style = *style;
                                    }
                                }
                            });
                            // Second row: the last 4 shapes
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                for style in &crate::theme::UiStyle::ALL[4..8] {
                                    if shape_button(ui, *style, self.ui_style == *style, current_accent, dark) {
                                        self.ui_style = *style;
                                    }
                                }
                            });
                            ui.add_space(crate::theme::SPACE_2);
                            // Acrylic: orthogonal to palette and shape, effective for every theme.
                            // Internal identifiers (`prefs.glass`, the `GLASS_*` constants) keep their names.
                            ui.checkbox(&mut self.glass, "亚克力");
                        });
                });
            });
            // Second row: config file / save format
            ui.horizontal(|ui| {
                // The row mixes 14px small icons, a taller input box, and 24px icon buttons.
                // Pin the row height to 24 first; every control vertically centers on that.
                ui.set_min_height(24.0);
                // Source (backend / agent) icons: only each backend's official icon shows (names in hover tips).
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
                // Enter confirms: reload from the currently typed path (egui single-line edits lose focus on Enter)
                if path_resp.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && self.config_path != self.loaded_path
                {
                    if self.config_path.trim().is_empty() {
                        // Empty + Enter = clear this page's path override, returning to the default path
                        let page = self.current_page;
                        self.reset_page_path(page);
                    } else {
                        self.reload();
                    }
                }
                // The "browse" button sits right of the path box, popping the file dialog on click.
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
                // Scroll-wheel switching: one continuous gesture switches once.
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

                // The top tool group: health, tokens, key visibility, and the preview panel toggle.
                // Right-aligned, later-added controls land further left, keeping the sidebar icon adjacent to the group.
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
                            // While open, rebuild the pending document from component state.
                            self.reset_preview_draft();
                        }
                    }

                    // Proxy support: allow / forbid "model latency tests" through the system proxy; lit = allowed.
                    let mut proxy_tip = if self.allow_model_test_with_proxy {
                        "代理支持：已放行「模型延迟测试」。".to_string()
                    } else {
                        "代理支持：放行后即使检测到代理 / VPN 也允许「模型延迟测试」。默认关闭。".to_string()
                    };
                    if let Some(reason) = &self.net_guard {
                        proxy_tip.push_str(&format!("\n当前检测：{reason}"));
                    } else {
                        proxy_tip.push_str(
                            "\n当前未检测到代理 / VPN：此开关暂无影响，查询与测试照常可用。\n\
                             检测到后（每 5 秒复查）未放行会拦截「模型延迟测试」。",
                        );
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
                        // Status-bar echo.
                        self.status = if self.allow_model_test_with_proxy {
                            "代理支持：已放行「模型延迟测试」".to_string()
                        } else {
                            "代理支持：已恢复拦截「模型延迟测试」".to_string()
                        };
                        if self.net_guard.is_none() {
                            self.status
                                .push_str("（当前未检测到代理 / VPN，开关暂无实际影响）");
                        }
                    }

                    // Global key visibility: one switch for every API key's plaintext / masked display.
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

                    // Tokens: station panel access token (PAT) management; required for account-level real balances.
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
                            // Clear drafts when reopening.
                            self.token_draft.clear();
                            self.token_uid_draft.clear();
                        }
                    }

                    // Health: gathers duplicate names / invalid numbers / suspicious URLs / cross-gateway references into one list.
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
