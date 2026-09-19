use eframe::egui;

/// 滚动区上下渐变淡出的遮罩高度（像素）。
///
/// 从边缘到内部的过渡带：内容在这里由「融进底色」逐渐转为全清晰，
/// 形成纵深的立体滚动感。太短像被硬切，太长会吃掉可视内容。
pub const SCROLL_FADE_HEIGHT: f32 = 34.0;

/// 顶部渐变要让开的吸顶条高度（像素）。
///
/// 吸顶标题始终要清晰可读，不能被渐变糊掉；过渡带从吸顶条下沿才开始。
pub const SCROLL_FADE_TOP_INSET: f32 = 30.0;

/// 该不该画上下遮罩：`(顶部, 底部)`。
///
/// 只有该方向**真的还能滚**时才画。滚到顶时画顶部遮罩会平白糊掉首行内容，
/// 内容不足一屏（`content_size <= 可视高`）时两侧都不画。
///
/// 抽成纯函数是为了能直接断言这三条边界。
pub fn scroll_fade_sides(visible_height: f32, content_height: f32, offset_y: f32) -> (bool, bool) {
    let max_offset = (content_height - visible_height).max(0.0);
    if max_offset <= 0.5 {
        return (false, false);
    }
    let can_up = offset_y > 0.5;
    let can_down = offset_y < max_offset - 0.5;
    (can_up, can_down)
}

/// 滚动区的上下渐变遮罩：把内容上下边缘融进面板底色，中间保持清晰。
///
/// egui 没有内置的渐变淡出，这里用带顶点色的 mesh 自己画：
/// 每侧一个从 `panel_fill`（不透明）到全透明的矩形，覆盖在内容之上。
///
/// `visible` 是滚动区可视矩形，`content_size` / `offset` 用来判断还能不能滚。
pub fn scroll_fade(
    ui: &egui::Ui,
    visible: egui::Rect,
    content_size: egui::Vec2,
    offset: egui::Vec2,
) {
    let base = ui.visuals().panel_fill;
    let clear = egui::Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), 0);
    let (can_up, can_down) = scroll_fade_sides(visible.height(), content_size.y, offset.y);
    let painter = ui.painter();
    let mut mesh = egui::Mesh::default();
    // 顶部：不透明 → 透明（自上而下）。起点让开吸顶条，标题始终清晰。
    if can_up {
        let top = visible.top() + SCROLL_FADE_TOP_INSET;
        let rect = egui::Rect::from_min_max(
            egui::pos2(visible.left(), top),
            egui::pos2(visible.right(), top + SCROLL_FADE_HEIGHT),
        );
        add_fade_quad(&mut mesh, rect, base, clear);
    }
    // 底部：透明 → 不透明（自上而下）
    if can_down {
        let rect = egui::Rect::from_min_max(
            egui::pos2(visible.left(), visible.bottom() - SCROLL_FADE_HEIGHT),
            visible.max,
        );
        add_fade_quad(&mut mesh, rect, clear, base);
    }
    if !mesh.is_empty() {
        painter.add(egui::Shape::mesh(mesh));
    }
}

/// 往 mesh 里加一个竖向渐变的矩形：`top_color` 在上、`bottom_color` 在下。
fn add_fade_quad(
    mesh: &mut egui::Mesh,
    rect: egui::Rect,
    top_color: egui::Color32,
    bottom_color: egui::Color32,
) {
    let idx = mesh.vertices.len() as u32;
    mesh.vertices.push(egui::epaint::Vertex {
        pos: rect.left_top(),
        uv: egui::epaint::WHITE_UV,
        color: top_color,
    });
    mesh.vertices.push(egui::epaint::Vertex {
        pos: rect.right_top(),
        uv: egui::epaint::WHITE_UV,
        color: top_color,
    });
    mesh.vertices.push(egui::epaint::Vertex {
        pos: rect.right_bottom(),
        uv: egui::epaint::WHITE_UV,
        color: bottom_color,
    });
    mesh.vertices.push(egui::epaint::Vertex {
        pos: rect.left_bottom(),
        uv: egui::epaint::WHITE_UV,
        color: bottom_color,
    });
    mesh.indices
        .extend_from_slice(&[idx, idx + 1, idx + 2, idx, idx + 2, idx + 3]);
}

