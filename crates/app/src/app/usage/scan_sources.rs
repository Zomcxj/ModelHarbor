//! Scan source paths + file fingerprints — the "change detection" behind incremental rescans.
//!
//! Problem: `usage::scan()` is a full scan; running it every 3 seconds would pin the CPU. So first run
//! an **extremely cheap fingerprint comparison**, and only kick off a rescan when source files really
//! changed.
//!
//! ## Why source paths live in this table
//!
//! The authoritative source paths live in the scan engine (`tokscale_core::ClientId`), but that crate is
//! **a dependency of core, not of app** (see `crates/core/Cargo.toml`) — the app layer can't reach it.
//! So this module maintains an explicit table: **paths are copied entry-by-entry from tokscale
//! v4.18.0's `ClientDef` (`root` + `relative`), not assembled from [`ConfigFormat::label`]** — the two
//! spellings disagree on 6 of 10 (`kilo` vs `kilocode`, `micode` vs `mimocode`, `dsh` vs
//! `deepseek-harness` …); string assembly would guarantee mistakes.
//!
//! [`source_roots`]'s `every_backend_has_a_fingerprint_root` test **fails immediately when a new agent
//! is added**, forcing this table to be updated rather than silently skipping one agent's change
//! detection.
//!
//! The consequence of a miss is **degradation, not error**: that agent's new sessions won't trigger a
//! rescan by themselves, but the next rescan triggered for any other reason still counts them (the
//! scan engine remains authoritative).
//!
//! ## Why fingerprints must be file-level
//!
//! Appending to a session file does **not** change the parent directory's mtime — watching only
//! directories would treat "a session being streamed to" as unchanged. So files are listed recursively,
//! recording each file's `(mtime, size)`.
//!
//! Recursion cost is controlled by **pruning**: opencode's data dir holds not just the session store
//! but snapshot git objects and logs; workbuddy even more. Pruning subtrees that **certainly contain no
//! session ledger** is what brings the full fingerprint scan down to an acceptable cost.

use crate::format::ConfigFormat;
use crate::prefs;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Recursion depth cap: session files sit at most four levels deep (project/session/sub-agent/run),
/// so 8 is plenty while still stopping unexpected deep trees.
const MAX_DEPTH: usize = 8;

/// Max files kept by a fingerprint scan: a defensive cap so an abnormally large directory can't exhaust memory.
/// Past the cap, collection stops (this fingerprint is incomplete → the next comparison may miss
/// changes, but will never invent them).
const MAX_FILES: usize = 20_000;

/// Directory names skipped outright during recursion (case-insensitive).
///
/// Every entry is a heavy directory that "certainly contains no session ledger", named after these
/// tools' directory structures:
/// - opencode: `snapshot` (git objects), `log`, `storage/session_diff`, etc.;
/// - workbuddy: `binaries`, `plugins`, `workspace`, `connectors-marketplace` …
///
/// Matching is by **directory name only**, never path fragments, so genuinely session-bearing
/// directories like `projects` / `sessions` / `storage/message` are never hit.
const PRUNE_DIRS: &[&str] = &[
    // Version control / snapshot objects
    "objects",
    "snapshot",
    "worktree",
    "repos",
    "node_modules",
    "target",
    "__pycache__",
    // Logs and audits
    "log",
    "logs",
    "traces",
    "audit-log",
    "shell-snapshots",
    // Caches and bulk data
    "cache",
    "blobs",
    "binaries",
    "tool-output",
    "workspace",
    "plugins",
    "connectors",
    "connectors-marketplace",
    "file-history",
    "changes-detail",
    "changes-index",
    "artifact-index",
    "local_storage",
    "file-tree-manifests",
    "tmp",
    "temp",
    // opencode's storage internals (only message / part are ledgers)
    "session_diff",
    "directory-readme",
    "migration",
    "agent-usage-reminder",
];

/// Fingerprint: file path → `(mtime secs, byte size)`.
pub(in crate::app) type Fingerprints = HashMap<PathBuf, (u64, u64)>;

