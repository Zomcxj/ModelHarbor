//! 扫描源路径 + 文件指纹 —— 增量重扫的「变更检测」。
//!
//! 问题：`usage::scan()` 是全量扫描（release 实测 2.4-2.7s，debug 14.6s），
//! 每 3 秒跑一次会把 CPU 占满。所以先做一次**极轻的指纹比对**，只有源文件真的
//! 变了才发起重扫。
//!
//! ## 为什么源路径写在这张表里
//!
//! 权威的源路径定义在扫描内核（`tokscale_core::ClientId`）里，但那个 crate 是
//! **core 的依赖，不是 app 的**（见 `crates/core/Cargo.toml`），app 层拿不到它。
//! 于是这里维护一张显式表：**路径逐条抄自 tokscale v4.18.0 的 `ClientDef`
//! （`root` + `relative`），不是从 [`ConfigFormat::label`] 拼出来的** —— 两者拼写
//! 有 6/10 不一致（`kilo` vs `kilocode`、`micode` vs `mimocode`、`dsh` vs
//! `deepseek-harness` …），拼字符串必错。
//!
//! [`source_roots`] 的 `every_backend_has_a_fingerprint_root` 单测会在**新增
//! agent 时直接失败**，逼着这张表跟着更新，而不是默默漏掉一个 agent 的变更检测。
//!
//! 漏掉的后果是**降级而非错误**：那个 agent 的新会话不会自动触发重扫，
//! 但下一次因别的原因触发重扫时它照样会被统计到（扫描内核是权威的）。
//!
//! ## 为什么指纹必须是文件的
//!
//! 往会话文件里追加内容**不会**改父目录的 mtime，只看目录会把「正在流式写入的
//! 会话」当成没变化。所以递归列出文件，记每个文件的 `(mtime, size)`。
//!
//! 递归成本由**剪枝**控制：opencode 的数据目录里既有会话库也有 snapshot 的 git
//! 对象和日志（本机实测 1500+ 文件），workbuddy 更是 18000+。剪掉这些**肯定不含
//! 会话账本**的子树后，本机全量指纹扫描从 ~1.1s 降到 **~7ms**。

use crate::format::ConfigFormat;
use crate::prefs;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 递归深度上限：会话文件最深也就「项目/会话/子 agent/run」四层，
/// 留 8 层足够，同时挡住意外的深目录。
const MAX_DEPTH: usize = 8;

/// 指纹扫描最多保留的文件数：防御性上限，避免异常大的目录把内存吃满。
/// 触顶后停止收录（本次指纹不完整 → 下一次比对可能漏报，但不会误报）。
const MAX_FILES: usize = 20_000;

/// 递归时直接跳过的目录名（大小写不敏感）。
///
/// 每个条目都是「肯定不含会话账本」的重目录，名字取自实测的目录树：
/// - opencode：`snapshot`（git 对象）、`log`、`storage/session_diff` 等；
/// - workbuddy：`binaries`、`plugins`、`workspace`、`connectors-marketplace` …
///
/// 只按**目录名**匹配，不按路径片段，所以不会误伤 `projects` / `sessions` /
/// `storage/message` 这类真正装会话的目录。
const PRUNE_DIRS: &[&str] = &[
    // 版本控制 / 快照对象
    "objects",
    "snapshot",
    "worktree",
    "repos",
    "node_modules",
    "target",
    "__pycache__",
    // 日志与审计
    "log",
    "logs",
    "traces",
    "audit-log",
    "shell-snapshots",
    // 缓存与批量数据
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
    // opencode 的 storage 内部目录（只有 message / part 才是账本）
    "session_diff",
    "directory-readme",
    "migration",
    "agent-usage-reminder",
];

/// 指纹：文件路径 → `(mtime 秒, 字节数)`。
pub(in crate::app) type Fingerprints = HashMap<PathBuf, (u64, u64)>;

/// 是否要跳过这个目录。
fn is_pruned(name: &str) -> bool {
    PRUNE_DIRS
        .iter()
        .any(|pruned| name.eq_ignore_ascii_case(pruned))
}

/// 一个扫描源：`client` 是它属于哪个 agent，`relative` 是相对家目录的路径。
///
/// `client` 不是装饰：一个 agent 可能有多处账本（workbuddy 的 `.workbuddy` 与
/// `.workbuddy-ai`），界面要按 agent 去重后才能报出「监视了几个 agent」。
///
/// 路径可能是**目录**（递归收文件）或**单个文件**（直接收这一条）。
struct ScanSource {
    client: ConfigFormat,
    relative: &'static str,
}