pub fn card_frame<R>(
    ui: &mut egui::Ui,
    open: bool,
    highlight: u8,
    id: egui::Id,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::Response {
    let corner = ui.visuals().widgets.noninteractive.corner_radius;
    let shape = crate::theme::active_style(ui.ctx());
    let fill = if open {
        ui.visuals().faint_bg_color
    } else {
        ui.visuals().extreme_bg_color
    };
    // 悬停 id 必须按卡片稳定派生：前面卡片展开 / 折叠会改变后面卡片的 auto_id，
    // 用 auto_id 会把悬停状态串到别的卡片上。矩形画完才有，存进临时存储
    // 供下一帧判定指针是否悬停——即悬停状态天然滞后一帧。
    let hover_id = id.with("hover");
    let mut hover_t = 0.0f32;
    let (stroke_color, stroke_width) = match highlight {
        1 => (egui::Color32::from_rgb(255, 180, 50), 2.0), // source: orange
        2 => (egui::Color32::from_rgb(100, 200, 100), 2.0), // target: green
        // 无高亮时跟随形状预设的描边宽度（恒宽），颜色向强调色做悬停过渡。
        //
        // egui 的 Frame 会把描边宽度算进占位尺寸
        // （`outer_rect = content + margin + 2 * stroke`），所以悬停描边
        // 绝不能改宽度：极简 / 云朵下 0 → 1px 会把卡片撑高 1px，指针脱离
        // 悬停后收回、再悬停……整列组件就这样来回波动。高亮环改由
        // [`draw_hover_ring`] 用 painter 画在矩形内侧，不参与布局。
        _ => {
            let last_rect = ui
                .ctx()
                .data(|data| data.get_temp::<egui::Rect>(hover_id.with("rect")));
            let pointer = ui.input(|input| input.pointer.hover_pos());
            let hovered =
                matches!((last_rect, pointer), (Some(rect), Some(pos)) if rect.contains(pos));
            let t = crate::motion::hover_t(ui.ctx(), hover_id, hovered);
            hover_t = t;
            let base = ui.visuals().widgets.noninteractive.bg_stroke.color;
            let target = hover_target_color(ui);
            let rest_width = if shape.has_card_shadow() {
                0.0
            } else {
                shape.border_width()
            };
            (crate::motion::lerp_color(base, target, t), rest_width)
        }
    };
    let mut frame = egui::Frame::NONE
        .fill(fill)
        .corner_radius(corner)
        .stroke(egui::Stroke::new(stroke_width, stroke_color))
        .inner_margin(egui::Margin::symmetric(12, 6));
    if shape.has_card_shadow() {
        frame = frame.shadow(shape.card_shadow(ui.visuals().dark_mode));
    }
    let frame = frame.show(ui, |ui| {
        ui.style_mut().spacing.item_spacing = egui::vec2(4.0, 2.0);
        add(ui);
    });
    if highlight == 0 {
        ui.ctx().data_mut(|data| {
            data.insert_temp(hover_id.with("rect"), frame.response.rect);
        });
    }
    draw_accent_bar(ui, frame.response.rect);
    draw_relief(ui, frame.response.rect);
    draw_hover_ring(ui, frame.response.rect, hover_t, stroke_width);
    frame.response
}

/// 石板 / 浮雕的「坐在底上」质感。
///
/// 接触影：向右下偏移 1px 的整圈暗色描边（跟随圆角，一半落在卡外），
/// 石板和浮雕都有。浮雕在此之上再画凸起受光线（卡内左上亮、右下暗）；
/// 石板是**平放**的板，只有描边和接触影，没有受光线——这是它和浮雕
/// 的机制区别，而不是圆角和强度差异。
fn draw_relief(ui: &egui::Ui, rect: egui::Rect) {
    let style = crate::theme::active_style(ui.ctx());
    if !style.has_contact_shadow() {
        return;
    }
    let shadow = style.contact_shadow_color(ui.visuals().dark_mode);
    let radius = ui.visuals().widgets.noninteractive.corner_radius;
    let painter = ui.painter();
    painter.rect_stroke(
        rect.translate(egui::vec2(1.0, 1.0)),
        radius,
        egui::Stroke::new(1.0, shadow),
        egui::StrokeKind::Inside,
    );
    if style.has_bevel() {
        let (light, dark) = style.bevel_colors(ui.visuals().dark_mode);
        let (light_lines, dark_lines) = bevel_segments(rect, radius.nw as f32);
        for segment in light_lines {
            painter.line_segment(segment, egui::Stroke::new(1.0, light));
        }
        for segment in dark_lines {
            painter.line_segment(segment, egui::Stroke::new(1.0, dark));
        }
    }
}

/// 色带在卡片上的覆盖区：**只有顶部 3px**。
///
/// 抽成纯函数是为了能直接断言「色带不越界到内容区」——曾经用
/// 「整卡填色再盖回」实现，覆盖动作发生在内容之后，把卡片文字全刷没了。
pub fn accent_bar_rect(rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        rect.min,
        egui::pos2(rect.right(), rect.top() + ACCENT_BAR_HEIGHT),
    )
}