/// Whether this directory should be skipped.
fn is_pruned(name: &str) -> bool {
    PRUNE_DIRS
        .iter()
        .any(|pruned| name.eq_ignore_ascii_case(pruned))
}

/// One scan source: `client` names the agent it belongs to; `relative` is the path relative to home.
///
/// `client` is not decoration: one agent may keep several ledgers (workbuddy's `.workbuddy` and
/// `.workbuddy-ai`), and the UI must dedupe by agent before reporting "how many agents are watched".
///
/// A path may be a **directory** (collect files recursively) or a **single file** (collect just it).
struct ScanSource {
    client: ConfigFormat,
    relative: &'static str,
}

/// Copied entry-by-entry from tokscale v4.18.0's scan targets (`ClientDef::root` + `relative_path`,
/// plus the few extra pushes inside `scan_all_clients_with_scanner_settings`).
///
/// **Only paths the scan engine actually reads are listed — not the whole agent directory.** This is
/// the performance crux: `~/.workbuddy` holds 18,000+ files on this machine (`binaries` / `plugins` /
/// `workspace` etc.), yet tokscale reads just two places from it: `workbuddy.db` and `projects/*.jsonl`.
/// Watching only those two drops the fingerprint scan from "seconds" to "tens of milliseconds", with no
/// need for pruning heuristics.
const SOURCES: &[ScanSource] = &[
    // XdgData root: `~/.local/share/<relative>`
    ScanSource {
        client: ConfigFormat::Opencode,
        relative: ".local/share/opencode",
    },
    ScanSource {
        client: ConfigFormat::Kilocode,
        relative: ".local/share/kilo",
    },
    ScanSource {
        client: ConfigFormat::Mimocode,
        relative: ".local/share/mimocode",
    },
    // Home root
    ScanSource {
        client: ConfigFormat::Pi,
        relative: ".pi/agent/sessions",
    },
    ScanSource {
        client: ConfigFormat::OhMyPi,
        relative: ".omp/agent/sessions",
    },
    ScanSource {
        client: ConfigFormat::ZCode,
        relative: ".zcode/projects",
    },
    ScanSource {
        client: ConfigFormat::QwenCode,
        relative: ".qwen/projects",
    },
    // WorkBuddy: sessions are `projects/*.jsonl`, plus one `workbuddy.db`.
    ScanSource {
        client: ConfigFormat::WorkBuddy,
        relative: ".workbuddy/projects",
    },
    ScanSource {
        client: ConfigFormat::WorkBuddy,
        relative: ".workbuddy/workbuddy.db",
    },
    // WorkBuddy 5.5 ("WorkBuddy AI") moved both sessions and the database to `.workbuddy-ai`.
    ScanSource {
        client: ConfigFormat::WorkBuddy,
        relative: ".workbuddy-ai/projects",
    },
    ScanSource {
        client: ConfigFormat::WorkBuddy,
        relative: ".workbuddy-ai/workbuddy.db",
    },
    // Kimi CLI and Kimi Code each have their own `wire.jsonl` directory.
    ScanSource {
        client: ConfigFormat::KimiCode,
        relative: ".kimi/sessions",
    },
    ScanSource {
        client: ConfigFormat::KimiCode,
        relative: ".kimi-code/sessions",
    },
];

/// Agent roots that need environment variables (users may relocate them).
///
/// tokscale uses `PathRoot::EnvVar` for these two: the env var wins when set, otherwise
/// `fallback_relative` applies. Following that behavior matters — after the user moves their home,
/// fingerprints would otherwise watch the wrong place.
fn env_root(var: &str, fallback_relative: &str, home: &Path) -> PathBuf {
    match std::env::var_os(var).filter(|value| !value.is_empty()) {
        Some(custom) => PathBuf::from(custom).join("sessions"),
        None => home.join(fallback_relative),
    }
}