/// 逐条抄自 tokscale v4.18.0 的扫描目标（`ClientDef::root` + `relative_path`，
/// 以及 `scan_all_clients_with_scanner_settings` 里额外 push 的那几处）。
///
/// **只列扫描内核真正会读的路径，不列整个 agent 目录。** 这一点是性能的关键：
/// `~/.workbuddy` 整个目录本机有 18000+ 文件（`binaries` / `plugins` / `workspace`
/// 等），而 tokscale 只从它读 `workbuddy.db` 与 `projects/*.jsonl` 两处。
/// 只盯这两处，指纹扫描就从「秒级」降到「几十毫秒」，也不需要靠剪枝去猜。
const SOURCES: &[ScanSource] = &[
    // XdgData 根：`~/.local/share/<relative>`
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
    // Home 根
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
    // WorkBuddy：会话是 `projects/*.jsonl`，另有一份 `workbuddy.db`。
    ScanSource {
        client: ConfigFormat::WorkBuddy,
        relative: ".workbuddy/projects",
    },
    ScanSource {
        client: ConfigFormat::WorkBuddy,
        relative: ".workbuddy/workbuddy.db",
    },
    // WorkBuddy 5.5（“WorkBuddy AI”）把会话与 db 都搬到了 `.workbuddy-ai`。
    ScanSource {
        client: ConfigFormat::WorkBuddy,
        relative: ".workbuddy-ai/projects",
    },
    ScanSource {
        client: ConfigFormat::WorkBuddy,
        relative: ".workbuddy-ai/workbuddy.db",
    },
    // Kimi CLI 与 Kimi Code 各有一套 `wire.jsonl` 目录。
    ScanSource {
        client: ConfigFormat::KimiCode,
        relative: ".kimi/sessions",
    },
    ScanSource {
        client: ConfigFormat::KimiCode,
        relative: ".kimi-code/sessions",
    },
];

/// 需要环境变量（可被用户改到别处）的 agent 根目录。
///
/// tokscale 对这两个用 `PathRoot::EnvVar`：环境变量有值就用它，否则回落到
/// `fallback_relative`。跟着走，否则用户把 home 挪走后指纹就盯错地方了。
fn env_root(var: &str, fallback_relative: &str, home: &Path) -> PathBuf {
    match std::env::var_os(var).filter(|value| !value.is_empty()) {
        Some(custom) => PathBuf::from(custom).join("sessions"),
        None => home.join(fallback_relative),
    }
}

/// 收一条文件记录（目录递归与单文件源共用）。
fn record_file(path: &Path, metadata: &std::fs::Metadata, out: &mut Fingerprints) {
    let mtime_secs = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    out.insert(path.to_path_buf(), (mtime_secs, metadata.len()));
}

/// 递归收集 `root` 下全部文件的指纹。
///
/// 用 [`std::fs::DirEntry::metadata`]：Windows 上它直接复用 `read_dir` 已经取回的
/// 目录项信息，不额外发起 syscall，比 `path.metadata()` 快得多（这正是本函数能每
/// 3 秒跑一次的前提）。文件类型也一并从目录项拿，避免对每个文件再 stat 一次。
///
/// `root` 也可以是一个**文件**（WorkBuddy 的 `workbuddy.db`）：那就只收这一条。
fn collect(root: &Path, out: &mut Fingerprints, depth: usize) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    // 单文件源：直接记它自己，不进目录分支。
    if root.is_file() {
        if let Ok(metadata) = root.metadata() {
            record_file(root, &metadata, out);
        }
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        // 读不到（权限 / 目录被删）就当这个子树没有文件。
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
            // 隐藏目录一律不进（`.git` 之类）；隐藏**文件**要留
            //（有些账本就是点开头的）。
            if name.starts_with('.') || is_pruned(&name) {
                continue;
            }
            collect(&entry.path(), out, depth + 1);
            continue;
        }
        if !file_type.is_file() {
            // 符号链接 / 设备文件：不跟，免得绕进环里。
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        record_file(&entry.path(), &metadata, out);
    }
}

/// 全部扫描源（已存在的才算），带上所属 agent。
///
/// 只收**存在**的路径：不存在的目录 stat 也是白 stat，而且会让指纹里堆一堆
/// 永远不出现的键。代价是「目录今天才被创建」这种情况要等到下一轮才被发现 ——
/// 但那时它已经存在了，下一轮就会收进来。
pub(in crate::app) fn source_roots() -> Vec<(ConfigFormat, PathBuf)> {
    let Some(home) = prefs::home_dir() else {
        return Vec::new();
    };
    let mut roots: Vec<(ConfigFormat, PathBuf)> = Vec::new();
    for source in SOURCES {
        push_unique(&mut roots, source.client, home.join(source.relative));
    }
    // DSH 走 `DSH_HOME`，回落 `.dsh/sessions`。
    push_unique(
        &mut roots,
        ConfigFormat::DeepSeekHarness,
        env_root("DSH_HOME", ".dsh/sessions", &home),
    );
    roots.retain(|(_, path)| path.exists());
    roots
}

