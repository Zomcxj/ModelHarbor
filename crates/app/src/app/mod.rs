use crate::backends;
use crate::format::{ConfigFormat, ConfigPaths};
use crate::model::{AgentRow, ProviderRow};
use crate::theme::Theme;
use eframe::egui;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

mod agents;
mod balance;
pub(crate) mod bars;
mod collapse;
mod diff;
mod drag;
mod fetch;
mod health;
mod ids;
mod lifecycle;
mod preview;
mod providers;
mod providers_form;
mod save;
mod syntax;
pub(in crate::app) mod usage;
mod windows;

use fetch::{FreeModelsState, LatencyState, ProbeGate};
use save::SaveTarget;

pub use save::{load_opencode_result, load_or_empty, strip_cross_format_containers};

/// Pick a config file (JSON / YAML / TOML). Depends on rfd, which is UI — hence app, not core.
pub(crate) fn show_file_dialog() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("选择配置文件")
        .add_filter(
            "配置文件（JSON / YAML / TOML）",
            crate::util::CONFIG_FILE_EXTENSIONS,
        )
        .add_filter("JSON", &["json", "jsonc"])
        .add_filter("YAML", &["yml", "yaml"])
        .add_filter("TOML", &["toml"])
        .add_filter("所有文件", &["*"])
        .pick_file()
        .map(|p| p.to_string_lossy().to_string())
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum SaveFormat {
    Current,
    #[default]
    Compact,
}

impl SaveFormat {
    /// Switch to the other save format.
    fn toggled(self) -> Self {
        match self {
            SaveFormat::Current => SaveFormat::Compact,
            SaveFormat::Compact => SaveFormat::Current,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Current => "默认格式",
            Self::Compact => "压缩格式",
        }
    }

    /// Stable identifier for persistence.
    fn key(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Compact => "compact",
        }
    }

    /// Restore from a persistence identifier; unknown / empty values fall back to the default format.
    fn from_key(key: &str) -> SaveFormat {
        match key {
            "current" => Self::Current,
            _ => Self::default(),
        }
    }
}

/// Which slice the central area shows: usage statistics or provider management.
///
/// This is a **view** switch, orthogonal to the top bar's `current_page` (agent tabs): switching tabs
/// doesn't change the view, and switching views doesn't change the tab.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(in crate::app) enum MainView {
    /// Provider management (the existing UI, default).
    #[default]
    Providers,
    /// Local usage, four-dimension statistics.
    Usage,
}

impl MainView {
    fn label(self) -> &'static str {
        match self {
            Self::Providers => "提供商管理",
            Self::Usage => "用量",
        }
    }
}

