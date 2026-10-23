//! Agent config backend registry — one backend module per config format.
//!
//! Adding an agent integration: create `src/backends/<id>.rs` implementing [`Backend`] and register it in
//! [`BACKENDS`]; load / save / detection / target resolution all go through this registry.
//!
//! Serialization style (pretty / compact JSON) is handled by the app layer; backends only produce a
//! `serde_json::Value` root; cross-format save merge semantics are built into each backend's `serialize_root`.

pub mod deepseek_harness;
pub mod kimi_code;
pub mod oh_my_pi;
pub mod opencode;
pub mod pi;
pub mod qwen_code;
pub mod workbuddy;
pub mod zcode;

use crate::format::ConfigFormat;
use crate::model::{AgentRow, ProviderRow};
use crate::util::{is_wsl_path, WslPathProbe};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// Complete snapshot of one backend's load result.
#[derive(Clone)]
pub struct BackendLoad {
    /// The complete original root (the base when saving the current file).
    pub root: Value,
    /// opencode's agents (empty for the pi family).
    pub agents: Vec<AgentRow>,
    /// providers (both formats have them).
    pub providers: Vec<ProviderRow>,
    /// This backend's unknown top-level fields: opencode holds the whole root, the pi family the top-level
    /// fields beyond `providers`, DSH the complete root.
    pub extras: Value,
}

/// Config backend: the load / save / detection / path knowledge of one agent config format.
pub trait Backend: Sync {
    /// The unique identifier (matching the ConfigFormat enum).
    fn id(&self) -> ConfigFormat;

    /// The default local path (Windows style).
    fn default_local_path(&self) -> String;

    /// The default WSL path (None = WSL unsupported).
    fn default_wsl_path(&self) -> Option<String>;

    /// A config path's equivalent candidates, in priority order (the primary first).
    fn path_candidates(&self, path: &str) -> Vec<String> {
        vec![path.to_string()]
    }

    /// The local file path actually read and written: the first candidate that really exists; all absent → the primary.
    fn resolve_local_path(&self, local_path: &str) -> String {
        self.path_candidates(local_path)
            .into_iter()
            .find(|c| Path::new(c.as_str()).exists())
            .unwrap_or_else(|| local_path.to_string())
    }

    /// Whether the local target is usable. Default verdict: the file or its parent exists.
    /// The opencode family overrides to accept only the file itself (see `path_candidates`).
    fn local_available(&self, local_path: &str) -> bool {
        let path = Path::new(local_path);
        path.exists() || path.parent().is_some_and(|dir| dir.exists())
    }

    /// Whether the WSL target is usable (the config file or its directory exists).
    /// Flags arrive from the registry's batch probe; backends never spawn processes themselves.
    /// The opencode family overrides to accept only the file itself.
    fn wsl_available(&self, probe: WslPathProbe) -> bool {
        probe.path_exists || probe.parent_dir_exists
    }

    /// Content detection: whether the content belongs to this format. `path` gives extension context (may be empty).
    fn detect(&self, content: &str, path: &str) -> bool;

    /// Content → structured data; read/parse failure returns Err.
    fn parse(&self, content: &str) -> Result<BackendLoad, String>;

    /// Parse with the source path. Defaults to `parse`; backends needing sibling sidecar files override.
    fn parse_at(&self, content: &str, _path: &str) -> Result<BackendLoad, String> {
        self.parse(content)
    }

    /// Build the root for saving.
    /// - `target_root = None`: current-file save — UI state written wholesale over `extras`;
    /// - `target_root = Some`: merged with the target's existing content (upsert).
    fn serialize_root(
        &self,
        agents: &[AgentRow],
        providers: &[ProviderRow],
        extras: &Value,
        target_root: Option<&Value>,
    ) -> Value;

    /// The save target's (non-current-file) existing root, the merge base.
    ///
    /// A missing file returns an empty root (creation scenario); an existing but unreadable / unparseable file
    /// returns Err, and the caller cancels the save.
    fn load_target_root(&self, path: &str) -> Result<Value, String>;

