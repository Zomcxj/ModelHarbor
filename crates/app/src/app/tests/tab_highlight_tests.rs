use crate::app::App;
use crate::format::ConfigFormat;
use eframe::egui;

/// Tab order (settings.json's `tab_order`).
const TAB_ORDER: [&str; 6] = [
    "opencode",
    "pi",
    "zcode",
    "workbuddy",
    "deepseek-harness",
    "oh-my-pi",
];

/// A tab button's visible state: outer rect, fill, border color.
type TabBox = (egui::Rect, egui::Color32, egui::Color32);

/// Icon rect: position + tint color.
type IconBox = (egui::Rect, egui::Color32);

/// Raw rects collected from the tab strip this frame:
/// (rect, fill, border color, is-brush (textured), border width, corner radius).
type RawRect = (egui::Rect, egui::Color32, egui::Color32, bool, f32, u8);

/// The three sections of `tab_shapes_at`: button bodies / icons / each slot's drop-target green ring.
/// The ring section carries rect and corner radius.
type TabShapes = (
    Vec<TabBox>,
    Vec<IconBox>,
    Vec<Option<(egui::Rect, egui::Color32, u8)>>,
);

/// Hover fill and hover border colors under the current theme.
fn hover_visuals() -> (egui::Color32, egui::Color32) {
    let ctx = egui::Context::default();
    crate::theme::Theme::from_key("dark").apply_style(
        &ctx,
        crate::theme::UiStyle::from_key("cloud"),
        false,
    );
    let hovered = ctx.style().visuals.widgets.hovered;
    (hovered.bg_fill, hovered.bg_stroke.color)
}

/// Run the top bar offscreen once, collecting button fills and icon tints within the tab strip area.
///
/// egui paints images as `RectShape`s with a `brush` (texture multiplied by `fill`); icons are recognized
/// by `brush.is_some()`, and the tint is its `fill`. One tab contributes two rects: the button fill (no
/// brush, ~44px wide) and the icon (brush, 16px). Filter by area first, sort by left edge; index = tab
/// position.
///
/// When `pointer` is given, the pointer is placed there for the frame.
fn tab_shapes_at(app: &mut App, pointer: Option<egui::Pos2>) -> TabShapes {
    tab_shapes_at_with(app, pointer, None)
}

