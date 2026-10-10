//! 真机扫描冒烟测试：验证 tokscale-core 能真的读出本机数据。
//!
//! 标 `#[ignore]`：依赖本机已安装的 agent 与真实会话文件，CI 上跑会失败。

#[test]
#[ignore = "依赖本机真实数据，手动跑：cargo test -p model_harbor_core --test usage_scan_real -- --ignored --nocapture"]
fn scans_real_local_sessions() {
    let result = model_harbor_core::usage::scan().expect("扫描应成功");
    println!("消息数: {}", result.message_count);
    println!("会话数: {}", result.sessions.len());
    println!("内核耗时: {}ms", result.processing_time_ms);

    let mut by_client: std::collections::HashMap<_, (i64, i64, i64, usize)> =
        std::collections::HashMap::new();
    for snapshot in result.sessions.values() {
        let entry = by_client.entry(snapshot.client.label()).or_default();
        entry.0 += snapshot.input;
        entry.1 += snapshot.output;
        entry.2 += snapshot.cache_read;
        entry.3 += 1;
    }
    let mut rows: Vec<_> = by_client.into_iter().collect();
    rows.sort_by_key(|(_, v)| -(v.0 + v.2));
    for (client, (input, output, cache_read, sessions)) in &rows {
        println!(
            "  {client:<18} in={input:>12} out={output:>10} cache_r={cache_read:>13} sessions={sessions}"
        );
    }
    assert!(!result.sessions.is_empty(), "本机应至少有一个 agent 有数据");
}
