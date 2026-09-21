use eframe::egui;

/// 本帧请求的自定义抓取光标。
///
/// 控件只「提出请求」，由 `App::update` 在帧末统一落到 Win32（见
/// `crate::cursor::set_custom_cursor`）。这样跨平台构建不需要 `cfg`，
/// 而且同一帧里多个热区只产生一次系统调用。
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum GrabCursor {
    #[default]
    None,
    /// 悬停在可拖动处：系统自带的手形光标（`IDC_HAND`）。
    Palm,
    /// 已按住：握起的拳头。
    Fist,
}

fn grab_cursor_id() -> egui::Id {
    egui::Id::new("model_harbor_grab_cursor")
}

/// 请求本帧的抓取光标。同一帧里取「更强」的一态（Fist > Palm > None）：
/// 拖动中的把手不应被旁边另一个只是悬停的热区降级成手掌。
pub fn request_grab_cursor(ctx: &egui::Context, want: GrabCursor) {
    if want == GrabCursor::None {
        return;
    }
    ctx.data_mut(|d| {
        let cur = d
            .get_temp::<GrabCursor>(grab_cursor_id())
            .unwrap_or_default();
        let next = if cur == GrabCursor::Fist || want == GrabCursor::Fist {
            GrabCursor::Fist
        } else {
            GrabCursor::Palm
        };
        d.insert_temp(grab_cursor_id(), next);
    });
}

/// 取出并清空本帧请求（帧末调用一次，避免状态泄漏到下一帧）。
pub fn take_grab_cursor(ctx: &egui::Context) -> GrabCursor {
    ctx.data_mut(|d| {
        let cur = d
            .get_temp::<GrabCursor>(grab_cursor_id())
            .unwrap_or_default();
        d.remove::<GrabCursor>(grab_cursor_id());
        cur
    })
}

/// 悬停态：手掌；按下态：拳头。
///
/// 用于拖动把手这类「按下才开始拖」的控件——`dragged()` 要越过拖动阈值才为真，
/// 按下但还没移动的那几帧会没有反馈，所以用 `is_pointer_button_down_on()` 判按下。
pub fn grab_cursor_for(resp: &egui::Response) -> GrabCursor {
    if resp.is_pointer_button_down_on() || resp.dragged() {
        GrabCursor::Fist
    } else if resp.hovered() {
        GrabCursor::Palm
    } else {
        GrabCursor::None
    }
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
        1 => (DRAG_SOURCE_COLOR, 2.0), // source: orange
        2 => (DROP_TARGET_COLOR, 2.0), // target: green
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
        egui::Stroke::new(1.0f32, shadow),
        egui::StrokeKind::Inside,
    );
    if style.has_bevel() {
        let (light, dark) = style.bevel_colors(ui.visuals().dark_mode);
        let (light_lines, dark_lines) = bevel_segments(rect, radius.nw as f32);
        for segment in light_lines {
            painter.line_segment(segment, egui::Stroke::new(1.0f32, light));
        }
        for segment in dark_lines {
            painter.line_segment(segment, egui::Stroke::new(1.0f32, dark));
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

/// 拖动**源**（正被抓着的那一项）的高亮色：卡片描边与把手点阵共用同一个值，
/// 两处各写一份常量时改了一边就会「把手是橙色、卡片是别的颜色」。
pub const DRAG_SOURCE_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 180, 50);

/// 拖动源把手的底色：同色压暗，垫在点阵下面，避免高饱和色块盖过点阵。
pub const DRAG_SOURCE_FILL: egui::Color32 = egui::Color32::from_rgba_premultiplied(70, 49, 13, 90);

/// 拖动**落点**（当前指针所在的目标项）的高亮色：绿色。
///
/// 与 [`DRAG_SOURCE_COLOR`] 同源：卡片描边、页签选中态都用这两个值，
/// 各写一份常量就会出现「卡片是绿、页签是别的颜色」。
pub const DROP_TARGET_COLOR: egui::Color32 = egui::Color32::from_rgb(100, 200, 100);

/// 落点/选中态色底的压暗版，垫在图标或点阵下面。
pub const DROP_TARGET_FILL: egui::Color32 = egui::Color32::from_rgba_premultiplied(27, 55, 27, 90);

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
        let dragging = resp.dragged();
        let active = resp.hovered() || dragging;
        // 拖动中除了提亮点阵，还铺一层高亮底色：卡片被拖时整卡会亮起橙色边框，
        // 把手作为「抓取点」也得有同源的选中态，否则光标一走开就看不出抓着谁。
        if dragging {
            ui.painter().rect_filled(resp.rect, 3.0, DRAG_SOURCE_FILL);
        }
        let color = if dragging {
            DRAG_SOURCE_COLOR
        } else if active {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        // 点阵半径在拖动时再放大一档，与边框一起构成「已抓起」的观感。
        let radius = if dragging {
            1.75
        } else if active {
            1.5
        } else {
            1.0
        };
        let painter = ui.painter();
        for c in drag_handle_dots(resp.rect) {
            painter.circle_filled(c, radius, color);
        }
        // 光标：悬停是张开的手掌、按住才是握起的拳头，与真实桌面软件一致
        // （见 `crate::cursor`）。**不要用 egui 的 `CursorIcon::Grab`**：它在 Windows 上
        // 经 winit 映射成 `IDC_SIZEALL`（四向箭头），看着像「可移动」而不是「抓住」。
        //
        // 自定义光标是整窗生效的（子类过程拦 WM_SETCURSOR），因此这里只提出请求，
        // 由 `App::update` 帧末统一提交；`PointingHand` 作为非 Windows 或光标
        // 句柄创建失败时的兜底。
        let want = grab_cursor_for(&resp);
        if want != GrabCursor::None {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            request_grab_cursor(ui.ctx(), want);
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

    /// 同一帧里多个热区报状态时取「更强」的一态：正被拖的把手（Fist）不能被旁边
    /// 只是悬停的控件降级成手掌，否则拖动中光标会在手掌/拳头之间闪。
    #[test]
    fn grab_cursor_requests_take_the_strongest_state_in_a_frame() {
        use super::{take_grab_cursor, GrabCursor};

        let ctx = egui::Context::default();
        // 没有任何请求：默认熄灭，且 take 之后必须清空（否则状态会泄漏到下一帧）。
        assert_eq!(take_grab_cursor(&ctx), GrabCursor::None);
        assert_eq!(take_grab_cursor(&ctx), GrabCursor::None);

        // None 请求本身不写入任何状态。
        super::request_grab_cursor(&ctx, GrabCursor::None);
        assert_eq!(take_grab_cursor(&ctx), GrabCursor::None);

        // 手掌先到、拳头后到 -> 拳头。
        super::request_grab_cursor(&ctx, GrabCursor::Palm);
        super::request_grab_cursor(&ctx, GrabCursor::Fist);
        assert_eq!(take_grab_cursor(&ctx), GrabCursor::Fist);

        // 拳头先到、手掌后到 -> 仍是拳头（不能降级）。
        super::request_grab_cursor(&ctx, GrabCursor::Fist);
        super::request_grab_cursor(&ctx, GrabCursor::Palm);
        assert_eq!(take_grab_cursor(&ctx), GrabCursor::Fist);

        // 只有手掌 -> 手掌。
        super::request_grab_cursor(&ctx, GrabCursor::Palm);
        assert_eq!(take_grab_cursor(&ctx), GrabCursor::Palm);

        // take 清空后，下一帧从零开始。
        assert_eq!(take_grab_cursor(&ctx), GrabCursor::None);
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
