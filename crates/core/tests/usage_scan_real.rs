//! A real-machine scan smoke test: verifying tokscale-core can genuinely read this machine's data.
//!
//! Tagged `#[ignore]`: it depends on agents actually installed locally and real session files — it would
//! fail on CI.

#[test]
#[ignore = "依赖本机真实数据，手动跑：cargo test -p model_harbor_core --test usage_scan_real -- --ignored --nocapture"]
fn scans_real_local_sessions() {
    let result =
        model_harbor_core::usage::scan(&std::collections::BTreeSet::new()).expect("扫描应成功");
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

/// Daily buckets' real-machine smoke test: the data foundation for the heatmap and range stats.
///
/// Two things to verify:
/// 1. `ParsedMessage.date` genuinely fills multi-day buckets (otherwise the heatmap has one cell);
/// 2. The day-summed total and the session-summed total **disagree** (the difference is exactly the
///    multi-day sessions spread across days) — the disagreement is correct: it proves the daily buckets
///    really split by day rather than piling whole sessions onto their last day.
#[test]
#[ignore = "依赖本机真实数据，手动跑：cargo test -p model_harbor_core --test usage_scan_real -- --ignored --nocapture"]
fn buckets_real_sessions_by_day() {
    let result =
        model_harbor_core::usage::scan(&std::collections::BTreeSet::new()).expect("扫描应成功");
    let daily = &result.daily;
    println!("按天桶数: {}", daily.len());
    assert!(daily.len() > 1, "本机应有多天数据，否则热力图没意义");

    // Print day by day (only the first 12 and last 3 days, avoiding a wall of output).
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

    // Every day's total must equal its three breakdowns (three views of the same messages).
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
        // Each agent row of the cross-breakdown must also equal that agent's total for the day.
        for (client, models) in &bucket.by_client_model {
            let sum: i64 = models.values().map(|t| t.total()).sum();
            assert_eq!(
                sum,
                bucket.by_client[client].total(),
                "{date} 的 {client} 交叉分解应等于该 agent 当天合计"
            );
        }
    }

    // Day-summed == session-summed? Multi-day sessions count fully on both sides, so totals should agree.
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
