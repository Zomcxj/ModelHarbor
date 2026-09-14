use crate::backends;
use crate::convert;
use crate::credentials;
use crate::format::{ConfigFormat, ConfigPaths};
use crate::model::{AgentRow, ModelRow, ProviderRow};
use crate::theme::Theme;
use crate::ui::{
    card_frame, card_list, field_label, merge_drag_target, move_item, numeric_text_edit,
    secret_text_edit, DragHandle,
};
use crate::util::{self, is_wsl_path, parse_number_text, show_file_dialog};
use eframe::egui;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

mod serialize;

pub(crate) use serialize::{compact_json, pretty_json};

// 序列化实现细节只在同模块测试中使用（glob 再导出不会穿透到子模块的 use super::*）。
#[cfg(test)]
use serialize::{serialize_object, CompactRole};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum SaveFormat {
    Current,
    #[default]
    Compact,
}

impl SaveFormat {
    fn label(self) -> &'static str {
        match self {
            Self::Current => "默认格式",
            Self::Compact => "压缩格式",
        }
    }
}

/// 一个保存目标的运行时状态（可用性 / 解析路径 / 勾选）。
struct SaveTarget {
    backend: ConfigFormat,
    available: bool,
    path: String,
}

/// 单个 provider 的模型获取状态（后台线程 + 通道）。
struct ModelFetchState {
    rx: Option<std::sync::mpsc::Receiver<Result<Vec<String>, String>>>,
    result: Option<Result<Vec<String>, String>>,
}

/// 新增 Provider 表单使用固定的内部 key 保存获取状态。
const NEW_PROVIDER_FETCH_KEY: &str = "__new_provider__";
/// 预览编辑框的固定 id（切页时需要主动释放焦点，见 reset_preview_draft）。
const PREVIEW_EDITOR_ID: &str = "preview_editor";
/// 预览框内停止输入多久后，允许用组件状态重建草稿（秒）。
const PREVIEW_EDIT_IDLE_SECS: f64 = 2.0;

/// 吸顶标题占位：在内容流中预留标题行高度，返回绘制锚点。
/// 必须与 [`sticky_end`] 配对，并在 section 内容渲染完成后调用 sticky_end，
/// 以保证标题最后绘制（否则会被下方滚动内容覆盖）。
fn sticky_begin(ui: &mut egui::Ui, height: f32) -> (f32, f32, f32, f32) {
    let avail = ui.available_rect_before_wrap();
    ui.allocate_exact_size(egui::vec2(avail.width(), height), egui::Sense::hover());
    (avail.top(), avail.left(), avail.right(), height)
}

/// 绘制吸顶标题：未滚过时留在内容流中；滚动越过视口顶部后吸附在滚动区顶部。
fn sticky_end(ui: &mut egui::Ui, anchor: (f32, f32, f32, f32), paint: impl FnOnce(&mut egui::Ui)) {
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

/// 错误摘要：HTTP 状态码简写（如 HTTP 403），其他错误截断为短文本。
fn short_err(err: &str) -> String {
    if let Some(rest) = err.strip_prefix("HTTP ") {
        let code = rest.split('（').next().unwrap_or(rest);
        format!("HTTP {}", code)
    } else {
        let t = err.trim();
        if t.chars().count() > 96 {
            let mut s: String = t.chars().take(96).collect();
            s.push('…');
            s
        } else {
            t.to_string()
        }
    }
}

/// 网络错误文本脱敏：ureq 的 Transport Display 会包含目标 URL，
/// 若用户把凭据放进 URL query（如 `?key=sk-…`）会随错误泄漏到
/// 状态栏/悬停提示；剥离 URL 的 query/fragment 后返回。
fn sanitize_network_error(text: &str) -> String {
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

// ---------- 预览语法高亮（VSCode Dark+ 配色） ----------

/// 预览文本语法：opencode 页面为 JSON(C)，pi / omp / DSH 为 YAML。
#[derive(Clone, Copy, PartialEq, Eq)]
enum PreviewSyntax {
    Json,
    Yaml,
}

const SYN_KEY: egui::Color32 = egui::Color32::from_rgb(0x9C, 0xDC, 0xFE);
const SYN_STRING: egui::Color32 = egui::Color32::from_rgb(0xCE, 0x91, 0x78);
const SYN_NUMBER: egui::Color32 = egui::Color32::from_rgb(0xB5, 0xCE, 0xA8);
const SYN_LITERAL: egui::Color32 = egui::Color32::from_rgb(0x56, 0x9C, 0xD6);
const SYN_COMMENT: egui::Color32 = egui::Color32::from_rgb(0x6A, 0x99, 0x55);
const SYN_PUNCT: egui::Color32 = egui::Color32::from_rgb(0xD4, 0xD4, 0xD4);

/// 按语法扫描文本，返回 `(字节起, 字节止, 颜色)` 段落（边界均在字符边界上）。
fn syntax_tokens(text: &str, syntax: PreviewSyntax) -> Vec<(usize, usize, egui::Color32)> {
    match syntax {
        PreviewSyntax::Json => json_tokens(text),
        PreviewSyntax::Yaml => yaml_tokens(text),
    }
}

/// JSON / JSONC：字符串（键与值分开着色）、注释、数字、字面量、标点。
fn json_tokens(text: &str) -> Vec<(usize, usize, egui::Color32)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        i = match b[i] {
            b'"' => json_string_at(b, i, &mut out),
            b'/' if b.get(i + 1) == Some(&b'/') => json_line_comment_at(b, i, &mut out),
            b'/' if b.get(i + 1) == Some(&b'*') => json_block_comment_at(b, i, &mut out),
            c if c == b'-' || c.is_ascii_digit() => json_number_at(b, i, &mut out),
            c if c.is_ascii_alphabetic() => json_literal_at(text, i, &mut out),
            b'{' | b'}' | b'[' | b']' | b',' | b':' => {
                out.push((i, i + 1, SYN_PUNCT));
                i + 1
            }
            _ => i + 1,
        };
    }
    out
}

/// 扫描一个 JSON 字符串（含两侧引号）并着色，返回结束位置。
fn json_string_at(b: &[u8], start: usize, out: &mut Vec<(usize, usize, egui::Color32)>) -> usize {
    let mut i = start + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => {
                i += 1;
                break;
            }
            _ => i += 1,
        }
    }
    let end = i.min(b.len());
    // 后面紧跟冒号则为对象键（VSCode Dark+ 用不同颜色）。
    let color = if json_followed_by_colon(b, end) {
        SYN_KEY
    } else {
        SYN_STRING
    };
    out.push((start, end, color));
    end
}

/// 跳过空白后是否为 `:`：用于判断字符串是对象键还是普通值。
fn json_followed_by_colon(b: &[u8], from: usize) -> bool {
    let mut j = from;
    while j < b.len() && (b[j] as char).is_ascii_whitespace() {
        j += 1;
    }
    b.get(j) == Some(&b':')
}

/// `//` 行注释：着色到行尾（不含换行）。
fn json_line_comment_at(
    b: &[u8],
    start: usize,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) -> usize {
    let mut i = start;
    while i < b.len() && b[i] != b'\n' {
        i += 1;
    }
    out.push((start, i, SYN_COMMENT));
    i
}

/// `/* ... */` 块注释：未闭合时着色到文本末尾。
fn json_block_comment_at(
    b: &[u8],
    start: usize,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) -> usize {
    let mut i = start + 2;
    while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
        i += 1;
    }
    let end = (i + 2).min(b.len());
    out.push((start, end, SYN_COMMENT));
    end
}

/// 数字（含前导 `-` 与指数部分）。
fn json_number_at(b: &[u8], start: usize, out: &mut Vec<(usize, usize, egui::Color32)>) -> usize {
    let mut i = start;
    if b[i] == b'-' {
        i += 1;
    }
    while i < b.len() && (b[i].is_ascii_digit() || matches!(b[i], b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        i += 1;
    }
    out.push((start, i, SYN_NUMBER));
    i
}

/// 字面量 `true` / `false` / `null`；未命中则前进一个字节。
fn json_literal_at(text: &str, i: usize, out: &mut Vec<(usize, usize, egui::Color32)>) -> usize {
    let rest = &text[i..];
    for lit in ["true", "false", "null"] {
        if rest.starts_with(lit) {
            out.push((i, i + lit.len(), SYN_LITERAL));
            return i + lit.len();
        }
    }
    i + 1
}

/// YAML：注释、`---` 文档标记、列表项、键、引号/裸标量、数字、布尔。
fn yaml_tokens(text: &str) -> Vec<(usize, usize, egui::Color32)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut line_start = 0;
    while line_start <= b.len() {
        let line_end = yaml_line_end(b, line_start);
        let indent_end = yaml_indent_end(b, line_start, line_end);
        let value_start = yaml_head_token(b, indent_end, line_end, &mut out);
        yaml_value_tokens(text, b, value_start, line_start, line_end, &mut out);
        if line_end >= b.len() {
            break;
        }
        line_start = line_end + 1;
    }
    out
}

/// 当前行换行符位置（无换行则为文本末尾）。
fn yaml_line_end(b: &[u8], line_start: usize) -> usize {
    b[line_start..]
        .iter()
        .position(|&c| c == b'\n')
        .map(|p| line_start + p)
        .unwrap_or(b.len())
}

/// 跳过行首空格 / 制表符后的位置。
fn yaml_indent_end(b: &[u8], line_start: usize, line_end: usize) -> usize {
    let mut i = line_start;
    while i < line_end && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    i
}

/// 行首标记：`---` / `...` 文档标记、`- ` 列表项、`key:` 键；返回值扫描起点。
fn yaml_head_token(
    b: &[u8],
    i: usize,
    line_end: usize,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) -> usize {
    if line_end.saturating_sub(i) >= 3 && (b[i..i + 3] == *b"---" || b[i..i + 3] == *b"...") {
        out.push((i, i + 3, SYN_PUNCT));
        return i + 3;
    }
    if i >= line_end || b[i] == b'#' {
        return i;
    }
    let mut i = i;
    // 列表项 "- "
    if b[i] == b'-' && (i + 1 >= line_end || b[i + 1] == b' ' || b[i + 1] == b'\t') {
        out.push((i, i + 1, SYN_PUNCT));
        i += 1;
    }
    // 键：到 ':' 为止（行内 '#' 起为注释）
    if let Some(colon) = yaml_key_colon(b, i, line_end) {
        if colon > i {
            out.push((i, colon, SYN_KEY));
        }
        out.push((colon, colon + 1, SYN_PUNCT));
        i = colon + 1;
    }
    i
}

/// 行内第一个键分隔冒号；遇到注释起点则视为无键。
fn yaml_key_colon(b: &[u8], from: usize, line_end: usize) -> Option<usize> {
    let mut j = from;
    while j < line_end {
        if b[j] == b'#' && (j == from || b[j - 1] == b' ' || b[j - 1] == b'\t') {
            return None;
        }
        if b[j] == b':' {
            return Some(j);
        }
        j += 1;
    }
    None
}

/// 行内值部分：空白、注释、引号标量、数字、裸标量。
fn yaml_value_tokens(
    text: &str,
    b: &[u8],
    mut i: usize,
    line_start: usize,
    line_end: usize,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) {
    while i < line_end {
        i = match b[i] {
            b' ' | b'\t' => i + 1,
            b'#' => {
                out.push((i, line_end, SYN_COMMENT));
                line_end
            }
            q @ (b'"' | b'\'') => yaml_quoted_at(b, i, line_end, q, out),
            c if c == b'-' || c.is_ascii_digit() => yaml_number_at(b, i, line_end, out),
            _ => yaml_plain_scalar(text, b, i, line_start, line_end, out),
        };
    }
}

/// 引号标量：`"` 支持反斜杠转义，`'` 不支持；未闭合时着色到行尾。
fn yaml_quoted_at(
    b: &[u8],
    start: usize,
    line_end: usize,
    quote: u8,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) -> usize {
    let mut i = start + 1;
    while i < line_end {
        if quote == b'"' && b[i] == b'\\' {
            i += 2;
            continue;
        }
        if b[i] == quote {
            i += 1;
            break;
        }
        i += 1;
    }
    out.push((start, i.min(line_end), SYN_STRING));
    i
}

/// 数字（含前导 `-` 与指数部分）。
fn yaml_number_at(
    b: &[u8],
    start: usize,
    line_end: usize,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) -> usize {
    let mut i = start;
    if b[i] == b'-' {
        i += 1;
    }
    while i < line_end
        && (b[i].is_ascii_digit() || matches!(b[i], b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        i += 1;
    }
    out.push((start, i, SYN_NUMBER));
    i
}

/// 裸标量：布尔 / null / `~` 等着色为字面量，其余为字符串。
fn yaml_plain_scalar(
    text: &str,
    b: &[u8],
    start: usize,
    line_start: usize,
    line_end: usize,
    out: &mut Vec<(usize, usize, egui::Color32)>,
) -> usize {
    let mut i = start;
    while i < line_end {
        if b[i] == b'#' && i > line_start && b[i - 1] == b' ' {
            break;
        }
        i += 1;
    }
    let color = match text[start..i].trim_end() {
        "true" | "false" | "null" | "~" | "yes" | "no" | "on" | "off" => SYN_LITERAL,
        _ => SYN_STRING,
    };
    out.push((start, i, color));
    i
}

/// 把查找命中的底色叠加到已按语法着色的 LayoutJob 上（按 section 拆分，保留前景色）。
fn apply_find_background(
    job: &mut egui::text::LayoutJob,
    matches: &[(usize, usize)],
    current: usize,
) {
    for (i, &(start, end)) in matches.iter().enumerate() {
        if start >= end {
            continue;
        }
        let bg = if i == current {
            egui::Color32::from_rgb(150, 105, 25)
        } else {
            egui::Color32::from_rgb(92, 80, 28)
        };
        let mut out = Vec::with_capacity(job.sections.len() + 2);
        for section in job.sections.drain(..) {
            let (s, e) = (section.byte_range.start, section.byte_range.end);
            let (is, ie) = (start.max(s), end.min(e));
            if is >= ie {
                out.push(section);
                continue;
            }
            if s < is {
                let mut pre = section.clone();
                pre.byte_range = s..is;
                out.push(pre);
            }
            let mut mid = section.clone();
            mid.byte_range = is..ie;
            mid.leading_space = 0.0;
            mid.format.background = bg;
            out.push(mid);
            if ie < e {
                let mut post = section;
                post.byte_range = ie..e;
                post.leading_space = 0.0;
                out.push(post);
            }
        }
        job.sections = out;
    }
}

/// 单个 provider 的延迟测试状态（provider 级 + 模型级并发）。
#[derive(Default)]
struct LatencyState {
    /// provider 级：模型列表接口往返耗时（毫秒）。
    provider: Option<Result<u64, String>>,
    provider_rx: Option<std::sync::mpsc::Receiver<Result<u64, String>>>,
    /// 模型级：模型 id → 往返耗时（毫秒）。
    models: HashMap<String, Result<u64, String>>,
    model_rx: Option<std::sync::mpsc::Receiver<(String, Result<u64, String>)>>,
    /// 模型级测试进度：已完成 / 总数。
    done: usize,
    total: usize,
    /// 本次测试中尚未返回结果的模型 id（用于显示测试中的乱码动画）。
    pending: HashSet<String>,
}

/// 延迟测试的读取超时与「超时」判定阈值（毫秒）：单个读操作 / 首字的等待上限。
const LATENCY_TIMEOUT_MS: u64 = 10_000;
/// 首字延迟着色阈值（毫秒）：低于此值为绿色。
///（首字延迟量级远小于整段生成耗时，不能沿用同步请求的 5 秒口径。）
const LATENCY_GOOD_MS: u64 = 2_000;
/// 首字延迟超过此值算慢（红）。
const LATENCY_SLOW_MS: u64 = 5_000;

/// 延迟配色：<2s 绿色、2~5s 黄色、≥5s 红色（超时同样显示红色错误）。
const LATENCY_GREEN: egui::Color32 = egui::Color32::from_rgb(90, 180, 110);
const LATENCY_YELLOW: egui::Color32 = egui::Color32::from_rgb(201, 162, 39);
const LATENCY_RED: egui::Color32 = egui::Color32::from_rgb(220, 90, 90);

/// 模型延迟探测的风控节流参数（中转站的「多 IP 检测 / 测活封号」）：
/// **同一个 provider** 的任意两次探测（同模型、不同模型都算）间隔 ≥5 秒；
/// **不同 provider 互不牵连**（不同中转站是不同站点，各自独立计数、可并行）。
const PROBE_PROVIDER_GAP_S: f64 = 5.0;

/// 探测用的中性短问句库：跨领域的常识名词（地理 / 天文 / 生物 / 化学 / 物理 /
/// 文学 / 艺术 / 音乐 / 历史），都是「一句话能答」的定论型题目。
///
/// 目的只是让请求看起来像普通对话而不是脚本测活（**不校验答案**：判定只看 HTTP
/// 是否成功与往返耗时），所以题目要求「短、无歧义、与政治 / 敏感话题无关」。
/// 题目太浅（如「1 加 1 等于几」）反而不像真人在问，所以选题偏向各领域的常识名词。
const PROBE_QUESTIONS: [&str; 24] = [
    "世界上最长的河流是哪条？只回答河名",
    "世界上面积最大的国家是哪个？只回答国名",
    "世界上最高的山峰叫什么？只回答山名",
    "澳大利亚的首都是哪座城市？只回答城市名",
    "尼罗河位于哪个大洲？只回答大洲名",
    "地中海位于欧洲和哪个大洲之间？只回答大洲名",
    "太阳系中体积最大的行星是哪颗？只回答行星名",
    "太阳系中离太阳最近的行星是哪颗？只回答行星名",
    "地球的天然卫星叫什么？只回答名称",
    "北斗七星属于哪个星座？只回答星座名",
    "人体面积最大的器官是什么？只回答名称",
    "一个健康的成年人全身大约有多少块骨头？只回答数字",
    "血液中负责运输氧气的是哪种细胞？只回答名称",
    "植物通过光合作用吸收哪种气体？只回答化学式",
    "水的化学式是什么？只回答化学式",
    "食盐的主要成分是什么？只回答化学式",
    "空气中含量最多的气体是什么？只回答化学式",
    "常温常压下唯一呈液态的金属是哪种？只回答名称",
    "《红楼梦》的作者是谁？只回答姓名",
    "《论语》主要记录了哪位思想家及其弟子的言行？只回答姓名",
    "《蒙娜丽莎》的作者是谁？只回答姓名",
    "《命运交响曲》的作曲家是谁？只回答姓名",
    "中国古代四大发明中用于辨别方向的是哪一项？只回答名称",
    "泰姬陵位于哪个国家？只回答国名",
];

/// FNV-1a：给 provider key 一个稳定的起始题号偏移，避免所有 provider 都从第 1 题开始。
fn fnv1a(value: &str) -> usize {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash as usize
}

/// 取一条探测问句：provider 偏移 + 轮换游标，同一 provider 连续探测不重复同一题。
fn probe_question(provider_key: &str, cursor: usize) -> &'static str {
    let index = fnv1a(provider_key).wrapping_add(cursor) % PROBE_QUESTIONS.len();
    PROBE_QUESTIONS[index]
}

/// 单次探测的门控结论。
#[derive(Clone, Debug, PartialEq)]
enum ProbeGateState {
    /// 可以探测。
    Ready,
    /// 节流冷却中，剩余秒数。
    Cooling(f64),
    /// 全局串行：上一个探测还没结束。
    Busy,
    /// 网络守卫拦截（系统代理 / VPN）。
    NetBlocked(String),
}

/// 模型延迟探测的节流与串行状态（按 provider 各自一份）。
///
/// 纯内存、重启清零：重启后只能手动一个个点，人手点击的节奏本身不构成突发，
/// 因此无需落盘（写盘也不增加实际安全性）。
#[derive(Default)]
struct ProbeGate {
    /// provider key → 上次探测时刻（egui 秒）。
    last_provider: HashMap<String, f64>,
    /// 正在飞的探测（provider key）：同一 provider 一次只允许一个。
    in_flight: HashSet<String>,
    /// 每个 provider 的题目轮换游标。
    cursor: HashMap<String, usize>,
}

impl ProbeGate {
    /// 累加一个「还需等待」的约束。
    fn accumulate(wait: &mut Option<f64>, left: Option<f64>) {
        if let Some(left) = left.filter(|left| *left > 0.0) {
            *wait = Some(match *wait {
                Some(current) => current.max(left),
                None => left,
            });
        }
    }

    /// 当前是否允许探测；不允许时给出原因（按钮禁用 + 倒计时 + 悬停说明）。
    ///
    /// 只看本 provider 的状态，不看别的 provider（跨厂商不排队）。
    fn state(&self, provider_key: &str, now: f64, net_guard: Option<&str>) -> ProbeGateState {
        if let Some(reason) = net_guard {
            return ProbeGateState::NetBlocked(reason.to_string());
        }
        // 同一 provider 一次只允许一个探测在飞：探测最长 10 秒，可能超过 5 秒间隔，
        // 否则会同时向同一个中转站发两个请求。
        if self.in_flight.contains(provider_key) {
            return ProbeGateState::Busy;
        }
        let mut wait: Option<f64> = None;
        Self::accumulate(
            &mut wait,
            self.last_provider
                .get(provider_key)
                .map(|at| PROBE_PROVIDER_GAP_S - (now - at)),
        );
        match wait {
            Some(left) => ProbeGateState::Cooling(left),
            None => ProbeGateState::Ready,
        }
    }

    /// 记录一次探测并占用该 provider 的串行位。
    fn start(&mut self, provider_key: &str, now: f64) {
        self.last_provider.insert(provider_key.to_string(), now);
        self.in_flight.insert(provider_key.to_string());
    }

    /// 探测结束（拿到结果 / 失败 / 超时）后释放该 provider 的串行位。
    fn finish(&mut self, provider_key: &str) {
        self.in_flight.remove(provider_key);
    }

    /// 探测状态整体被丢弃时释放串行位（重载配置 / 关闭新增表单）。
    ///
    /// 不释放的话，被丢弃的通道不会再有结果回传，该 provider 会永久卡在 Busy。
    /// `provider_key` 为 `None` 表示全部释放。
    fn release(&mut self, provider_key: Option<&str>) {
        match provider_key {
            Some(key) => {
                self.in_flight.remove(key);
            }
            None => self.in_flight.clear(),
        }
    }

    /// 取该 provider 的下一条探测问句并推进游标。
    fn next_question(&mut self, provider_key: &str) -> &'static str {
        let cursor = self.cursor.entry(provider_key.to_string()).or_insert(0);
        let question = probe_question(provider_key, *cursor);
        *cursor = cursor.wrapping_add(1);
        question
    }
}

/// 延迟测试用的 HTTP 客户端（较短超时，避免卡住 UI 线程池）。
///
/// **不要给它设置 `Proxy`**：ureq 默认不使用系统代理，探测请求始终直连，开着 Clash /
/// VPN 时也不会从代理出口发出（中转站的「多 IP 检测」看的正是出口 IP）。
/// 一旦在这里引入 `Proxy::try_from_env()`，探测就会改走代理口，务必保持直连。
fn latency_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(5))
        .timeout_read(std::time::Duration::from_millis(LATENCY_TIMEOUT_MS))
        .build()
}

