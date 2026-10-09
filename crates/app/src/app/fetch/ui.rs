use super::*;

/// 延迟配色：<2s 绿色、2~5s 黄色、≥5s 红色（超时同样显示红色错误）。
///
/// 色值由 [`crate::theme::semantics`] 提供，全部主题共享一套语义色。
pub(crate) fn latency_color(ms: u64, colors: crate::theme::Semantics) -> egui::Color32 {
    if ms < LATENCY_GOOD_MS {
        colors.ok
    } else if ms < LATENCY_SLOW_MS {
        colors.warn
    } else {
        colors.err
    }
}

/// 延迟测试进行中的「乱码」动画字符集（半角片假名 + 数字）。
pub(crate) const MATRIX_CHARS: &str = "ｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝ0123456789";

/// 乱码动画每帧时长（毫秒）与每行字符数。
pub(crate) const MATRIX_FRAME_MS: u64 = 70;

pub(crate) const MATRIX_LEN: usize = 8;

/// 生成一帧乱码：同一帧号 + 同一 salt 结果稳定。
pub(crate) fn matrix_glyphs(frame: u64, salt: &str, len: usize) -> String {
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
pub(crate) fn matrix_label(ui: &mut egui::Ui, salt: &str) {
    let frame = (ui.ctx().input(|i| i.time) * 1000.0 / MATRIX_FRAME_MS as f64) as u64;
    ui.label(
        egui::RichText::new(matrix_glyphs(frame, salt, MATRIX_LEN))
            .monospace()
            .color(crate::theme::semantics(ui).ok),
    )
    .on_hover_text("延迟测试进行中");
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(MATRIX_FRAME_MS));
}

/// 模型行延迟显示：测试中显示乱码动画，完成后按阈值着色。
pub(crate) fn model_latency_label(ui: &mut egui::Ui, state: Option<&LatencyState>, model_id: &str) {
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
                latency_color(*ms, crate::theme::semantics(ui)),
            );
            response.on_hover_text(format!(
                "最近一次首字延迟 {} ms（流式：请求发出 → 第一个字）",
                ms
            ));
        }
        Some(Err(err)) => {
            ui.label(egui::RichText::new(short_err(err)).color(crate::theme::semantics(ui).err))
                .on_hover_text(err);
        }
        None if state.pending.contains(model_id) => matrix_label(ui, model_id),
        None => {}
    }
}

/// 模型行上的单模型延迟测试按钮（位于拖动按钮右侧，结果标签就在它右侧）。
///
/// 返回 `true` 表示用户点了测试；调用方负责在 UI 循环外真正发起探测。
pub(crate) fn model_probe_button(
    ui: &mut egui::Ui,
    gate: &ProbeGate,
    provider_key: &str,
    now: f64,
    net_guard: Option<&str>,
) -> bool {
    let state = gate.state(provider_key, now, net_guard);
    // 状态一律写在按钮文案里（`测试中` / `测试(5s)`），不挂悬停提示。
    let label = match &state {
        ProbeGateState::Ready => "测试".to_string(),
        ProbeGateState::Busy => "测试中".to_string(),
        ProbeGateState::Cooling(left) => format!("测试({}s)", left.ceil() as u64),
        ProbeGateState::NetBlocked(_) => "测试".to_string(),
    };
    let enabled = matches!(state, ProbeGateState::Ready);
    ui.add_enabled(enabled, egui::Button::new(label)).clicked()
}