/// 色带高度（像素）。
pub const ACCENT_BAR_HEIGHT: f32 = 3.0;

/// 色带：卡片顶部一条 3px 主题强调色，随顶角圆弧收边。
///
/// 必须**只画顶部条带**（用 painter 的裁剪区限制），不能「整卡填色再盖回」——
/// 那样覆盖动作发生在内容画完之后，会把卡片里的文字一起刷掉。
/// 条带用整卡圆角矩形 + 裁剪到顶部 3px 得到：顶角圆弧天然对齐。
/// painter 绘制，不参与布局。
fn draw_accent_bar(ui: &egui::Ui, rect: egui::Rect) {
    let style = crate::theme::active_style(ui.ctx());
    if !style.has_accent_bar() {
        return;
    }
    let accent = ui.visuals().hyperlink_color;
    let corner = ui.visuals().widgets.noninteractive.corner_radius;
    let painter = ui.painter().with_clip_rect(accent_bar_rect(rect));
    painter.rect_filled(rect, corner, accent);
}

/// 悬停高亮的目标色。
///
/// 深色主题用强调色：彩环在暗底上好看。浅色主题的强调色多是饱和的
/// 深蓝 / 深紫（亮色主题是 1D4ED8），整圈环会显得突兀刺眼，改用中性
/// 深灰——悬停反馈照样清楚，但不和主题色打架。
fn hover_target_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        ui.visuals().hyperlink_color
    } else {
        egui::Color32::from_gray(110)
    }
}

/// 悬停高亮环：只给**平时无边框**的形状（极简 / 云朵卡片）画，
/// 沿卡片内侧 1px，宽度随悬停进度浮现。
///
/// 必须用 painter 而不是 Frame 描边：后者会把宽度算进占位尺寸，
/// 0 → 1px 的变化会让卡片在「撑开 → 收回」间震荡，整列跟着波动。
fn draw_hover_ring(ui: &egui::Ui, rect: egui::Rect, hover_t: f32, rest_stroke_width: f32) {
    if hover_t <= 0.01 || rest_stroke_width > 0.0 {
        return;
    }
    let color = hover_target_color(ui);
    let corner = ui.visuals().widgets.noninteractive.corner_radius;
    ui.painter().rect_stroke(
        rect,
        corner,
        egui::Stroke::new(hover_t, color),
        egui::StrokeKind::Inside,
    );
}