pub struct App {
    root: Value,
    agents: Vec<AgentRow>,
    providers: Vec<ProviderRow>,
    new_agent: AgentRow,
    new_provider: ProviderRow,
    config_path: String,
    /// The path actually loaded last; treated as "not loaded" when it differs from `config_path`.
    loaded_path: String,
    status: String,
    show_new_agent: bool,
    show_new_provider: bool,
    /// Collapsed cards (`page/category/name`, see `prefs::collapsed_id`).
    /// Newly loaded cards default to expanded.
    collapsed: HashSet<String>,
    variant_open: HashSet<String>,
    agent_drag_src: Option<String>,
    agent_drag_target: Option<String>,
    provider_drag_src: Option<String>,
    provider_drag_target: Option<String>,
    model_drag_src: Option<String>,
    model_drag_target: Option<String>,
    /// The top-bar page currently being dragged (reordering within the installed segment).
    tab_drag_src: Option<ConfigFormat>,
    /// Each provider's "discover models" state (key → state; existing = the result panel is open).
    discovery: HashMap<String, fetch::DiscoveryState>,
    /// Discovery-result cache: the same base_url within 24 hours hits directly, no request (holds no credentials).
    discovery_cache: model_harbor_core::discovery::DiscoveryCache,
    /// Each backend's built-in gateway free models (backend → state).
    ///
    /// opencode and kilocode each have their own gateway and list; mimocode has no free tier (absent from
    /// this table). Startup fills from the persisted cache; only missing or expired entries refetch in the background.
    free_models: HashMap<ConfigFormat, FreeModelsState>,
    /// Each opencode-family page's **own** agent model view memory:
    /// **(file identity, page) → agent key → model**.
    ///
    /// Written only when **leaving** a page; the current page's authoritative value is always `agents` itself.
    /// Keys carry [`App::config_id`] (file identity); after switching files, mismatched old memories are skipped.
    agent_models_by_page: HashMap<(String, ConfigFormat), HashMap<String, String>>,
    /// Backends needing an automatic background fetch on the first frame (cache missing / expired); cleared once fetched.
    free_models_auto: Vec<ConfigFormat>,
    /// Each provider's latency-test state (key → state).
    latency: HashMap<String, LatencyState>,
    /// Each provider's "used / balance" query state (key → state).
    balance: HashMap<String, balance::BalanceState>,
    /// A "query user data for all" batch is running: one summary lands in the status bar when everything finishes.
    balance_batch: bool,
    /// Station panel access tokens (PAT, `.modelharbor/tokens.json`): station-level, shared by several providers.
    tokens: crate::tokens::StationTokens,
    /// Whether the token management panel is expanded.
    show_tokens: bool,
    /// Text being edited in the token panel (key = station origin), unsaved drafts.
    token_draft: HashMap<String, String>,
    /// Drafts of the token panel's "user ID" (the `New-Api-User` header; may stay empty).
    token_uid_draft: HashMap<String, String>,
    /// Stations whose "show" was clicked (per-row mask toggle, defaulting to the global "show keys").
    token_reveal: HashSet<String>,
    /// Throttle and serialization state for model latency probes (in-memory only, reset on restart).
    probe: ProbeGate,
    /// Network-guard verdict: `Some(reason)` = a system proxy / VPN was detected and model latency tests are disabled.
    net_guard: Option<String>,
    /// Whether model latency tests are still allowed when a proxy is detected (from settings.json; block by default).
    allow_model_test_with_proxy: bool,
    /// Whether the first-use guidance bar was dismissed (from settings.json).
    guide_dismissed: bool,
    /// Whether the config health popover is open (see [`crate::app::health`]).
    show_health: bool,
    /// Which slice the central area currently shows (usage / provider management).
    main_view: MainView,
    /// The usage view's time range (today / week / month / all).
    usage_range: usage::range::Range,
    /// The usage view's agent filter: `None` = all agents.
    ///
    /// Stored separately from `current_page`: clicking a tab under the usage view is a filter and must not
    /// touch the config page's state (switching back to provider management should show the page the user left).
    usage_filter: Option<ConfigFormat>,
    /// The trend chart's mode (bars / lines, bars by default).
    usage_chart_kind: usage::ChartKind,
    /// The trend chart's own time range (7 / 30 / 90 days / a year / all).
    ///
    /// Separate from the global `usage_range`: the chart answers "the recent trend", where a fixed window fits
    /// better than natural ranges like today / week; those still govern the overview card and the four tables.
    usage_chart_range: usage::range::ChartRange,
    /// Session-table rows where "delete" was clicked but not yet confirmed (ledger keys).
    ///
    /// Two-step confirmation against mis-clicks: first click arms, clicking the same row again really deletes.
    usage_delete_pending: Option<String>,
    /// Source-side errors while deleting a session (process running / path gone / database write failed).
    ///
    /// When the source can't be deleted, the ledger still withdraws details (so the UI never sticks at
    /// "deleted but came back"), so this notice and "removed from statistics" hold simultaneously.
    usage_source_error: Option<String>,
    /// The usage tables' sort state (shared across dimensions: default "total descending").
    usage_sort: usage::view::SortState,
    /// Local usage scan state (see [`crate::app::usage`]).
    usage: usage::UsageState,
    /// UI shape presets (default corner radius + border width).
    ui_style: crate::theme::UiStyle,
    /// Acrylic: translucent panels / cards + DWM acrylic blur (orthogonal to theme and shape).
    glass: bool,
    /// The top bar's installed-page drag order (backend identifiers; unlisted ones append alphabetically).
    tab_order: Vec<String>,
    /// Resident taskbar: clicking X only tucks the main window into the tray; the tray icon can restore / quit.
    ///
    /// On by default (user asked for resident-by-default); the toggle sits left of the top bar's appearance button.
    tray_resident: bool,
    /// When the network guard last checked (egui seconds).
    net_guard_at: f64,
    theme: Theme,
    /// The theme already applied to egui (theme + shape + acrylic).
    applied_theme: Option<(Theme, crate::theme::UiStyle, bool)>,
    /// The palette currently on screen (the cross-fade blends from it).
    applied_palette: Option<crate::theme::Palette>,
    /// An in-flight theme cross-fade: the previous palette plus the start time (egui seconds).
    theme_fade: Option<(crate::theme::Palette, f64)>,
    save_format: SaveFormat,
    /// The scroll-wheel format-switch latch: one continuous scroll gesture switches once.
    save_format_wheel_latch: bool,
    source_format: ConfigFormat,
    config_paths: ConfigPaths,
    targets: Vec<SaveTarget>,
    current_page: ConfigFormat,
    sync_wsl: bool,
    /// Global API-key visibility: one switch for every key input's plaintext/masked display.
    show_api_keys: bool,
    /// The persisted UI-preferences snapshot; differs from current values → write to disk.
    prefs_saved: crate::prefs::Prefs,
    /// Whether the right-side config preview / editor panel is open.
    show_preview: bool,
    /// The preview panel's width as a share of window width (drag the splitter; window resizes scale by this ratio).
    preview_ratio: f32,
    /// Whether the preview text box holds focus (while editing, text wins; after blur, component state wins).
    preview_focused: bool,
    /// The preview text box buffer (the pending document; rewritten live from component state on blur).
    preview_draft: String,
    /// The moment of the last text edit (ctx time), for debounced auto-save.
    preview_dirty_at: Option<f64>,
    /// The frame time of the user's most recent input in the preview box.
    preview_edit_at: Option<f64>,
    /// Whether the last text parse succeeded; on failure nothing is written to disk nor over the text.
    preview_parse_ok: bool,
    /// The last preview parse failure's error (`None` on success), for the in-panel red hint.
    preview_parse_error: Option<String>,
    /// Preview Ctrl+F find: query, open flag, current hit index.
    preview_find: String,
    preview_find_active: bool,
    preview_find_index: usize,
    /// The find box needs focus stolen next frame.
    preview_find_focus: bool,
    /// The hit's byte offset pending a jump (Enter / button jumps and scrolls the cursor there).
    preview_find_jump: Option<usize>,
    /// Diff view: shows "target file on disk → pending document" line-by-line changes.
    ///
    /// Read-only: the text box doesn't render in diff mode.
    preview_diff_mode: bool,
    /// Diff result cache (signature → display lines and stats).
    preview_diff_cache: Option<(u64, Vec<diff::DiffLine>, diff::DiffSummary)>,
    /// The diff signature pending recomputation and its first-seen moment (see `ui_preview_diff`'s debounce).
    preview_diff_pending: Option<(u64, f64)>,
    /// Successful disk-write count; folded into the diff cache signature — a write invalidates the cache.
    save_serial: u64,
    /// The cursor's line (1-based; the last position is kept on blur).
    preview_cursor_line: usize,
    load_error: Option<String>,
    pi_extras: Value,
    /// Each backend's official icon textures (aligned with BACKENDS order; lazily loaded on the first frame).
    backend_icons: Vec<Option<egui::TextureHandle>>,
    /// Top toolbar icon textures (Lucide SVG rasterized; lazily loaded on the first frame).
    toolbar_icons: ToolbarIcons,
    /// Each target file's last auto-snapshot content hash (path → hash), for "external modification" detection.
    snapshot_hashes: HashMap<String, String>,
    /// Auto-snapshot directory override (tests inject a tempdir; None = backups/ under the config dir).
    snapshots_root: Option<std::path::PathBuf>,
}

