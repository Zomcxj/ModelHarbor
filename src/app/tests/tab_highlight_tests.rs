use crate::app::App;
use crate::format::ConfigFormat;
use eframe::egui;

/// 页签顺序（settings.json 的 `tab_order`）。
const TAB_ORDER: [&str; 6] = [
    "opencode",
    "pi",
    "zcode",
    "workbuddy",
    "deepseek-harness",
    "oh-my-pi",
];

/// 页签按钮的可见状态：外框矩形、底色、描边色。
type TabBox = (egui::Rect, egui::Color32, egui::Color32);

/// 图标矩形：位置 + tint 色。
type IconBox = (egui::Rect, egui::Color32);

/// 页签条这一帧收集到的原始矩形：
/// (矩形, 填充, 描边色, 是否贴图(brush), 描边宽度, 圆角)。
type RawRect = (egui::Rect, egui::Color32, egui::Color32, bool, f32, u8);

/// `tab_shapes_at` 的三段结果：按钮本体 / 图标 / 每个槽位的落点绿环。
/// 绿环那一段带上矩形与圆角。
type TabShapes = (
    Vec<TabBox>,
    Vec<IconBox>,
    Vec<Option<(egui::Rect, egui::Color32, u8)>>,
);

/// 当前主题下的悬浮底色与悬浮描边色。
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

/// 离屏跑一遍顶部栏，收集页签条区域内的按钮底色与图标 tint。
///
/// egui 把图片画成带 `brush` 的 `RectShape`（贴图与 `fill` 相乘），图标靠 `brush.is_some()`
/// 认，tint 就是它的 `fill`。一个页签贡献两个矩形：按钮底色（无 brush、约 44px 宽）与
/// 图标（有 brush、16px）。先按区域筛、再按左边缘排序，下标即页签序号。
///
/// `pointer` 给出时，本帧把指针放在该位置。
fn tab_shapes_at(app: &mut App, pointer: Option<egui::Pos2>) -> TabShapes {
    tab_shapes_at_with(app, pointer, None)
}

/// `press_at` 给出时，第一帧在该处按下主键并保持，第二帧把指针移到 `pointer`。
///
/// 拖动时按键落在源页签上，指针移到目标页签。
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
    // 图标由 `update()` 惰性加载，测试直接调 `ui_top_bar`，需先手动加载贴图。
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
    // 帧序列：按下那一帧 → 指针移到目标那一帧。egui 的交互判定用上一帧登记的控件矩形，
    // 每个位置跑两帧才生效。
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
    // 页签条的横向范围按页签数量算：每个页签占 48px（44 宽 + 4 间距）。
    let strip_right = 48.0 * crate::backends::BACKENDS.len() as f32 + 8.0;
    // top 上界落在两行之间：页签本体在第 1 行（top=2、bottom=24），路径行按钮 top=35。
    let in_strip = |r: &egui::Rect| {
        r.top() < 28.0 && r.left() < strip_right && r.width() < 60.0 && r.height() < 40.0
    };
    // 悬停时同一个页签画两个矩形：egui 的 hover 框（外扩 1px、底色 `#383838`、
    // 描边强调色）与落点绿环（无填充）；非悬停时只有一个本体。按 x 中心聚类，
    // 取其中填充非透明（或最宽）的那个当本体。
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
        // 本体 = 该簇里填充非透明的那一个；若全透明（不该发生）取最宽的。
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

/// 把矩形按 x 中心分组：中心相差不到 3px 的算同一个页签（hover 框与本体差 1px）。
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

/// 把指针停在第 `slot` 个页签上再跑一帧，返回每个槽位的绿环（无则 `None`）。
fn tab_rings_with_pointer_on(
    app: &mut App,
    slot: usize,
) -> Vec<Option<(egui::Rect, egui::Color32, u8)>> {
    // 先跑一帧拿到几何，再按目标槽位的中心点跑第二帧。
    let boxes = tab_fills(app);
    let center = boxes[slot].0.center();
    tab_shapes_at(app, Some(center)).2
}

/// 同上，但模拟真实拖拽：先在第 `from` 个页签按下主键，再把指针移到第 `to` 个。
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

/// 让全部后端都算「已安装」，页签顺序与跑测试这台机器无关。
///
/// 槽位由 `ConfigPaths::validate_target`（配置文件是否真实存在）决定：已安装的排在前面、
/// 顺序取 `tab_order`，未安装的按名字排在后面。这里给每个后端在临时目录里造一份配置文件，
/// 全部「已安装」后顺序只由 `TAB_ORDER` 决定，未列进去的几家按字母序补在其后。
fn install_every_backend(app: &mut App) {
    // 全进程共用一份临时目录。
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

/// 选中页签的底色 = 悬浮色（`widgets.hovered.bg_fill`）。
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
    // 其余未选中：既不是悬浮底色，也不带 2px 描边。
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
    // 拖动源色（橙）与选中色（悬浮加重）不同色。
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
    // 图标 tint 只能是白（已安装）或压淡色（未安装），逐槽位比对：换个选中页，
    // 同一个槽位的 tint 必须一模一样。
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
    // 拖动中：指针所在的别的页签画绿环（换位目标），拖动源用橙。
    let mut app = app_with_tabs();
    app.current_page = ConfigFormat::Opencode;
    app.tab_drag_src = Some(ConfigFormat::Opencode);
    // 指针落在第 3 个槽位（ZCode）上。
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
    // 拖动源用橙色底色，不是绿环。
    let tabs = tab_fills(&mut app);
    assert_eq!(
        tabs[0].1,
        crate::ui::DRAG_SOURCE_FILL,
        "拖动源用橙色，不是绿色"
    );
}

#[test]
fn no_green_ring_appears_when_nothing_is_being_dragged() {
    // 绿环只属于拖动中的落点；不在拖动时不该出现。
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
    // 真实拖拽：键按在源页签上、指针移到目标页签上。
    let mut app = app_with_tabs();
    app.current_page = ConfigFormat::Opencode;
    app.tab_drag_src = Some(ConfigFormat::Opencode);
    // 从第 1 个槽位（源）拖到第 3 个（目标）。
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

/// 换位绿环与按钮同圆角，只换颜色、不改形状。
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
    // 按钮本体的圆角用主题值核对（`TabBox` 只带颜色）。
    let expected = crate::theme::UiStyle::from_key("cloud").radius();
    assert_eq!(
        radius, expected,
        "绿环圆角必须等于按钮圆角（云朵档 = {expected}），不能写死"
    );
    assert_ne!(radius, 3, "曾经写死 3.0，正是「变圆角了」的根因");
    // 绿环与按钮本体几何一致（矩形完全相同）。
    assert_eq!(
        rings[to].map(|(r, _, _)| r),
        Some(bodies[to].0),
        "绿环矩形必须与按钮本体完全重合"
    );
}
