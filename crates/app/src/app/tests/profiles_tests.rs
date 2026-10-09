//! 保存写盘前的自动快照（Auto tag）与写盘成功后的保留策略清理。
//!
//! 快照目录通过 `App.snapshots_root` 注入 tempdir，不碰真实用户目录。

use crate::app::App;
use crate::format::ConfigFormat;
use std::path::{Path, PathBuf};

fn temp_dir(tag: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "model_harbor_profiles_{}_{}_{}",
        tag,
        std::process::id(),
        nonce
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 备份目录里以指定后缀（`.auto.bak` 等）结尾的文件数。
fn count_backups(backups: &Path, suffix: &str) -> usize {
    std::fs::read_dir(backups)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.ends_with(suffix))
                })
                .count()
        })
        .unwrap_or(0)
}

/// 指向 tempdir 配置文件、快照目录注入 tempdir 的 App。
fn app_for(file: &Path, backups: &Path) -> App {
    App {
        config_path: file.display().to_string(),
        loaded_path: file.display().to_string(),
        source_format: ConfigFormat::Pi,
        current_page: ConfigFormat::Pi,
        snapshots_root: Some(backups.to_path_buf()),
        ..App::default()
    }
}

#[test]
fn save_snapshots_externally_changed_file_before_overwriting_it() {
    let dir = temp_dir("snapshot");
    let backups = dir.join("backups");
    let file = dir.join("pi-models.json");
    std::fs::write(&file, "{\"v\": 1}").unwrap();
    let mut app = app_for(&file, &backups);
    let path = file.display().to_string();

    // 第一次保存：本会话从没快照过，盘上旧内容先留底再覆盖。
    app.save_backend_to(ConfigFormat::Pi, &path).expect("保存");
    assert_eq!(
        count_backups(&backups, ".auto.bak"),
        1,
        "写盘前生成 Auto 快照"
    );

    // 内容没变（磁盘上就是上次自己写的）：再保存不重复快照。
    app.save_backend_to(ConfigFormat::Pi, &path).expect("保存");
    assert_eq!(
        count_backups(&backups, ".auto.bak"),
        1,
        "hash 没变不重复快照"
    );

    // 外部改了文件：下次保存先把外部改动留底。
    std::fs::write(&file, "{\"v\": 2}").unwrap();
    app.save_backend_to(ConfigFormat::Pi, &path).expect("保存");
    assert_eq!(
        count_backups(&backups, ".auto.bak"),
        2,
        "外部改动在覆盖前留底"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn save_prunes_auto_backups_with_the_default_policy() {
    let dir = temp_dir("prune");
    let backups = dir.join("backups");
    std::fs::create_dir_all(&backups).unwrap();
    let file = dir.join("pi-models.json");
    std::fs::write(&file, "{\"v\": 1}").unwrap();

    // 预置 22 份更旧的 Auto 备份：加上本次快照共 23 份，默认策略留 20 份。
    for i in 0..22u32 {
        let name = format!("pi-models.json.20240101_0000{i:02}.auto.bak");
        std::fs::write(backups.join(name), "x").unwrap();
    }

    let mut app = app_for(&file, &backups);
    app.save_backend_to(ConfigFormat::Pi, &file.display().to_string())
        .expect("保存");

    assert_eq!(
        count_backups(&backups, ".auto.bak"),
        20,
        "写盘成功后按默认策略清理（Auto 留最近 20 份）"
    );
    std::fs::remove_dir_all(&dir).ok();
}