/// Startup dialect detection: **file content** wins; the page owning the path is only a fallback.
///
/// Keeps the page-based inference when the file can't be read.
/// Startup page: follows the top bar's "installed" segment display order (drag to reorder), taking the
/// first installed page.
///
/// Sorts with the same `tab_order` as the top bar — what you see is what loads: whichever tab the user
/// dragged to the front is the one whose default path loads at startup. With no installed pages it
/// returns `None` and the caller falls back to content sniffing. The detection predicate is injectable
/// so tests can simulate installed sets.
fn startup_page_with(
    paths: &ConfigPaths,
    tab_order: &[String],
    installed: impl Fn(ConfigFormat) -> bool,
) -> Option<(ConfigFormat, String)> {
    paths
        .tab_order(tab_order, &installed)
        .into_iter()
        .find(|id| installed(*id))
        .map(|id| (id, backends::target_path(id, &paths.local_path(id))))
}

impl Default for App {
    fn default() -> Self {
        let prefs = crate::prefs::Prefs::load();
        // Paths: every page uses the official default path. Overrides no longer persist (restart returns to defaults);
        // switching to another file within a session takes effect immediately and saves to that file (user decision).
        let paths = ConfigPaths::default();
        // The startup page follows the top bar's "installed" segment drag order: whoever is first loads.
        // No more content-based page switching: the first agent always opens its own page (user decision).
        // With no installed pages, fall back to the old sniffing.
        let (format, path) =
            startup_page_with(&paths, &prefs.tab_order, |id| paths.validate_target(id))
                .or_else(ConfigPaths::detect)
                .unwrap_or((ConfigFormat::Opencode, String::new()));
        // Each backend's built-in gateway free models: read the persisted cache once per backend; missing or expired
        // ones are refetched in the background on the first frame. State is created only for backends with a real free tier.
        let mut free_models: HashMap<ConfigFormat, FreeModelsState> = HashMap::new();
        let mut free_models_auto: Vec<ConfigFormat> = Vec::new();
        for format in [
            ConfigFormat::Opencode,
            ConfigFormat::Kilocode,
            ConfigFormat::Mimocode,
        ] {
            if crate::opencode_models::source_for(format).is_none() {
                continue;
            }
            match crate::opencode_models::load_cache(format) {
                Some(cache) => {
                    if !cache.fresh {
                        free_models_auto.push(format);
                    }
                    free_models.insert(
                        format,
                        FreeModelsState {
                            models: cache.models,
                            ..FreeModelsState::default()
                        },
                    );
                }
                None => {
                    free_models_auto.push(format);
                    free_models.insert(format, FreeModelsState::default());
                }
            }
        }
        let mut app = Self {
            root: Value::Object(Map::new()),
            agents: Vec::new(),
            providers: Vec::new(),
            new_agent: AgentRow::new(),
            new_provider: ProviderRow::new(),
            config_path: path,
            loaded_path: String::new(),
            status: String::new(),
            show_new_agent: false,
            show_new_provider: false,
            collapsed: prefs.collapsed.iter().cloned().collect(),
            variant_open: HashSet::new(),
            agent_drag_src: None,
            agent_drag_target: None,
            provider_drag_src: None,
            tab_drag_src: None,
            provider_drag_target: None,
            model_drag_src: None,
            model_drag_target: None,
            discovery: HashMap::new(),
            discovery_cache: model_harbor_core::discovery::DiscoveryCache::default(),
            // Free models: the persisted cache first; missing / expired ones are filled by the first frame's background refresh.
            free_models,
            free_models_auto,
            agent_models_by_page: HashMap::new(),
            latency: HashMap::new(),
            balance: HashMap::new(),
            balance_batch: false,
            tokens: crate::tokens::StationTokens::load(),
            show_tokens: false,
            token_draft: HashMap::new(),
            token_uid_draft: HashMap::new(),
            token_reveal: HashSet::new(),
            probe: ProbeGate::default(),
            net_guard: crate::netguard::detect(),
            allow_model_test_with_proxy: prefs.allow_model_test_with_proxy,
            guide_dismissed: prefs.guide_dismissed,
            show_health: false,
            main_view: MainView::default(),
            usage_range: usage::range::Range::from_key(&prefs.usage_range),
            usage_filter: None,
            usage_chart_kind: usage::ChartKind::default(),
            usage_chart_range: usage::range::ChartRange::default(),
            usage_delete_pending: None,
            usage_source_error: None,
            usage_sort: usage::view::SortState::by_total(),
            usage: usage::UsageState::default(),
            ui_style: crate::theme::UiStyle::from_key(&prefs.ui_style),
            glass: prefs.glass,
            tab_order: prefs.tab_order.clone(),
            tray_resident: prefs.tray_resident,
            net_guard_at: 0.0,
            // UI settings come from the home directory's .modelharbor/settings.json.
            theme: Theme::from_key(&prefs.theme),
            // Handed to the first frame's apply_theme_if_changed
            applied_theme: None,
            applied_palette: None,
            theme_fade: None,
            save_format: SaveFormat::from_key(&prefs.save_format),
            prefs_saved: prefs.clone(),
            save_format_wheel_latch: false,
            source_format: format,
            config_paths: paths,
            targets: Vec::new(),
            current_page: format,
            sync_wsl: prefs.sync_wsl,
            show_api_keys: prefs.show_api_keys,
            show_preview: false,
            preview_ratio: 0.38,
            preview_focused: false,
            preview_draft: String::new(),
            preview_dirty_at: None,
            preview_edit_at: None,
            preview_parse_ok: true,
            preview_parse_error: None,
            preview_find: String::new(),
            preview_find_active: false,
            preview_find_index: 0,
            preview_find_focus: false,
            preview_find_jump: None,
            preview_diff_mode: false,
            preview_diff_cache: None,
            preview_diff_pending: None,
            save_serial: 0,
            preview_cursor_line: 1,
            load_error: None,
            pi_extras: Value::Object(Map::new()),
            backend_icons: Vec::new(),
            toolbar_icons: ToolbarIcons::default(),
            snapshot_hashes: HashMap::new(),
            snapshots_root: None,
        };
        // The WSL master switch and the sync checkbox initialize together.
        crate::util::wsl_set_enabled(prefs.sync_wsl);
        app.apply_load();
        // Resident taskbar: on by default, tray icon created at startup (user asked for resident-by-default).
        // Tests don't create a real tray, keeping offscreen tests from hanging.
        #[cfg(all(target_os = "windows", not(test)))]
        if app.tray_resident {
            crate::tray::ensure();
        }
        app
    }
}

