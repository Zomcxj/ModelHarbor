use super::*;

/// Latency coloring: <2s green, 2–5s yellow, ≥5s red (timeouts show the red error too).
///
/// Color values come from [`crate::theme::semantics`] — all themes share one semantic set.
pub(crate) fn latency_color(ms: u64, colors: crate::theme::Semantics) -> egui::Color32 {
    if ms < LATENCY_GOOD_MS {
        colors.ok
    } else if ms < LATENCY_SLOW_MS {
        colors.warn
    } else {
        colors.err
    }
}

/// The scrambling animation's character set while a latency test runs (half-width katakana + digits).
pub(crate) const MATRIX_CHARS: &str = "ｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝ0123456789";

/// The scramble animation's per-frame duration (ms) and characters per line.
pub(crate) const MATRIX_FRAME_MS: u64 = 70;

pub(crate) const MATRIX_LEN: usize = 8;

/// Generate one scramble frame: the same frame number + salt gives a stable result.
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

/// The scrambling label while a test runs: a new character set each frame, requesting the next repaint.
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

/// A model row's latency display: the scramble animation while testing, threshold-colored once done.
pub(crate) fn model_latency_label(ui: &mut egui::Ui, state: Option<&LatencyState>, model_id: &str) {
    let Some(state) = state else {
        return;
    };
    match state.models.get(model_id) {
        Some(Ok(ms)) => {
            // Fixed-width, right-aligned: the button doesn't jitter left-right as digit counts change.
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

/// The model row's single-model latency test button (right of the drag button, its result label to its right).
///
/// Returns `true` when the user clicked test; the caller actually issues the probe outside the UI loop.
pub(crate) fn model_probe_button(
    ui: &mut egui::Ui,
    gate: &ProbeGate,
    provider_key: &str,
    now: f64,
    net_guard: Option<&str>,
) -> bool {
    let state = gate.state(provider_key, now, net_guard);
    // State always lives in the button's text (`testing` / `test(5s)`), never a hover tip.
    let label = match &state {
        ProbeGateState::Ready => "测试".to_string(),
        ProbeGateState::Busy => "测试中".to_string(),
        ProbeGateState::Cooling(left) => format!("测试({}s)", left.ceil() as u64),
        ProbeGateState::NetBlocked(_) => "测试".to_string(),
    };
    let enabled = matches!(state, ProbeGateState::Ready);
    ui.add_enabled(enabled, egui::Button::new(label)).clicked()
}
