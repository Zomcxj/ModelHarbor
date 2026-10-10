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

/// 按天分桶的真机冒烟：热力图与区间统计的数据基础。
///
/// 重点验证两件事：
/// 1. `ParsedMessage.date` 真的能填出多天的桶（否则热力图只有一格）；
/// 2. 按天求和的合计与按会话求和的合计**对不上**（差的就是跨天会话被摊开的部分）——
///    对不上才是对的，说明按天分桶真的在按天切，而不是把整会话堆到最后一天。
#[test]
#[ignore = "依赖本机真实数据，手动跑：cargo test -p model_harbor_core --test usage_scan_real -- --ignored --nocapture"]
fn buckets_real_sessions_by_day() {
    let result = model_harbor_core::usage::scan().expect("扫描应成功");
    let daily = &result.daily;
    println!("按天桶数: {}", daily.len());
    assert!(daily.len() > 1, "本机应有多天数据，否则热力图没意义");

    // 逐天打印（只打前 12 天与最后 3 天，免得刷屏）。
    let days: Vec<_> = daily.iter().collect();
    for (index, (date, bucket)) in days.iter().enumerate() {
        if index < 12 || index + 3 >= days.len() {
            println!(
                "  {date}  in={:>12} out={:>10} cache_r={:>13} agents={} models={} sessions={}",
                bucket.totals.input,
                bucket.totals.output,
                bucket.totals.cache_read,
                bucket.by_client.len(),
                bucket.by_model.len(),
                bucket.by_session.len(),
            );
        } else if index == 12 {
            println!("  …（中间 {} 天省略）", days.len() - 15);
        }
    }

    // 每一天的合计都应等于它的三个分解（同一批消息的三个视图）。
    for (date, bucket) in daily {
        let sum_clients: i64 = bucket.by_client.values().map(|t| t.total()).sum();
        let sum_models: i64 = bucket.by_model.values().map(|t| t.total()).sum();
        let sum_sessions: i64 = bucket.by_session.values().map(|t| t.total()).sum();
        assert_eq!(
            bucket.totals.total(),
            sum_clients,
            "{date} 的按 agent 分解应等于合计"
        );
        assert_eq!(
            bucket.totals.total(),
            sum_models,
            "{date} 的按模型分解应等于合计"
        );
        assert_eq!(
            bucket.totals.total(),
            sum_sessions,
            "{date} 的按会话分解应等于合计"
        );
        // 交叉分解的每个 agent 行也应等于该 agent 当天的合计。
        for (client, models) in &bucket.by_client_model {
            let sum: i64 = models.values().map(|t| t.total()).sum();
            assert_eq!(
                sum,
                bucket.by_client[client].total(),
                "{date} 的 {client} 交叉分解应等于该 agent 当天合计"
            );
        }
    }

    // 按天求和 == 按会话求和？跨天会话在两边都应完整计入，所以总量应一致。
    let by_day: i64 = daily.values().map(|b| b.totals.total()).sum();
    let by_session: i64 = result
        .sessions
        .values()
        .map(model_harbor_core::usage::SessionSnapshot::total)
        .sum();
    println!("\n按天合计:   {by_day}");
    println!("按会话合计: {by_session}");
    assert_eq!(by_day, by_session, "两个视角看的是同一批消息，总量必须相等");
}