/// 凸起浮雕的内嵌线段：`(亮边, 暗边)`，各两条。
///
/// 凸起的受光方向：亮边在内侧左上，暗边在内侧右下——顶光打在凸出元素
/// 的上沿，下沿留下阴影，看起来是「立起来」而不是「凹进去」。
/// 抽成纯函数是为了能直接断言「线落在矩形内侧、且两端避开圆角」。
pub fn bevel_segments(
    rect: egui::Rect,
    radius: f32,
) -> ([[egui::Pos2; 2]; 2], [[egui::Pos2; 2]; 2]) {
    let inner = rect.shrink(1.0);
    let r = radius.min(inner.width() / 2.0).min(inner.height() / 2.0);
    let light = [
        [
            egui::pos2(inner.left() + r, inner.top()),
            egui::pos2(inner.right() - r, inner.top()),
        ],
        [
            egui::pos2(inner.left(), inner.top() + r),
            egui::pos2(inner.left(), inner.bottom() - r),
        ],
    ];
    let dark = [
        [
            egui::pos2(inner.left() + r, inner.bottom()),
            egui::pos2(inner.right() - r, inner.bottom()),
        ],
        [
            egui::pos2(inner.right(), inner.top() + r),
            egui::pos2(inner.right(), inner.bottom() - r),
        ],
    ];
    (light, dark)
}