/// When `press_at` is given, frame one presses the primary button there and holds, frame two moves the pointer to `pointer`.
///
/// While dragging, the press lands on the source tab and the pointer moves onto the target tab.
fn tab_shapes_at_with(
    app: &mut App,
    pointer: Option<egui::Pos2>,
    press_at: Option<egui::Pos2>,
) -> TabShapes {
    let ctx = egui::Context::default();
    crate::theme::Theme::from_key("dark").apply_style(
        &ctx,
        crate::theme::UiStyle::from_key("cloud"),
        false,
    );
    // Icons load lazily in `update()`; tests call `ui_top_bar` directly, so load the textures manually first.
    app.load_backend_icons(&ctx);
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 800.0));
    let moved = |p: egui::Pos2| vec![egui::Event::PointerMoved(p)];
    let press = |p: egui::Pos2| {
        vec![
            egui::Event::PointerMoved(p),
            egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
        ]
    };
    // Frame sequence: the press frame → the pointer-move frame. egui's interaction verdicts use the previous
    // frame's registered widget rects — two frames per position to take effect.
    let frames: Vec<Vec<egui::Event>> = match (press_at, pointer) {
        (Some(from), Some(to)) => vec![press(from), press(from), moved(to), moved(to)],
        (None, Some(to)) => vec![moved(to), moved(to)],
        _ => vec![Vec::new()],
    };
    let mut last = None;
    for events in frames {
        last = Some(ctx.run(
            egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            },
            |ctx| {
                app.ui_top_bar(ctx);
            },
        ));
    }
    let out = last.expect("至少跑一帧");
    let mut rects: Vec<RawRect> = Vec::new();
    fn collect(shape: &egui::Shape, out: &mut Vec<RawRect>) {
        match shape {
            egui::Shape::Rect(r) => out.push((
                r.rect,
                r.fill,
                r.stroke.color,
                r.brush.is_some(),
                r.stroke.width,
                r.corner_radius.nw,
            )),
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    for cs in &out.shapes {
        collect(&cs.shape, &mut rects);
    }
    // The tab strip's horizontal range follows the tab count: each tab takes 48px (44 wide + 4 gap).
    let strip_right = 48.0 * crate::backends::BACKENDS.len() as f32 + 8.0;
    // The top bound falls between the rows: tab bodies are in row 1 (top=2, bottom=24); the path row's buttons start at top=35.
    let in_strip = |r: &egui::Rect| {
        r.top() < 28.0 && r.left() < strip_right && r.width() < 60.0 && r.height() < 40.0
    };
    // Hovering paints two rects for the same tab: egui's hover frame (1px outward, fill `#383838`,
    // accent border) and the drop-target green ring (no fill); without hover there's only the body.
    // Cluster by x center and take the non-transparent (or widest) fill as the body.
    let is_ring = |fill: &egui::Color32, stroke: &egui::Color32, textured: bool, w: f32| {
        !textured
            && w > 0.0
            && *fill == egui::Color32::TRANSPARENT
            && *stroke == crate::ui::DROP_TARGET_COLOR
    };
    let mut candidates: Vec<RawRect> = rects
        .iter()
        .filter(|(r, _, _, textured, w, _)| in_strip(r) && !*textured && *w > 0.0)
        .copied()
        .collect();
    candidates.sort_by(|a, b| a.0.center().x.partial_cmp(&b.0.center().x).unwrap());

    let mut bodies: Vec<TabBox> = Vec::new();
    let mut rings: Vec<Option<(egui::Rect, egui::Color32, u8)>> = Vec::new();
    for group in cluster_by_center_x(&candidates) {
        // The body = the cluster's non-transparent-filled one; if all transparent (shouldn't happen), take the widest.
        let body = group
            .iter()
            .find(|(_, fill, _, _, _, _)| *fill != egui::Color32::TRANSPARENT)
            .or_else(|| {
                group
                    .iter()
                    .max_by(|a, b| a.0.width().partial_cmp(&b.0.width()).unwrap())
            })
            .expect("簇非空");
        bodies.push((body.0, body.1, body.2));
        rings.push(
            group
                .iter()
                .find(|(_, fill, stroke, textured, w, _)| is_ring(fill, stroke, *textured, *w))
                .map(|(rect, _, stroke, _, _, radius)| (*rect, *stroke, *radius)),
        );
    }

    let mut icons: Vec<IconBox> = rects
        .iter()
        .filter(|(r, _, _, textured, _, _)| in_strip(r) && *textured)
        .map(|(r, fill, _, _, _, _)| (*r, *fill))
        .collect();
    icons.sort_by(|a, b| a.0.left().partial_cmp(&b.0.left()).unwrap());
    (bodies, icons, rings)
}

/// Group rects by x center: centers within 3px count as the same tab (hover frame and body differ by 1px).
fn cluster_by_center_x(sorted: &[RawRect]) -> Vec<Vec<RawRect>> {
    let mut groups: Vec<Vec<RawRect>> = Vec::new();
    for item in sorted {
        match groups.last_mut() {
            Some(g) if (g[0].0.center().x - item.0.center().x).abs() < 3.0 => g.push(*item),
            _ => groups.push(vec![*item]),
        }
    }
    groups
}

fn tab_fills(app: &mut App) -> Vec<TabBox> {
    tab_shapes_at(app, None).0
}

fn tab_icon_tints(app: &mut App) -> Vec<egui::Color32> {
    tab_shapes_at(app, None)
        .1
        .into_iter()
        .map(|(_, fill)| fill)
        .collect()
}

/// Park the pointer over tab `slot` and run another frame; returns each slot's green ring (`None` when absent).
fn tab_rings_with_pointer_on(
    app: &mut App,
    slot: usize,
) -> Vec<Option<(egui::Rect, egui::Color32, u8)>> {
    // Run one frame for geometry, then a second frame at the target slot's center.
    let boxes = tab_fills(app);
    let center = boxes[slot].0.center();
    tab_shapes_at(app, Some(center)).2
}

/// Same, but simulating a real drag: press the primary button on tab `from`, then move the pointer to `to`.
fn tab_rings_while_dragging(
    app: &mut App,
    from: usize,
    to: usize,
) -> Vec<Option<(egui::Rect, egui::Color32, u8)>> {
    let boxes = tab_fills(app);
    let start = boxes[from].0.center();
    let end = boxes[to].0.center();
    tab_shapes_at_with(app, Some(end), Some(start)).2
}

/// Make every backend count as "installed" so tab order is independent of the machine running the tests.
///
/// Slots are decided by `ConfigPaths::validate_target` (does the config file really exist): installed
/// first, ordered by `tab_order`; uninstalled ones trail by name. This creates a config file for every
/// backend in a temp directory — once all are "installed", order is decided by `TAB_ORDER` alone, with
/// the unlisted few appended alphabetically.
fn install_every_backend(app: &mut App) {
    // One temp directory for the whole process.
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    let root = DIR.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("model-harbor-tabs-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("建临时配置目录");
        root
    });
    for backend in crate::backends::BACKENDS {
        let id = backend.id();
        let file = root.join(format!("{}.json", id.label()));
        if !file.exists() {
            std::fs::write(&file, "{}").expect("写临时配置");
        }
        app.config_paths.set_local_path(id, &file.to_string_lossy());
    }
}