impl eframe::App for App {
    /// Window clear color: the acrylic tier returns transparent, non-acrylic tiers return the theme's panel color.
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        if self.glass {
            egui::Color32::TRANSPARENT.to_normalized_gamma_f32()
        } else {
            visuals.panel_fill.to_normalized_gamma_f32()
        }
    }

    /// Resident taskbar: intercept close events + consume tray menu events.
    ///
    /// In resident mode, clicking X doesn't quit — the window tucks into the tray; only the tray menu's "quit" really closes.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        #[cfg(target_os = "windows")]
        {
            // Resident mode: intercept the close and hide the window instead.
            // `close_requested` is captured before hooks can act; the event can't be plucked out of the input,
            // and eframe decides by whether the output commands contain `CancelClose` — sending it suffices.
            // A tray-menu "quit" click lets it through (`PostMessageW(WM_CLOSE)` takes exactly this path).
            if self.tray_resident
                && !crate::tray::exit_requested()
                && raw_input.viewport().close_requested()
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                crate::tray::hide_main_window();
            }
        }
        let _ = (ctx, raw_input);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // The very first thing: apply the theme. Both startup and theme switches come through here.
        self.apply_theme_if_changed(ctx);
        let dropped = ctx.input(|i| {
            i.raw
                .dropped_files
                .first()
                .and_then(|f| f.path.as_ref().map(|p| p.to_string_lossy().into_owned()))
        });
        if let Some(path) = dropped {
            self.config_path = path;
            self.reload();
        }
        // Recheck the system proxy / VPN every 5 seconds; the verdict disables model latency tests.
        let frame_time = ctx.input(|i| i.time);
        if frame_time - self.net_guard_at >= 5.0 {
            self.net_guard = crate::netguard::detect();
            self.net_guard_at = frame_time;
        }
        // Lazily load backend icons and toolbar icons on the first frame
        self.load_backend_icons(ctx);
        self.load_toolbar_icons(ctx);
        self.poll_discovery();
        self.poll_latency();
        self.poll_balance();
        // Local usage: background scan + 3s change detection (no scanning while idle).
        self.poll_usage(ctx);
        self.poll_free_models();
        // First launch / cache expired: auto-fetch once only (cleared once fetched); the user can refetch manually.
        for format in std::mem::take(&mut self.free_models_auto) {
            self.start_free_models_fetch(format);
        }
        self.persist_prefs_if_changed();
        self.ui_top_bar(ctx);
        self.ui_status_bar(ctx);
        // The right-side config preview / editor panel: width is governed by preview_ratio, scaling with window resizes;
        // 220px floor, components keep at least 320px. `show_animated` keeps the sidebar placeholder and tweens
        // the width while opening / hiding; the real preview content mounts only after the animation ends.
        let screen_w = ctx.content_rect().width().max(1.0);
        let max_w = (screen_w - 320.0).max(220.0);
        let preview_w = (screen_w * self.preview_ratio).clamp(220.0, max_w);
        let side = egui::SidePanel::right("preview_panel")
            .exact_width(preview_w)
            .show_animated(ctx, self.show_preview, |ui| {
                self.ui_preview_panel(ui);
            });
        if let Some(side) = side {
            // The splitter registers as its own hot zone on the Foreground layer.
            self.ui_preview_resizer(ctx, side.response.rect, screen_w);
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            self.ui_page_header(ui);
            // The usage view's status line (filter / freshness / errors + range switch) paints outside the scroll
            // area: pinned to the top, not scrolling with the page (user request). Even deep in a long table, the
            // current range and filter stay visible.
            if matches!(self.main_view, MainView::Usage) {
                self.ui_usage_status(ui);
                ui.add_space(crate::theme::SPACE_4);
            }
            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                .scroll_source(egui::scroll_area::ScrollSource {
                    drag: false,
                    ..egui::scroll_area::ScrollSource::ALL
                })
                .show(ui, |ui| match self.main_view {
                    MainView::Providers => {
                        // Providers on top, Agents below; Agents belong only to the opencode page and affect UI order only.
                        self.ui_providers_section(ui);
                        ui.add_space(crate::theme::SPACE_2);
                        if self.current_page.is_opencode_family() {
                            self.ui_agents_section(ui);
                            ui.add_space(crate::theme::SPACE_2);
                        }
                    }
                    // The usage view is independent of the current tab: it counts every agent on this machine.
                    MainView::Usage => self.ui_usage_section(ui),
                });
        });
        // Token management: an independent popover (draggable / closable), outside the main layout.
        self.ui_tokens_window(ctx);
        // Config health: also an independent popover, opened to double-check before saving.
        self.ui_health_window(ctx);
        self.paint_drag_ghost(ctx);
        // Grab cursor: widgets only express wishes while drawing; this commits them at frame end.
        //
        // Even with no hot zone reporting a state mid-drag, the fist is kept.
        let mut want = crate::ui::take_grab_cursor(ctx);
        if self.is_dragging_anything() {
            want = crate::ui::GrabCursor::Fist;
        }
        #[cfg(target_os = "windows")]
        crate::cursor::set_custom_cursor(match want {
            crate::ui::GrabCursor::None => crate::cursor::CustomCursor::None,
            crate::ui::GrabCursor::Palm => crate::cursor::CustomCursor::Palm,
            crate::ui::GrabCursor::Fist => crate::cursor::CustomCursor::Fist,
        });
        #[cfg(not(target_os = "windows"))]
        let _ = want;
    }
}