/// Record one file entry (shared by directory recursion and single-file sources).
fn record_file(path: &Path, metadata: &std::fs::Metadata, out: &mut Fingerprints) {
    let mtime_secs = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    out.insert(path.to_path_buf(), (mtime_secs, metadata.len()));
}

/// Recursively collect fingerprints of every file under `root`.
///
/// Uses [`std::fs::DirEntry::metadata`]: on Windows it reuses the directory-entry info already fetched
/// by `read_dir` with no extra syscall — much faster than `path.metadata()` (this is precisely what
/// makes running the function every 3 seconds viable). File types come from the directory entry too,
/// avoiding one stat per file.
///
/// `root` may also be a **file** (workbuddy's `workbuddy.db`): then only that entry is collected.
fn collect(root: &Path, out: &mut Fingerprints, depth: usize) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    // Single-file source: recorded directly, skipping the directory branch.
    if root.is_file() {
        if let Ok(metadata) = root.metadata() {
            record_file(root, &metadata, out);
        }
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        // Unreadable (permissions / directory deleted) counts as this subtree having no files.
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if file_type.is_dir() {
            // Hidden directories never enter (`.git` etc.); hidden **files** stay
            // (some ledgers genuinely start with a dot).
            if name.starts_with('.') || is_pruned(&name) {
                continue;
            }
            collect(&entry.path(), out, depth + 1);
            continue;
        }
        if !file_type.is_file() {
            // Symlinks / device files: not followed, avoiding cycles.
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        record_file(&entry.path(), &metadata, out);
    }
}

/// All scan sources (existing ones only), with their owning agent.
///
/// Only **existing** paths are collected: stat-ing missing directories is wasted effort, and they'd
/// stuff the fingerprint with keys that never appear. The cost: "directory created today" is noticed
/// only on the next round — but by then it exists and gets picked up.
pub(in crate::app) fn source_roots() -> Vec<(ConfigFormat, PathBuf)> {
    let Some(home) = prefs::home_dir() else {
        return Vec::new();
    };
    let mut roots: Vec<(ConfigFormat, PathBuf)> = Vec::new();
    for source in SOURCES {
        push_unique(&mut roots, source.client, home.join(source.relative));
    }
    // DSH uses `DSH_HOME`, falling back to `.dsh/sessions`.
    push_unique(
        &mut roots,
        ConfigFormat::DeepSeekHarness,
        env_root("DSH_HOME", ".dsh/sessions", &home),
    );
    roots.retain(|(_, path)| path.exists());
    roots
}

/// Add if unseen (keeping first-seen order for readable tests and logs).
fn push_unique(roots: &mut Vec<(ConfigFormat, PathBuf)>, client: ConfigFormat, path: PathBuf) {
    if !roots.iter().any(|(_, existing)| *existing == path) {
        roots.push((client, path));
    }
}

/// Number of agents being watched (deduped by agent; used by the status line).
#[cfg(test)]
pub(in crate::app) fn watched_agent_count() -> usize {
    let mut seen: Vec<ConfigFormat> = Vec::new();
    for (client, _) in source_roots() {
        if !seen.contains(&client) {
            seen.push(client);
        }
    }
    seen.len()
}

/// Sweep every source directory and return the current fingerprint.
pub(in crate::app) fn fingerprint() -> Fingerprints {
    let mut out = Fingerprints::new();
    for (_, root) in source_roots() {
        collect(&root, &mut out, 0);
    }
    out
}

