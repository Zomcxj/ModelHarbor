pub mod app;
#[cfg(target_os = "windows")]
pub mod cursor;
pub mod motion;
pub mod theme;
#[cfg(target_os = "windows")]
pub mod tray;
pub mod ui;
pub mod windowfx;

// The core logic lives in model_harbor_core. This re-exports verbatim, keeping existing paths like
// `crate::backends` usable inside app, sparing every `crate::x` reference a rewrite.
pub use model_harbor_core::{
    backends, billing, convert, credentials, format, http_status, model, netguard, opencode_models,
    prefs, presets, profiles, serialize, tokens, usage, util,
};