fn app_with_tabs() -> App {
    let mut app = App {
        tab_order: TAB_ORDER.iter().map(|s| (*s).to_string()).collect(),
        ..Default::default()
    };
    install_every_backend(&mut app);
    app
}

/// The selected tab's fill = hover color (`widgets.hovered.bg_fill`).
#[test]
fn the_selected_tab_uses_the_hover_fill_with_a_bold_ring() {
    let mut app = app_with_tabs();
    app.current_page = ConfigFormat::Opencode;
    let tabs = tab_fills(&mut app);
    assert_eq!(
        tabs.len(),
        crate::backends::BACKENDS.len(),
        "每个后端图标各一个按钮"
    );
    assert_eq!(
        tabs[0].1,
        hover_visuals().0,
        "选中页签的底色必须是悬浮色（选中 = 悬浮的加重版）"
    );
    assert_eq!(tabs[0].2, hover_visuals().1, "选中页签的描边用悬浮描边色");
    // The other unselected ones: neither the hover fill nor a 2px border.
    for (i, (_, fill, stroke)) in tabs.iter().enumerate().skip(1) {
        assert_ne!(*fill, hover_visuals().0, "第 {i} 个未选中，不该用悬浮底色");
        assert_ne!(
            *stroke,
            hover_visuals().1,
            "第 {i} 个未选中，不该有选中描边"
        );
    }
}

#[test]
fn highlight_follows_the_current_page() {
    let mut app = app_with_tabs();
    app.current_page = ConfigFormat::ZCode;
    let tabs = tab_fills(&mut app);
    assert_eq!(
        tabs[2].1,
        hover_visuals().0,
        "ZCode 在第 3 个槽位，必须是选中底色"
    );
    assert_eq!(
        tabs.iter()
            .filter(|(_, f, _)| *f == hover_visuals().0)
            .count(),
        1,
        "同一时刻只该有一个选中页签"
    );
}

#[test]
fn the_grabbed_tab_is_orange_and_differs_from_the_selected_one() {
    // The drag-source color (orange) differs from the selected color (intensified hover).
    let mut app = app_with_tabs();
    app.current_page = ConfigFormat::Opencode;
    app.tab_drag_src = Some(ConfigFormat::ZCode);
    let tabs = tab_fills(&mut app);
    assert_eq!(tabs[0].1, hover_visuals().0, "当前页面保持选中态");
    assert_eq!(
        tabs[2].1,
        crate::ui::DRAG_SOURCE_FILL,
        "被抓住的页签用橙色（拖动源色）"
    );
    assert_ne!(
        crate::ui::DRAG_SOURCE_FILL,
        hover_visuals().0,
        "拖动源色与选中色必须是两个不同的颜色"
    );
}

#[test]
fn the_icon_tint_does_not_depend_on_which_tab_is_selected() {
    // Icon tints can only be white (installed) or dimmed (not) — compare slot by slot: switch the selected
    // page and the same slot's tint must be identical.
    let tints_for = |page| {
        let mut app = app_with_tabs();
        app.current_page = page;
        let tints = tab_icon_tints(&mut app);
        assert_eq!(
            tints.len(),
            crate::backends::BACKENDS.len(),
            "每个页签各有一个图标 tint"
        );
        tints
    };
    let baseline = tints_for(ConfigFormat::Opencode);
    for (i, tint) in baseline.iter().enumerate() {
        let is_black = tint.r() == 0 && tint.g() == 0 && tint.b() == 0;
        assert!(!is_black, "第 {i} 个页签的图标被 tint 成了黑色");
    }
    for page in [
        ConfigFormat::Pi,
        ConfigFormat::ZCode,
        ConfigFormat::WorkBuddy,
        ConfigFormat::DeepSeekHarness,
        ConfigFormat::OhMyPi,
    ] {
        assert_eq!(
            tints_for(page),
            baseline,
            "{page:?} 被选中后图标 tint 变了；tint 不该随选中态变化"
        );
    }
}