/// Difference from the last fingerprint: counts of added / removed / changed (mtime or size moved) files.
///
/// Anything other than 0 means sources changed and a rescan is due.
pub(in crate::app) fn changed_files(previous: &Fingerprints, current: &Fingerprints) -> usize {
    let mut changed = current
        .iter()
        .filter(|(path, stamp)| previous.get(*path) != Some(*stamp))
        .count();
    changed += previous
        .keys()
        .filter(|path| !current.contains_key(*path))
        .count();
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp directory: process id + atomic counter, so parallel tests don't collide.
    fn temp_dir(name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "model-harbor-usage-fp-{}-{}-{}",
            std::process::id(),
            n,
            name
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn collect_records_every_file() {
        let dir = temp_dir("collect");
        std::fs::write(dir.join("a.jsonl"), "one").unwrap();
        std::fs::create_dir_all(dir.join("nested/deep")).unwrap();
        std::fs::write(dir.join("nested/deep/b.jsonl"), "two").unwrap();

        let mut out = Fingerprints::new();
        collect(&dir, &mut out, 0);
        assert_eq!(out.len(), 2);
        assert!(out.keys().any(|p| p.ends_with("a.jsonl")));
        assert!(out.keys().any(|p| p.ends_with("b.jsonl")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn collect_prunes_the_heavy_subtrees() {
        let dir = temp_dir("prune");
        std::fs::write(dir.join("session.jsonl"), "kept").unwrap();
        for pruned in ["snapshot", "log", "plugins", "node_modules"] {
            let sub = dir.join(pruned);
            std::fs::create_dir_all(&sub).unwrap();
            std::fs::write(sub.join("noise.jsonl"), "noise").unwrap();
        }
        // Hidden directories don't enter either.
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git/HEAD"), "ref").unwrap();

        let mut out = Fingerprints::new();
        collect(&dir, &mut out, 0);
        assert_eq!(out.len(), 1, "只应留下会话文件：{out:?}");
        assert!(out.keys().next().unwrap().ends_with("session.jsonl"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Pruning must not hit genuinely session-bearing directory names.
    #[test]
    fn pruning_keeps_the_real_session_directories() {
        for kept in [
            "projects", "sessions", "storage", "message", "part", "agent",
        ] {
            assert!(!is_pruned(kept), "{kept} 是会话目录，不能被剪掉");
        }
        for pruned in ["snapshot", "logs", "binaries", "workspace"] {
            assert!(is_pruned(pruned), "{pruned} 应该被剪掉");
            // Case-insensitive.
            assert!(is_pruned(&pruned.to_uppercase()));
        }
    }

    #[test]
    fn changed_files_detects_append_and_delete() {
        let dir = temp_dir("changed");
        let file = dir.join("s.jsonl");
        std::fs::write(&file, "a").unwrap();
        let before = {
            let mut out = Fingerprints::new();
            collect(&dir, &mut out, 0);
            out
        };
        assert_eq!(changed_files(&before, &before), 0, "同一份指纹应无差异");

        // Appended content: the size changed (mtime's granularity is seconds; size covers same-second writes).
        std::fs::write(&file, "a-much-longer").unwrap();
        let grown = {
            let mut out = Fingerprints::new();
            collect(&dir, &mut out, 0);
            out
        };
        assert_eq!(changed_files(&before, &grown), 1);

        // Deleted files count as changes too.
        std::fs::remove_file(&file).unwrap();
        let removed = {
            let mut out = Fingerprints::new();
            collect(&dir, &mut out, 0);
            out
        };
        assert_eq!(changed_files(&grown, &removed), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn changed_files_counts_new_files() {
        let empty = Fingerprints::new();
        let dir = temp_dir("new");
        std::fs::write(dir.join("fresh.jsonl"), "x").unwrap();
        let mut after = Fingerprints::new();
        collect(&dir, &mut after, 0);
        assert_eq!(changed_files(&empty, &after), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn collect_stops_at_the_depth_limit() {
        let dir = temp_dir("depth");
        let mut deep = dir.clone();
        for level in 0..(MAX_DEPTH + 3) {
            deep = deep.join(format!("d{level}"));
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("too-deep.jsonl"), "x").unwrap();

        let mut out = Fingerprints::new();
        collect(&dir, &mut out, 0);
        assert!(out.is_empty(), "超过深度上限的目录不应被收录");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_root_is_not_an_error() {
        let mut out = Fingerprints::new();
        collect(Path::new("definitely/not/a/real/dir"), &mut out, 0);
        assert!(out.is_empty());
    }

    /// **The gatekeeper test for new agents**: every backend must have an entry in [`SOURCES`], or be explicitly
    /// listed in the "located via environment variable" whitelist.
    ///
    /// Without this test, adding an agent would silently break its change detection (numbers update only when
    /// some other reason triggers a rescan) and nobody would notice.
    #[test]
    fn every_backend_has_a_fingerprint_root() {
        // Agents located via environment variables (see [`env_root`]).
        const ENV_ROOTED: &[ConfigFormat] = &[ConfigFormat::DeepSeekHarness];
        for backend in crate::backends::BACKENDS {
            let id = backend.id();
            let covered =
                ENV_ROOTED.contains(&id) || SOURCES.iter().any(|source| source.client == id);
            assert!(
                covered,
                "{} 没有指纹源：请照 tokscale v4.18.0 的扫描目标补进 SOURCES",
                id.label()
            );
        }
    }

    /// The source table must hold no duplicate paths (they'd be scanned for nothing).
    #[test]
    fn roots_are_unique() {
        let mut seen: Vec<&str> = Vec::new();
        for source in SOURCES {
            assert!(
                !seen.contains(&source.relative),
                "{} 重复了",
                source.relative
            );
            seen.push(source.relative);
        }
    }

    /// Every source must be a path relative to home (never absolute or drive-lettered).
    #[test]
    fn roots_are_home_relative() {
        for source in SOURCES {
            let path = Path::new(source.relative);
            assert!(path.is_relative(), "{} 应是相对路径", source.relative);
            assert!(
                !source.relative.contains(':') && !source.relative.contains('\\'),
                "{} 应是 posix 风格、不含盘符",
                source.relative
            );
        }
    }

    /// A single-file source (WorkBuddy's `workbuddy.db`) must be collectible directly.
    #[test]
    fn a_file_source_records_exactly_that_file() {
        let dir = temp_dir("file-source");
        let db = dir.join("workbuddy.db");
        std::fs::write(&db, "sqlite-ish").unwrap();
        // A file beside it that must NOT be collected.
        std::fs::write(dir.join("noise.txt"), "noise").unwrap();

        let mut out = Fingerprints::new();
        collect(&db, &mut out, 0);
        assert_eq!(out.len(), 1);
        assert!(out.contains_key(&db));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Environment-variable roots: the variable wins when set, else the relative path applies.
    #[test]
    fn env_root_follows_the_variable_then_falls_back() {
        let home = Path::new("/home/someone");
        // Reading and writing the same process's environment isn't safe (parallel tests interfere),
        // so only the "fallback" branch is pinned; the set branch rests on `env_root`'s two-line implementation
        // plus type guarantees. A nearly-impossible variable name guarantees the fallback path.
        let fallback = env_root(
            "MODEL_HARBOR_DEFINITELY_UNSET_FOR_TESTS",
            ".dsh/sessions",
            home,
        );
        assert_eq!(fallback, home.join(".dsh/sessions"));
    }

    /// The watched-agent count can't exceed the backend total, and every root carries a valid backend.
    #[test]
    fn watched_agents_stay_within_the_backend_list() {
        let count = watched_agent_count();
        assert!(count <= crate::backends::BACKENDS.len());
        for (client, path) in source_roots() {
            assert!(
                crate::backends::BACKENDS.iter().any(|b| b.id() == client),
                "{} 不是已注册的后端",
                client.label()
            );
            assert!(path.exists(), "只应收存在路径：{}", path.display());
        }
    }

    /// The same path can't be collected twice (it would be scanned for nothing).
    #[test]
    fn source_roots_are_deduplicated() {
        let roots = source_roots();
        for (index, (_, path)) in roots.iter().enumerate() {
            assert!(
                !roots[..index].iter().any(|(_, other)| other == path),
                "{} 重复了",
                path.display()
            );
        }
    }
}