fn http_error(err: ureq::Error, elapsed: u64) -> String {
    match err {
        ureq::Error::Status(code, _) => format!("HTTP {}（{} ms）", code, elapsed),
        ureq::Error::Transport(t) => {
            format!("网络错误：{}", sanitize_network_error(&t.to_string()))
        }
    }
}

fn latency_color(ms: u64) -> egui::Color32 {
    if ms < LATENCY_GOOD_MS {
        LATENCY_GREEN
    } else if ms < LATENCY_SLOW_MS {
        LATENCY_YELLOW
    } else {
        LATENCY_RED
    }
}

/// 延迟测试进行中的「乱码」动画字符集（半角片假名 + 数字，参考 MemoPaws 密钥页）。
const MATRIX_CHARS: &str = "ｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝ0123456789";
/// 乱码动画每帧时长（毫秒）与每行字符数。
const MATRIX_FRAME_MS: u64 = 70;
const MATRIX_LEN: usize = 8;

/// 生成一帧乱码：同一帧号 + 同一 salt 结果稳定（不保存随机状态）。
fn matrix_glyphs(frame: u64, salt: &str, len: usize) -> String {
    let mut state = 0xcbf2_9ce4_8422_2325u64 ^ frame.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    for byte in salt.as_bytes() {
        state ^= u64::from(*byte);
        state = state.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let chars: Vec<char> = MATRIX_CHARS.chars().collect();
    let mut out = String::with_capacity(len);
    for _ in 0..len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.push(chars[(state % chars.len() as u64) as usize]);
    }
    out
}

/// 测试进行中的乱码标签：每帧换一组字符，并请求下一次重绘。
fn matrix_label(ui: &mut egui::Ui, salt: &str) {
    let frame = (ui.ctx().input(|i| i.time) * 1000.0 / MATRIX_FRAME_MS as f64) as u64;
    ui.label(
        egui::RichText::new(matrix_glyphs(frame, salt, MATRIX_LEN))
            .monospace()
            .color(LATENCY_GREEN),
    )
    .on_hover_text("延迟测试进行中");
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(MATRIX_FRAME_MS));
}

/// 模型行延迟显示：测试中显示乱码动画，完成后按阈值着色。
fn model_latency_label(ui: &mut egui::Ui, state: Option<&LatencyState>, model_id: &str) {
    let Some(state) = state else {
        return;
    };
    match state.models.get(model_id) {
        Some(Ok(ms)) => {
            // 固定宽度右对齐：数字位数变化时按钮不会左右跳动。
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(72.0, ui.spacing().interact_size.y),
                egui::Sense::hover(),
            );
            ui.painter().text(
                rect.left_center(),
                egui::Align2::LEFT_CENTER,
                format!("{}ms", ms),
                egui::FontId::proportional(13.0),
                latency_color(*ms),
            );
            response.on_hover_text(format!(
                "最近一次首字延迟 {} ms（流式：请求发出 → 第一个字）",
                ms
            ));
        }
        Some(Err(err)) => {
            ui.label(egui::RichText::new(short_err(err)).color(LATENCY_RED))
                .on_hover_text(err);
        }
        None if state.pending.contains(model_id) => matrix_label(ui, model_id),
        None => {}
    }
}

/// 模型行上的单模型延迟测试按钮（位于拖动按钮右侧，结果标签就在它右侧）。
///
/// 返回 `true` 表示用户点了测试；调用方负责在 UI 循环外真正发起探测
/// （节流/串行的权威判定也在那里再做一次）。
fn model_probe_button(
    ui: &mut egui::Ui,
    gate: &ProbeGate,
    provider_key: &str,
    now: f64,
    net_guard: Option<&str>,
) -> bool {
    let state = gate.state(provider_key, now, net_guard);
    let (label, hint) = match &state {
        ProbeGateState::Ready => ("测试".to_string(), None),
        ProbeGateState::Busy => (
            "测试中".to_string(),
            Some("该厂商上一个延迟测试尚未结束（同一厂商一次只测一个）".to_string()),
        ),
        ProbeGateState::Cooling(left) => (
            format!("测试({}s)", left.ceil() as u64),
            Some(format!(
                "节流中：{} 秒后可再测（同一厂商的任意两次探测（同模型 / 不同模型都算）至少间隔 5 秒，避免中转站测活风控；其他厂商不受影响）",
                left.ceil() as u64
            )),
        ),
        ProbeGateState::NetBlocked(reason) => (
            "测试".to_string(),
            Some(format!(
                "{}：{}。请关闭系统代理 / VPN 后重试",
                crate::netguard::BLOCK_PREFIX,
                reason
            )),
        ),
    };
    let enabled = matches!(state, ProbeGateState::Ready);
    let button = ui.add_enabled(enabled, egui::Button::new(label));
    match hint {
        Some(hint) if !enabled => button.on_disabled_hover_text(hint).clicked(),
        Some(hint) => button.on_hover_text(hint).clicked(),
        None => button.clicked(),
    }
}

/// 线上协议（api）的调用形状：端点、鉴权与最小请求体各不相同。
/// 未列出的值按 OpenAI Chat Completions 兼容层处理，与 [`crate::convert::npm_to_api`] 的口径一致。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ApiWire {
    /// `openai-completions` / `mistral-conversations` / 未知值。
    ChatCompletions,
    /// `openai-responses` / `openai-codex-responses`。
    Responses,
    /// `azure-openai-responses`（鉴权头为 `api-key`）。
    AzureResponses,
    /// `anthropic-messages`。
    AnthropicMessages,
    /// `google-generative-ai`（密钥走 `?key=` 查询参数）。
    GoogleGenerativeAi,
    /// `google-vertex`（Bearer + `publishers/google` 路径）。
    GoogleVertex,
    /// `pi-messages`。
    PiMessages,
    /// 需要专有签名或私有网关，无法用最小请求测延迟。
    Unsupported,
}

/// 请求鉴权方式。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AuthKind {
    /// `Authorization: Bearer <key>`。
    Bearer,
    /// `x-api-key` + `anthropic-version`。
    AnthropicKey,
    /// Azure OpenAI 的 `api-key`。
    AzureKey,
    /// 密钥已放进 URL 查询参数（Google 系），不再加鉴权头。
    QueryKey,
}

fn api_wire(api: &str) -> ApiWire {
    match api.trim() {
        "anthropic-messages" => ApiWire::AnthropicMessages,
        "openai-responses" | "openai-codex-responses" => ApiWire::Responses,
        "azure-openai-responses" => ApiWire::AzureResponses,
        "google-generative-ai" => ApiWire::GoogleGenerativeAi,
        "google-vertex" => ApiWire::GoogleVertex,
        "pi-messages" => ApiWire::PiMessages,
        // 这两个需要专有签名 / 私有网关（SigV4、CloudCode），最小请求测不出真实可用性。
        "bedrock-converse-stream" | "google-gemini-cli" => ApiWire::Unsupported,
        _ => ApiWire::ChatCompletions,
    }
}

fn auth_kind(wire: ApiWire) -> AuthKind {
    match wire {
        ApiWire::AnthropicMessages => AuthKind::AnthropicKey,
        ApiWire::AzureResponses => AuthKind::AzureKey,
        ApiWire::GoogleGenerativeAi | ApiWire::GoogleVertex => AuthKind::QueryKey,
        _ => AuthKind::Bearer,
    }
}

/// 该协议是否支持用最小请求测延迟 / 拉模型列表；不支持时给出原因。
fn unsupported_reason(api: &str) -> Option<String> {
    match api_wire(api) {
        ApiWire::Unsupported => Some(format!(
            "协议 {} 需要专有鉴权（签名 / 私有网关），暂不支持自动测试",
            api.trim()
        )),
        _ => None,
    }
}

/// 按鉴权方式给请求加鉴权头；`QueryKey` 的密钥已在 URL 里。
fn apply_auth(request: ureq::Request, auth: AuthKind, secret: &str) -> ureq::Request {
    match auth {
        AuthKind::Bearer => request.set("Authorization", &format!("Bearer {}", secret)),
        AuthKind::AnthropicKey => request
            .set("x-api-key", secret)
            .set("anthropic-version", "2023-06-01"),
        AuthKind::AzureKey => request.set("api-key", secret),
        AuthKind::QueryKey => request,
    }
}

/// Google 系把密钥放查询参数；其余协议原样返回。
fn with_query_key(url: &str, auth: AuthKind, secret: &str) -> String {
    if auth == AuthKind::QueryKey && !secret.is_empty() {
        // URL 里可能已经带了查询参数（Google 流式端点的 `?alt=sse`），要用 `&` 接着拼
        let separator = if url.contains('?') { '&' } else { '?' };
        format!("{}{}key={}", url, separator, secret)
    } else {
        url.to_string()
    }
}

/// 探测请求的 User-Agent：按协议伪装成主流客户端。
///
/// 中转站普遍只放行白名单客户端：实测同一站点同一 key，`ureq/2.12.1`、不传 UA、
/// `pi/0.1.0` 一律返回 `401 unauthorized client detected`（有的站点直接卡住到超时），
/// 而 `claude-cli/*` 与 `opencode/*` 正常返回 200。版本号不被校验（`claude-cli/9.9.9`
/// 同样放行），但写成真实存在的版本更自然（版本号取自本机安装的客户端）。
fn probe_user_agent(wire: ApiWire) -> &'static str {
    match wire {
        // Anthropic 系中转站本来就是给 Claude Code 用的
        ApiWire::AnthropicMessages | ApiWire::PiMessages => "claude-cli/1.18.30 (external, cli)",
        // OpenAI 兼容 / Responses / Google 系用 opencode 的身份
        _ => "opencode/1.18.30",
    }
}

/// 测量 provider 模型列表接口的往返延迟（毫秒）。
fn measure_provider_latency(url: &str, secret: &str, api: &str) -> Result<u64, String> {
    if let Some(reason) = unsupported_reason(api) {
        return Err(reason);
    }
    if url.is_empty() {
        return Err("缺少 baseURL".to_string());
    }
    if secret.is_empty() {
        return Err("缺少 API Key".to_string());
    }
    let wire = api_wire(api);
    let auth = auth_kind(wire);
    let target = with_query_key(url, auth, secret);
    let agent = latency_agent();
    let request = apply_auth(
        agent
            .get(&target)
            .set("User-Agent", probe_user_agent(wire))
            .set("Accept", "application/json"),
        auth,
        secret,
    );
    let started = std::time::Instant::now();
    let result = request.call();
    let elapsed = started.elapsed().as_millis() as u64;
    if elapsed >= LATENCY_TIMEOUT_MS {
        return Err(format!("超时（{} ms）", elapsed));
    }
    match result {
        Ok(_) => Ok(elapsed),
        Err(err) => Err(http_error(err, elapsed)),
    }
}

/// 最小对话请求的地址：按协议决定路径（Google 系需要模型名参与路径）。
/// Google 系的推理动作：非流式 `:generateContent`，流式 `:streamGenerateContent?alt=sse`。
///
/// Google 的流式靠**换端点**区分，而不是请求体里的 `stream` 字段。
fn google_action(stream: bool) -> &'static str {
    if stream {
        ":streamGenerateContent?alt=sse"
    } else {
        ":generateContent"
    }
}

fn chat_url(base_url: &str, api: &str, model: &str, stream: bool) -> String {
    let base = base_url.trim().trim_end_matches('/');
    let model = model.trim();
    match api_wire(api) {
        ApiWire::AnthropicMessages => {
            if base.ends_with("/v1") {
                format!("{}/messages", base)
            } else {
                format!("{}/v1/messages", base)
            }
        }
        ApiWire::Responses | ApiWire::AzureResponses => format!("{}/responses", base),
        ApiWire::GoogleGenerativeAi => {
            format!("{}/models/{}{}", base, model, google_action(stream))
        }
        ApiWire::GoogleVertex => format!(
            "{}/publishers/google/models/{}{}",
            base,
            model,
            google_action(stream)
        ),
        ApiWire::PiMessages => format!("{}/messages", base),
        ApiWire::ChatCompletions | ApiWire::Unsupported => format!("{}/chat/completions", base),
    }
}

/// 单次探测的请求体：内容是题库里的中性短问句。
///
/// - **不校验答案**：目的只是让请求看起来像正常对话（规避中转站测活特征），
///   判定只看 HTTP 是否成功与往返耗时。
/// - token 上限给到 16：太小会让推理模型返回空内容甚至直接报错。
/// - 不设 `temperature`：部分推理模型只接受默认值，设了反而报错。
fn minimal_body(wire: ApiWire, model: &str, question: &str) -> Value {
    match wire {
        ApiWire::Responses | ApiWire::AzureResponses => serde_json::json!({
            "model": model,
            "max_output_tokens": 16,
            "input": question,
            "stream": true
        }),
        ApiWire::GoogleGenerativeAi | ApiWire::GoogleVertex => serde_json::json!({
            "contents": [{ "role": "user", "parts": [{ "text": question }] }],
            "generationConfig": { "maxOutputTokens": 16 }
        }),
        ApiWire::AnthropicMessages | ApiWire::PiMessages => serde_json::json!({
            "model": model,
            "max_tokens": 16,
            "stream": true,
            "messages": [{ "role": "user", "content": question }]
        }),
        ApiWire::ChatCompletions | ApiWire::Unsupported => serde_json::json!({
            "model": model,
            "max_tokens": 16,
            "stream": true,
            "messages": [{ "role": "user", "content": question }]
        }),
    }
}

/// 判断一个 SSE 载荷是不是「第一个字」（即真的开始出内容了）。
///
/// 只看内容类字段，不看 role / usage / 各种元事件：中转站常常在模型真正开始生成前
/// 先推一个 role 块或心跳块，把它当首字，测出来的就不是用户体感的「首字延迟」。
fn chunk_has_content(wire: ApiWire, chunk: &Value) -> bool {
    match wire {
        ApiWire::ChatCompletions | ApiWire::Unsupported => {
            let delta = &chunk["choices"][0]["delta"];
            // 推理模型先出 reasoning_content（思维链）也算已经出字
            non_empty_text(&delta["content"]) || non_empty_text(&delta["reasoning_content"])
        }
        ApiWire::Responses | ApiWire::AzureResponses => {
            // 形如 {"type":"response.output_text.delta","delta":"你"}
            chunk["type"]
                .as_str()
                .is_some_and(|kind| kind.ends_with(".delta"))
                && non_empty_text(&chunk["delta"])
        }
        ApiWire::AnthropicMessages | ApiWire::PiMessages => {
            // 形如 {"type":"content_block_delta","delta":{"text":"你"}}
            let delta = &chunk["delta"];
            non_empty_text(&delta["text"]) || non_empty_text(&delta["thinking"])
        }
        ApiWire::GoogleGenerativeAi | ApiWire::GoogleVertex => chunk["candidates"][0]["content"]
            ["parts"]
            .as_array()
            .is_some_and(|parts| parts.iter().any(|part| non_empty_text(&part["text"]))),
    }
}

/// 字段是否含有非空文本（兼容「字符串 / 内容块数组 / 嵌套对象」三种形态）。
fn non_empty_text(value: &Value) -> bool {
    match value {
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => items.iter().any(non_empty_text),
        Value::Object(fields) => fields.values().any(non_empty_text),
        _ => false,
    }
}

/// 流结束标记：OpenAI 兼容的 `[DONE]`、Anthropic 的 `message_stop`、Responses 的 `response.completed`。
///
/// Anthropic 会先发一行 `event: message_stop` 再发 `data: {"type":"message_stop"}`，
/// 两种形态都要认（裸标记行没有引号，所以按子串匹配）。
/// 站点发完标记后未必立刻关连接，靠它提前收尾，避免一直读到读超时才结束。
fn is_stream_end(line: &str) -> bool {
    line.contains("[DONE]")
        || line.contains("message_stop")
        || line.contains("response.completed")
        || line.contains("response.incomplete")
        || line.contains("response.failed")
}

/// 读完流式响应，返回**首字延迟**（毫秒，从请求发出算起）。
///
/// - 边读边解析 SSE 的 `data:` 载荷，碰到第一个带内容的块立刻记下耗时；
/// - 记下之后**继续把流读完**（`[DONE]` / `message_stop` / EOF / 64 KB 上限）再关闭连接：
///   真实客户端不会拿到流就断，匆匆断开在中转站日志里反而像探测流量；
/// - 全程没有数据返回时返回 `None`（调用方按超时 / 协议不支持流式处理）；
/// - 有数据但认不出内容块（形态罕见）时退回「第一个 `data:` 包到达的时刻」。
fn read_stream_ttft(
    reader: impl std::io::Read,
    wire: ApiWire,
    started: std::time::Instant,
) -> Option<u64> {
    use std::io::BufRead;
    /// 读取上限：足够装下 16 token 的流式响应，又能顶住发完不关连接的站点。
    const MAX_STREAM_BYTES: usize = 64 * 1024;
    let mut buffer = std::io::BufReader::new(reader);
    let mut line = String::new();
    let mut read_bytes = 0usize;
    let mut first_data: Option<u64> = None;
    let mut ttft: Option<u64> = None;
    loop {
        line.clear();
        match buffer.read_line(&mut line) {
            Ok(0) => break,
            Ok(read) => read_bytes += read,
            // 读超时 / 连接中断：保留已经测到的首字
            Err(_) => break,
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if is_stream_end(trimmed) {
            break;
        }
        if let Some(payload) = trimmed.strip_prefix("data:") {
            if let Ok(chunk) = serde_json::from_str::<Value>(payload.trim()) {
                if first_data.is_none() {
                    first_data = Some(started.elapsed().as_millis() as u64);
                }
                if ttft.is_none() && chunk_has_content(wire, &chunk) {
                    ttft = Some(started.elapsed().as_millis() as u64);
                }
            }
        }
        if read_bytes >= MAX_STREAM_BYTES {
            break;
        }
    }
    ttft.or(first_data)
}

/// 对单个模型发一个**流式**探测请求，测量**首字延迟**（毫秒）。
///
/// 走流式是为了不像脚本测活：主流客户端（Claude Code / opencode / pi 等）默认全部流式，
/// 同步请求在中转站日志里会显示成「类型：同步」，反而是少数派特征。
/// 解响应体只是为了找第一个内容块（测首字），**不校验答案**。
/// 端点、鉴权与请求体都按所选协议构造；失败仍会报出耗时，便于判断服务是否可达。
fn measure_model_latency(
    base_url: &str,
    secret: &str,
    api: &str,
    model: &str,
    question: &str,
) -> Result<u64, String> {
    if let Some(reason) = unsupported_reason(api) {
        return Err(reason);
    }
    if base_url.trim().is_empty() {
        return Err("缺少 baseURL".to_string());
    }
    if secret.is_empty() {
        return Err("缺少 API Key".to_string());
    }
    let wire = api_wire(api);
    let auth = auth_kind(wire);
    let url = with_query_key(&chat_url(base_url, api, model, true), auth, secret);
    let body = minimal_body(wire, model, question).to_string();
    // 首字延迟：读取超时固定为 LATENCY_TIMEOUT_MS，超过即视为超时。
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(5))
        .timeout_read(std::time::Duration::from_millis(LATENCY_TIMEOUT_MS))
        .build();
    let started = std::time::Instant::now();
    // Accept 与主流 SDK 的流式口径一致；UA 伪装成白名单客户端（见 probe_user_agent）。
    let result = apply_auth(
        agent
            .post(&url)
            .set("User-Agent", probe_user_agent(wire))
            .set("Accept", "text/event-stream")
            .set("Content-Type", "application/json"),
        auth,
        secret,
    )
    .send_string(&body);
    let response = match result {
        Ok(response) => response,
        Err(err) => return Err(http_error(err, started.elapsed().as_millis() as u64)),
    };
    match read_stream_ttft(response.into_reader(), wire, started) {
        Some(ms) if ms < LATENCY_TIMEOUT_MS => Ok(ms),
        Some(ms) => Err(format!("超时（{} ms）", ms)),
        None => {
            let waited = started.elapsed().as_millis() as u64;
            Err(if waited >= LATENCY_TIMEOUT_MS {
                format!("超时（{} ms）", waited)
            } else {
                "流式响应没有数据".to_string()
            })
        }
    }
}

/// 调用 OpenAI 兼容 /models 接口获取模型 id 列表（后台线程内执行）。
fn fetch_models_remote(url: &str, secret: &str, api: &str) -> Result<Vec<String>, String> {
    if let Some(reason) = unsupported_reason(api) {
        return Err(reason);
    }
    if url.is_empty() {
        return Err("缺少 baseURL，无法获取模型".to_string());
    }
    if secret.is_empty() {
        return Err("缺少 API Key，无法获取模型".to_string());
    }
    let wire = api_wire(api);
    let auth = auth_kind(wire);
    let target = with_query_key(url, auth, secret);
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout_read(std::time::Duration::from_secs(30))
        .build();
    let request = apply_auth(
        agent
            .get(&target)
            .set("User-Agent", probe_user_agent(wire))
            .set("Accept", "application/json"),
        auth,
        secret,
    );
    let response = request.call().map_err(|err| match err {
        ureq::Error::Status(code, resp) => format!("HTTP {}：{}", code, resp.status_text()),
        ureq::Error::Transport(transport) => format!(
            "网络错误：{}",
            sanitize_network_error(&transport.to_string())
        ),
    })?;
    let text = response.into_string().map_err(|err| err.to_string())?;
    parse_models_response(&text)
}

/// 解析 /models 响应中的模型 id（兼容 OpenAI/Anthropic/Gemini 等格式）。
fn parse_models_response(text: &str) -> Result<Vec<String>, String> {
    let root: Value = serde_json::from_str(text).map_err(|err| {
        let snippet = text.chars().take(160).collect::<String>();
        format!("响应不是合法 JSON（{}）：{}", err, snippet)
    })?;
    if let Some(error) = root.get("error") {
        let msg = error
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| error.as_str())
            .unwrap_or("未知错误");
        return Err(msg.to_string());
    }
    fn push_ids(item: &Value, seen: &mut HashSet<String>, ids: &mut Vec<String>) {
        let raw = item
            .get("id")
            .and_then(Value::as_str)
            .or_else(|| item.get("name").and_then(Value::as_str))
            .unwrap_or("");
        let id = raw.trim().trim_start_matches("models/").to_string();
        if !id.is_empty() && seen.insert(id.clone()) {
            ids.push(id);
        }
    }
    let mut ids: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    if let Some(arr) = root.as_array() {
        for item in arr {
            push_ids(item, &mut seen, &mut ids);
        }
    }
    for key in ["data", "models"] {
        if let Some(arr) = root.get(key).and_then(Value::as_array) {
            for item in arr {
                push_ids(item, &mut seen, &mut ids);
            }
        }
    }
    Ok(ids)
}

/// 本页写入路径的解析结果。
#[derive(Clone)]
enum PageTarget {
    /// 当前文件（已加载）：整体替换 agent / provider。
    Current(String),
    /// 路径已修改但未重新加载：按“先读后合并”写入，不破坏目标文件已有配置。
    Modified(String),
    /// 该后端默认目标（Windows 本地路径；WSL 仅在勾选「WSL同步」后写入）。
    Default(String),
}

