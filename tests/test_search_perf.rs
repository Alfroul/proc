//! v0.6.0 阶段 4 — 搜索性能 sanity 测试。
//!
//! 验证 `ProcessInfo::name_lower.contains(query_lower)` 在 500 进程规模下
//! 耗时 < 10ms（release mode 通常 < 100µs，10ms 是宽松上限）。这覆盖了
//! `rebuild_sorted_cache` 过滤路径的核心开销，无需启动 App / worker。

use proc::collect::ProcessInfo;
use proc::search::SearchState;
use std::time::Instant;

#[test]
fn name_lower_search_500_processes_under_10ms() {
    let procs: Vec<ProcessInfo> = (0..500)
        .map(|i| {
            let name = format!("proc_{i}.exe");
            let name_arc: std::sync::Arc<str> = std::sync::Arc::from(name.as_str());
            ProcessInfo {
                pid: i,
                name: std::sync::Arc::clone(&name_arc),
                name_lower: std::sync::Arc::from(name_arc.to_lowercase().as_str()),
                ..ProcessInfo::default()
            }
        })
        .collect();

    let mut search = SearchState::new();
    search.active = true;
    search.query = "proc_42".to_string();
    search.query_lower = "proc_42".to_string();

    // 跑 100 次取平均，避免单次抖动。
    const ITERATIONS: u32 = 100;
    let mut total = std::time::Duration::ZERO;
    let mut last_match_count = 0;
    for _ in 0..ITERATIONS {
        let start = Instant::now();
        let matches: Vec<&ProcessInfo> = procs
            .iter()
            .filter(|p| p.name_lower.contains(search.query_lower()))
            .collect();
        total += start.elapsed();
        last_match_count = matches.len();
    }
    let avg = total / ITERATIONS;

    assert_eq!(last_match_count, 11, "proc_42 匹配 proc_42 + proc_420..429");
    assert!(
        avg.as_millis() < 10,
        "avg per-call latency = {:?}, expected < 10ms",
        avg
    );
}
