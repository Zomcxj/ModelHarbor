use crate::app::diff::{diff_hunks, DiffSummary, LineKind};
use crate::app::preview::diff_signature;

/// The diff view shows exactly "what one save click would change":
/// an opencode config walks the full "original file → edited draft" chain.
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

/// A cross-format save replaces the target file's provider container wholesale; the diff must faithfully
/// show "the old container vanishing, the new one appearing".
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

/// When the target file doesn't exist (an empty disk side), the whole document counts as inserted.
#[test]
fn a_file_that_does_not_exist_yet_reads_as_all_new() {
    let pending = "{\n  \"provider\": {}\n}";
    let (_, summary) = diff_hunks("", pending, 3);
    assert_eq!(summary.removed, 0, "空文件没有可删的行");
    assert_eq!(summary.added, 3, "整份文档都算新增");
}

/// The diff cache signature includes the path: same-named drafts of different files never share cache results.
#[test]
fn switching_pages_invalidates_the_cache_via_the_path() {
    let draft = "{\n  \"provider\": {}\n}";
    assert_ne!(
        diff_signature("/home/u/.config/opencode/opencode.json", draft, 0),
        diff_signature("/home/u/.config/kilo/kilo.json", draft, 0),
        "目标文件不同，签名必须不同"
    );
}