pub struct App {
    root: Value,
    agents: Vec<AgentRow>,
    providers: Vec<ProviderRow>,
    new_agent: AgentRow,
    new_provider: ProviderRow,
    config_path: String,
    /// 最近一次实际加载的路径（config_path 与之不等时按“未加载”处理，防止误覆盖）。
    loaded_path: String,
    status: String,
    show_new_agent: bool,
    show_new_provider: bool,
    agent_open: HashSet<String>,
    provider_open: HashSet<String>,
    variant_open: HashSet<String>,
    agent_drag_src: Option<String>,
    agent_drag_target: Option<String>,
    provider_drag_src: Option<String>,
    provider_drag_target: Option<String>,
    model_drag_src: Option<String>,
    model_drag_target: Option<String>,
    /// 每个 provider 的模型获取状态（key → 状态）。
    model_fetch: HashMap<String, ModelFetchState>,
    /// 已展开的模型获取面板（provider key）。
    model_fetch_open: HashSet<String>,
    /// 每个 provider 的延迟测试状态（key → 状态）。
    latency: HashMap<String, LatencyState>,
    /// 模型延迟探测的节流与串行状态（纯内存，重启清零）。
    probe: ProbeGate,
    /// 网络守卫结论：`Some(reason)` 表示检测到系统代理 / VPN，模型延迟测试被禁用。
    net_guard: Option<String>,
    /// 上次网络守卫检测时刻（egui 秒）。
    net_guard_at: f64,
    theme: Theme,
    save_format: SaveFormat,
    /// 滚轮切换保存格式的门门：一次连续滚动手势只切换一次。
    save_format_wheel_latch: bool,
    source_format: ConfigFormat,
    config_paths: ConfigPaths,
    targets: Vec<SaveTarget>,
    current_page: ConfigFormat,
    sync_wsl: bool,
    /// 全局 API Key 显隐：一键控制所有密钥输入框的明文/掩码显示。
    show_api_keys: bool,
    /// 右侧配置预览/编辑面板是否打开。
    show_preview: bool,
    /// 预览面板宽度占窗口宽度的比例（拖动分隔条调整；窗口缩放时按此比例适配）。
    preview_ratio: f32,
    /// 预览文本框是否持有焦点（编辑中以文本为准，失焦后以组件状态为准）。
    preview_focused: bool,
    /// 预览文本框缓冲（待保存文档；失焦时由组件状态实时重写）。
    preview_draft: String,
    /// 最近一次文本编辑的时间点（ctx 时间），用于防抖自动保存。
    preview_dirty_at: Option<f64>,
    /// 用户最近一次在预览框内输入的帧时间（用于判断「正在手改预览」）。
    preview_edit_at: Option<f64>,
    /// 最近一次文本解析是否成功（解析失败不写盘、不覆盖文本）。
    preview_parse_ok: bool,
    /// 最近一次预览文本解析失败的报错（成功时为 None），用于面板内红字提示。
    preview_parse_error: Option<String>,
    /// 预览 Ctrl+F 查找：查询词、是否打开、当前命中下标。
    preview_find: String,
    preview_find_active: bool,
    preview_find_index: usize,
    /// 下一帧需要给查找框抢焦点。
    preview_find_focus: bool,
    /// 待跳转的命中字节偏移（Enter/按钮跳转后用光标滚动到该处）。
    preview_find_jump: Option<usize>,
    /// 光标所在行（1-based；失焦时保留最后位置）。
    preview_cursor_line: usize,
    load_error: Option<String>,
    pi_extras: Value,
    /// 各后端官方图标纹理（与 BACKENDS 顺序对齐，首帧惰性加载）。
    backend_icons: Vec<Option<egui::TextureHandle>>,
}

impl Default for App {
    fn default() -> Self {
        let paths = ConfigPaths::default();
        let (format, path) =
            ConfigPaths::detect().unwrap_or((ConfigFormat::Opencode, String::new()));
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
            agent_open: HashSet::new(),
            provider_open: HashSet::new(),
            variant_open: HashSet::new(),
            agent_drag_src: None,
            agent_drag_target: None,
            provider_drag_src: None,
            provider_drag_target: None,
            model_drag_src: None,
            model_drag_target: None,
            model_fetch: HashMap::new(),
            model_fetch_open: HashSet::new(),
            latency: HashMap::new(),
            probe: ProbeGate::default(),
            net_guard: crate::netguard::detect(),
            net_guard_at: 0.0,
            theme: Theme::default(),
            save_format: SaveFormat::default(),
            save_format_wheel_latch: false,
            source_format: format,
            config_paths: paths,
            targets: Vec::new(),
            current_page: format,
            sync_wsl: false,
            show_api_keys: false,
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
            preview_cursor_line: 1,
            load_error: None,
            pi_extras: Value::Object(Map::new()),
            backend_icons: Vec::new(),
        };
        app.apply_load();
        app
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
        // 每 5 秒复查一次系统代理 / VPN：用户可能在运行期间开关 Clash 等，
        // 守卫结论用于禁用模型延迟测试（见 netguard 模块说明）。
        let frame_time = ctx.input(|i| i.time);
        if frame_time - self.net_guard_at >= 5.0 {
            self.net_guard = crate::netguard::detect();
            self.net_guard_at = frame_time;
        }
        // 首帧惰性加载各后端官方图标
        if self.backend_icons.is_empty() {
            self.backend_icons = backends::BACKENDS
                .iter()
                .map(|b| {
                    b.icon_rgba().map(|(rgba, w, h)| {
                        let image = egui::ColorImage::from_rgba_unmultiplied(
                            [w as usize, h as usize],
                            rgba,
                        );
                        ctx.load_texture(
                            format!("backend_icon_{}", b.id().label()),
                            image,
                            egui::TextureOptions::LINEAR,
                        )
                    })
                })
                .collect();
        }
        self.poll_model_fetch();
        self.poll_latency();
        self.ui_top_bar(ctx);
        self.ui_status_bar(ctx);
        // 右侧配置预览/编辑面板：宽度由 preview_ratio 控制（拖动左边缘分隔条调整），
        // 窗口缩放时按该比例适配；窄窗口下限 220px，并保证组件区至少 320px。
        if self.show_preview {
            let screen_w = ctx.content_rect().width().max(1.0);
            let max_w = (screen_w - 320.0).max(220.0);
            let preview_w = (screen_w * self.preview_ratio).clamp(220.0, max_w);
            let side = egui::SidePanel::right("preview_panel")
                .exact_width(preview_w)
                .show(ctx, |ui| {
                    self.ui_preview_panel(ui);
                });
            // 分隔条用 Foreground 层的独立热区：同层注册会被占满面板的文本框抢走拖拽。
            self.ui_preview_resizer(ctx, side.response.rect, screen_w);
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            self.ui_page_header(ui);
            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .scroll_source(egui::scroll_area::ScrollSource {
                    drag: false,
                    ..egui::scroll_area::ScrollSource::ALL
                })
                .show(ui, |ui| {
                    ui.add_space(4.0);
                    // Agents 仅属于 opencode 页面；区块标题吸顶，滚动时始终显示在顶部。
                    if self.current_page == ConfigFormat::Opencode {
                        self.ui_agents_section(ui);
                        ui.add_space(8.0);
                    }
                    self.ui_providers_section(ui);
                    ui.add_space(8.0);
                });
        });
        self.paint_drag_ghost(ctx);
        // 仅拖拽中显示抓取光标（避免任意控件按下时全局变光标）
        let dragging = self.agent_drag_src.is_some()
            || self.provider_drag_src.is_some()
            || self.model_drag_src.is_some();
        #[cfg(target_os = "windows")]
        crate::cursor::set_custom_cursor_active(dragging);
    }
}

/// provider / model 表单的字段可见性与方言标签（opencode / pi / omp / DSH 共用）。
#[derive(Clone, Copy)]
struct ProviderFormFlags {
    show_oc: bool,
    show_omp: bool,
    show_dsh: bool,
    show_provider_base_url: bool,
    show_provider_timeout: bool,
    show_model_name: bool,
    show_model_context: bool,
    show_model_output: bool,
    show_model_input: bool,
    show_model_variants: bool,
    show_model_reasoning: bool,
    show_model_tool_call: bool,
    show_model_store: bool,
    base_label: &'static str,
    api_key_label: &'static str,
    context_label: &'static str,
    output_label: &'static str,
    input_label: &'static str,
}

impl ProviderFormFlags {
    fn new(app: &App) -> Self {
        let show_oc = app.current_page == ConfigFormat::Opencode;
        let show_dsh = app.current_page == ConfigFormat::DeepSeekHarness;
        Self {
            show_oc,
            show_omp: app.current_page == ConfigFormat::OhMyPi,
            show_dsh,
            show_provider_base_url: app.page_has_provider_field("base_url"),
            // opencode 的 options.timeout 始终显示（文件未写该字段时默认 180000ms）
            show_provider_timeout: show_oc || app.page_has_provider_field("timeout"),
            show_model_name: app.page_has_model_field("name"),
            show_model_context: app.page_has_model_field("context"),
            show_model_output: app.page_has_model_field("output"),
            show_model_input: app.page_has_model_field("input"),
            show_model_variants: app.page_has_model_field("variants"),
            show_model_reasoning: app.page_has_model_field("reasoning"),
            show_model_tool_call: app.page_has_model_field("tool_call"),
            show_model_store: app.page_has_model_field("store"),
            base_label: if show_oc {
                "options.baseURL"
            } else if show_dsh {
                "baseURL"
            } else {
                "baseUrl"
            },
            api_key_label: if show_oc {
                "options.apiKey"
            } else if show_dsh {
                "apiKeyEnv"
            } else {
                "apiKey"
            },
            context_label: if show_oc {
                "limit.context"
            } else {
                "contextWindow"
            },
            output_label: if show_oc { "limit.output" } else { "maxTokens" },
            input_label: if show_oc { "modalities.input" } else { "input" },
        }
    }
}

/// npm 包下拉（opencode 专用）；`id_salt` 区分同一页面内的多个表单实例。
fn provider_npm_combo(
    ui: &mut egui::Ui,
    p: &mut ProviderRow,
    id_salt: &str,
    empty_has_label: bool,
) {
    const NPM_OPTIONS: [&str; 5] = [
        "",
        "@ai-sdk/openai",
        "@ai-sdk/anthropic",
        "@ai-sdk/google",
        "@ai-sdk/openai-compatible",
    ];
    field_label(ui, 120.0, "npm");
    let current = p.npm.clone();
    let mut selected = NPM_OPTIONS.iter().position(|n| *n == current.as_str());
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(if current.is_empty() {
            "选择 npm 包..."
        } else {
            &current
        })
        .width(220.0)
        .show_ui(ui, |ui| {
            for (i, npm) in NPM_OPTIONS.iter().enumerate() {
                let label = if npm.is_empty() && empty_has_label {
                    "(空)"
                } else {
                    *npm
                };
                if ui.selectable_label(selected == Some(i), label).clicked() {
                    selected = Some(i);
                }
            }
        });
    if let Some(i) = selected {
        p.npm = NPM_OPTIONS[i].to_string();
    }
}

/// api / npm 下拉里的「(空)」标签：表示未指定协议。
const EMPTY_API_LABEL: &str = "(空)";

/// api 下拉：omp 官方 9 值 / pi KnownApi 10 值；首项「(空)」与 opencode 页 npm 的空选项同义。
fn provider_api_combo(ui: &mut egui::Ui, p: &mut ProviderRow, show_omp: bool, id_salt: &str) {
    const OMP_APIS: [&str; 9] = [
        "openai-completions",
        "openai-responses",
        "openai-codex-responses",
        "azure-openai-responses",
        "anthropic-messages",
        "bedrock-converse-stream",
        "google-generative-ai",
        "google-gemini-cli",
        "google-vertex",
    ];
    const PI_APIS: [&str; 10] = [
        "openai-completions",
        "mistral-conversations",
        "openai-responses",
        "azure-openai-responses",
        "openai-codex-responses",
        "anthropic-messages",
        "bedrock-converse-stream",
        "google-generative-ai",
        "google-vertex",
        "pi-messages",
    ];
    let options: &[&str] = if show_omp { &OMP_APIS } else { &PI_APIS };
    // 「(空)」= 未指定协议。四页共用同一份数据，故以 npm / pi_api / raw.api
    // 是否都为空判定，显示值统一走 effective_api()，与写盘、延迟测试同口径。
    let explicit = p.has_explicit_api();
    let current = p.effective_api();
    field_label(ui, 120.0, "api");
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(if explicit {
            current.as_str()
        } else {
            EMPTY_API_LABEL
        })
        .width(180.0)
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(!explicit, EMPTY_API_LABEL)
                .on_hover_text("不指定协议：写盘时按兼容层 openai-completions 处理")
                .clicked()
            {
                p.clear_api();
            }
            for &api in options {
                if ui
                    .selectable_label(explicit && current == api, api)
                    .clicked()
                {
                    p.pi_api = api.to_string();
                    p.npm = convert::api_to_npm(api);
                }
            }
        });
}

/// 思考档位多选：按钮展开、勾选写回逗号分隔文本。
/// `normalize` 为 true 时按规范档位顺序写回，避免重新勾选后被追加到末尾。
fn variant_selector(
    ui: &mut egui::Ui,
    variants: &mut String,
    names: &[&'static str],
    open_key: String,
    open_set: &mut HashSet<String>,
    normalize: bool,
) {
    let current = variants.clone();
    let mut selected: Vec<String> = current
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let display = if selected.is_empty() {
        "选择...".to_string()
    } else {
        current.clone()
    };
    let is_open = open_set.contains(&open_key);
    if ui.button(display).clicked() {
        if is_open {
            open_set.remove(&open_key);
        } else {
            open_set.insert(open_key.clone());
        }
    }
    if !is_open {
        return;
    }
    for name in names {
        let mut checked = selected.iter().any(|s| s == name);
        if ui.checkbox(&mut checked, *name).changed() {
            if checked {
                if !selected.iter().any(|s| s == name) {
                    selected.push((*name).to_string());
                }
            } else {
                selected.retain(|s| s != name);
            }
            *variants = if normalize {
                crate::model::ordered_variants(selected.iter().map(String::as_str)).join(", ")
            } else {
                selected.join(", ")
            };
        }
    }
}

/// 「获取模型」面板的分栏参数：列间水平间距、单列最小宽度、最大列数。
const FETCH_GRID_GAP_X: f32 = 28.0;
const FETCH_GRID_MIN_COL_W: f32 = 150.0;
const FETCH_GRID_MAX_COLS: usize = 5;

/// 计算模型勾选面板的横向分栏：列数由**可用宽度**推出来（而非写死 5 列），列宽再把总宽
/// 均分，所以 `列数 * 列宽 + 间距 * (列数 - 1)` 恒等于可用宽度，面板不会横向溢出。
/// 返回 `(列数, 每列宽度)`。
fn fetch_grid_columns(id_count: usize, avail_width: f32) -> (usize, f32) {
    if id_count == 0 {
        return (1, avail_width.max(1.0));
    }
    let max_cols = FETCH_GRID_MAX_COLS.min(id_count);
    // 浮点转整数在 Rust 中是饱和转换（NaN -> 0），因此不会 panic；clamp 也只是兜底。
    let cols = (((avail_width + FETCH_GRID_GAP_X) / (FETCH_GRID_MIN_COL_W + FETCH_GRID_GAP_X))
        .floor() as usize)
        .clamp(1, max_cols);
    let col_w = ((avail_width - FETCH_GRID_GAP_X * (cols - 1) as f32) / cols as f32).max(1.0);
    (cols, col_w)
}

/// 模型获取结果的勾选面板：勾选后把其中未配置的模型追加到 `models`。
fn model_fetch_popup<H: std::hash::Hash>(
    ui: &mut egui::Ui,
    state: Option<&ModelFetchState>,
    models: &mut Vec<ModelRow>,
    current_page: ConfigFormat,
    id_salt: H,
) {
    let Some(state) = state else {
        ui.label(egui::RichText::new("尚未获取，请先点击「获取模型」").weak());
        return;
    };
    if state.rx.is_some() {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(16.0));
            ui.label(egui::RichText::new("正在获取模型…").weak());
        });
        return;
    }
    let Some(result) = &state.result else {
        return;
    };
    match result {
        Ok(models_remote) if models_remote.is_empty() => {
            ui.label(egui::RichText::new("接口未返回任何模型").weak());
        }
        Ok(models_remote) => {
            let ids = models_remote.clone();
            ui.label(egui::RichText::new("勾选可新增未配置的模型：").weak());
            // 区域高度固定为 22 行，每列超出部分在区域内垂直滚动查看。
            // 注意 1：ScrollArea 内不能用 ui.columns —— columns 会把内容裁到当前可用高度，
            // 导致内容不进入滚动区、无法滚动。改为横向排布 + 纵向子列。
            // 注意 2：列数必须在滚动区**内部**按可用宽度计算，才能把「始终可见的滚动条」
            // 占用的那一条宽度也扣掉，否则整块内容会向右溢出界面。
            let row_h = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
            egui::ScrollArea::vertical()
                .id_salt(id_salt)
                .max_height(row_h * 22.5)
                .auto_shrink([false, true])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| {
                    let (cols, col_w) = fetch_grid_columns(ids.len(), ui.available_width());
                    let per_col = ids.len().div_ceil(cols);
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = FETCH_GRID_GAP_X;
                        for ci in 0..cols {
                            ui.vertical(|ui| {
                                // 定宽列 + 截断：超长模型名悬停看全名，而不是把列撑宽。
                                ui.set_width(col_w);
                                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                                for id in ids.iter().skip(ci * per_col).take(per_col) {
                                    let mut checked = models.iter().any(|m| m.id.trim() == id);
                                    if ui
                                        .checkbox(&mut checked, id)
                                        .on_hover_text(id.as_str())
                                        .changed()
                                        && checked
                                    {
                                        let mut row = ModelRow::new();
                                        row.id = id.clone();
                                        row.name = id.clone();
                                        row.source_format = Some(current_page);
                                        models.push(row);
                                    }
                                }
                            });
                        }
                    });
                });
        }
        Err(err) => {
            ui.label(
                egui::RichText::new(format!("获取失败：{}", err))
                    .color(egui::Color32::from_rgb(220, 90, 90)),
            );
        }
    }
}

impl App {
    /// 解析各保存目标的可用性与实际路径（避免在渲染循环中频繁拉起 wsl 进程）。
    fn refresh_targets(&mut self) {
        // 默认目标固定为 Windows 本地路径；WSL 侧仅通过“WSL同步”勾选写入，
        // 且写入前按页面检测对应 agent 是否已安装。
        self.targets = backends::BACKENDS
            .iter()
            .map(|b| {
                let id = b.id();
                let local = self.config_paths.local_path(id);
                SaveTarget {
                    backend: id,
                    available: self.config_paths.validate_target(id),
                    path: local,
                }
            })
            .collect();
    }

    /// 按 source_format 加载当前 config_path；失败时置空数据并记录 load_error。
    fn apply_load(&mut self) {
        let path = self.config_path.clone();
        self.loaded_path = path.clone();
        let result = backends::load_backend(self.source_format, &path);
        match result {
            Ok(load) => {
                self.root = load.root;
                self.agents = load.agents;
                self.providers = load.providers;
                self.pi_extras = load.extras;
                self.load_error = None;
                self.status = format!(
                    "已加载 ({}): {} agents, {} providers",
                    self.source_format.label(),
                    self.agents.len(),
                    self.providers.len()
                );
            }
            Err(e) => {
                self.root = Value::Object(Map::new());
                self.agents = Vec::new();
                self.providers = Vec::new();
                self.pi_extras = Value::Object(Map::new());
                self.load_error = Some(e.clone());
                self.status = format!("加载失败: {}", e);
            }
        }
        self.agent_open = self.agents.iter().map(|a| a.key.clone()).collect();
        self.provider_open = self.providers.iter().map(|p| p.key.clone()).collect();
        // 重新加载后丢弃旧的模型获取状态
        self.model_fetch.clear();
        self.model_fetch_open.clear();
        self.latency.clear();
        // 被丢弃的探测不会再回传结果：释放全局串行位，否则门控会一直卡在 Busy。
        self.probe.release(None);
        // 加载后跳转到来源格式对应的页面
        self.current_page = self.source_format;
        self.reset_preview_draft();
        self.refresh_targets();
    }