    /// Save sidecar files beside the main config (none by default).
    /// Called before the main config writes; a sidecar failure cancels the save.
    fn save_sidecars(&self, _path: &str, _providers: &[ProviderRow]) -> Result<(), String> {
        Ok(())
    }

    /// The official icon: 32×32 non-premultiplied RGBA raw bytes and size; None = no icon.
    fn icon_rgba(&self) -> Option<(&'static [u8], u32, u32)> {
        None
    }

    /// root → file content. JSON backends share the default; YAML backends (omp / DSH) and WorkBuddy's array root override.
    fn render(&self, root: &Value, compact: bool) -> Result<String, String> {
        Ok(if compact {
            crate::serialize::compact_json(root)
        } else {
            crate::serialize::pretty_json(root)
        })
    }
}

/// All backends. Order is semantics: entry 0 is the detection fallback; the rest are ordered
/// "more specific first" (omp before pi; workbuddy / zcode carry unique top-level markers and sit before
/// the pi family).
///
/// The three opencode-family members share content shapes and are told apart by path (see
/// [`opencode::path_matches`]); their relative order doesn't affect detection.
pub static BACKENDS: &[&dyn Backend] = &[
    &opencode::BACKEND,
    &opencode::KILOCODE_BACKEND,
    &opencode::MIMOCODE_BACKEND,
    &workbuddy::BACKEND,
    &zcode::BACKEND,
    &qwen_code::BACKEND,
    &deepseek_harness::BACKEND,
    // kimi-code detects via the `[providers.` + `[models.` twin tables plus the `max_context_size` fingerprint,
    // ranking before the pi family.
    &kimi_code::BACKEND,
    &oh_my_pi::BACKEND,
    &pi::BACKEND,
];

/// Find a backend by identifier.
pub fn backend(id: ConfigFormat) -> &'static dyn Backend {
    BACKENDS
        .iter()
        .copied()
        .find(|b| b.id() == id)
        .expect("BACKENDS 必须覆盖所有 ConfigFormat 变体")
}

/// Content detection: walk non-fallback backends for the first hit, else fall back to entry 0.
pub fn detect_format(content: &str, path: &str) -> ConfigFormat {
    for b in BACKENDS.iter().skip(1) {
        if b.detect(content, path) {
            return b.id();
        }
    }
    BACKENDS[0].id()
}

/// Generic load: read the file (local or WSL) + parse per backend.
pub fn load_backend(id: ConfigFormat, path: &str) -> Result<BackendLoad, String> {
    let content = crate::util::read_config_content(path)?;
    backend(id).parse_at(&content, path)
}

/// WSL target path (returned when it exists).
///
/// Probing runs on a background thread, cached per process; returns `None` until the probe completes
/// (use [`wsl_target_state`] when "not installed" and "no result yet" must be distinguished).
/// Agents newly installed on the WSL side during runtime aren't noticed until the app restarts.
pub fn wsl_target(id: ConfigFormat) -> Option<String> {
    target_from_state(wsl_target_state(id))
}

/// Three-state → writable path: only "installed" has a value; "probing" and "confirmed missing" return `None`.
fn target_from_state(state: WslTargetState) -> Option<String> {
    match state {
        WslTargetState::Installed(path) => Some(path),
        WslTargetState::Unknown | WslTargetState::NotInstalled => None,
    }
}

/// Take one backend's state from the probe-result map.
///
/// "Not in the map" = probing finished and that backend is not installed.
fn state_from_map(map: &HashMap<ConfigFormat, Option<String>>, id: ConfigFormat) -> WslTargetState {
    match map.get(&id).and_then(|path| path.clone()) {
        Some(path) => WslTargetState::Installed(path),
        None => WslTargetState::NotInstalled,
    }
}

