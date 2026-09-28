use crate::app::diff::{diff_hunks, DiffSummary, LineKind};
use crate::app::preview::diff_signature;

/// 对比视图的核心语义：显示的就是「按一下保存会改掉什么」。
///
/// 这里用一份 opencode 配置走完「原文件 → 改过的草稿」的完整链路，
/// 断言新增/删除的行数落在用户真正改过的那几处，而不是整份文件。
#[test]
fn a_one_line_change_shows_up_as_one_add_and_one_remove() {
    let original = r#"{
  "provider": {
    "p1": { "options": { "baseURL": "https://old.example/v1" } }
  }
}"#;
    let edited = original.replace("old.example", "new.example");
    let (lines, summary) = diff_hunks(original, &edited, 3);
    assert_eq!(
        summary,
        DiffSummary {
            added: 1,
            removed: 1
        },
        "只改了一行 baseURL，不该报成整份文件重写"
    );
    let removed: Vec<&str> = lines
        .iter()
        .filter(|l| l.kind == LineKind::Removed)
        .map(|l| l.text.as_str())
        .collect();
    assert_eq!(
        removed,
        vec!["    \"p1\": { \"options\": { \"baseURL\": \"https://old.example/v1\" } }"]
    );
}

/// 跨格式保存会把目标文件的 provider 容器整段换成界面里的那份：
/// 对比必须把「旧容器消失、新容器出现」如实显示出来，不能只说一句「有改动」。
#[test]
fn replacing_a_container_shows_both_the_old_and_the_new_entries() {
    let on_disk = r#"{
  "provider": {
    "keep": { "npm": "@ai-sdk/openai" },
    "gone": { "npm": "@ai-sdk/anthropic" }
  }
}"#;
    let pending = r#"{
  "provider": {
    "keep": { "npm": "@ai-sdk/openai" },
    "added": { "npm": "@ai-sdk/google" }
  }
}"#;
    let (lines, summary) = diff_hunks(on_disk, pending, 3);
    assert_eq!(
        summary,
        DiffSummary {
            added: 1,
            removed: 1
        }
    );
    let removed: Vec<&str> = lines
        .iter()
        .filter(|l| l.kind == LineKind::Removed)
        .map(|l| l.text.as_str())
        .collect();
    let added: Vec<&str> = lines
        .iter()
        .filter(|l| l.kind == LineKind::Added)
        .map(|l| l.text.as_str())
        .collect();
    assert!(
        removed.iter().any(|l| l.contains("gone")),
        "被删的 provider 要看得见"
    );
    assert!(
        added.iter().any(|l| l.contains("added")),
        "新增的 provider 要看得见"
    );
    assert!(
        !removed.iter().any(|l| l.contains("keep")),
        "两侧都有的 provider 不该被算成改动"
    );
}

/// 新建文件（目标不存在 → 磁盘侧为空）时，整份文档都是新增。
#[test]
fn a_file_that_does_not_exist_yet_reads_as_all_new() {
    let pending = "{\n  \"provider\": {}\n}";
    let (_, summary) = diff_hunks("", pending, 3);
    assert_eq!(summary.removed, 0, "空文件没有可删的行");
    assert_eq!(summary.added, 3, "整份文档都算新增");
}

/// 对比缓存签名必须把路径也算进去：切页会换目标文件，不同文件的同名草稿
/// 不能共用一份缓存结果。
#[test]
fn switching_pages_invalidates_the_cache_via_the_path() {
    let draft = "{\n  \"provider\": {}\n}";
    assert_ne!(
        diff_signature("/home/u/.config/opencode/opencode.json", draft, 0),
        diff_signature("/home/u/.config/kilo/kilo.json", draft, 0),
        "目标文件不同，签名必须不同"
    );
}
