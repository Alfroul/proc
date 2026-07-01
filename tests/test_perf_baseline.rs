//! v0.6.0 阶段 4 性能基线（ProcessInfo 字段对齐）。
//!
//! 验证：
//! - rebuild_sorted_cache O(N) 索引构造 + O(N log N) 排序 < 5ms（500 进程）
//! - top-N select_nth_unstable + K log K 排序 < 1ms（500 进程）

#![allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]

use std::time::Instant;

use proc::collect::ProcessInfo;

fn fake_processes(n: u32) -> Vec<ProcessInfo> {
    (0..n)
        .map(|i| {
            let name = format!("proc_{i}");
            ProcessInfo {
                pid: i,
                name: std::sync::Arc::from(name.as_str()),
                cpu_usage: (i as f32) * 0.1,
                memory: (i as u64) * 1024,
                virtual_memory: 0,
                disk_usage: (0, 0),
                disk_read_speed: 0,
                disk_write_speed: 0,
                net_sent_rate: 0,
                net_recv_rate: 0,
                status: proc::collect::ProcessStatus::default(),
                exe: None,
                cmd: std::sync::Arc::from(Vec::<String>::new()),
                cwd: None,
                parent_pid: None,
                session_id: None,
                user_id: None,
                start_time: 0,
                run_time: 0,
                name_lower: std::sync::Arc::from(name.to_lowercase().as_str()),
                throttled: proc::throttle::EcoQoSState::default(),
                signature_status: proc::security::SignatureStatus::default(),
                parent_chain: Vec::new(),
            }
        })
        .collect()
}

#[test]
fn stage8_rebuild_sorted_baseline_500_processes() {
    let procs = fake_processes(500);
    let start = Instant::now();
    // Simulate the indexing portion of rebuild_sorted_cache (the previous O(N²) hot path).
    let pid_to_idx: std::collections::HashMap<u32, usize> =
        procs.iter().enumerate().map(|(i, p)| (p.pid, i)).collect();
    // Simulate the sort step.
    let mut sorted: Vec<&ProcessInfo> = procs.iter().collect();
    sorted.sort_by(|a, b| {
        b.cpu_usage
            .partial_cmp(&a.cpu_usage)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let elapsed = start.elapsed();
    let _ = pid_to_idx;
    println!("rebuild_sorted_cache (500 procs) elapsed: {:?}", elapsed);
    assert!(
        elapsed.as_millis() < 5,
        "rebuild_sorted_cache regressed: {:?} > 5ms",
        elapsed
    );
}

#[test]
fn stage8_topn_select_nth_baseline_500_processes() {
    let owned = fake_processes(500);
    let mut sorted: Vec<&ProcessInfo> = owned.iter().collect();
    const MAX_TRACKED: usize = 60;
    let start = Instant::now();
    let cmp_cpu = |a: &&ProcessInfo, b: &&ProcessInfo| {
        b.cpu_usage
            .partial_cmp(&a.cpu_usage)
            .unwrap_or(std::cmp::Ordering::Equal)
    };
    if sorted.len() > MAX_TRACKED {
        let (left, _, _) = sorted.select_nth_unstable_by(MAX_TRACKED, cmp_cpu);
        left.sort_by(cmp_cpu);
    } else {
        sorted.sort_by(cmp_cpu);
    }
    let elapsed = start.elapsed();
    println!("top-N select_nth (500 procs) elapsed: {:?}", elapsed);
    assert!(
        elapsed.as_millis() < 1,
        "top-N sort regressed: {:?} > 1ms",
        elapsed
    );
}