#[derive(Default)]
struct ToolbarIcons {
    panel_right: Option<egui::TextureHandle>,
    eye: Option<egui::TextureHandle>,
    eye_off: Option<egui::TextureHandle>,
    key_round: Option<egui::TextureHandle>,
    activity: Option<egui::TextureHandle>,
    palette: Option<egui::TextureHandle>,
    folder_open: Option<egui::TextureHandle>,
    unfold: Option<egui::TextureHandle>,
    database: Option<egui::TextureHandle>,
    globe: Option<egui::TextureHandle>,
    save: Option<egui::TextureHandle>,
    layers: Option<egui::TextureHandle>,
    file_output: Option<egui::TextureHandle>,
    bot: Option<egui::TextureHandle>,
    server: Option<egui::TextureHandle>,
    zap: Option<egui::TextureHandle>,
    /// The resident-taskbar toggle (lit-state icon).
    pin: Option<egui::TextureHandle>,
}

impl App {
    /// Lazily load each backend's official icons (once, on the first frame).
    fn load_backend_icons(&mut self, ctx: &egui::Context) {
        if !self.backend_icons.is_empty() {
            return;
        }
        self.backend_icons = backends::BACKENDS
            .iter()
            .map(|b| {
                b.icon_rgba().map(|(rgba, w, h)| {
                    let image =
                        egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba);
                    ctx.load_texture(
                        format!("backend_icon_{}", b.id().label()),
                        image,
                        egui::TextureOptions::LINEAR,
                    )
                })
            })
            .collect();
    }

    /// Lazily load toolbar icons: SVG assets rasterized and handed to egui's texture system.
    fn load_toolbar_icons(&mut self, ctx: &egui::Context) {
        if self.toolbar_icons.panel_right.is_some() {
            return;
        }
        let options = &Default::default();
        let load = |name: &str, svg: &[u8]| {
            let image = egui_extras::image::load_svg_bytes_with_size(
                svg,
                egui::SizeHint::Size {
                    width: 20,
                    height: 20,
                    maintain_aspect_ratio: true,
                },
                options,
            )
            .ok()?;
            Some(ctx.load_texture(name, image, egui::TextureOptions::LINEAR))
        };
        self.toolbar_icons.panel_right = load(
            "toolbar_icon_panel_right",
            include_bytes!("../../assets/icons/panel-right.svg"),
        );
        self.toolbar_icons.eye = load(
            "toolbar_icon_eye",
            include_bytes!("../../assets/icons/eye.svg"),
        );
        self.toolbar_icons.eye_off = load(
            "toolbar_icon_eye_off",
            include_bytes!("../../assets/icons/eye-off.svg"),
        );
        self.toolbar_icons.key_round = load(
            "toolbar_icon_key_round",
            include_bytes!("../../assets/icons/key-round.svg"),
        );
        self.toolbar_icons.activity = load(
            "toolbar_icon_activity",
            include_bytes!("../../assets/icons/activity.svg"),
        );
        self.toolbar_icons.palette = load(
            "toolbar_icon_palette",
            include_bytes!("../../assets/icons/palette.svg"),
        );
        self.toolbar_icons.folder_open = load(
            "toolbar_icon_folder_open",
            include_bytes!("../../assets/icons/folder-open.svg"),
        );
        self.toolbar_icons.save = load(
            "toolbar_icon_save",
            include_bytes!("../../assets/icons/save.svg"),
        );
        self.toolbar_icons.layers = load(
            "toolbar_icon_layers",
            include_bytes!("../../assets/icons/layers.svg"),
        );
        self.toolbar_icons.file_output = load(
            "toolbar_icon_file_output",
            include_bytes!("../../assets/icons/file-output.svg"),
        );
        self.toolbar_icons.bot = load(
            "toolbar_icon_bot",
            include_bytes!("../../assets/icons/bot.svg"),
        );
        self.toolbar_icons.server = load(
            "toolbar_icon_server",
            include_bytes!("../../assets/icons/server.svg"),
        );
        self.toolbar_icons.zap = load(
            "toolbar_icon_zap",
            include_bytes!("../../assets/icons/zap.svg"),
        );
        self.toolbar_icons.pin = load(
            "toolbar_icon_pin",
            include_bytes!("../../assets/icons/pin.svg"),
        );
        self.toolbar_icons.unfold = load(
            "toolbar_icon_unfold",
            include_bytes!("../../assets/icons/unfold.svg"),
        );
        self.toolbar_icons.database = load(
            "toolbar_icon_database",
            include_bytes!("../../assets/icons/database.svg"),
        );
        self.toolbar_icons.globe = load(
            "toolbar_icon_globe",
            include_bytes!("../../assets/icons/globe.svg"),
        );
    }
}