/// Three-state result of WSL target probing.
///
/// "Not installed" and "no result yet" are kept apart.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum WslTargetState {
    /// Probing (or the master switch is off).
    Unknown,
    /// Installed: the target path.
    Installed(String),
    /// Probing finished and confirmed not installed.
    NotInstalled,
}

/// Receiver-side payload of the background probe task: `(generation, per-backend target paths)`.
type WslProbePayload = (u64, HashMap<ConfigFormat, Option<String>>);

/// Background probe task: receiver + finished results + generation.
#[derive(Default)]
struct WslProbeTask {
    rx: Option<std::sync::mpsc::Receiver<WslProbePayload>>,
    done: Option<HashMap<ConfigFormat, Option<String>>>,
    /// Generation: every switch-off bumps it. In-flight results with mismatched generations are discarded.
    generation: u64,
}

static WSL_PROBE: std::sync::Mutex<Option<WslProbeTask>> = std::sync::Mutex::new(None);

/// Take the WSL target state, starting a background probe when necessary (non-blocking).
pub fn wsl_target_state(id: ConfigFormat) -> WslTargetState {
    // Switch off: drop the cache and bump the generation; the next enable probes fresh.
    if !crate::util::wsl_enabled() {
        if let Ok(mut guard) = WSL_PROBE.lock() {
            if let Some(task) = guard.as_mut() {
                task.generation += 1;
                task.rx = None;
                task.done = None;
            }
        }
        return WslTargetState::Unknown;
    }
    let Ok(mut guard) = WSL_PROBE.lock() else {
        return WslTargetState::Unknown;
    };
    let task = guard.get_or_insert_with(WslProbeTask::default);
    // Collect in-flight results: mismatched generations are dropped.
    if task.done.is_none() {
        if let Some(rx) = &task.rx {
            match rx.try_recv() {
                Ok((gen, map)) => {
                    if gen == task.generation {
                        task.done = Some(map);
                    }
                    task.rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // The thread died abnormally: drop rx, never reading "unknown" as "not installed".
                    task.rx = None;
                }
            }
        }
    }
    match &task.done {
        Some(map) => state_from_map(map, id),
        None => {
            if task.rx.is_none() {
                let (tx, rx) = std::sync::mpsc::channel();
                let generation = task.generation;
                std::thread::spawn(move || {
                    let map = probe_wsl_targets();
                    let _ = tx.send((generation, map));
                });
                task.rx = Some(rx);
            }
            WslTargetState::Unknown
        }
    }
}

/// Whether the background probe is still running (App uses this to decide on repaint requests).
pub fn wsl_probe_pending() -> bool {
    WSL_PROBE
        .lock()
        .map(|guard| {
            guard
                .as_ref()
                .is_some_and(|task| task.done.is_none() && task.rx.is_some())
        })
        .unwrap_or(false)
}

/// Probe every backend's default WSL path in one `wsl` call; returns paths of "installed" backends.
///
/// Each backend may have several equivalent candidate paths; all are probed together. Selection order:
/// candidates that are genuinely files first; the loose fallback condition applies only when no file exists.
fn probe_wsl_targets() -> HashMap<ConfigFormat, Option<String>> {
    let mut out = HashMap::new();
    // (backend, candidate paths) — candidates in priority order (primary first).
    let entries: Vec<(ConfigFormat, Vec<String>)> = BACKENDS
        .iter()
        .filter_map(|b| {
            let base = b.default_wsl_path()?;
            Some((b.id(), b.path_candidates(&base)))
        })
        .collect();
    if entries.is_empty() {
        return out;
    }
    // Flatten, probe in one pass, then split back per backend.
    let mut flat: Vec<String> = Vec::new();
    for (_, candidates) in &entries {
        flat.extend(candidates.iter().cloned());
    }
    let probes = crate::util::wsl_batch_probe(&flat);
    let mut cursor = 0;
    for (id, candidates) in entries {
        let slice = &probes[cursor..cursor + candidates.len()];
        cursor += candidates.len();
        let backend = backend(id);
        // Preferred: candidates that are genuinely existing files.
        let by_file = candidates
            .iter()
            .zip(slice)
            .find(|(_, probe)| probe.file_exists)
            .map(|(path, _)| path.clone());
        // Then: the backend's own loose verdict.
        let by_rule = candidates
            .iter()
            .zip(slice)
            .find(|(_, probe)| backend.wsl_available(**probe))
            .map(|(path, _)| path.clone());
        if let Some(path) = by_file.or(by_rule) {
            out.insert(id, Some(path));
        }
    }
    out
}