#[test]
fn the_swap_target_is_marked_green_and_the_source_is_not() {
    // Mid-drag: other tabs under the pointer paint green rings (reorder targets); the drag source is orange.
    let mut app = app_with_tabs();
    app.current_page = ConfigFormat::Opencode;
    app.tab_drag_src = Some(ConfigFormat::Opencode);
    // The pointer sits on the 3rd slot (ZCode).
    let hovered = 2usize;
    let rings = tab_rings_with_pointer_on(&mut app, hovered);
    assert_eq!(
        rings[hovered].map(|(_, c, _)| c),
        Some(crate::ui::DROP_TARGET_COLOR),
        "指针所在的槽位必须是绿色换位目标"
    );
    for (i, ring) in rings.iter().enumerate() {
        if i == hovered {
            continue;
        }
        assert_ne!(
            ring.map(|(_, c, _)| c),
            Some(crate::ui::DROP_TARGET_COLOR),
            "第 {i} 个不是落点，不该有绿色环"
        );
    }
    // The drag source uses an orange fill, not a green ring.
    let tabs = tab_fills(&mut app);
    assert_eq!(
        tabs[0].1,
        crate::ui::DRAG_SOURCE_FILL,
        "拖动源用橙色，不是绿色"
    );
}

#[test]
fn no_green_ring_appears_when_nothing_is_being_dragged() {
    // Green rings belong only to mid-drag drop targets; they mustn't appear when not dragging.
    let mut app = app_with_tabs();
    app.current_page = ConfigFormat::Opencode;
    let rings = tab_rings_with_pointer_on(&mut app, 2);
    for (i, ring) in rings.iter().enumerate() {
        assert_ne!(
            ring.map(|(_, c, _)| c),
            Some(crate::ui::DROP_TARGET_COLOR),
            "第 {i} 个：没在拖动却画了绿色落点环"
        );
    }
}

#[test]
fn the_green_ring_shows_up_while_the_button_is_actually_held() {
    // A real drag: the key presses on the source tab and the pointer moves onto the target tab.
    let mut app = app_with_tabs();
    app.current_page = ConfigFormat::Opencode;
    app.tab_drag_src = Some(ConfigFormat::Opencode);
    // Drag from slot 1 (source) to slot 3 (target).
    let (from, to) = (0usize, 2usize);
    let rings = tab_rings_while_dragging(&mut app, from, to);
    assert_eq!(
        rings[to].map(|(_, c, _)| c),
        Some(crate::ui::DROP_TARGET_COLOR),
        "按住键拖动时，指针所在的槽位仍然必须是绿色换位目标"
    );
    for (i, ring) in rings.iter().enumerate() {
        if i == to {
            continue;
        }
        assert_ne!(
            ring.map(|(_, c, _)| c),
            Some(crate::ui::DROP_TARGET_COLOR),
            "第 {i} 个不是落点，不该有绿色环"
        );
    }
}

/// The reorder green ring shares the button's corner radius — only the color changes, never the shape.
#[test]
fn the_green_ring_shares_the_buttons_corner_radius() {
    let mut app = app_with_tabs();
    app.current_page = ConfigFormat::Opencode;
    app.tab_drag_src = Some(ConfigFormat::Opencode);
    let (from, to) = (0usize, 2usize);
    let shapes = {
        let boxes = tab_fills(&mut app);
        let start = boxes[from].0.center();
        let end = boxes[to].0.center();
        tab_shapes_at_with(&mut app, Some(end), Some(start))
    };
    let (bodies, _, rings) = shapes;
    let (_, _, radius) = rings[to].expect("目标槽位必须有绿环");
    // The button body's corner radius checks against the theme value (`TabBox` carries only colors).
    let expected = crate::theme::UiStyle::from_key("cloud").radius();
    assert_eq!(
        radius, expected,
        "绿环圆角必须等于按钮圆角（云朵档 = {expected}），不能写死"
    );
    assert_ne!(radius, 3, "曾经写死 3.0，正是「变圆角了」的根因");
    // The green ring's geometry matches the button body's exactly (identical rects).
    assert_eq!(
        rings[to].map(|(r, _, _)| r),
        Some(bodies[to].0),
        "绿环矩形必须与按钮本体完全重合"
    );
}