/// 去重后加入（保持首次出现的顺序，便于测试与日志阅读）。
fn push_unique(roots: &mut Vec<(ConfigFormat, PathBuf)>, client: ConfigFormat, path: PathBuf) {
    if !roots.iter().any(|(_, existing)| *existing == path) {
        roots.push((client, path));
    }
}

/// 正在监视的 agent 个数（按 agent 去重；界面状态行用）。
pub(in crate::app) fn watched_agent_count() -> usize {
    let mut seen: Vec<ConfigFormat> = Vec::new();
    for (client, _) in source_roots() {
        if !seen.contains(&client) {
            seen.push(client);
        }
    }
    seen.len()
}

/// 扫一遍全部源目录，返回当前指纹。
pub(in crate::app) fn fingerprint() -> Fingerprints {
    let mut out = Fingerprints::new();
    for (_, root) in source_roots() {
        collect(&root, &mut out, 0);
    }
    out
}

/// 与上次指纹的差异：新增 / 删除 / 改动（mtime 或大小变了）的文件数。
///
/// 只要不是 0 就说明源变了，需要重扫。
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

    /// 临时目录：进程 id + 原子计数器，避免并行测试撞目录。
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
        // 隐藏目录也不进。
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git/HEAD"), "ref").unwrap();

        let mut out = Fingerprints::new();
        collect(&dir, &mut out, 0);
        assert_eq!(out.len(), 1, "只应留下会话文件：{out:?}");
        assert!(out.keys().next().unwrap().ends_with("session.jsonl"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 剪枝不能误伤真正装会话的目录名。
    #[test]
    fn pruning_keeps_the_real_session_directories() {
        for kept in [
            "projects", "sessions", "storage", "message", "part", "agent",
        ] {
            assert!(!is_pruned(kept), "{kept} 是会话目录，不能被剪掉");
        }
        for pruned in ["snapshot", "logs", "binaries", "workspace"] {
            assert!(is_pruned(pruned), "{pruned} 应该被剪掉");
            // 大小写不敏感。
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

        // 追加内容：大小变了（mtime 精度是秒，靠 size 兜住同一秒内的写入）。
        std::fs::write(&file, "a-much-longer").unwrap();
        let grown = {
            let mut out = Fingerprints::new();
            collect(&dir, &mut out, 0);
            out
        };
        assert_eq!(changed_files(&before, &grown), 1);

        // 删除文件也算变化。
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

    /// **新增 agent 时的守门测试**：每个后端都必须在 [`SOURCES`] 里有一条，
    /// 或者被明确列进「用环境变量定位」的白名单。
    ///
    /// 没有这条测试，加一个 agent 之后它的变更检测会静默失效（数字只在
    /// 别的原因触发重扫时才更新），而且没人会发现。
    #[test]
    fn every_backend_has_a_fingerprint_root() {
        // 用环境变量定位的 agent（见 [`env_root`]）。
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

    /// 源表里不能有重复路径（会白扫一遍）。
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

    /// 每个源都必须是相对家目录的相对路径（不能是绝对路径或带盘符）。
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

    /// 单个文件源（WorkBuddy 的 `workbuddy.db`）要能直接被收进来。
    #[test]
    fn a_file_source_records_exactly_that_file() {
        let dir = temp_dir("file-source");
        let db = dir.join("workbuddy.db");
        std::fs::write(&db, "sqlite-ish").unwrap();
        // 旁边放一个不应被收进来的文件。
        std::fs::write(dir.join("noise.txt"), "noise").unwrap();

        let mut out = Fingerprints::new();
        collect(&db, &mut out, 0);
        assert_eq!(out.len(), 1);
        assert!(out.contains_key(&db));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 环境变量根：有值时用它，没值时回落相对路径。
    #[test]
    fn env_root_follows_the_variable_then_falls_back() {
        let home = Path::new("/home/someone");
        // 用同一个进程内的环境变量读写不安全（并行测试会互相干扰），
        // 所以只钉住「回落」分支；有值分支靠 `env_root` 的两行实现 + 类型保证。
        // 用一个几乎不可能存在的变量名，确保走回落。
        let fallback = env_root(
            "MODEL_HARBOR_DEFINITELY_UNSET_FOR_TESTS",
            ".dsh/sessions",
            home,
        );
        assert_eq!(fallback, home.join(".dsh/sessions"));
    }

    /// 监视的 agent 数不可能超过后端总数，且每条根目录都带一个合法后端。
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

    /// 同一路径不能重复收（否则白扫一遍）。
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