    fn ui_top_bar(&mut self, ctx: &egui::Context) {
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
                    ui.label("主题:");
                    let theme_btn = ui.button(self.theme.label());
                    egui::Popup::menu(&theme_btn)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClick)
                        .show(|ui| {
                            ui.set_min_width(80.0);
                            for t in Theme::ALL {
                                if ui.selectable_label(self.theme == t, t.label()).clicked() {
                                    self.theme = t;
                                    t.apply(ctx);
                                }
                            }
                        },
                    );
                });
            });
            // 第二行：配置文件 / 保存格式
            ui.horizontal(|ui| {
                ui.label("配置文件:");
                let path_resp =
                    ui.add(egui::TextEdit::singleline(&mut self.config_path).desired_width(420.0));
                // 回车确认：按当前输入路径重新加载（egui 单行编辑回车即失焦）
                if path_resp.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && self.config_path != self.loaded_path
                {
                    self.reload();
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
                    ui.label(egui::RichText::new("未加载").small().color(egui::Color32::from_rgb(220, 160, 60)))
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

    fn ui_status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("bottom")
            .exact_height(32.0)
            .show(ctx, |ui| {
                // 底部文字：靠下（不垂直居中）且左对齐，右侧统计仍靠右。
                ui.with_layout(egui::Layout::left_to_right(egui::Align::BOTTOM), |ui| {
                    if let Some(err) = &self.load_error {
                        ui.label(
                            egui::RichText::new(format!("⚠ 加载失败: {}", err))
                                .color(egui::Color32::from_rgb(220, 90, 90)),
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

    /// Agents 区块：标题行吸顶（滚动时始终显示在顶部），内容紧跟其下。
    fn ui_agents_section(&mut self, ui: &mut egui::Ui) {
        let anchor = sticky_begin(ui, 30.0);
        let matched: Vec<usize> = (0..self.agents.len()).collect();

        if self.agents.is_empty() && !self.show_new_agent {
            self.show_new_agent = true;
        }

        let mut to_remove: Option<usize> = None;
        let mut to_copy: Option<usize> = None;
        let mut hover_target: Option<String> = None;
        card_list(ui, &matched, 0.0, |ui, idx| {
            self.render_agent_card(ui, idx, &mut to_remove, &mut to_copy, &mut hover_target);
        });
        if let Some(idx) = to_remove {
            self.agents.remove(idx);
            self.status = "已删除 agent".into();
        }
        if let Some(idx) = to_copy {
            let mut a = self.agents[idx].clone();
            a.key = format!("{}_copy", a.key);
            self.agents.push(a);
            self.status = "已复制 agent".into();
        }
        if self.agent_drag_src.is_some() {
            self.agent_drag_target = hover_target;
        } else {
            self.agent_drag_target = None;
        }

        ui.add_space(6.0);
        if ui.button("新增 Agent").clicked() {
            self.show_new_agent = !self.show_new_agent;
        }
        if self.show_new_agent {
            self.ui_new_agent_form(ui);
        }
        sticky_end(ui, anchor, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Agents");
                // agents 只属于 opencode 页：来源不是 opencode 时界面没有 agents 数据，
                // 跨格式保存不会接管目标文件的 agent 容器（避免静默清空）。
                if self.source_format != ConfigFormat::Opencode {
                    let src = self.source_format.label();
                    ui.label(
                        egui::RichText::new(format!(
                            "（来源 {}：目标文件已有 agents 会保留，不被清空）",
                            src
                        ))
                        .small()
                        .weak(),
                    );
                }
                if !self.agents.is_empty() {
                    let all_open = self.agents.iter().all(|a| self.agent_open.contains(&a.key));
                    if ui
                        .button(if all_open {
                            "收起全部卡片"
                        } else {
                            "展开全部卡片"
                        })
                        .clicked()
                    {
                        if all_open {
                            self.agent_open.clear();
                        } else {
                            self.agent_open = self.agents.iter().map(|a| a.key.clone()).collect();
                        }
                    }
                }
            });
        });
    }

    fn render_agent_card(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        to_remove: &mut Option<usize>,
        to_copy: &mut Option<usize>,
        hover_target: &mut Option<String>,
    ) {
        let key = self.agents[idx].key.clone();
        let open = self.agent_open.contains(&key);
        let highlight = if self.agent_drag_target.as_deref() == Some(key.as_str()) {
            2
        } else if self.agent_drag_src.as_deref() == Some(key.as_str()) {
            1
        } else {
            0
        };
        let resp = card_frame(ui, open, highlight, |ui| {
            ui.horizontal(|ui| {
                let h = ui.add(DragHandle);
                if h.drag_started() {
                    self.agent_drag_src = Some(key.clone());
                    self.agent_drag_target = None;
                }
                if h.drag_stopped() {
                    if self.agent_drag_src == Some(key.clone()) {
                        if let Some(dst) = self.agent_drag_target.clone() {
                            let s = self.agents.iter().position(|a| a.key == key);
                            let d = self.agents.iter().position(|a| a.key == dst);
                            if let (Some(s), Some(d)) = (s, d) {
                                move_item(&mut self.agents, s, d);
                            }
                        }
                    }
                    self.agent_drag_src = None;
                    self.agent_drag_target = None;
                }
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(if open { "▼" } else { "▶" }).size(14.0),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    if open {
                        self.agent_open.remove(&key);
                    } else {
                        self.agent_open.insert(key.clone());
                    }
                }
                ui.strong(&self.agents[idx].key);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("删除").clicked() {
                        *to_remove = Some(idx);
                    }
                    if ui.button("复制").clicked() {
                        *to_copy = Some(idx);
                    }
                });
            });
            if open {
                self.render_agent_form(ui, idx);
            }
        });
        if let Some(src_key) = &self.agent_drag_src {
            if src_key != &key && resp.contains_pointer() && hover_target.is_none() {
                *hover_target = Some(key.clone());
            }
        }
    }

    fn render_agent_form(&mut self, ui: &mut egui::Ui, idx: usize) {
        let prev_key = self.agents[idx].key.clone();
        let other_keys: HashSet<String> = self
            .agents
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != idx)
            .map(|(_, a)| a.key.trim().to_string())
            .collect();
        let a = &mut self.agents[idx];
        ui.horizontal_wrapped(|ui| {
            field_label(ui, 120.0, "key");
            let key_resp = ui.add(egui::TextEdit::singleline(&mut a.key).desired_width(120.0));
            if !a.key.trim().is_empty() && other_keys.contains(a.key.trim()) {
                key_resp.on_hover_text("key 与其他 agent 重复，保存将被阻止");
                ui.label(
                    egui::RichText::new("⚠ 重复")
                        .small()
                        .color(egui::Color32::from_rgb(220, 90, 90)),
                );
            }
            field_label(ui, 120.0, "mode");
            ui.add(egui::TextEdit::singleline(&mut a.mode).desired_width(120.0));
            field_label(ui, 120.0, "description");
            ui.add(egui::TextEdit::singleline(&mut a.description).desired_width(450.0));
        });
        ui.horizontal_wrapped(|ui| {
            field_label(ui, 120.0, "model");
            let mut model_options: Vec<String> = self
                .providers
                .iter()
                .flat_map(|p| p.models.iter().map(|m| format!("{}/{}", p.key, m.id)))
                .collect();
            model_options.extend_from_slice(&[
                "opencode/mimo-v2.5-free".into(),
                "opencode/big-pickle".into(),
            ]);
            model_options.sort();
            model_options.dedup();
            let current = a.model.clone();
            let mut selected_idx = model_options.iter().position(|m| m == &current);
            egui::ComboBox::from_id_salt(format!("agent_model_{}", a.key))
                .selected_text(if current.is_empty() {
                    "选择模型..."
                } else {
                    &current
                })
                .width(180.0)
                .show_ui(ui, |ui| {
                    for (i, model) in model_options.iter().enumerate() {
                        let is_selected = selected_idx == Some(i);
                        if ui.selectable_label(is_selected, model.as_str()).clicked() {
                            selected_idx = Some(i);
                        }
                    }
                });
            if let Some(idx) = selected_idx {
                a.model = model_options[idx].clone();
            }
            field_label(ui, 120.0, "variant");
            let variant_options = ["", "low", "medium", "high", "xhigh", "max", "ultra"];
            let current_variant = a.variant.clone();
            let mut selected_variant = variant_options
                .iter()
                .position(|v| *v == current_variant.as_str());
            egui::ComboBox::from_id_salt(format!("agent_variant_{}", a.key))
                .selected_text(if current_variant.is_empty() {
                    "选择..."
                } else {
                    &current_variant
                })
                .width(100.0)
                .show_ui(ui, |ui| {
                    for (i, v) in variant_options.iter().enumerate() {
                        let label = if v.is_empty() { "(空)" } else { v };
                        let is_selected = selected_variant == Some(i);
                        if ui.selectable_label(is_selected, label).clicked() {
                            selected_variant = Some(i);
                        }
                    }
                });
            if let Some(idx) = selected_variant {
                a.variant = variant_options[idx].to_string();
            }
        });
        ui.horizontal_wrapped(|ui| {
            field_label(ui, 120.0, "temperature");
            numeric_text_edit(ui, &mut a.temperature, 120.0, "");
            field_label(ui, 120.0, "color");
            ui.add(egui::TextEdit::singleline(&mut a.color).desired_width(120.0));
            field_label(ui, 120.0, "system");
            ui.add(egui::TextEdit::singleline(&mut a.system).desired_width(450.0));
        });
        // key 重命名后同步展开状态（避免改名导致卡片收起）
        let new_key = self.agents[idx].key.clone();
        if new_key != prev_key {
            self.sync_agent_rename(&prev_key, &new_key);
        }
    }

    fn ui_new_agent_form(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.horizontal_wrapped(|ui| {
                field_label(ui, 120.0, "key");
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_agent.key)
                        .hint_text("coding-assistant")
                        .desired_width(120.0),
                );
                field_label(ui, 120.0, "mode");
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_agent.mode)
                        .hint_text("subagent")
                        .desired_width(120.0),
                );
                field_label(ui, 120.0, "description");
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_agent.description)
                        .hint_text("简要描述此 agent 的用途")
                        .desired_width(450.0),
                );
            });
            ui.horizontal_wrapped(|ui| {
                field_label(ui, 120.0, "model");
                let mut model_options: Vec<String> = self
                    .providers
                    .iter()
                    .flat_map(|p| p.models.iter().map(|m| format!("{}/{}", p.key, m.id)))
                    .collect();
                model_options.extend_from_slice(&[
                    "opencode/mimo-v2.5-free".into(),
                    "opencode/big-pickle".into(),
                ]);
                model_options.sort();
                model_options.dedup();
                let current = self.new_agent.model.clone();
                let mut selected_idx = model_options.iter().position(|m| m == &current);
                let _response = egui::ComboBox::from_id_salt("new_agent_model")
                    .selected_text(if current.is_empty() {
                        "选择模型..."
                    } else {
                        &current
                    })
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        for (i, model) in model_options.iter().enumerate() {
                            let is_selected = selected_idx == Some(i);
                            if ui.selectable_label(is_selected, model.as_str()).clicked() {
                                selected_idx = Some(i);
                            }
                        }
                    });
                if let Some(idx) = selected_idx {
                    self.new_agent.model = model_options[idx].clone();
                }
                field_label(ui, 120.0, "variant");
                let variant_options = ["", "low", "medium", "high", "xhigh", "max", "ultra"];
                let current_variant = self.new_agent.variant.clone();
                let mut selected_variant = variant_options
                    .iter()
                    .position(|v| *v == current_variant.as_str());
                egui::ComboBox::from_id_salt("new_agent_variant")
                    .selected_text(if current_variant.is_empty() {
                        "选择..."
                    } else {
                        &current_variant
                    })
                    .width(100.0)
                    .show_ui(ui, |ui| {
                        for (i, v) in variant_options.iter().enumerate() {
                            let label = if v.is_empty() { "(空)" } else { v };
                            let is_selected = selected_variant == Some(i);
                            if ui.selectable_label(is_selected, label).clicked() {
                                selected_variant = Some(i);
                            }
                        }
                    });
                if let Some(idx) = selected_variant {
                    self.new_agent.variant = variant_options[idx].to_string();
                }
            });
            ui.horizontal_wrapped(|ui| {
                field_label(ui, 120.0, "temperature");
                numeric_text_edit(ui, &mut self.new_agent.temperature, 120.0, "0.7");
                field_label(ui, 120.0, "color");
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_agent.color)
                        .hint_text("#00ccff")
                        .desired_width(120.0),
                );
                field_label(ui, 120.0, "system");
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_agent.system)
                        .hint_text("系统提示词")
                        .desired_width(450.0),
                );
            });
            ui.horizontal(|ui| {
                ui.add_space(60.0);
                if ui.button("确认").clicked() {
                    let key = self.new_agent.key.trim().to_string();
                    if key.is_empty() {
                        self.status = "请填写 agent key".into();
                    } else if self.agents.iter().any(|a| a.key.trim() == key) {
                        self.status = format!("agent key \"{}\" 已存在", key);
                    } else {
                        let na = self.new_agent.clone();
                        self.agents.push(na);
                        self.new_agent = AgentRow::new();
                        self.show_new_agent = false;
                        self.status = "已添加 agent".into();
                    }
                }
                if ui.button("取消").clicked() {
                    self.new_agent = AgentRow::new();
                    self.show_new_agent = false;
                }
            });
        });
    }

    /// 当前 agent 文件是否使用某个 provider 级字段。
    /// 判断范围是整个文件，不是单个 provider；切换到尚未加载的目标页时
    /// 使用完整 schema，保证新增 provider 可以输入所有专属字段。
    fn page_has_provider_field(&self, field: &str) -> bool {
        if self.source_format != self.current_page {
            return true;
        }
        self.providers
            .iter()
            .any(|provider| match self.current_page {
                ConfigFormat::Opencode => match field {
                    "base_url" => provider
                        .raw
                        .get("options")
                        .and_then(|v| v.get("baseURL"))
                        .is_some(),
                    "timeout" => provider
                        .raw
                        .get("options")
                        .and_then(|v| v.get("timeout"))
                        .is_some(),
                    _ => false,
                },
                ConfigFormat::Pi | ConfigFormat::OhMyPi => match field {
                    "base_url" => provider.raw.get("baseUrl").is_some(),
                    _ => false,
                },
                ConfigFormat::DeepSeekHarness => match field {
                    "base_url" => provider.raw.get("baseURL").is_some(),
                    _ => false,
                },
            })
    }

    /// 当前 agent 文件是否使用某个 model 级字段。字段存在性按整个
    /// agent 文件判断，避免单个模型缺字段时导致同一页面布局跳变。
    fn page_has_model_field(&self, field: &str) -> bool {
        if self.source_format != self.current_page {
            return true;
        }
        self.providers.iter().any(|provider| {
            provider.models.iter().any(|model| match self.current_page {
                ConfigFormat::Opencode => match field {
                    "name" => model.raw.get("name").is_some(),
                    "reasoning" => model.raw.get("reasoning").is_some(),
                    "tool_call" => model.raw.get("tool_call").is_some(),
                    "store" => model
                        .raw
                        .get("options")
                        .and_then(|v| v.get("store"))
                        .is_some(),
                    "context" => model
                        .raw
                        .get("limit")
                        .and_then(|v| v.get("context"))
                        .is_some(),
                    "output" => model
                        .raw
                        .get("limit")
                        .and_then(|v| v.get("output"))
                        .is_some(),
                    "input" => model
                        .raw
                        .get("modalities")
                        .and_then(|v| v.get("input"))
                        .is_some(),
                    "variants" => model.raw.get("variants").is_some(),
                    _ => false,
                },
                ConfigFormat::Pi | ConfigFormat::OhMyPi => match field {
                    "name" => model.raw.get("name").is_some(),
                    // reasoning 也可由 pi/omp 的 thinking 块 / thinkingLevelMap 表达，
                    // 只写了这些键时同样应显示（并勾选）reasoning。
                    "reasoning" => {
                        model.raw.get("reasoning").is_some()
                            || model.raw.get("thinkingLevelMap").is_some()
                            || model.raw.get("thinking").is_some()
                            || model.raw.get("reasoningEfforts").is_some()
                    }
                    "context" => model.raw.get("contextWindow").is_some(),
                    "output" => model.raw.get("maxTokens").is_some(),
                    "input" => model.raw.get("input").is_some(),
                    "variants" => {
                        model.raw.get("thinkingLevelMap").is_some()
                            || model.raw.get("thinking").is_some()
                    }
                    _ => false,
                },
                ConfigFormat::DeepSeekHarness => match field {
                    "name" => model.raw.get("name").is_some(),
                    "context" => model.raw.get("contextWindow").is_some(),
                    "output" => model.raw.get("maxTokens").is_some(),
                    "input" => model.raw.get("input").is_some(),
                    "variants" => model.raw.get("reasoningEfforts").is_some(),
                    _ => false,
                },
            })
        })
    }

    /// 将已加载的公共 provider 凭据投影到 DSH 专属字段。
    /// 只在进入 DSH 页面时执行一次，避免用户在页面内主动清空后被立即回填。
    fn project_dsh_credentials(&mut self) {
        for provider in &mut self.providers {
            if provider.api_key_env.trim().is_empty() {
                provider.api_key_env = credentials::default_env_name(&provider.key);
            }
        }
    }

    /// 页面切换时保持 provider 密钥一致：DSH 页使用 api_key_secret（对应
    /// .credentials.yaml 的 refs），其他页面使用 api_key。切换时把非空值
    /// 同步到目标页字段；若用户在 DSH 页明确清空过密钥（原本有、当前空），
    /// 不再用其他页面的旧值覆盖。
    fn sync_provider_secrets(&mut self, target: ConfigFormat) {
        for provider in &mut self.providers {
            if target == ConfigFormat::DeepSeekHarness {
                let cleared_on_dsh = !provider.original_api_key_secret.is_empty()
                    && provider.api_key_secret.is_empty();
                if !provider.api_key.trim().is_empty() && !cleared_on_dsh {
                    provider.api_key_secret = provider.api_key.clone();
                }
            } else {
                let cleared_on_dsh = !provider.original_api_key_secret.is_empty()
                    && provider.api_key_secret.is_empty();
                if cleared_on_dsh {
                    // 与 DSH 方向对称：在 DSH 页明确清空过的密钥（原本有、当前空）
                    // 不再用旧值填充其他页面，避免已清空的密钥被写回 opencode 等配置。
                    provider.api_key = String::new();
                } else if !provider.api_key_secret.trim().is_empty() {
                    provider.api_key = provider.api_key_secret.clone();
                }
            }
        }
    }

    /// 按 provider 的 api 类型构造模型列表接口地址。
    fn models_url(base_url: &str, api: &str) -> String {
        let base = base_url.trim().trim_end_matches('/');
        if base.is_empty() {
            return String::new();
        }
        match api_wire(api) {
            // Anthropic 的模型接口固定为 /v1/models。
            ApiWire::AnthropicMessages => {
                if base.ends_with("/v1") {
                    format!("{}/models", base)
                } else {
                    format!("{}/v1/models", base)
                }
            }
            // Vertex 的模型列表挂在 publishers/google 下。
            ApiWire::GoogleVertex => format!("{}/publishers/google/models", base),
            _ => format!("{}/models", base),
        }
    }

    /// 启动后台线程获取 provider 模型列表。
    fn start_model_fetch(&mut self, key: &str, base_url: &str, secret: &str, api: &str) {
        let url = Self::models_url(base_url, api);
        let secret = secret.trim().to_string();
        let api = api.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = fetch_models_remote(&url, &secret, &api);
            let _ = tx.send(result);
        });
        self.model_fetch.insert(
            key.to_string(),
            ModelFetchState {
                rx: Some(rx),
                result: None,
            },
        );
    }

    /// 启动 provider 级延迟测试（后台线程，结果经通道回传）。
    /// 接收 `&mut HashMap` 而非 `&mut self`，以便与 `providers[idx]` 借用共存。
    fn start_provider_latency(
        latency: &mut HashMap<String, LatencyState>,
        key: &str,
        base_url: &str,
        secret: &str,
        api: &str,
    ) {
        let url = Self::models_url(base_url, api);
        let secret = secret.trim().to_string();
        let api = api.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(measure_provider_latency(&url, &secret, &api));
        });
        let state = latency.entry(key.to_string()).or_default();
        state.provider = None;
        state.provider_rx = Some(rx);
    }

    /// 单模型探测的统一入口：门控 → 选问句 → 启动后台线程 → 返回状态栏消息。
    ///
    /// 写成关联函数（不借 `&mut self`）是为了能在 provider 卡片内部调用：
    /// 那里 `providers` 字段已经被可变借用，只能按字段拆分借用。
    #[allow(clippy::too_many_arguments)]
    fn run_model_probe(
        probe: &mut ProbeGate,
        latency: &mut HashMap<String, LatencyState>,
        net_guard: Option<&str>,
        provider_key: &str,
        model_id: &str,
        now: f64,
        base_url: &str,
        secret: &str,
        api: &str,
    ) -> String {
        match probe.state(provider_key, now, net_guard) {
            ProbeGateState::Ready => {
                let question = probe.next_question(provider_key);
                probe.start(provider_key, now);
                Self::start_model_latency(
                    latency,
                    provider_key,
                    base_url,
                    secret,
                    api,
                    model_id,
                    question,
                );
                format!("正在测试 {} 的 {} 延迟…", provider_key, model_id)
            }
            ProbeGateState::Cooling(left) => {
                format!("节流中：{} 秒后可再测", left.ceil() as u64)
            }
            ProbeGateState::Busy => "上一个延迟测试尚未结束（一次只测一个模型）".to_string(),
            ProbeGateState::NetBlocked(reason) => {
                format!("{}：{}", crate::netguard::BLOCK_PREFIX, reason)
            }
        }
    }

    /// 启动单个模型的延迟探测（后台线程，结果经通道回传）。
    ///
    /// **一次只测一个**：中转站的测活风控对「批量扫模型」最敏感，因此不再提供批量
    /// 入口，节流与串行统一由 [`ProbeGate`] 把关。
    fn start_model_latency(
        latency: &mut HashMap<String, LatencyState>,
        key: &str,
        base_url: &str,
        secret: &str,
        api: &str,
        model: &str,
        question: &str,
    ) {
        let base = base_url.to_string();
        let secret = secret.trim().to_string();
        let api = api.to_string();
        let model = model.trim().to_string();
        let question = question.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker_model = model.clone();
        std::thread::spawn(move || {
            let result = measure_model_latency(&base, &secret, &api, &worker_model, &question);
            let _ = tx.send((worker_model, result));
        });
        let state = latency.entry(key.to_string()).or_default();
        state.done = 0;
        state.total = 1;
        state.pending.clear();
        state.pending.insert(model);
        state.model_rx = Some(rx);
    }

    /// 每帧轮询延迟测试结果，并更新状态栏。
    fn poll_latency(&mut self) {
        let mut notices: Vec<String> = Vec::new();
        for (key, state) in self.latency.iter_mut() {
            if let Some(rx) = &state.provider_rx {
                match rx.try_recv() {
                    Ok(result) => {
                        // 失败原因已就地显示在厂商行（红字 + 悬停详情），
                        // 不再重复推送到页面底部状态栏。
                        if let Ok(ms) = &result {
                            notices.push(format!("provider 延迟测试完成：{} ms", ms));
                        }
                        state.provider = Some(result);
                        state.provider_rx = None;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        // 测量线程异常退出（panic 等）时收不到结果：
                        // 终止等待，避免 Spinner/进度永久卡死；不在底部报错。
                        state.provider_rx = None;
                    }
                }
            }
            if let Some(rx) = &state.model_rx {
                loop {
                    match rx.try_recv() {
                        Ok((id, result)) => {
                            state.pending.remove(&id);
                            state.models.insert(id, result);
                            state.done += 1;
                            // 单模型探测：拿到结果即释放该 provider 的串行位。
                            self.probe.finish(key);
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            state.model_rx = None;
                            state.total = state.done;
                            // 单模型探测：total 恒为 1，只区分「已回传结果」与「线程异常退出」。
                            let unfinished = !state.pending.is_empty();
                            state.pending.clear();
                            notices.push(if unfinished {
                                "模型延迟测试中断（线程异常退出）".to_string()
                            } else {
                                "模型延迟测试完成".to_string()
                            });
                            break;
                        }
                    }
                }
            }
        }
        for msg in notices {
            self.status = msg;
        }
    }

    /// 每帧轮询后台线程的模型获取结果，并更新状态栏。
    fn poll_model_fetch(&mut self) {
        let mut finished: Vec<String> = Vec::new();
        for (key, state) in self.model_fetch.iter_mut() {
            if let Some(rx) = &state.rx {
                if let Ok(result) = rx.try_recv() {
                    state.result = Some(result);
                    state.rx = None;
                    finished.push(key.clone());
                }
            }
        }
        for key in finished {
            let msg = match &self.model_fetch[&key].result {
                Some(Ok(models)) if models.is_empty() => {
                    format!("接口未返回任何模型（{}）", key)
                }
                Some(Ok(models)) => format!("已获取 {} 个模型（{}）", models.len(), key),
                Some(Err(err)) => format!("获取模型失败（{}）: {}", key, err),
                _ => continue,
            };
            self.status = msg;
        }
    }

    /// 当前页面的思考档位标签。
    fn dialect_variants(&self) -> (&'static str, &'static [&'static str]) {
        match self.current_page {
            ConfigFormat::Opencode => (
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
        }
    }

    /// Providers 区块：标题行吸顶（滚动时始终显示在顶部），内容紧跟其下。
    fn ui_providers_section(&mut self, ui: &mut egui::Ui) {
        let anchor = sticky_begin(ui, 30.0);
        let matched: Vec<usize> = (0..self.providers.len()).collect();

        if self.providers.is_empty() && !self.show_new_provider {
            self.show_new_provider = true;
        }

        let mut to_remove: Option<usize> = None;
        let mut to_copy: Option<usize> = None;
        // 拖拽落点必须在**所有卡片渲染完之后**统一聚合再写入 self：
        // 卡片各自赋值会被后渲染的卡片用 None 覆盖（模型卡片曾因此丢失绿色落点边框）。
        let mut hover_target: Option<String> = None;
        let mut model_hover_target: Option<String> = None;
        card_list(ui, &matched, 0.0, |ui, idx| {
            self.render_provider_card(
                ui,
                idx,
                &mut to_remove,
                &mut to_copy,
                &mut hover_target,
                &mut model_hover_target,
            );
        });
        if let Some(idx) = to_remove {
            self.providers.remove(idx);
            self.status = "已删除 provider".into();
        }
        if let Some(idx) = to_copy {
            let mut p = self.providers[idx].clone();
            p.key = format!("{}_copy", p.key);
            self.providers.push(p);
            self.status = "已复制 provider".into();
        }
        if self.provider_drag_src.is_some() {
            self.provider_drag_target = hover_target;
        } else {
            self.provider_drag_target = None;
        }
        // 模型拖拽落点：与 provider 同样在外层聚合，保证任意展开顺序下被拖到的
        // 模型卡片都能拿到绿色边框（见 render_provider_form 里的说明）。
        if self.model_drag_src.is_some() {
            self.model_drag_target = model_hover_target;
        } else {
            self.model_drag_target = None;
        }

        ui.add_space(10.0);
        if ui.button("新增 Provider").clicked() {
            self.show_new_provider = !self.show_new_provider;
        }
        if self.show_new_provider {
            self.ui_new_provider_form(ui);
        }
        sticky_end(ui, anchor, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Providers");
                if !self.providers.is_empty() {
                    let all_open = self
                        .providers
                        .iter()
                        .all(|p| self.provider_open.contains(&p.key));
                    if ui
                        .button(if all_open {
                            "收起全部卡片"
                        } else {
                            "展开全部卡片"
                        })
                        .clicked()
                    {
                        if all_open {
                            self.provider_open.clear();
                        } else {
                            self.provider_open =
                                self.providers.iter().map(|p| p.key.clone()).collect();
                        }
                    }
                }
                // 连通性测试：放在标题行右侧，收起全部卡片时也始终可见。
                if ui
                    .button("连通性测试")
                    .on_hover_text("并发测试当前页面全部厂商的接口连通性")
                    .clicked()
                {
                    let targets: Vec<(String, String, String, String)> = self
                        .providers
                        .iter()
                        .map(|p| {
                            let api = p.effective_api();
                            (
                                p.key.clone(),
                                p.base_url.clone(),
                                credentials::effective_secret(p),
                                api,
                            )
                        })
                        .collect();
                    let count = targets.len();
                    for (key, base, secret, api) in targets {
                        Self::start_provider_latency(&mut self.latency, &key, &base, &secret, &api);
                    }
                    self.status = format!("已开始连通性测试（{} 个厂商）", count);
                }
                // 网络守卫提示：检测到系统代理 / VPN 时禁用模型延迟测试
                //（中转站的「多 IP 检测 / 测活封号」可能因此触发）。
                if let Some(reason) = self.net_guard.clone() {
                    ui.label(
                        egui::RichText::new(format!("{}：{reason}", crate::netguard::BLOCK_PREFIX))
                            .small()
                            .color(LATENCY_RED),
                    )
                    .on_hover_text(
                        "中转站常见多 IP 检测 / 测活风控，经代理做推理探测可能被封号；\n\
                         因此已禁用模型延迟测试。请关闭系统代理 / VPN 后重试。\n\
                         （厂商「连通性测试」不受影响：它只拉模型列表，不做推理。）",
                    );
                }
                // 全局 API Key 显示/隐藏：一键切换全部密钥的明文/掩码。
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
                // 配置预览：右侧面板实时展示当前页面的序列化内容，可编辑并应用回组件。
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
            });
        });
    }

    fn render_provider_card(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        to_remove: &mut Option<usize>,
        to_copy: &mut Option<usize>,
        hover_target: &mut Option<String>,
        model_hover_target: &mut Option<String>,
    ) {
        let key = self.providers[idx].key.clone();
        let open = self.provider_open.contains(&key);
        let highlight = if self.provider_drag_target.as_deref() == Some(key.as_str()) {
            2
        } else if self.provider_drag_src.as_deref() == Some(key.as_str()) {
            1
        } else {
            0
        };
        let resp = card_frame(ui, open, highlight, |ui| {
            ui.horizontal(|ui| {
                let h = ui.add(DragHandle);
                if h.drag_started() {
                    self.provider_drag_src = Some(key.clone());
                    self.provider_drag_target = None;
                }
                if h.drag_stopped() {
                    if self.provider_drag_src == Some(key.clone()) {
                        if let Some(dst) = self.provider_drag_target.clone() {
                            let s = self.providers.iter().position(|p| p.key == key);
                            let d = self.providers.iter().position(|p| p.key == dst);
                            if let (Some(s), Some(d)) = (s, d) {
                                move_item(&mut self.providers, s, d);
                            }
                        }
                    }
                    self.provider_drag_src = None;
                    self.provider_drag_target = None;
                }
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(if open { "▼" } else { "▶" }).size(14.0),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    if open {
                        self.provider_open.remove(&key);
                    } else {
                        self.provider_open.insert(key.clone());
                    }
                }
                ui.strong(&self.providers[idx].key);
                // 连通性测试结果：显示在厂商名字右侧，卡片收起时也可见。
                if let Some(state) = self.latency.get(&key) {
                    if state.provider_rx.is_some() {
                        matrix_label(ui, &key);
                    } else if let Some(res) = &state.provider {
                        match res {
                            Ok(ms) => {
                                ui.label(
                                    egui::RichText::new(format!("{}ms", ms))
                                        .color(latency_color(*ms)),
                                );
                            }
                            Err(err) => {
                                ui.label(egui::RichText::new(short_err(err)).color(LATENCY_RED))
                                    .on_hover_text(err);
                            }
                        }
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("删除").clicked() {
                        *to_remove = Some(idx);
                    }
                    if ui.button("复制").clicked() {
                        *to_copy = Some(idx);
                    }
                });
            });
            if open {
                self.render_provider_form(ui, idx, model_hover_target);
            }
        });
        if let Some(src_key) = &self.provider_drag_src {
            if src_key != &key && resp.contains_pointer() && hover_target.is_none() {
                *hover_target = Some(key.clone());
            }
        }
    }

    fn render_provider_form(
        &mut self,
        ui: &mut egui::Ui,
        idx: usize,
        model_hover_target: &mut Option<String>,
    ) {
        let prev_key = self.providers[idx].key.clone();
        let other_keys: HashSet<String> = self
            .providers
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != idx)
            .map(|(_, p)| p.key.trim().to_string())
            .collect();
        let (variants_label, variant_names) = self.dialect_variants();
        let ProviderFormFlags {
            show_oc,
            show_omp,
            show_dsh,
            show_provider_base_url,
            show_provider_timeout,
            show_model_name,
            show_model_context,
            show_model_output,
            show_model_input,
            show_model_variants,
            show_model_reasoning,
            show_model_tool_call,
            show_model_store,
            base_label,
            api_key_label,
            context_label,
            output_label,
            input_label,
            ..
        } = ProviderFormFlags::new(self);
        let p = &mut self.providers[idx];
        ui.horizontal_wrapped(|ui| {
            field_label(ui, 120.0, "key");
            let key_resp = ui.add(egui::TextEdit::singleline(&mut p.key).desired_width(120.0));
            if !p.key.trim().is_empty() && other_keys.contains(p.key.trim()) {
                key_resp.on_hover_text("key 与其他 provider 重复，保存将被阻止");
                ui.label(
                    egui::RichText::new("⚠ 重复")
                        .small()
                        .color(egui::Color32::from_rgb(220, 90, 90)),
                );
            }
            if show_oc {
                let salt = format!("provider_npm_{}", p.key);
                provider_npm_combo(ui, p, &salt, true);
            }
            if !show_oc {
                let salt = format!("provider_api_{}", p.key);
                provider_api_combo(ui, p, show_omp, &salt);
            }
            // timeout / timeoutMs 与 npm/api 同排（第一行）。
            if show_oc && show_provider_timeout {
                field_label(ui, 120.0, "options.timeout");
                numeric_text_edit(ui, &mut p.timeout, 70.0, "180000");
            }
            if show_dsh {
                field_label(ui, 120.0, "timeoutMs");
                numeric_text_edit(ui, &mut p.dsh_timeout_ms, 70.0, "180000");
            }
            // pi / omp 的 compat 与 api 同排显示（紧跟 api 之后）。
            if !show_oc && !show_dsh {
                field_label(ui, 120.0, "compat");
                ui.checkbox(&mut p.compat, "supportsDeveloperRole");
                // pi / omp 相互映射字段：加载 opencode/dsh 时缺省不勾选。
                let requires_label = if show_omp {
                    "requiresReasoningContentForAllAssistantTurns"
                } else {
                    "requiresReasoningContentOnAssistantMessages"
                };
                ui.checkbox(&mut p.requires_reasoning_content, requires_label);
            }
            if show_dsh {
                field_label(ui, 120.0, "retryPolicy.mode");
                ui.add(egui::TextEdit::singleline(&mut p.dsh_retry_mode).desired_width(100.0));
                field_label(ui, 120.0, "maxRetries");
                numeric_text_edit(ui, &mut p.dsh_max_retries, 55.0, "3");
            }
        });
        ui.horizontal_wrapped(|ui| {
            if show_provider_base_url {
                field_label(ui, 120.0, base_label);
                ui.add(egui::TextEdit::singleline(&mut p.base_url).desired_width(200.0));
            }
            field_label(ui, 120.0, api_key_label);
            if show_dsh {
                ui.add(egui::TextEdit::singleline(&mut p.api_key_env).desired_width(192.0));
                field_label(ui, 120.0, "API Key");
                secret_text_edit(ui, &mut p.api_key_secret, self.show_api_keys, 408.0, "");
            } else {
                secret_text_edit(ui, &mut p.api_key, self.show_api_keys, 408.0, "");
            }
        });

        ui.add_space(6.0);
        let mut fetch_request: Option<(String, String, String, String)> = None;
        let mut close_fetch = false;
        // 本帧用户点下的探测请求（provider key, model id），UI 循环外统一发起。
        let mut probe_request: Option<(String, String)> = None;
        ui.horizontal(|ui| {
            ui.strong("Models");
            let fetch_api = p.effective_api();
            let fetch_secret = credentials::effective_secret(p);
            if ui.button("获取模型").clicked() {
                fetch_request = Some((
                    p.key.clone(),
                    p.base_url.clone(),
                    fetch_secret.clone(),
                    fetch_api.clone(),
                ));
            }
            if self.model_fetch_open.contains(&p.key) && ui.button("关闭").clicked() {
                close_fetch = true;
            }
            // 模型延迟已无批量入口：每个模型行右侧各有一个「测试」按钮，
            // 一次只测一个（同模型 60s、同厂商 10s 节流，规避测活风控）。
            if self
                .latency
                .get(&p.key)
                .is_some_and(|s| s.model_rx.is_some())
            {
                ui.label(egui::RichText::new("延迟测试中…").small());
            }
        });
        if let Some((key, base, secret, api)) = fetch_request {
            // 后台线程拉取模型列表（避免阻塞 UI），结果经通道回传。
            let url = Self::models_url(&base, &api);
            let secret = secret.trim().to_string();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let result = fetch_models_remote(&url, &secret, &api);
                let _ = tx.send(result);
            });
            self.model_fetch.insert(
                key.clone(),
                ModelFetchState {
                    rx: Some(rx),
                    result: None,
                },
            );
            self.model_fetch_open.insert(key);
        }
        if close_fetch {
            self.model_fetch_open.remove(&p.key);
        }
        if self.model_fetch_open.contains(&p.key) {
            let fetch_key = p.key.clone();
            card_frame(ui, false, 0, |ui| {
                model_fetch_popup(
                    ui,
                    self.model_fetch.get(&fetch_key),
                    &mut p.models,
                    self.current_page,
                    ("model_fetch_scroll", fetch_key.as_str()),
                );
            });
        }
        let mut rm: Option<usize> = None;
        let mut model_hover_here: Option<String> = None;
        let mut model_drag_stopped = false;
        for j in 0..p.models.len() {
            let model_key = format!("{}\u{1f}{}", p.key, p.models[j].id);
            let model_highlight = if self.model_drag_target.as_deref() == Some(model_key.as_str()) {
                2
            } else if self.model_drag_src.as_deref() == Some(model_key.as_str()) {
                1
            } else {
                0
            };
            let other_ids: HashSet<String> = p
                .models
                .iter()
                .enumerate()
                .filter(|(j2, _)| *j2 != j)
                .map(|(_, m)| m.id.trim().to_string())
                .collect();
            let model_response = card_frame(ui, true, model_highlight, |ui| {
                ui.horizontal(|ui| {
                    let handle = ui.add(DragHandle);
                    if handle.drag_started() {
                        self.model_drag_src = Some(model_key.clone());
                        self.model_drag_target = None;
                    }
                    if handle.drag_stopped() {
                        model_drag_stopped = true;
                    }
                    // 单模型延迟测试：按钮在拖动按钮右侧，结果显示在按钮右侧。
                    let model_id = p.models[j].id.trim().to_string();
                    if model_probe_button(
                        ui,
                        &self.probe,
                        &p.key,
                        ui.input(|i| i.time),
                        self.net_guard.as_deref(),
                    ) {
                        probe_request = Some((p.key.clone(), model_id.clone()));
                    }
                    let latency = self.latency.get(&p.key);
                    model_latency_label(ui, latency, &model_id);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("删").clicked() {
                            rm = Some(j);
                        }
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    field_label(ui, 120.0, "id:");
                    let id_resp = ui
                        .add(egui::TextEdit::singleline(&mut p.models[j].id).desired_width(120.0));
                    if !p.models[j].id.trim().is_empty()
                        && other_ids.contains(p.models[j].id.trim())
                    {
                        id_resp.on_hover_text("id 与同 provider 内其他模型重复，保存将被阻止");
                        ui.label(
                            egui::RichText::new("⚠ 重复")
                                .small()
                                .color(egui::Color32::from_rgb(220, 90, 90)),
                        );
                    }
                    if show_model_name {
                        field_label(ui, 120.0, "name:");
                        ui.add(
                            egui::TextEdit::singleline(&mut p.models[j].name).desired_width(120.0),
                        );
                    }
                    if show_model_reasoning && (show_oc || !show_dsh) {
                        ui.checkbox(&mut p.models[j].reasoning, "reasoning");
                    }
                    if show_model_tool_call && show_oc {
                        ui.checkbox(&mut p.models[j].tool_call, "tool_call");
                    }
                    if show_model_store && show_oc {
                        ui.checkbox(&mut p.models[j].store, "store");
                    }
                    if show_model_context {
                        field_label(ui, 120.0, context_label);
                        numeric_text_edit(ui, &mut p.models[j].context, 53.0, "");
                    }
                    if show_model_output {
                        field_label(ui, 120.0, output_label);
                        numeric_text_edit(ui, &mut p.models[j].output, 53.0, "");
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if show_model_input {
                        field_label(ui, 120.0, input_label);
                        ui.add(
                            egui::TextEdit::singleline(&mut p.models[j].modalities_input)
                                .desired_width(80.0),
                        );
                    }
                    if show_oc {
                        field_label(ui, 120.0, "modalities.output");
                        ui.add(
                            egui::TextEdit::singleline(&mut p.models[j].modalities_output)
                                .desired_width(80.0),
                        );
                    }
                    if show_model_variants {
                        field_label(ui, 120.0, variants_label);
                    }
                    let variant_key = format!("variant_open_{}_{}", p.key, j);
                    variant_selector(
                        ui,
                        &mut p.models[j].variants,
                        variant_names,
                        variant_key,
                        &mut self.variant_open,
                        true,
                    );
                });
            });
            if let Some(src) = &self.model_drag_src {
                if src != &model_key
                    && model_response.contains_pointer()
                    && model_hover_here.is_none()
                {
                    model_hover_here = Some(model_key.clone());
                }
            }
        }
        if let Some((provider_key, model_id)) = probe_request {
            let now = ui.input(|i| i.time);
            let base_url = p.base_url.clone();
            let secret = credentials::effective_secret(p);
            let api = p.effective_api();
            self.status = Self::run_model_probe(
                &mut self.probe,
                &mut self.latency,
                self.net_guard.as_deref(),
                &provider_key,
                &model_id,
                now,
                &base_url,
                &secret,
                &api,
            );
        }
        // 只登记「本 provider 内被拖到的模型」，跨卡片的聚合交给调用方
        // （ui_providers_section 在全部卡片渲染完之后统一写入 self.model_drag_target）。
        merge_drag_target(model_hover_target, model_hover_here);
        if model_drag_stopped {
            if let Some(src) = self.model_drag_src.take() {
                let target = self.model_drag_target.take();
                if let Some(dst) = target {
                    let source = p
                        .models
                        .iter()
                        .position(|m| format!("{}\u{1f}{}", p.key, m.id) == src);
                    let destination = p
                        .models
                        .iter()
                        .position(|m| format!("{}\u{1f}{}", p.key, m.id) == dst);
                    if let (Some(source), Some(destination)) = (source, destination) {
                        move_item(&mut p.models, source, destination);
                    }
                }
            }
        }
        if let Some(j) = rm {
            p.models.remove(j);
            // 删除后下标错位：关闭该 provider 的档位弹窗，避免状态串到其他模型
            let prefix = format!("variant_open_{}_", p.key);
            self.variant_open.retain(|k| !k.starts_with(&prefix));
        }
        ui.add_space(6.0);
        let show_new_model_key = format!("show_new_model_{}", p.key);
        let show_new_model = self.variant_open.contains(&show_new_model_key);
        let btn_text = if show_new_model {
            "收起"
        } else {
            "添加 Model"
        };
        if ui
            .add_sized([120.0, 20.0], egui::Button::new(btn_text))
            .clicked()
        {
            if show_new_model {
                self.variant_open.remove(&show_new_model_key);
            } else {
                self.variant_open.insert(show_new_model_key.clone());
            }
        }
        if show_new_model {
            ui.horizontal_wrapped(|ui| {
                field_label(ui, 120.0, "id:");
                ui.add(egui::TextEdit::singleline(&mut p.new_model.id).desired_width(120.0));
                field_label(ui, 120.0, "name:");
                ui.add(egui::TextEdit::singleline(&mut p.new_model.name).desired_width(120.0));
                if show_oc || !show_dsh {
                    ui.checkbox(&mut p.new_model.reasoning, "reasoning");
                }
                if show_oc {
                    ui.checkbox(&mut p.new_model.tool_call, "tool_call");
                    ui.checkbox(&mut p.new_model.store, "store");
                }
                field_label(ui, 120.0, context_label);
                numeric_text_edit(ui, &mut p.new_model.context, 53.0, "");
                field_label(ui, 120.0, output_label);
                numeric_text_edit(ui, &mut p.new_model.output, 53.0, "");
            });
            ui.horizontal_wrapped(|ui| {
                field_label(ui, 120.0, input_label);
                ui.add(
                    egui::TextEdit::singleline(&mut p.new_model.modalities_input)
                        .desired_width(80.0),
                );
                if show_oc {
                    field_label(ui, 120.0, "modalities.output");
                    ui.add(
                        egui::TextEdit::singleline(&mut p.new_model.modalities_output)
                            .desired_width(80.0),
                    );
                }
                field_label(ui, 120.0, variants_label);
                let variant_key = format!("new_model_variant_{}", p.key);
                variant_selector(
                    ui,
                    &mut p.new_model.variants,
                    variant_names,
                    variant_key,
                    &mut self.variant_open,
                    false,
                );
            });
            ui.horizontal(|ui| {
                ui.add_space(60.0);
                if ui.button("添加").clicked() && !p.new_model.id.trim().is_empty() {
                    p.models.push(p.new_model.clone());
                    p.new_model = ModelRow::new();
                    self.variant_open.remove(&show_new_model_key);
                }
            });
        }
        // key 重命名后同步展开状态与弹窗键
        let new_key = self.providers[idx].key.clone();
        if new_key != prev_key {
            self.sync_provider_rename(&prev_key, &new_key);
        }
    }

    fn ui_new_provider_form(&mut self, ui: &mut egui::Ui) {
        let (variants_label, variant_names) = self.dialect_variants();
        let ProviderFormFlags {
            show_oc,
            show_omp,
            show_dsh,
            base_label,
            api_key_label,
            context_label,
            output_label,
            input_label,
            ..
        } = ProviderFormFlags::new(self);
        ui.group(|ui| {
            ui.horizontal_wrapped(|ui| {
                field_label(ui, 120.0, "key");
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_provider.key)
                        .hint_text("openai")
                        .desired_width(120.0),
                );
                if show_oc {
                    let p = &mut self.new_provider;
                    provider_npm_combo(ui, p, "new_provider_npm", false);
                }
                if !show_oc {
                    let p = &mut self.new_provider;
                    provider_api_combo(ui, p, show_omp, "new_provider_api");
                }
                // timeout 与 npm/api 同排（第一行）。
                if show_oc {
                    field_label(ui, 120.0, "options.timeout");
                    numeric_text_edit(ui, &mut self.new_provider.timeout, 70.0, "180000");
                }
                // pi / omp 的 compat 与 api 同排显示（紧跟 api 之后）。
                if !show_oc && !show_dsh {
                    field_label(ui, 120.0, "compat");
                    ui.checkbox(&mut self.new_provider.compat, "supportsDeveloperRole");
                    let requires_label = if show_omp {
                        "requiresReasoningContentForAllAssistantTurns"
                    } else {
                        "requiresReasoningContentOnAssistantMessages"
                    };
                    ui.checkbox(
                        &mut self.new_provider.requires_reasoning_content,
                        requires_label,
                    );
                }
            });
            ui.horizontal_wrapped(|ui| {
                field_label(ui, 120.0, base_label);
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_provider.base_url)
                        .hint_text("https://api.openai.com/v1")
                        .desired_width(200.0),
                );
                field_label(ui, 120.0, api_key_label);
                if show_dsh {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_provider.api_key_env)
                            .hint_text("DEEPSEEK_API_KEY")
                            .desired_width(192.0),
                    );
                    field_label(ui, 120.0, "API Key");
                    secret_text_edit(
                        ui,
                        &mut self.new_provider.api_key_secret,
                        self.show_api_keys,
                        408.0,
                        "实际密钥",
                    );
                } else {
                    secret_text_edit(
                        ui,
                        &mut self.new_provider.api_key,
                        self.show_api_keys,
                        408.0,
                        "sk-xxx",
                    );
                }
            });
            ui.add_space(6.0);
            let mut fetch_request: Option<(String, String, String)> = None;
            let mut close_fetch = false;
            ui.horizontal(|ui| {
                ui.strong("Models");
                let fetch_api = self.new_provider.effective_api();
                let fetch_secret = credentials::effective_secret(&self.new_provider);
                if ui.button("获取模型").clicked() {
                    fetch_request = Some((
                        self.new_provider.base_url.clone(),
                        fetch_secret.clone(),
                        fetch_api.clone(),
                    ));
                }
                if self.model_fetch_open.contains(NEW_PROVIDER_FETCH_KEY)
                    && ui.button("关闭").clicked()
                {
                    close_fetch = true;
                }
                // 模型延迟已无批量入口：每个模型行右侧各有一个「测试」按钮。
                if self
                    .latency
                    .get(NEW_PROVIDER_FETCH_KEY)
                    .is_some_and(|s| s.model_rx.is_some())
                {
                    ui.label(egui::RichText::new("延迟测试中…").small());
                }
            });
            if let Some((base, secret, api)) = fetch_request {
                self.start_model_fetch(NEW_PROVIDER_FETCH_KEY, &base, &secret, &api);
                self.model_fetch_open
                    .insert(NEW_PROVIDER_FETCH_KEY.to_string());
            }
            if close_fetch {
                self.model_fetch_open.remove(NEW_PROVIDER_FETCH_KEY);
            }
            if self.model_fetch_open.contains(NEW_PROVIDER_FETCH_KEY) {
                card_frame(ui, false, 0, |ui| {
                    model_fetch_popup(
                        ui,
                        self.model_fetch.get(NEW_PROVIDER_FETCH_KEY),
                        &mut self.new_provider.models,
                        self.current_page,
                        "new_provider_fetch_scroll",
                    );
                });
            }
            let mut rm_new: Option<usize> = None;
            let mut move_new_request: Option<(usize, usize)> = None;
            // 本帧用户点下的探测请求（新 provider 固定用 NEW_PROVIDER_FETCH_KEY 做节流键）。
            let mut probe_request: Option<(String, String)> = None;
            for j in 0..self.new_provider.models.len() {
                let model_count = self.new_provider.models.len();
                card_frame(ui, true, 0, |ui| {
                    ui.horizontal(|ui| {
                        if j > 0 && ui.button("↑").clicked() {
                            move_new_request = Some((j, j - 1));
                        }
                        if j + 1 < model_count && ui.button("↓").clicked() {
                            move_new_request = Some((j, j + 1));
                        }
                        // 单模型延迟测试：按钮在调整按钮右侧，结果显示在按钮右侧。
                        let model_id = self.new_provider.models[j].id.trim().to_string();
                        if model_probe_button(
                            ui,
                            &self.probe,
                            NEW_PROVIDER_FETCH_KEY,
                            ui.input(|i| i.time),
                            self.net_guard.as_deref(),
                        ) {
                            probe_request =
                                Some((NEW_PROVIDER_FETCH_KEY.to_string(), model_id.clone()));
                        }
                        let latency = self.latency.get(NEW_PROVIDER_FETCH_KEY);
                        model_latency_label(ui, latency, &model_id);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("删").clicked() {
                                rm_new = Some(j);
                            }
                        });
                    });
                    ui.horizontal_wrapped(|ui| {
                        field_label(ui, 120.0, "id:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_provider.models[j].id)
                                .desired_width(120.0),
                        );
                        field_label(ui, 120.0, "name:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_provider.models[j].name)
                                .desired_width(120.0),
                        );
                        if show_oc || !show_dsh {
                            ui.checkbox(&mut self.new_provider.models[j].reasoning, "reasoning");
                        }
                        if show_oc {
                            ui.checkbox(&mut self.new_provider.models[j].tool_call, "tool_call");
                            ui.checkbox(&mut self.new_provider.models[j].store, "store");
                        }
                        field_label(ui, 120.0, context_label);
                        numeric_text_edit(ui, &mut self.new_provider.models[j].context, 53.0, "");
                        field_label(ui, 120.0, output_label);
                        numeric_text_edit(ui, &mut self.new_provider.models[j].output, 53.0, "");
                    });
                });
            }
            if let Some((provider_key, model_id)) = probe_request {
                let now = ui.input(|i| i.time);
                let base_url = self.new_provider.base_url.clone();
                let secret = credentials::effective_secret(&self.new_provider);
                let api = self.new_provider.effective_api();
                self.status = Self::run_model_probe(
                    &mut self.probe,
                    &mut self.latency,
                    self.net_guard.as_deref(),
                    &provider_key,
                    &model_id,
                    now,
                    &base_url,
                    &secret,
                    &api,
                );
            }
            if let Some((from, to)) = move_new_request {
                move_item(&mut self.new_provider.models, from, to);
            }
            if let Some(j) = rm_new {
                self.new_provider.models.remove(j);
            }
            ui.add_space(6.0);
            let show_new_model_key = format!("new_provider_show_model_{}", self.new_provider.key);
            let show_new_model = self.variant_open.contains(&show_new_model_key);
            if ui
                .button(if show_new_model {
                    "收起"
                } else {
                    "添加 Model"
                })
                .clicked()
            {
                if show_new_model {
                    self.variant_open.remove(&show_new_model_key);
                } else {
                    self.variant_open.insert(show_new_model_key.clone());
                }
            }
            if show_new_model {
                ui.horizontal_wrapped(|ui| {
                    field_label(ui, 120.0, "id:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_provider.new_model.id)
                            .desired_width(120.0),
                    );
                    field_label(ui, 120.0, "name:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_provider.new_model.name)
                            .desired_width(120.0),
                    );
                    if show_oc || !show_dsh {
                        ui.checkbox(&mut self.new_provider.new_model.reasoning, "reasoning");
                    }
                    if show_oc {
                        ui.checkbox(&mut self.new_provider.new_model.tool_call, "tool_call");
                        ui.checkbox(&mut self.new_provider.new_model.store, "store");
                    }
                    field_label(ui, 120.0, context_label);
                    numeric_text_edit(ui, &mut self.new_provider.new_model.context, 53.0, "");
                    field_label(ui, 120.0, output_label);
                    numeric_text_edit(ui, &mut self.new_provider.new_model.output, 53.0, "");
                });
                ui.horizontal_wrapped(|ui| {
                    field_label(ui, 120.0, input_label);
                    ui.add(
                        egui::TextEdit::singleline(
                            &mut self.new_provider.new_model.modalities_input,
                        )
                        .desired_width(80.0),
                    );
                    if show_oc {
                        field_label(ui, 120.0, "modalities.output");
                        ui.add(
                            egui::TextEdit::singleline(
                                &mut self.new_provider.new_model.modalities_output,
                            )
                            .desired_width(80.0),
                        );
                    }
                    field_label(ui, 120.0, variants_label);
                    variant_selector(
                        ui,
                        &mut self.new_provider.new_model.variants,
                        variant_names,
                        "new_provider_new_model_variant".to_string(),
                        &mut self.variant_open,
                        false,
                    );
                });
                ui.horizontal(|ui| {
                    ui.add_space(60.0);
                    if ui.button("添加").clicked()
                        && !self.new_provider.new_model.id.trim().is_empty()
                    {
                        self.new_provider
                            .models
                            .push(self.new_provider.new_model.clone());
                        self.new_provider.new_model = ModelRow::new();
                        self.variant_open.remove(&show_new_model_key);
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.add_space(60.0);
                if ui.button("确认").clicked() {
                    let key = self.new_provider.key.trim().to_string();
                    if key.is_empty() {
                        self.status = "请填写 provider key".into();
                    } else if self.providers.iter().any(|p| p.key.trim() == key) {
                        self.status = format!("provider key \"{}\" 已存在", key);
                    } else {
                        let np = self.new_provider.clone();
                        self.providers.push(np);
                        self.new_provider = ProviderRow::new();
                        self.show_new_provider = false;
                        self.clear_new_provider_state();
                        self.status = "已添加 provider".into();
                    }
                }
                if ui.button("取消").clicked() {
                    self.new_provider = ProviderRow::new();
                    self.show_new_provider = false;
                    self.clear_new_provider_state();
                }
            });
        });
    }

    /// 关闭新增 provider 表单时清理其测试/获取状态，避免下次打开残留旧结果。
    fn clear_new_provider_state(&mut self) {
        self.latency.remove(NEW_PROVIDER_FETCH_KEY);
        self.model_fetch.remove(NEW_PROVIDER_FETCH_KEY);
        self.model_fetch_open.remove(NEW_PROVIDER_FETCH_KEY);
        // 表单关掉后探测结果无处显示：释放它的串行位。
        self.probe.release(Some(NEW_PROVIDER_FETCH_KEY));
    }

    fn reload(&mut self) {
        let (fmt, _) = ConfigPaths::detect_for_path(&self.config_path);
        self.source_format = fmt;
        self.apply_load();
    }

    fn paint_drag_ghost(&self, ctx: &egui::Context) {
        let label = if let Some(k) = &self.agent_drag_src {
            self.agents
                .iter()
                .find(|a| &a.key == k)
                .map(|a| a.key.as_str())
                .unwrap_or("")
        } else if let Some(k) = &self.provider_drag_src {
            self.providers
                .iter()
                .find(|p| &p.key == k)
                .map(|p| p.key.as_str())
                .unwrap_or("")
        } else if let Some(k) = &self.model_drag_src {
            k.split_once('\u{1f}')
                .map(|(_, model)| model)
                .unwrap_or(k.as_str())
        } else {
            return;
        };
        if label.is_empty() {
            return;
        }
        let Some(pointer) = ctx.pointer_hover_pos() else {
            return;
        };
        let layer_id = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("drag_ghost"));
        let painter = ctx.layer_painter(layer_id);
        let visuals = &ctx.style().visuals;
        let font_id = egui::FontId::proportional(14.0);
        let text_color = visuals.text_color();
        let bg = visuals.faint_bg_color;
        let stroke_color = visuals.widgets.noninteractive.bg_stroke.color;
        let galley = painter.layout(
            label.to_string(),
            font_id.clone(),
            text_color,
            f32::INFINITY,
        );
        let label_w = galley.size().x;
        let size = egui::vec2(label_w + 24.0, 30.0);
        let ghost_rect = egui::Rect::from_min_size(pointer + egui::vec2(20.0, -15.0), size);
        painter.rect_filled(ghost_rect, 8.0, bg);
        painter.rect_stroke(
            ghost_rect,
            8.0,
            egui::Stroke::new(1.0, stroke_color),
            egui::StrokeKind::Inside,
        );
        painter.text(
            ghost_rect.left_center() + egui::vec2(12.0, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            font_id,
            text_color,
        );
    }

    /// agent key 重命名后同步 UI 状态（卡片展开集合），避免改名后卡片收起。
    fn sync_agent_rename(&mut self, old: &str, new: &str) {
        if old == new || new.is_empty() {
            return;
        }
        if self.agent_open.remove(old) {
            self.agent_open.insert(new.to_string());
        }
    }

    /// provider key 重命名后同步 UI 状态（展开集合 + 弹窗键）。
    fn sync_provider_rename(&mut self, old: &str, new: &str) {
        if old == new || new.is_empty() {
            return;
        }
        if self.provider_open.remove(old) {
            self.provider_open.insert(new.to_string());
        }
        // 下标型弹窗键直接关闭（避免前缀歧义），需要时重新打开即可
        let variant_prefix = format!("variant_open_{}_", old);
        let show_key = format!("show_new_model_{}", old);
        let new_variant_key = format!("new_model_variant_{}", old);
        self.variant_open
            .retain(|k| !k.starts_with(&variant_prefix) && k != &show_key && k != &new_variant_key);
    }

    /// 统计非法数字字段数（非空且解析失败），保存后提示用户它们被忽略。
    fn count_invalid_numeric_fields(&self) -> usize {
        fn bad(s: &str) -> bool {
            !s.trim().is_empty() && parse_number_text(s).is_none()
        }
        let mut n = 0;
        for a in &self.agents {
            if bad(&a.temperature) {
                n += 1;
            }
        }
        for p in &self.providers {
            if bad(&p.timeout) {
                n += 1;
            }
            for m in &p.models {
                if bad(&m.context) || bad(&m.output) {
                    n += 1;
                }
            }
        }
        n
    }

    /// 校验 agent / provider / model key 唯一性，返回首个冲突描述。
    fn find_duplicate_keys(&self) -> Option<String> {
        let mut seen = HashSet::new();
        for a in &self.agents {
            let k = a.key.trim();
            if !k.is_empty() && !seen.insert(k.to_string()) {
                return Some(format!("agent \"{}\"", k));
            }
        }
        let mut seen_p = HashSet::new();
        for p in &self.providers {
            let k = p.key.trim();
            if k.is_empty() {
                continue;
            }
            if !seen_p.insert(k.to_string()) {
                return Some(format!("provider \"{}\"", k));
            }
            let mut seen_m = HashSet::new();
            for m in &p.models {
                let mk = m.id.trim();
                if !mk.is_empty() && !seen_m.insert(mk.to_string()) {
                    return Some(format!("provider \"{}\" 的 model \"{}\"", k, mk));
                }
            }
        }
        None
    }

    /// 本页写入路径：
    /// - 当前文件属于本页格式且已加载 → 当前文件（整体替换）；
    /// - 路径已修改但未加载 → 仍写该路径，但按“先读后合并”（防止覆盖目标文件已有配置）；
    /// - 其余 → 该后端默认目标（Windows 本地；WSL 需勾选「WSL同步」）。
    fn page_save_path(&self, fmt: ConfigFormat) -> PageTarget {
        if !self.config_path.is_empty() && self.config_path != self.loaded_path {
            // 用户已经在路径框中明确指定了目标文件，即使尚未点击“加载”，
            // 也必须使用该路径；保存流程会先读目标并按目标格式合并，不能静默回落默认路径。
            return PageTarget::Modified(self.config_path.clone());
        }
        if self.source_format == fmt && !self.config_path.is_empty() {
            return PageTarget::Current(self.config_path.clone());
        }
        let path = self
            .targets
            .iter()
            .find(|t| t.backend == fmt)
            .map(|t| t.path.clone())
            .unwrap_or_default();
        PageTarget::Default(path)
    }

    /// 页头：本页保存按钮 + 写入路径。
    fn ui_page_header(&mut self, ui: &mut egui::Ui) {
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
            ui.label(egui::RichText::new(format!("写入: {}", path)).weak())
                .on_hover_text(kind);
            // 该格式不支持的区块提前提示，避免保存后才发现数据没写入
            if fmt != ConfigFormat::Opencode && !self.agents.is_empty() {
                ui.label(
                    egui::RichText::new(format!(
                        "⚠ {} 个 agents 不会写入该格式",
                        self.agents.len()
                    ))
                    .small()
                    .color(egui::Color32::from_rgb(220, 160, 60)),
                )
                .on_hover_text("该格式不支持 agent 定义，保存时将忽略");
            }
        });
        ui.separator();
    }

    /// 保存当前页面：写入该页格式对应的路径，并按需同步 WSL。
    fn save_page(&mut self, fmt: ConfigFormat) {
        if let Some(dup) = self.find_duplicate_keys() {
            self.status = format!("key 重复: {}，已取消保存", dup);
            return;
        }
        let target = self.page_save_path(fmt);
        let path = match &target {
            PageTarget::Current(p) | PageTarget::Modified(p) | PageTarget::Default(p) => p.clone(),
        };
        match &target {
            PageTarget::Current(_) => {
                if let Some(err) = self.load_error.clone() {
                    self.status = format!("当前文件: 加载失败({})，已跳过", err);
                    return;
                }
            }
            PageTarget::Default(_) => {
                if !self.targets.iter().any(|t| t.backend == fmt && t.available) {
                    self.status = format!("{}: 未安装（本地与 WSL 均未找到配置）", fmt.label());
                    return;
                }
            }
            PageTarget::Modified(_) => {}
        }
        let res = self.save_backend_to(fmt, &path);
        let ok = res.is_ok();
        self.status = match res {
            Ok(backup) => {
                let mut msg = format!("{}: 已保存", fmt.label());
                if let Some(backup) = backup {
                    msg.push_str(&format!("（跨格式转换，原文件已备份为 {}）", backup));
                }
                msg
            }
            Err(e) => format!("{}: 保存失败({})", fmt.label(), e),
        };
        if ok {
            // 该格式不支持 agents 时明确告知，避免误以为已写入
            if fmt != ConfigFormat::Opencode && !self.agents.is_empty() {
                self.status.push_str(&format!(
                    "（{} 个 agents 未写入：该格式不支持）",
                    self.agents.len()
                ));
            }
            let bad = self.count_invalid_numeric_fields();
            if bad > 0 {
                self.status
                    .push_str(&format!("（已忽略 {} 个无效数字字段）", bad));
            }
        }
        // WSL 同步：仅勾选“WSL同步”且写入路径为本地时，同步到 WSL 侧默认路径；
        // 写入前检测对应 agent 是否已安装（未安装则跳过并提示）。
        if self.sync_wsl && ok && !is_wsl_path(&path) {
            match backends::wsl_target(fmt) {
                Some(wsl_path) => match self.save_backend_to(fmt, &wsl_path) {
                    Ok(_) => self
                        .status
                        .push_str(&format!("; {}(WSL): 已同步", fmt.label())),
                    Err(e) => {
                        self.status
                            .push_str(&format!("; {}(WSL): 同步失败({})", fmt.label(), e))
                    }
                },
                None => self
                    .status
                    .push_str(&format!("; {}(WSL): 未安装，跳过同步", fmt.label())),
            }
        }
    }

    /// 通用保存：按后端构造 root、渲染内容并写入。
    fn save_backend_to(&mut self, fmt: ConfigFormat, path: &str) -> Result<Option<String>, String> {
        let backend = backends::backend(fmt);
        // 已加载的当前文件整体替换；同格式的其他路径（WSL 同步等）先读后合并；
        // 跨格式目标（来源格式不同）做「干净转换」：目标文件里由组件状态接管的
        // provider / agent 容器整体丢弃（条目与顺序都来自界面），其余顶层字段保留。
        let is_current = self.source_format == fmt && path == self.loaded_path;
        let cross_format = self.source_format != fmt;
        let target_root: Option<Value> = if is_current {
            None
        } else {
            let mut target = backend.load_target_root(path);
            if cross_format {
                strip_cross_format_containers(fmt, &mut target, !self.agents.is_empty());
            }
            Some(target)
        };
        let root = backend.serialize_root(
            &self.agents,
            &self.providers,
            self.extras_for(fmt),
            target_root.as_ref(),
        );
        let content = if fmt == ConfigFormat::DeepSeekHarness && is_current {
            // 未发生任何结构化修改时直接保留原始 YAML，避免无意义的
            // 缩进、引号、键顺序变化；实际修改后再使用稳定的 DSH 渲染器。
            match util::read_config_content(path) {
                Ok(original)
                    if util::parse_yaml_content(&original).ok().as_ref() == Some(&root) =>
                {
                    original
                }
                _ => backend.render(&root, self.save_format == SaveFormat::Compact)?,
            }
        } else {
            backend.render(&root, self.save_format == SaveFormat::Compact)?
        };
        let dsh_sidecar_backup = if fmt == ConfigFormat::DeepSeekHarness {
            let sidecar = credentials::sidecar_path(path);
            if util::config_exists(&sidecar) {
                Some((sidecar.clone(), util::read_config_content(&sidecar)?))
            } else {
                Some((sidecar, String::new()))
            }
        } else {
            None
        };
        // 跨格式转换会整体接管目标文件的 provider/agent：先把原文件滚动备份为 .bak，
        // 备份失败则取消保存（宁可不让存，也不能把旧配置静默抵掉）。
        let backup = if cross_format && !is_current {
            match util::read_config_content(path) {
                Ok(old) if !old.is_empty() && old != content => {
                    let backup_path = format!("{}.bak", path);
                    backends::write_config(&backup_path, &old).map_err(|e| {
                        format!("跨格式转换前备份失败（{}），已取消保存: {}", backup_path, e)
                    })?;
                    Some(backup_path)
                }
                _ => None,
            }
        } else {
            None
        };
        if fmt == ConfigFormat::DeepSeekHarness {
            backend.save_sidecars(path, &self.providers)?;
        }
        if let Err(error) = backends::write_config(path, &content) {
            if let Some((sidecar, old_content)) = dsh_sidecar_backup {
                let restore = if old_content.is_empty() {
                    if util::config_exists(&sidecar) {
                        util::remove_config(&sidecar)
                    } else {
                        Ok(())
                    }
                } else {
                    backends::write_config(&sidecar, &old_content)
                };
                if let Err(restore_error) = restore {
                    return Err(format!("{}；凭据回滚失败: {}", error, restore_error));
                }
            }
            return Err(error);
        }
        // 当前文件保存成功后，回填 opencode 的 extras 载体（self.root）保持与磁盘一致
        if is_current && fmt == ConfigFormat::Opencode {
            self.root = root;
        }
        Ok(backup)
    }

    /// 当前文件保存时使用的基底 extras（按后端取对应载体）。
    fn extras_for(&self, fmt: ConfigFormat) -> &Value {
        match fmt {
            ConfigFormat::Opencode => &self.root,
            // pi 系（pi / oh-my-pi）共用 extras 载体：providers 之外的顶层字段
            ConfigFormat::Pi | ConfigFormat::OhMyPi => &self.pi_extras,
            ConfigFormat::DeepSeekHarness => &self.root,
        }
    }

    /// 预览面板左边缘的拖动分隔条：拖拽调整预览宽度比例（窗口缩放时按比例适配）。
    /// 用 Foreground 层的 Area 承载热区，避免被同层的文本框/滚动区抢走拖拽。
    fn ui_preview_resizer(&mut self, ctx: &egui::Context, panel_rect: egui::Rect, screen_w: f32) {
        let strip = egui::Rect::from_min_max(
            egui::pos2(panel_rect.left() - 4.0, panel_rect.top()),
            egui::pos2(panel_rect.left() + 4.0, panel_rect.bottom()),
        );
        let resp = egui::Area::new(egui::Id::new("preview_resizer"))
            .order(egui::Order::Foreground)
            .fixed_pos(strip.min)
            .show(ctx, |ui| {
                let (rect, resp) =
                    ui.allocate_exact_size(strip.size(), egui::Sense::click_and_drag());
                let active = resp.hovered() || resp.dragged();
                if active {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                let color = if active {
                    ui.visuals().widgets.hovered.bg_stroke.color
                } else {
                    ui.visuals().widgets.noninteractive.bg_stroke.color
                };
                ui.painter().vline(
                    rect.center().x,
                    rect.y_range(),
                    egui::Stroke::new(1.0, color),
                );
                resp
            })
            .inner;
        if resp.dragged() {
            // 分隔条向右拖 → 预览变窄 → 比例减小。
            let dx = resp.drag_delta().x;
            self.preview_ratio = (self.preview_ratio - dx / screen_w.max(1.0)).clamp(0.15, 0.85);
        }
    }

    /// 预览文本语法：opencode / pi 为 JSON(C)，omp / DSH 为 YAML；
    /// 另按内容首字符兜底（`{` / `[` 视为 JSON），避免格式与页面不匹配时高亮错乱。
    fn preview_syntax(&self, text: &str) -> PreviewSyntax {
        // 内容兜底：以 `{` / `[` 开头一律按 JSON 处理（例如误把 JSON 当 YAML 页面导入）。
        if matches!(
            text.trim_start().as_bytes().first(),
            Some(b'{') | Some(b'[')
        ) {
            return PreviewSyntax::Json;
        }
        // opencode: opencode.json(c) / pi: ~/.pi/agent/models.json（JSONC）
        // omp: models.yml / DSH: settings.yaml（YAML）
        match self.current_page {
            ConfigFormat::Opencode | ConfigFormat::Pi => PreviewSyntax::Json,
            ConfigFormat::OhMyPi | ConfigFormat::DeepSeekHarness => PreviewSyntax::Yaml,
        }
    }

    /// 重置预览编辑状态，让草稿在下一帧按当前组件状态重建。
    /// 切页/重新加载/打开预览时调用：草稿只在「预览未聚焦且上次解析成功」时
    /// 才跟随组件状态，否则会停留在上一页的内容上。
    fn reset_preview_draft(&mut self) {
        self.preview_focused = false;
        self.preview_parse_ok = true;
        self.preview_parse_error = None;
        self.preview_dirty_at = None;
        self.preview_edit_at = None;
    }

    /// 预览面板：右侧实时展示「待保存文档」（与保存按钮同路径、同合并语义）；
    /// 文本框始终可编辑：编辑内容实时解析并应用回组件，停止输入后自动写盘。
    fn ui_preview_panel(&mut self, ui: &mut egui::Ui) {
        let now = ui.ctx().input(|i| i.time);
        // 待保存文档：与 page_save_path / save_backend_to 相同路径与合并逻辑。
        let doc = self.preview_document();
        // 组件状态是「待保存文档」的唯一来源：只要用户没在预览框里手改（停止输入
        // 超过 PREVIEW_EDIT_IDLE_SECS）且上次解析没失败，就按组件状态重建草稿。
        // 不再依赖「预览是否持有焦点」—— 焦点残留或解析失败会让预览停在旧内容上，
        // 用户再动一下预览还会把旧内容解析回组件，导致保存写回旧配置。
        if preview_should_rebuild(
            self.preview_parse_error.is_some(),
            self.preview_edit_at,
            now,
        ) {
            if let Ok((_, text)) = &doc {
                if text != &self.preview_draft {
                    self.preview_draft = text.clone();
                }
            }
        }
        // 顶部：标题 + 行数/总行数（不显示路径）；格式报错直接排在行数右侧。
        let total_lines = self.preview_draft.chars().filter(|c| *c == '\n').count() + 1;
        let mut regenerate = false;
        ui.horizontal(|ui| {
            ui.strong("预览编辑");
            ui.label(
                egui::RichText::new(format!("{} / {} 行", self.preview_cursor_line, total_lines))
                    .small()
                    .weak(),
            )
            .on_hover_text("光标所在行 / 待保存文档总行数");
            if let Some(e) = &self.preview_parse_error {
                ui.colored_label(egui::Color32::from_rgb(255, 120, 120), "⚠ 格式错误");
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(e)
                            .small()
                            .color(egui::Color32::from_rgb(235, 170, 170)),
                    )
                    .wrap(),
                )
                .on_hover_text("继续编辑修正，或点「重新生成」/ 切走再切回以撤销文本修改");
                regenerate = ui
                    .button("重新生成")
                    .on_hover_text("放弃预览里的修改，按左侧组件状态重新生成待保存文档")
                    .clicked();
            } else if let Err(e) = &doc {
                ui.colored_label(egui::Color32::from_rgb(220, 90, 90), "生成失败")
                    .on_hover_text(e);
            }
        });
        if regenerate {
            self.reset_preview_draft();
        }
        ui.separator();
        // Ctrl+F：激活查找（读原始按键事件，避免被文本框消耗）。
        let ctrl_f = ui.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key {
                        key: egui::Key::F,
                        pressed: true,
                        modifiers,
                        ..
                    } if modifiers.command
                )
            })
        });
        if ctrl_f {
            self.preview_find_active = true;
            self.preview_find_focus = true;
        }
        // 查找栏：Enter 下一个 / Shift+Enter 上一个 / Esc 关闭。
        if self.preview_find_active {
            ui.horizontal(|ui| {
                let resp =
                    ui.add(egui::TextEdit::singleline(&mut self.preview_find).desired_width(150.0));
                if self.preview_find_focus {
                    resp.request_focus();
                    self.preview_find_focus = false;
                }
                let matches = find_matches(&self.preview_draft, &self.preview_find);
                let total = matches.len();
                if total == 0 {
                    ui.colored_label(egui::Color32::from_rgb(220, 90, 90), "0 处");
                } else {
                    if self.preview_find_index >= total {
                        self.preview_find_index = 0;
                    }
                    ui.label(
                        egui::RichText::new(format!("{} / {}", self.preview_find_index + 1, total))
                            .small()
                            .weak(),
                    );
                }
                let mut step: i32 = 0;
                if ui.button("⬆").clicked() {
                    step = -1;
                }
                if ui.button("⬇").clicked() {
                    step = 1;
                }
                if ui.button("×").clicked() {
                    self.preview_find_active = false;
                    self.preview_find.clear();
                    self.preview_find_index = 0;
                    self.preview_find_jump = None;
                }
                if resp.has_focus() {
                    let keys = ui.input(|i| {
                        (
                            i.key_pressed(egui::Key::Enter),
                            i.key_pressed(egui::Key::Escape),
                            i.modifiers.shift,
                        )
                    });
                    if keys.0 {
                        step = if keys.2 { -1 } else { 1 };
                    }
                    if keys.1 {
                        self.preview_find_active = false;
                        self.preview_find.clear();
                        self.preview_find_index = 0;
                        self.preview_find_jump = None;
                    }
                }
                if step != 0 && total > 0 {
                    self.preview_find_index =
                        (self.preview_find_index as i32 + step).rem_euclid(total as i32) as usize;
                    if let Some((start, _)) = matches.get(self.preview_find_index) {
                        self.preview_find_jump = Some(*start);
                    }
                }
            });
        }
        // 文本框：常规自上而下布局的最后一个元素，占满剩余高度，
        // 滚轮/滚动条均正常（用 bottom_up 会把滚动错位到底部）。
        let text_width = (ui.available_width() - 14.0).max(120.0);
        let mut edited = false;
        let mut cursor_line: Option<usize> = None;
        // 查找高亮：命中段加底色，当前命中用更亮的底色。
        let find_query = self.preview_find.clone();
        let find_active = self.preview_find_active && !find_query.is_empty();
        let find_matches = if find_active {
            find_matches(&self.preview_draft, &find_query)
        } else {
            Vec::new()
        };
        let find_current = self
            .preview_find_index
            .min(find_matches.len().saturating_sub(1));
        let find_jump = self.preview_find_jump.take();
        let syntax = self.preview_syntax(&self.preview_draft);
        let mut layouter = move |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
            let text = text.as_str();
            let font_id = egui::TextStyle::Monospace.resolve(ui.style());
            let mut job = egui::text::LayoutJob::default();
            // 自适应换行：使用 TextEdit 传入的换行宽度，长行不再溢出面板。
            job.wrap.max_width = wrap_width;
            let base = ui.visuals().text_color();
            let push = |job: &mut egui::text::LayoutJob, seg: &str, color: egui::Color32| {
                job.append(
                    seg,
                    0.0,
                    egui::TextFormat {
                        font_id: font_id.clone(),
                        color,
                        ..Default::default()
                    },
                );
            };
            // 1) 语法着色（VSCode Dark+ 配色，opencode=JSON / 其余=YAML）
            let mut pos = 0;
            for (start, end, color) in syntax_tokens(text, syntax) {
                if start > pos {
                    push(&mut job, &text[pos..start], base);
                }
                if end > start {
                    push(&mut job, &text[start..end], color);
                }
                pos = pos.max(end);
            }
            if pos < text.len() {
                push(&mut job, &text[pos..], base);
            }
            // 2) 查找命中底色叠加在语法色之上
            if !find_matches.is_empty() {
                apply_find_background(&mut job, &find_matches, find_current);
            }
            ui.painter().layout_job(job)
        };
        egui::ScrollArea::vertical()
            .id_salt("preview_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let edit = egui::TextEdit::multiline(&mut self.preview_draft)
                    .id(egui::Id::new(PREVIEW_EDITOR_ID))
                    .font(egui::TextStyle::Monospace)
                    .code_editor()
                    .desired_width(text_width)
                    .desired_rows(24)
                    .hint_text("在此直接编辑：改动实时应用到左侧组件，停止输入约 0.8s 后自动保存")
                    .layouter(&mut layouter);
                // 用 show 而非 add：需要 output.cursor_range 计算光标所在行。
                let output = edit.show(ui);
                let resp = output.response;
                // 查找命中跳转：把光标移到命中处并写回状态，滚动区随之滚动。
                if let Some(byte) = find_jump {
                    let bounded = byte.min(self.preview_draft.len());
                    let char_idx = self.preview_draft[..bounded].chars().count();
                    let ccursor = egui::text::CCursor::new(char_idx);
                    let mut state = output.state.clone();
                    state
                        .cursor
                        .set_char_range(Some(egui::text_selection::CCursorRange::two(
                            ccursor, ccursor,
                        )));
                    state.store(ui.ctx(), resp.id);
                    // 主动把命中位置滚入视野：egui 只在文本框内容变化时自动滚动到光标，
                    // 通过按钮/Enter 跳转时光标是外部设置的，需要自己请求滚动。
                    let rect = output
                        .galley
                        .pos_from_cursor(ccursor)
                        .translate(output.galley_pos.to_vec2());
                    ui.scroll_to_rect(rect, Some(egui::Align::Center));
                }
                self.preview_focused = resp.has_focus();
                if let Some(range) = output.cursor_range {
                    let idx = range.primary.index;
                    let line = self
                        .preview_draft
                        .chars()
                        .take(idx)
                        .filter(|c| *c == '\n')
                        .count()
                        + 1;
                    cursor_line = Some(line);
                }
                if resp.changed() && self.preview_focused {
                    edited = true;
                }
            });
        if let Some(line) = cursor_line {
            self.preview_cursor_line = line;
        }
        // 编辑 → 实时解析并应用回组件状态（解析失败不写盘、不覆盖）。
        if edited {
            self.apply_preview_draft();
            self.preview_dirty_at = Some(now);
            self.preview_edit_at = Some(now);
        }
        // 防抖自动保存：解析成功且停止输入 0.8s 后写盘。
        if let Some(at) = self.preview_dirty_at {
            if now - at > 0.8 {
                self.preview_dirty_at = None;
                self.preview_autosave();
            }
        }
    }

    /// 生成当前页面「待保存文档」：目标路径 + 序列化内容。
    /// 与保存一致：非当前文件目标先读目标文件并按目标格式合并（upsert）。
    fn preview_document(&self) -> Result<(String, String), String> {
        let fmt = self.current_page;
        let backend = backends::backend(fmt);
        let target = self.page_save_path(fmt);
        let path = match &target {
            PageTarget::Current(p) | PageTarget::Modified(p) | PageTarget::Default(p) => p.clone(),
        };
        let is_current = self.source_format == fmt && path == self.loaded_path;
        // 与 save_backend_to 同一套语义：跨格式目标做干净转换（provider 容器由界面接管），
        // 预览显示的内容就是保存将要写出的内容。
        let target_root = if is_current {
            None
        } else {
            let mut target = backend.load_target_root(&path);
            if self.source_format != fmt {
                strip_cross_format_containers(fmt, &mut target, !self.agents.is_empty());
            }
            Some(target)
        };
        let root = backend.serialize_root(
            &self.agents,
            &self.providers,
            self.extras_for(fmt),
            target_root.as_ref(),
        );
        let content = backend.render(&root, self.save_format == SaveFormat::Compact)?;
        Ok((path, content))
    }

    /// 把预览编辑内容解析并写回左侧组件状态；成功返回 true。
    /// 仅更新内存状态，不落盘（落盘由自动保存/立即保存负责）。
    fn apply_preview_draft(&mut self) -> bool {
        let fmt = self.current_page;
        let content = self.preview_draft.clone();
        match backends::backend(fmt).parse_at(&content, &self.config_path) {
            Ok(load) => {
                self.root = load.root;
                self.agents = load.agents;
                self.providers = load.providers;
                self.pi_extras = load.extras;
                self.load_error = None;
                self.source_format = fmt;
                self.preview_parse_ok = true;
                self.preview_parse_error = None;
                self.agent_open = self.agents.iter().map(|a| a.key.clone()).collect();
                self.provider_open = self.providers.iter().map(|p| p.key.clone()).collect();
                self.model_fetch.clear();
                self.model_fetch_open.clear();
                self.latency.clear();
                self.probe.release(None);
                true
            }
            Err(e) => {
                self.preview_parse_ok = false;
                self.preview_parse_error = Some(e.clone());
                self.status = format!("预览内容解析失败：{}（继续编辑或撤销）", e);
                false
            }
        }
    }

    /// 实时保存：把当前待保存文档写入目标文件（仅本地，不触发 WSL 同步；
    /// 解析失败、目标不可用时跳过并提示，绝不写坏文件）。
    fn preview_autosave(&mut self) {
        if !self.preview_parse_ok {
            self.status = "预览内容解析失败，未保存（修正文本后会自动保存）".into();
            return;
        }
        let fmt = self.current_page;
        let target = self.page_save_path(fmt);
        let path = match &target {
            PageTarget::Current(p) | PageTarget::Modified(p) | PageTarget::Default(p) => p.clone(),
        };
        let usable = match &target {
            PageTarget::Default(_) => self.targets.iter().any(|t| t.backend == fmt && t.available),
            _ => true,
        };
        if !usable {
            self.status = format!(
                "{}: 目标不可用（{}），未实时保存——请用保存按钮",
                fmt.label(),
                path
            );
            return;
        }
        match self.save_backend_to(fmt, &path) {
            Ok(backup) => {
                self.status = match backup {
                    Some(backup) => format!(
                        "{}: 已实时保存（跨格式转换，原文件已备份为 {}）",
                        fmt.label(),
                        backup
                    ),
                    None => format!("{}: 已实时保存", fmt.label()),
                }
            }
            Err(e) => self.status = format!("{}: 实时保存失败({})", fmt.label(), e),
        }
    }

    /// 按格式取官方图标纹理（图标未加载时返回 None）。
    fn icon_for(&self, fmt: ConfigFormat) -> Option<&egui::TextureHandle> {
        let idx = backends::BACKENDS.iter().position(|b| b.id() == fmt)?;
        self.backend_icons.get(idx).and_then(|o| o.as_ref())
    }
}