/// 表单字段标签：**左对齐**且宽度按文本内容自适应（上限 `max_width`），
/// 既不在左侧留空白，也紧贴其后的输入框；超长时截断并悬停显示完整文本。
///
/// 注意：不能用 `add_sized` —— 它内部是 `Layout::centered_and_justified`，
/// 会把文本居中在定宽槽内。
pub fn field_label(ui: &mut egui::Ui, max_width: f32, text: impl Into<String>) -> egui::Response {
    let text = text.into();
    let font = egui::TextStyle::Body.resolve(ui.style());
    let text_width = ui
        .painter()
        .layout_no_wrap(text.clone(), font, egui::Color32::WHITE)
        .size()
        .x;
    let width = text_width.min(max_width);
    let label = egui::Label::new(egui::RichText::new(text.as_str()).weak()).truncate();
    let resp = ui
        .allocate_ui_with_layout(
            egui::vec2(width, 24.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| ui.add(label),
        )
        .inner;
    resp.on_hover_text(text)
}

/// 依次渲染卡片列表（每卡片之间附加行间距）。
pub fn card_list(
    ui: &mut egui::Ui,
    keys: &[usize],
    row_gap: f32,
    mut f: impl FnMut(&mut egui::Ui, usize),
) {
    for &idx in keys {
        f(ui, idx);
        ui.add_space(row_gap);
    }
}

/// 拖动把手的点阵间距（像素）。
pub const DRAG_HANDLE_GAP: f32 = 5.0;

/// 拖动把手点阵的圆心：按给定矩形**居中**排布。
///
/// 抽成纯函数是为了能直接断言「点阵中心与控件中心重合」——写死偏移时，
/// 控件被 `interact_size` 撑高后点阵会留在偏上的位置，与同一行其他控件对不齐。
pub fn drag_handle_dots(rect: egui::Rect) -> Vec<egui::Pos2> {
    let center = rect.center();
    let mut dots = Vec::with_capacity(6);
    for row in 0..3 {
        for col in 0..2 {
            dots.push(egui::pos2(
                center.x - DRAG_HANDLE_GAP / 2.0 + col as f32 * DRAG_HANDLE_GAP,
                center.y - DRAG_HANDLE_GAP + row as f32 * DRAG_HANDLE_GAP,
            ));
        }
    }
    dots
}

pub struct DragHandle;

impl egui::Widget for DragHandle {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let button = egui::Button::new("")
            .frame(false)
            .sense(egui::Sense::click_and_drag())
            .min_size(egui::vec2(14.0, 18.0));
        let resp = ui.add(button);
        let painter = ui.painter();
        let active = resp.hovered() || resp.dragged();
        let color = if active {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        for c in drag_handle_dots(resp.rect) {
            painter.circle_filled(c, if active { 1.5 } else { 1.0 }, color);
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        resp
    }
}

pub fn move_item<T>(items: &mut Vec<T>, from: usize, to: usize) {
    if from == to || from >= items.len() || to >= items.len() {
        return;
    }
    let item = items.remove(from);
    items.insert(to, item);
}

/// 把一张卡片的拖拽落点并入本帧的聚合结果。
///
/// 语义是「只增不减」：`found` 为 `None` 时必须保留 `acc` 中已有的落点。卡片是逐张渲染的，
/// 若每张卡片各自写入落点，后面渲染的卡片会把前面命中的落点清成 `None`，被拖到的卡片就拿不到
/// 落点边框（模型卡片的绿色边框曾因此消失）。
pub fn merge_drag_target(acc: &mut Option<String>, found: Option<String>) {
    if acc.is_none() {
        *acc = found;
    }
}

/// 密钥输入框：`show` 为 false 时掩码显示（圆点），内容仍保留。
/// 显隐切换由工具栏「显示密钥/隐藏密钥」全局按钮控制。
pub fn secret_text_edit(
    ui: &mut egui::Ui,
    value: &mut String,
    show: bool,
    width: f32,
    hint: &str,
) -> egui::Response {
    ui.add(
        egui::TextEdit::singleline(value)
            .password(!show)
            .desired_width(width)
            .hint_text(hint),
    )
}

/// 数字文本编辑框：内容非空且无法解析为数字时红色高亮并悬停提示。
/// （保存时非法数字字段会被丢弃——这里让用户在丢弃前就看到。）
pub fn numeric_text_edit(
    ui: &mut egui::Ui,
    s: &mut String,
    width: f32,
    hint: &str,
) -> egui::Response {
    let valid = s.trim().is_empty() || crate::util::parse_number_text(s).is_some();
    let edit = egui::TextEdit::singleline(s)
        .desired_width(width)
        .hint_text(hint);
    if valid {
        ui.add(edit)
    } else {
        ui.add(edit.text_color(crate::theme::semantics(ui).err))
            .on_hover_text("无效数字：保存时该字段将被忽略")
    }
}

#[cfg(test)]
mod tests {
    use super::{drag_handle_dots, merge_drag_target, DRAG_HANDLE_GAP};
    use eframe::egui;

    /// 遮罩只在「该方向还能滚」时画：滚到顶不画顶部，滚到底不画底部，
    /// 内容不足一屏两侧都不画（否则首 / 末行会被平白糊掉）。
    #[test]
    fn scroll_fade_only_shows_where_more_content_exists() {
        // 内容 1000，可视 400：可滚范围 600
        assert_eq!(super::scroll_fade_sides(400.0, 1000.0, 0.0), (false, true));
        assert_eq!(super::scroll_fade_sides(400.0, 1000.0, 300.0), (true, true));
        assert_eq!(
            super::scroll_fade_sides(400.0, 1000.0, 600.0),
            (true, false)
        );
        // 内容不足一屏：两侧都不画
        assert_eq!(super::scroll_fade_sides(400.0, 400.0, 0.0), (false, false));
        assert_eq!(super::scroll_fade_sides(400.0, 120.0, 0.0), (false, false));
        // 亚像素抖动不触发遮罩
        assert_eq!(super::scroll_fade_sides(400.0, 1000.0, 0.4), (false, true));
        assert_eq!(
            super::scroll_fade_sides(400.0, 1000.0, 599.8),
            (true, false)
        );
    }

    /// 色带只覆盖卡片顶部 3px：越界就会盖住卡片文字（曾经整卡刷白）。
    #[test]
    fn accent_bar_covers_only_the_top_strip() {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(200.0, 60.0));
        let bar = super::accent_bar_rect(rect);
        assert_eq!(bar.height(), super::ACCENT_BAR_HEIGHT);
        assert_eq!(bar.top(), rect.top());
        assert_eq!(bar.left(), rect.left());
        assert_eq!(bar.right(), rect.right());
        // 必须远小于卡片高度，绝不能盖到内容。
        assert!(bar.bottom() < rect.top() + 10.0, "色带侵入内容区：{bar:?}");
    }

    /// 点阵的几何中心必须与控件矩形中心重合：控件被 `interact_size` 撑高
    /// （14x18 的按钮放进 24px 高的行里）后，点阵仍要落在中间。
    #[test]
    fn drag_handle_dots_are_centered_in_the_rect() {
        for rect in [
            egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(14.0, 18.0)),
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(14.0, 24.0)),
            egui::Rect::from_min_size(egui::pos2(3.0, 7.0), egui::vec2(14.0, 30.0)),
        ] {
            let dots = drag_handle_dots(rect);
            assert_eq!(dots.len(), 6);
            let min_x = dots.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
            let max_x = dots.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
            let min_y = dots.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
            let max_y = dots.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);
            let cx = (min_x + max_x) / 2.0;
            let cy = (min_y + max_y) / 2.0;
            assert!(
                (cx - rect.center().x).abs() < 0.01 && (cy - rect.center().y).abs() < 0.01,
                "点阵中心 ({cx}, {cy}) 与控件中心 {:?} 不重合",
                rect.center()
            );
            // 间距就是常量：两列相距 gap，三行跨 2*gap。
            assert!((max_x - min_x - DRAG_HANDLE_GAP).abs() < 0.01);
            assert!((max_y - min_y - 2.0 * DRAG_HANDLE_GAP).abs() < 0.01);
        }
    }

    /// 凸起浮雕线要落在矩形内侧，且两端避开圆角；方向必须是
    /// 「亮边在上 / 左、暗边在下 / 右」（凸起受光，不是凹进去）。
    #[test]
    fn bevel_lines_stay_inside_and_clear_the_corners() {
        use super::bevel_segments;
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(120.0, 60.0));
        let radius = 4.0;
        let (light, dark) = bevel_segments(rect, radius);
        let [top, left] = light;
        // 都在矩形内侧 1px（Frame 的描边占掉那一圈）。
        assert!(top[0].y > rect.top() && top[0].y < rect.bottom());
        assert!(left[0].x > rect.left() && left[0].x < rect.right());
        assert!((top[0].x - (rect.left() + 1.0 + radius)).abs() < 0.01);
        assert!((top[1].x - (rect.right() - 1.0 - radius)).abs() < 0.01);
        assert!((left[0].y - (rect.top() + 1.0 + radius)).abs() < 0.01);
        assert!((left[1].y - (rect.bottom() - 1.0 - radius)).abs() < 0.01);
        // 暗边在下 / 右，与亮边对称（凸起受光方向）。
        let [bottom, right] = dark;
        assert!(bottom[0].y < rect.bottom() && bottom[0].y > rect.top());
        assert!(right[0].x < rect.right() && right[0].x > rect.left());
        assert!((bottom[0].x - top[0].x).abs() < 0.01);
        assert!((right[0].y - left[0].y).abs() < 0.01);
    }

    /// 圆角大到超过矩形一半时，线段不能反向（起点跑到终点右边）。
    #[test]
    fn bevel_lines_stay_ordered_with_a_huge_radius() {
        use super::bevel_segments;
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(10.0, 8.0));
        let (light, dark) = bevel_segments(rect, 40.0);
        let [top, left] = light;
        assert!(top[0].x <= top[1].x, "上边线反向了：{top:?}");
        assert!(left[0].y <= left[1].y, "左边线反向了：{left:?}");
        let [bottom, right] = dark;
        assert!(bottom[0].x <= bottom[1].x, "下边线反向了：{bottom:?}");
        assert!(right[0].y <= right[1].y, "右边线反向了：{right:?}");
    }

    #[test]
    fn merge_drag_target_keeps_earlier_hit() {
        let mut acc = None;
        // 先渲染的卡片未命中：聚合结果保持为空
        merge_drag_target(&mut acc, None);
        assert_eq!(acc, None);
        // 命中落点
        merge_drag_target(&mut acc, Some("p\u{1f}m".to_string()));
        assert_eq!(acc.as_deref(), Some("p\u{1f}m"));
        // 后面还有卡片展开且未命中：不能把已命中的落点清掉
        merge_drag_target(&mut acc, None);
        assert_eq!(acc.as_deref(), Some("p\u{1f}m"));
        // 已命中时后续命中不覆盖：保持首个落点，行为稳定
        merge_drag_target(&mut acc, Some("other".to_string()));
        assert_eq!(acc.as_deref(), Some("p\u{1f}m"));
    }

    #[test]
    fn merge_drag_target_stays_none_without_hit() {
        let mut acc = None;
        merge_drag_target(&mut acc, None);
        merge_drag_target(&mut acc, None);
        assert_eq!(acc, None);
    }
}