impl App {
    /// The current UI preferences.
    fn current_prefs(&self) -> crate::prefs::Prefs {
        let mut collapsed: Vec<String> = self.collapsed.iter().cloned().collect();
        collapsed.sort();
        crate::prefs::Prefs {
            show_api_keys: self.show_api_keys,
            save_format: self.save_format.key().to_string(),
            theme: self.theme.key().to_string(),
            sync_wsl: self.sync_wsl,
            // Path overrides no longer persist: always written as empty, so every page returns to its official default after restart.
            // Dirty values in old settings (e.g. accidentally pointing at a desktop file) clear on the first save.
            config_paths: crate::prefs::ConfigPathPrefs::default(),
            collapsed,
            allow_model_test_with_proxy: self.allow_model_test_with_proxy,
            guide_dismissed: self.guide_dismissed,
            ui_style: self.ui_style.key().to_string(),
            glass: self.glass,
            tab_order: self.tab_order.clone(),
            tray_resident: self.tray_resident,
            usage_range: self.usage_range.key().to_string(),
        }
    }

    /// The layer hosting the station token popover.
    ///
    /// `Foreground`: above the preview splitter's `Middle`, below `Tooltip`.
    const TOKENS_WINDOW_ORDER: egui::Order = egui::Order::Foreground;

    /// Take a format's official icon texture (`None` when not yet loaded).
    fn icon_for(&self, fmt: ConfigFormat) -> Option<&egui::TextureHandle> {
        let idx = backends::BACKENDS.iter().position(|b| b.id() == fmt)?;
        self.backend_icons.get(idx).and_then(|o| o.as_ref())
    }
}

// ---------- Compat re-exports: keep existing external paths working ----------
pub use crate::backends::opencode::merge_opencode_root;
pub use crate::util::parse_config_content;

#[cfg(test)]
mod tests;
