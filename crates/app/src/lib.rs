pub mod app;
#[cfg(target_os = "windows")]
pub mod cursor;
pub mod motion;
pub mod theme;
pub mod ui;
pub mod windowfx;

// 核心逻辑在 model_harbor_core。此处原样 re-export，保持 `crate::backends` 等
// 既有路径在 app 内部继续可用，避免逐个改动全部 `crate::x` 引用。
pub use model_harbor_core::{
    backends, billing, convert, credentials, format, http_status, model, netguard, opencode_models,
    prefs, presets, profiles, serialize, tokens, usage, util,
};