/// Whether a target is usable (local or WSL).
///
/// A probing WSL target doesn't count as "usable" (the result feeds tab ordering / save-target counts).
pub fn target_available(id: ConfigFormat, local_path: &str) -> bool {
    backend(id).local_available(local_path) || wsl_target(id).is_some()
}

/// The local path actually read and written: backends may fall back to equivalent file names beyond the default.
pub fn resolve_local_path(id: ConfigFormat, local_path: &str) -> String {
    backend(id).resolve_local_path(local_path)
}

/// The actual write path: local first; when local is unavailable, falls back to WSL; finally the local default.
///
/// The WSL side only accepts "installed": probing and confirmed-missing both fall back to the local default.
pub fn target_path(id: ConfigFormat, local_path: &str) -> String {
    let local = backend(id).resolve_local_path(local_path);
    if Path::new(&local).exists() {
        return local;
    }
    if let Some(wsl) = wsl_target(id) {
        return wsl;
    }
    local
}

/// Unified write: WSL paths go through the wsl command, local paths write atomically (temp file first, then replace).
pub fn write_config(path: &str, content: &str) -> Result<(), String> {
    if is_wsl_path(path) {
        crate::util::write_wsl_file(path, content)
    } else {
        crate::util::atomic_write_text(std::path::Path::new(path), content)
    }
}

/// Shared implementation of [`Backend::load_target_root`]: each backend only supplies a parse function and an empty root for "file missing".
///
/// An existing but unreadable / unparseable file returns Err, with the path attached.
pub(crate) fn load_target_root_with(
    path: &str,
    parse: impl Fn(&str) -> Result<Value, String>,
    empty: impl FnOnce() -> Value,
) -> Result<Value, String> {
    let content =
        crate::util::read_config_content(path).map_err(|e| format!("读取失败（{path}）: {e}"))?;
    if content.trim().is_empty() {
        return Ok(empty());
    }
    parse(&content).map_err(|e| format!("解析失败（{path}）: {e}"))
}

#[cfg(test)]
mod wsl_state_tests {
    use super::*;

    /// Probe-result map → three states: an entry with a non-empty path = installed; `None` or missing = not installed.
    #[test]
    fn map_maps_to_installed_or_not_installed() {
        let mut map: HashMap<ConfigFormat, Option<String>> = HashMap::new();
        map.insert(
            ConfigFormat::Opencode,
            Some("/home/u/.config/opencode/opencode.json".into()),
        );
        map.insert(ConfigFormat::Pi, None);

        assert_eq!(
            state_from_map(&map, ConfigFormat::Opencode),
            WslTargetState::Installed("/home/u/.config/opencode/opencode.json".into())
        );
        assert_eq!(
            state_from_map(&map, ConfigFormat::Pi),
            WslTargetState::NotInstalled
        );
        // Entirely absent from the map (e.g. the backend has no default WSL path): also "not installed".
        assert_eq!(
            state_from_map(&map, ConfigFormat::KimiCode),
            WslTargetState::NotInstalled
        );
    }

    /// Only "installed" produces a writable path; "unknown" and "not installed" both give `None`.
    #[test]
    fn only_installed_yields_a_writable_path() {
        assert_eq!(
            target_from_state(WslTargetState::Installed("/home/u/x.json".into())),
            Some("/home/u/x.json".to_string())
        );
        assert_eq!(target_from_state(WslTargetState::Unknown), None);
        assert_eq!(target_from_state(WslTargetState::NotInstalled), None);
    }
}