/// 预览查找：大小写不敏感的字符级匹配，返回不重叠的字节区间。
fn preview_should_rebuild(parse_failed: bool, edited_at: Option<f64>, now: f64) -> bool {
    if parse_failed {
        return false;
    }
    edited_at.is_none_or(|at| now - at >= PREVIEW_EDIT_IDLE_SECS)
}

fn find_matches(text: &str, query: &str) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    let needle: Vec<char> = query.to_lowercase().chars().collect();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    if needle.is_empty() || chars.len() < needle.len() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i + needle.len() <= chars.len() {
        let matched = needle
            .iter()
            .enumerate()
            .all(|(k, qc)| chars[i + k].1.to_lowercase().next() == Some(*qc));
        if matched {
            let start = chars[i].0;
            let last = chars[i + needle.len() - 1];
            out.push((start, last.0 + last.1.len_utf8()));
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

// ---------- 配置加载 ----------

/// 读取配置文件内容；本地与 WSL 路径统一处理，文件不存在视为新建场景返回空串。
// —— 兼容再导出：实现迁移至 util / backends，保持既有测试路径可用 ——
pub use crate::backends::opencode::merge_opencode_root;
pub use crate::util::parse_config_content;

/// 跨格式转换前，从目标 root 中剔除由当前组件状态接管的容器：
/// provider（opencode 的 provider / pi 系与 DSH 的 providers）与 opencode 的 agent。
///
/// 目的：跨格式保存/预览时，provider 条目与顺序完全以界面为准（干净转换），
/// 同时目标文件的其他顶层字段（如 DSH 的 llm-pi-ai 下其他设置）原样保留。
/// 同格式目标（WSL 同步等）不走这里，仍用保守合并。
///
/// `agents_owned` 表示界面确实持有 agents 数据。agents 只属于 opencode 页：
/// 数据来自 pi / oh-my-pi / DSH（或空载启动）时界面无从表达 agents，
/// 此时必须保留目标文件里的 agent 容器，否则会把它们静默删掉。
pub fn strip_cross_format_containers(fmt: ConfigFormat, root: &mut Value, agents_owned: bool) {
    let Some(obj) = root.as_object_mut() else {
        return;
    };
    match fmt {
        ConfigFormat::Opencode => {
            obj.remove("provider");
            if agents_owned {
                obj.remove("agent");
            }
        }
        ConfigFormat::Pi | ConfigFormat::OhMyPi => {
            obj.remove("providers");
        }
        ConfigFormat::DeepSeekHarness => {
            if let Some(llm) = obj.get_mut("llm-pi-ai").and_then(Value::as_object_mut) {
                llm.remove("providers");
            }
        }
    }
}

/// 加载 opencode 配置；读取/解析失败返回 Err。
pub fn load_opencode_result(
    path: &str,
) -> Result<(Value, Vec<AgentRow>, Vec<ProviderRow>), String> {
    let load = crate::backends::load_backend(ConfigFormat::Opencode, path)?;
    Ok((load.root, load.agents, load.providers))
}

/// 兼容包装：失败时回退空状态（供测试与旧调用方使用）。
pub fn load_or_empty(path: &str) -> (Value, Vec<AgentRow>, Vec<ProviderRow>) {
    load_opencode_result(path)
        .unwrap_or_else(|_| (Value::Object(Map::new()), Vec::new(), Vec::new()))
}

/// 加载 pi 配置（支持本地与 WSL 路径）；读取/解析失败返回 Err。
pub fn load_pi_result(path: &str) -> Result<(Value, Vec<ProviderRow>, Value), String> {
    let load = crate::backends::load_backend(ConfigFormat::Pi, path)?;
    Ok((load.root, load.providers, load.extras))
}

#[cfg(test)]
mod compact_tests {
    use super::*;

    #[test]
    fn pretty_agent_fields_combine_on_same_line() {
        let root = serde_json::json!({
            "agent": {
                "writing": {
                    "mode": "subagent",
                    "description": "写技术文档",
                    "model": "sensenova/sensenova-6.8-flash-lite",
                    "variant": "high",
                    "temperature": 0.3,
                    "color": "info",
                    "system": "负责文档"
                }
            }
        });
        let output = pretty_json(&root);
        println!("=== PRETTY OUTPUT ===\n{}", output);
        // model, variant, temperature, color should be on the same line
        assert!(
            output.contains(
                "\"model\": \"sensenova/sensenova-6.8-flash-lite\", \"variant\": \"high\","
            ),
            "agent model+variant should combine:\n{}",
            output
        );
        assert!(
            output.contains("\"temperature\": 0.3, \"color\": \"info\", \"system\": \"负责文档\""),
            "agent temperature+color+system should combine:\n{}",
            output
        );
    }

    #[test]
    fn compact_variants_preserve_values() {
        let value = serde_json::json!({
            "variants": {
                "medium": { "reasoningEffort": "medium" },
                "high": { "reasoningEffort": "high" }
            }
        });
        let output = serialize_object(value.as_object().unwrap(), 1, CompactRole::Target);
        assert!(output.contains(
            "\"variants\": { \"medium\": {\"reasoningEffort\":\"medium\"}, \"high\": {\"reasoningEffort\":\"high\"} }"
        ));
    }

    #[test]
    fn pi_models_array_expands_to_multiple_lines() {
        let root = serde_json::json!({
            "providers": {
                "openai": {
                    "api": "openai-completions",
                    "models": [
                        { "id": "gpt-4o", "name": "GPT-4o" },
                        { "id": "gpt-4o-mini", "name": "GPT-4o mini" }
                    ]
                }
            }
        });
        let pretty = pretty_json(&root);
        assert!(pretty.contains("\"models\": [\n"));
        assert!(pretty.contains("\"id\": \"gpt-4o\", \"name\": \"GPT-4o\""));
        assert!(pretty.contains("\"id\": \"gpt-4o-mini\", \"name\": \"GPT-4o mini\""));

        let compact = compact_json(&root);
        assert!(compact.contains("\"models\": [\n"));
        assert!(compact.contains("\"id\": \"gpt-4o\", \"name\": \"GPT-4o\""));
        assert!(compact.contains("\"id\": \"gpt-4o-mini\", \"name\": \"GPT-4o mini\""));
    }

    #[test]
    fn compact_targets_agent_fields_only() {
        let root = serde_json::json!({
            "agent": { "writer": { "mode": "subagent", "description": "text", "model": "p/m" } },
            "other": { "a": 1, "b": 2 }
        });
        let output = compact_json(&root);
        assert!(output.contains("\"writer\": {"));
        assert!(output.contains("\"mode\": \"subagent\", \"description\": \"text\""));
        assert!(output.contains("\"other\": {\n    \"a\": 1,"));
    }

    #[test]
    fn compact_targets_provider_model_fields() {
        let root = serde_json::json!({
            "provider": {
                "p": {
                    "models": {
                        "m": {
                            "name": "Model",
                            "reasoning": true,
                            "variants": { "medium": {}, "high": {} }
                        }
                    }
                }
            }
        });
        let output = compact_json(&root);
        assert!(output.contains("\"models\": {\"m\":{"));
        assert!(output.contains("\"name\":\"Model\",\"reasoning\":true"));
        assert!(output.contains("\"variants\":{\"medium\":{},\"high\":{}}"));
    }

    #[test]
    fn compact_wraps_mcp_and_options_from_second_level() {
        let root = serde_json::json!({
            "mcp": {
                "server": {
                    "command": "node",
                    "args": ["server.js"],
                    "enabled": true
                }
            },
            "provider": {
                "p": {
                    "options": {
                        "baseURL": "https://example.com/v1",
                        "apiKey": "sk-test",
                        "timeout": 30000
                    }
                }
            }
        });
        let output = compact_json(&root);

        assert!(output.contains(
            "\"server\": {\"command\":\"node\",\"args\":[\"server.js\"],\"enabled\":true"
        ));
        assert!(output.contains("\"options\": {\"baseURL\":\"https://example.com/v1\",\"apiKey\":\"sk-test\",\"timeout\":30000"));
    }

    #[test]
    fn default_format_keeps_nested_leaf_objects_on_one_line() {
        let root = serde_json::json!({
            "provider": {
                "p": {
                    "options": { "baseURL": "https://example.com/v1", "timeout": 30000 }
                }
            }
        });

        let output = compact_json(&root);

        assert!(output
            .contains("\"options\": {\"baseURL\":\"https://example.com/v1\",\"timeout\":30000}"));
        assert!(!output.contains("\"options\": {\n"));
    }

    #[test]
    fn save_serializers_preserve_variant_settings() {
        let root = serde_json::json!({
            "provider": {
                "p": {
                    "models": {
                        "m": {
                            "variants": { "high": { "reasoningEffort": "high" } }
                        }
                    }
                }
            }
        });

        assert!(compact_json(&root).contains("reasoningEffort"));
        assert!(compact_json(&root).contains("\"high\":{\"reasoningEffort\":\"high\"}"));
    }
}

#[cfg(test)]
mod model_fetch_tests {
    use super::{
        chat_url, fetch_grid_columns, parse_models_response, sanitize_network_error, App,
        FETCH_GRID_GAP_X,
    };

    #[test]
    fn fetch_grid_columns_never_exceed_available_width() {
        // 总宽 = 列数 * 列宽 + 间距 * (列数 - 1)，任何宽度下都不得超过可用宽度。
        let total = |(cols, col_w): (usize, f32)| {
            cols as f32 * col_w + FETCH_GRID_GAP_X * (cols - 1) as f32
        };
        // 宽窗口：取满 5 列并把宽度均分（5 * 217.6 + 4 * 28 = 1200）。
        let wide = fetch_grid_columns(50, 1200.0);
        assert_eq!(wide.0, 5);
        assert!((wide.1 - 217.6).abs() < 0.01);
        assert!(total(wide) <= 1200.5);
        // 中等宽度：列数随可用宽度下降，仍恰好铺满。
        let mid = fetch_grid_columns(50, 400.0);
        assert_eq!(mid.0, 2);
        assert!(total(mid) <= 400.5);
        // 极窄窗口：退化为单列，宽度不超过可用宽度。
        let narrow = fetch_grid_columns(50, 120.0);
        assert_eq!(narrow.0, 1);
        assert!(narrow.1 <= 120.0);
        // 模型很少时不空出多余列。
        assert_eq!(fetch_grid_columns(3, 1200.0).0, 3);
        // 空列表（防御性）不 panic，也不返回 0 列。
        assert_eq!(fetch_grid_columns(0, 300.0).0, 1);
    }

    #[test]
    fn parse_openai_style_models() {
        let text = r#"{"object":"list","data":[{"id":"gpt-4o","object":"model"},{"id":"gpt-4o-mini","object":"model"}]}"#;
        let ids = parse_models_response(text).unwrap();
        assert_eq!(ids, vec!["gpt-4o", "gpt-4o-mini"]);
    }

    #[test]
    fn parse_anthropic_style_models() {
        let text = r#"{"data":[{"type":"model","id":"claude-3-7-sonnet-20250219"},{"type":"model","id":"claude-sonnet-4-20250514"}]}"#;
        let ids = parse_models_response(text).unwrap();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"claude-sonnet-4-20250514".to_string()));
    }

    #[test]
    fn parse_gemini_style_models() {
        let text =
            r#"{"models":[{"name":"models/gemini-2.0-flash"},{"name":"models/gemini-2.5-pro"}]}"#;
        let ids = parse_models_response(text).unwrap();
        assert_eq!(ids, vec!["gemini-2.0-flash", "gemini-2.5-pro"]);
    }

    #[test]
    fn parse_error_message() {
        let text = r#"{"error":{"message":"Invalid API key"}}"#;
        let err = parse_models_response(text).unwrap_err();
        assert!(err.contains("Invalid API key"));
    }

    #[test]
    fn parse_dedupes_ids_and_ignores_missing() {
        let text = r#"{"data":[{"id":"a"},{"id":"a"},{"name":"b"},{"foo":"c"}]}"#;
        let ids = parse_models_response(text).unwrap();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn models_url_openai_variants() {
        assert_eq!(
            App::models_url("https://api.openai.com/v1", "openai-completions"),
            "https://api.openai.com/v1/models"
        );
        assert_eq!(
            App::models_url("https://api.openai.com/v1/", "openai-completions"),
            "https://api.openai.com/v1/models"
        );
        assert_eq!(
            App::models_url("https://gw.example.com", "openai-completions"),
            "https://gw.example.com/models"
        );
    }

    #[test]
    fn models_url_anthropic_uses_v1() {
        assert_eq!(
            App::models_url("https://api.anthropic.com", "anthropic-messages"),
            "https://api.anthropic.com/v1/models"
        );
        assert_eq!(
            App::models_url("https://api.anthropic.com/v1", "anthropic-messages"),
            "https://api.anthropic.com/v1/models"
        );
    }

    #[test]
    fn chat_url_openai_and_anthropic() {
        assert_eq!(
            chat_url(
                "https://api.openai.com/v1",
                "openai-completions",
                "gpt-4o",
                true
            ),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            chat_url(
                "https://api.anthropic.com",
                "anthropic-messages",
                "claude-sonnet-4",
                true
            ),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn chat_url_streaming_only_changes_google() {
        // 除 Google 系外，流式与非流式是同一个端点（靠请求体里的 stream 字段区分）
        assert_eq!(
            chat_url(
                "https://gw.example.com/v1",
                "openai-completions",
                "gpt-4o",
                false
            ),
            chat_url(
                "https://gw.example.com/v1",
                "openai-completions",
                "gpt-4o",
                true
            )
        );
        // Google 系要换动词（streamGenerateContent）并加 SSE 查询参数
        assert_eq!(
            chat_url(
                "https://generativelanguage.googleapis.com/v1beta",
                "google-generative-ai",
                "gemini-2.5-pro",
                true
            ),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:streamGenerateContent?alt=sse"
        );
        assert_eq!(
            chat_url(
                "https://us-central1-aiplatform.googleapis.com/v1",
                "google-vertex",
                "gemini-2.5-pro",
                true
            ),
            "https://us-central1-aiplatform.googleapis.com/v1/publishers/google/models/gemini-2.5-pro:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn query_key_appends_after_existing_query() {
        use super::{with_query_key, AuthKind};
        assert_eq!(
            with_query_key("https://gw.example.com/models", AuthKind::QueryKey, "sk-1"),
            "https://gw.example.com/models?key=sk-1"
        );
        // Google 流式端点已经带了 ?alt=sse，密钥要用 & 接着拼
        assert_eq!(
            with_query_key(
                "https://gw.example.com/models/m:streamGenerateContent?alt=sse",
                AuthKind::QueryKey,
                "sk-1"
            ),
            "https://gw.example.com/models/m:streamGenerateContent?alt=sse&key=sk-1"
        );
    }

    #[test]
    fn chat_url_follows_selected_protocol() {
        let base = "https://gw.example.com/v1";
        // Responses 系走 /responses（含 Azure）
        assert_eq!(
            chat_url(base, "openai-responses", "gpt-4o", false),
            "https://gw.example.com/v1/responses"
        );
        assert_eq!(
            chat_url(base, "azure-openai-responses", "gpt-4o", false),
            "https://gw.example.com/v1/responses"
        );
        // Mistral 会话协议仍是 chat/completions
        assert_eq!(
            chat_url(base, "mistral-conversations", "mistral-large", false),
            "https://gw.example.com/v1/chat/completions"
        );
        // pi 自己的协议是 /messages（不带 v1 前缀时也直接用 base）
        assert_eq!(
            chat_url(base, "pi-messages", "some-model", false),
            "https://gw.example.com/v1/messages"
        );
        // Google 系需要模型名参与路径
        assert_eq!(
            chat_url(
                "https://generativelanguage.googleapis.com/v1beta",
                "google-generative-ai",
                "gemini-2.5-pro",
                false
            ),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:generateContent"
        );
        assert_eq!(
            chat_url(
                "https://us-central1-aiplatform.googleapis.com/v1",
                "google-vertex",
                "gemini-2.5-pro",
                false
            ),
            "https://us-central1-aiplatform.googleapis.com/v1/publishers/google/models/gemini-2.5-pro:generateContent"
        );
    }

    #[test]
    fn api_wire_classifies_and_flags_unsupported() {
        use super::{api_wire, auth_kind, unsupported_reason, ApiWire, AuthKind};
        // 未知值与空值都归到兼容层，不会漏掉协议分支
        assert_eq!(api_wire("openai-completions"), ApiWire::ChatCompletions);
        assert_eq!(api_wire("unknown-api"), ApiWire::ChatCompletions);
        assert_eq!(api_wire(""), ApiWire::ChatCompletions);
        assert_eq!(api_wire("openai-codex-responses"), ApiWire::Responses);
        assert_eq!(api_wire("anthropic-messages"), ApiWire::AnthropicMessages);
        assert_eq!(api_wire("pi-messages"), ApiWire::PiMessages);
        // 鉴权方式随协议变化
        assert_eq!(
            auth_kind(api_wire("anthropic-messages")),
            AuthKind::AnthropicKey
        );
        assert_eq!(
            auth_kind(api_wire("azure-openai-responses")),
            AuthKind::AzureKey
        );
        assert_eq!(
            auth_kind(api_wire("google-generative-ai")),
            AuthKind::QueryKey
        );
        assert_eq!(auth_kind(api_wire("openai-responses")), AuthKind::Bearer);
        // 需要专有鉴权的协议提前报错，不发无意义的请求
        assert!(unsupported_reason("google-gemini-cli").is_some());
        assert!(unsupported_reason("bedrock-converse-stream").is_some());
        assert!(unsupported_reason("openai-completions").is_none());
    }

    #[test]
    fn minimal_body_matches_protocol() {
        use super::{api_wire, minimal_body};
        let question = "世界上最长的河流是哪条？只回答河名";
        let chat = minimal_body(api_wire("openai-completions"), "m", question);
        assert_eq!(chat["messages"][0]["content"], question);
        assert_eq!(chat["max_tokens"], 16);
        let resp = minimal_body(api_wire("openai-responses"), "m", question);
        assert_eq!(resp["input"], question);
        assert_eq!(resp["max_output_tokens"], 16);
        let anthropic = minimal_body(api_wire("anthropic-messages"), "m", question);
        assert_eq!(anthropic["messages"][0]["content"], question);
        let google = minimal_body(api_wire("google-generative-ai"), "m", question);
        assert_eq!(google["contents"][0]["parts"][0]["text"], question);
        assert_eq!(google["generationConfig"]["maxOutputTokens"], 16);
        // 全部走流式（Google 除外：它的流式靠换端点，请求体里没有 stream 字段）
        assert_eq!(chat["stream"], true);
        assert_eq!(resp["stream"], true);
        assert_eq!(anthropic["stream"], true);
        assert!(google.get("stream").is_none());
        // 字段名不得互相串用（Responses 没有 messages，Google 没有 model）
        assert!(resp.get("messages").is_none());
        assert!(google.get("model").is_none());
    }

    #[test]
    fn probe_user_agent_matches_whitelisted_clients() {
        use super::{api_wire, probe_user_agent};
        // Anthropic 系用 Claude Code 的身份，其余用 opencode 的身份
        // （中转站只放行这两种；ureq 默认 UA / 无 UA / pi 的 UA 都会 401）
        assert!(probe_user_agent(api_wire("anthropic-messages")).starts_with("claude-cli/"));
        assert!(probe_user_agent(api_wire("pi-messages")).starts_with("claude-cli/"));
        assert!(probe_user_agent(api_wire("openai-completions")).starts_with("opencode/"));
        assert!(probe_user_agent(api_wire("openai-responses")).starts_with("opencode/"));
        assert!(probe_user_agent(api_wire("google-generative-ai")).starts_with("opencode/"));
    }

    #[test]
    fn chunk_content_detection_per_protocol() {
        use super::{api_wire, chunk_has_content};
        let parse = |text: &str| match serde_json::from_str::<serde_json::Value>(text) {
            Ok(value) => value,
            Err(err) => panic!("测试用例不是合法 JSON：{err}"),
        };
        // Chat Completions：role 块不算出字，content / reasoning_content 才算
        let chat = api_wire("openai-completions");
        assert!(!chunk_has_content(
            chat,
            &parse(r#"{"choices":[{"delta":{"role":"assistant"}}]}"#)
        ));
        assert!(chunk_has_content(
            chat,
            &parse(r#"{"choices":[{"delta":{"content":"尼罗河"}}]}"#)
        ));
        assert!(chunk_has_content(
            chat,
            &parse(r#"{"choices":[{"delta":{"reasoning_content":"嗯"}}]}"#)
        ));
        // 内容块数组形态也要认（部分中转站回 parts）
        assert!(chunk_has_content(
            chat,
            &parse(r#"{"choices":[{"delta":{"content":[{"text":"你"}]}}]}"#)
        ));
        // Responses：只有 *.delta 事件且带 delta 文本才算
        let responses = api_wire("openai-responses");
        assert!(!chunk_has_content(
            responses,
            &parse(r#"{"type":"response.created"}"#)
        ));
        assert!(chunk_has_content(
            responses,
            &parse(r#"{"type":"response.output_text.delta","delta":"你"}"#)
        ));
        // Anthropic：content_block_delta 里的 text / thinking
        let anthropic = api_wire("anthropic-messages");
        assert!(!chunk_has_content(
            anthropic,
            &parse(r#"{"type":"message_start","message":{"id":"x"}}"#)
        ));
        assert!(chunk_has_content(
            anthropic,
            &parse(r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"你"}}"#)
        ));
        // Google：candidates[0].content.parts[*].text
        let google = api_wire("google-generative-ai");
        assert!(!chunk_has_content(
            google,
            &parse(r#"{"candidates":[{"content":{"parts":[{"text":""}]}}]}"#)
        ));
        assert!(chunk_has_content(
            google,
            &parse(r#"{"candidates":[{"content":{"parts":[{"text":"尼罗河"}]}}]}"#)
        ));
    }

    #[test]
    fn stream_ttft_reads_first_content_and_drains() {
        use super::{api_wire, read_stream_ttft};
        let started = std::time::Instant::now();
        // role 块在前：首字必须落在 content 块上，且读完流后不能报错
        let sse = concat!(
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"尼罗河\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let ttft = read_stream_ttft(
            std::io::Cursor::new(sse.as_bytes()),
            api_wire("openai-completions"),
            started,
        );
        assert!(matches!(ttft, Some(ms) if ms < 1_000));
        // 只有元事件（没有任何内容）：退回第一个数据包的时刻，不返回 None
        let meta = "data: {\"type\":\"response.created\"}\n\n";
        let fallback = read_stream_ttft(
            std::io::Cursor::new(meta.as_bytes()),
            api_wire("openai-responses"),
            started,
        );
        assert!(fallback.is_some());
        // 空流（连数据包都没有）才算失败
        let empty: &[u8] = b"";
        assert!(read_stream_ttft(
            std::io::Cursor::new(empty),
            api_wire("openai-completions"),
            started
        )
        .is_none());
    }

    #[test]
    fn stream_end_markers_are_recognized() {
        use super::is_stream_end;
        assert!(is_stream_end("data: [DONE]"));
        assert!(is_stream_end("event: message_stop"));
        assert!(is_stream_end("data: {\"type\":\"response.completed\"}"));
        // 普通内容块不能被误判成结束
        assert!(!is_stream_end(
            "data: {\"choices\":[{\"delta\":{\"content\":\"尼罗河\"}}]}"
        ));
        // content_block_stop 里含 stop，但不是结束事件
        assert!(!is_stream_end(
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\"}"
        ));
    }

    #[test]
    fn probe_gate_throttles_per_provider() {
        use super::{ProbeGate, ProbeGateState, PROBE_PROVIDER_GAP_S};
        let mut gate = ProbeGate::default();
        assert_eq!(gate.state("a", 100.0, None), ProbeGateState::Ready);
        gate.start("a", 100.0);
        // 同一 provider 在飞期间 Busy（探测最长 10s，可能超过 5s 间隔，
        // 否则会同时向同一中转站发两个请求）
        assert_eq!(gate.state("a", 101.0, None), ProbeGateState::Busy);
        // 不同 provider 互不牵连：另一个厂商立即可测（可并行）
        assert_eq!(gate.state("b", 100.0, None), ProbeGateState::Ready);
        gate.start("b", 100.0);
        gate.finish("a");
        // 同一 provider 的任意两次探测（同模型 / 不同模型都算）统一间隔 5 秒
        match gate.state("a", 102.0, None) {
            ProbeGateState::Cooling(left) => {
                assert!(left > 0.0 && left <= PROBE_PROVIDER_GAP_S)
            }
            other => panic!("应为 Cooling，实际 {other:?}"),
        }
        assert_eq!(gate.state("a", 105.0, None), ProbeGateState::Ready);
        // b 仍在飞，不影响 a
        assert_eq!(gate.state("b", 106.0, None), ProbeGateState::Busy);
        gate.finish("b");
        // 网络守卫优先级最高：即使冷却结束也不允许探测
        assert!(matches!(
            gate.state("a", 200.0, Some("检测到系统代理")),
            ProbeGateState::NetBlocked(_)
        ));
        // 释放：重载 / 关闭表单时不能把某个 provider 永久卡在 Busy
        gate.start("c", 300.0);
        gate.release(Some("a"));
        assert_eq!(gate.state("c", 400.0, None), ProbeGateState::Busy);
        gate.release(None);
        assert_eq!(gate.state("c", 400.0, None), ProbeGateState::Ready);
    }

    #[test]
    fn probe_questions_rotate_per_provider() {
        use super::{probe_question, ProbeGate, PROBE_QUESTIONS};
        // 同一 provider 连续取题不重复（避免每次都问同一句）
        let mut gate = ProbeGate::default();
        let first = gate.next_question("relay-a");
        let second = gate.next_question("relay-a");
        assert_ne!(first, second);
        // 不同 provider 的起始偏移不同（同一游标下题目不同）
        let other = probe_question("relay-b", 0);
        assert_ne!(probe_question("relay-a", 0), other);
        assert!(PROBE_QUESTIONS.contains(&other));
    }

    #[test]
    fn models_url_google_vertex_uses_publishers_path() {
        assert_eq!(
            App::models_url(
                "https://us-central1-aiplatform.googleapis.com/v1",
                "google-vertex"
            ),
            "https://us-central1-aiplatform.googleapis.com/v1/publishers/google/models"
        );
        assert_eq!(
            App::models_url(
                "https://generativelanguage.googleapis.com/v1beta",
                "google-generative-ai"
            ),
            "https://generativelanguage.googleapis.com/v1beta/models"
        );
    }

    #[test]
    fn sanitize_network_error_strips_url_query() {
        let msg = r#"connection failed: Connection refused (os error 10061) for URL "https://example.com/v1?key=sk-super-secret#frag""#;
        let out = sanitize_network_error(msg);
        assert!(!out.contains("sk-super-secret"), "泄露了 query: {}", out);
        assert!(!out.contains('#'), "泄露了 fragment: {}", out);
        assert!(out.contains("https://example.com/v1"));
    }

    #[test]
    fn sanitize_network_error_passthrough_without_url() {
        let msg = "dns error: failed to lookup address";
        assert_eq!(sanitize_network_error(msg), msg);
    }

    #[test]
    fn sanitize_network_error_url_no_query_untouched() {
        let msg = r#"connection failed for URL "https://api.example.com/v1/models""#;
        assert_eq!(sanitize_network_error(msg), msg);
    }
}

#[cfg(test)]
mod syntax_highlight_tests {
    use super::{
        json_tokens, syntax_tokens, yaml_tokens, PreviewSyntax, SYN_COMMENT, SYN_KEY, SYN_NUMBER,
        SYN_STRING,
    };
    use eframe::egui;

    /// 段落必须落在字符边界上，否则 LayoutJob 切片会 panic。
    fn assert_boundaries(text: &str, tokens: &[(usize, usize, egui::Color32)]) {
        for &(s, e, _) in tokens {
            assert!(
                text.is_char_boundary(s),
                "start {} 不在字符边界: {:?}",
                s,
                text
            );
            assert!(
                text.is_char_boundary(e),
                "end {} 不在字符边界: {:?}",
                e,
                text
            );
            assert!(s <= e);
        }
    }

    #[test]
    fn json_distinguishes_key_and_value_strings() {
        let text = r#"{"apiKey": "sk-xxx", "count": 12, "on": true}"#;
        let tokens = json_tokens(text);
        assert_boundaries(text, &tokens);
        let color_of = |needle: &str| {
            let start = text.find(needle).unwrap();
            tokens
                .iter()
                .find(|(s, e, _)| *s <= start && start < *e)
                .map(|(_, _, c)| *c)
                .unwrap()
        };
        assert_eq!(color_of("apiKey"), SYN_KEY);
        assert_eq!(color_of("sk-xxx"), SYN_STRING);
        assert_eq!(color_of("12"), SYN_NUMBER);
    }

    #[test]
    fn json_handles_comments_and_non_ascii() {
        let text =
            "{\n  // 中文注释 \"引号\"\n  \"名前\": \"值\",\n  /* 块注释 */\n  \"n\": 1.5e3\n}";
        let tokens = json_tokens(text);
        assert_boundaries(text, &tokens);
        let comment_start = text.find("//").unwrap();
        let comment = tokens
            .iter()
            .find(|(s, _, c)| *s == comment_start && *c == SYN_COMMENT);
        assert!(comment.is_some(), "未识别行注释");
        assert!(tokens
            .iter()
            .any(|(s, _, c)| *s == text.find("\"名前\"").unwrap() && *c == SYN_KEY));
    }

    #[test]
    fn yaml_colors_keys_comments_and_literals() {
        let text = "# 顶部注释\nbaseURL: \"https://example.com/v1\"\ntimeout: 180000\nenabled: true\n# 中文注释\n";
        let tokens = yaml_tokens(text);
        assert_boundaries(text, &tokens);
        let color_of = |needle: &str| {
            let start = text.find(needle).unwrap();
            tokens
                .iter()
                .find(|(s, e, _)| *s <= start && start < *e)
                .map(|(_, _, c)| *c)
                .unwrap()
        };
        assert_eq!(color_of("baseURL"), SYN_KEY);
        assert_eq!(color_of("https://example.com/v1"), SYN_STRING);
        assert_eq!(color_of("180000"), SYN_NUMBER);
    }

    #[test]
    fn syntax_dispatch_matches_page_kind() {
        assert_eq!(syntax_tokens("{}", PreviewSyntax::Json).len(), 2);
        assert!(syntax_tokens("a: 1\n", PreviewSyntax::Yaml).len() >= 3);
    }
}

#[cfg(test)]
mod latency_tests {
    use super::{
        latency_color, matrix_glyphs, LATENCY_GOOD_MS, LATENCY_GREEN, LATENCY_RED, LATENCY_SLOW_MS,
        LATENCY_YELLOW, MATRIX_CHARS, MATRIX_LEN,
    };

    #[test]
    fn latency_color_thresholds() {
        assert_eq!(latency_color(0), LATENCY_GREEN);
        assert_eq!(latency_color(LATENCY_GOOD_MS - 1), LATENCY_GREEN);
        assert_eq!(latency_color(LATENCY_GOOD_MS), LATENCY_YELLOW);
        assert_eq!(latency_color(LATENCY_SLOW_MS - 1), LATENCY_YELLOW);
        assert_eq!(latency_color(LATENCY_SLOW_MS), LATENCY_RED);
    }

    #[test]
    fn matrix_glyphs_shape_and_variation() {
        let frame = matrix_glyphs(7, "provider/model", MATRIX_LEN);
        assert_eq!(frame.chars().count(), MATRIX_LEN);
        assert!(frame.chars().all(|c| MATRIX_CHARS.contains(c)));
        // 同一帧 + 同一 salt 稳定（不依赖保存的随机状态）
        assert_eq!(frame, matrix_glyphs(7, "provider/model", MATRIX_LEN));
        // 换行（salt 不同）或换帧都会刷新字符
        assert_ne!(frame, matrix_glyphs(7, "provider/other", MATRIX_LEN));
        assert!((8..16).any(|f| frame != matrix_glyphs(f, "provider/model", MATRIX_LEN)));
    }
}

#[cfg(test)]
mod preview_sync_tests {
    use super::preview_should_rebuild;

    #[test]
    fn rebuild_gate_ignores_focus_but_keeps_user_text() {
        // 没在预览里手改：始终按组件状态重建（与焦点无关）
        assert!(preview_should_rebuild(false, None, 100.0));
        // 刚在预览里输入（2 秒内）：保留用户文本，避免打断手改
        assert!(!preview_should_rebuild(false, Some(99.5), 100.0));
        assert!(!preview_should_rebuild(false, Some(100.0), 100.0));
        // 停止输入超过 2 秒：回到组件状态（预览不会一直停在旧内容上）
        assert!(preview_should_rebuild(false, Some(97.9), 100.0));
        // 上次解析失败：保留用户文本，等用户修正或点「重新生成」
        assert!(!preview_should_rebuild(true, None, 100.0));
        assert!(!preview_should_rebuild(true, Some(1.0), 100.0));
    }
}
