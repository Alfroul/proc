//! v0.13 阶段 1：搜索 + 排序 hot path benchmark。
//!
//! 测 `src/app.rs::App::rebuild_sorted_cache` 核心算法（pid_to_idx 构建 +
//! filter + sort）。**不调 App**——App 紧耦合 SystemSnapshot，benchmark
//! 无法构造。本 bench 抽取核心算法到独立函数，与生产逻辑结构对齐：
//!
//! 1. **filter**：按 `QueryMode::Substring`（name_lower.contains）或
//!    `QueryMode::FilterExpr`（FilterExpr::apply）筛进程。
//! 2. **pid_to_idx**：HashMap 一次性建索引，避免 N 次 O(N) position 查找。
//! 3. **sort**：按 SortField 排序，name 路径走预计算 name_lower 不重算 lowercase。
//!
//! 3 档 fixture：100 / 500 / 1000 进程（与 stage doc 任务 4.1 一致）。
//!
//! 对照 v0.6 stage 6 实测（500 进程）：rebuild_sorted_cache 38.2 µs / top-N 6.1 µs。

mod common;

use std::collections::HashMap;

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use proc::classify;
use proc::collect::{ProcessInfo, SortField};
use proc::filter::{EvalCtx, FilterExpr};
use proc::security::score::SecurityScore;
use proc::security::signature::SignatureStatus;

use common::{make_filter_expr, make_processes};

/// 抽取 rebuild_sorted_cache 核心算法（Substring 路径）。
/// 与 src/app.rs:1805-1920 的生产逻辑结构一致，但不依赖 App / SystemSnapshot。
fn rebuild_substring(
    processes: &[ProcessInfo],
    query_lower: &str,
    query: &str,
    sort_field: SortField,
) -> Vec<(usize, classify::ProcessClass)> {
    let filtered: Vec<&ProcessInfo> = if query.is_empty() {
        processes.iter().collect()
    } else {
        processes
            .iter()
            .filter(|p| p.name_lower.contains(query_lower) || p.pid.to_string().contains(query))
            .collect()
    };

    let pid_to_idx: HashMap<u32, usize> = processes
        .iter()
        .enumerate()
        .map(|(i, p)| (p.pid, i))
        .collect();

    let mut result: Vec<(classify::ProcessClass, &ProcessInfo)> = filtered
        .into_iter()
        .map(|p| (classify::classify_process(p), p))
        .collect();

    if matches!(sort_field, SortField::Name) {
        let mut keyed: Vec<(std::sync::Arc<str>, classify::ProcessClass, &ProcessInfo)> = result
            .into_iter()
            .map(|(class, p)| (std::sync::Arc::clone(&p.name_lower), class, p))
            .collect();
        keyed.sort_by(|a, b| a.0.cmp(&b.0).then(a.2.pid.cmp(&b.2.pid)));
        return keyed
            .iter()
            .map(|(_, class, p)| (*pid_to_idx.get(&p.pid).unwrap_or(&0), *class))
            .collect();
    }

    result.sort_by(|a, b| match sort_field {
        SortField::Cpu => {
            b.1.cpu_usage
                .partial_cmp(&a.1.cpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.pid.cmp(&b.1.pid))
        }
        SortField::Memory => b.1.memory.cmp(&a.1.memory).then(a.1.pid.cmp(&b.1.pid)),
        SortField::Pid => a.1.pid.cmp(&b.1.pid),
        SortField::Name => unreachable!("Name 路径在 sort_field 分支前已处理"),
        SortField::Security => std::cmp::Ordering::Equal,
        SortField::DiskRead => {
            b.1.disk_read_speed
                .cmp(&a.1.disk_read_speed)
                .then(a.1.pid.cmp(&b.1.pid))
        }
        SortField::DiskWrite => {
            b.1.disk_write_speed
                .cmp(&a.1.disk_write_speed)
                .then(a.1.pid.cmp(&b.1.pid))
        }
        SortField::NetSent => {
            b.1.net_sent_rate
                .cmp(&a.1.net_sent_rate)
                .then(a.1.pid.cmp(&b.1.pid))
        }
        SortField::NetRecv => {
            b.1.net_recv_rate
                .cmp(&a.1.net_recv_rate)
                .then(a.1.pid.cmp(&b.1.pid))
        }
    });

    result
        .iter()
        .map(|(class, p)| (*pid_to_idx.get(&p.pid).unwrap_or(&0), *class))
        .collect()
}

/// 抽取 rebuild_sorted_cache 核心算法（FilterExpr 路径）。
fn rebuild_filter_expr(
    processes: &[ProcessInfo],
    expr: &FilterExpr,
    security_scores: &HashMap<u32, SecurityScore>,
    total_memory: u64,
    sort_field: SortField,
) -> Vec<(usize, classify::ProcessClass)> {
    let filtered: Vec<&ProcessInfo> = processes
        .iter()
        .filter(|p| {
            let score = security_scores.get(&p.pid).map(|s| s.score);
            let ctx = EvalCtx {
                process: p,
                security_score: score,
                total_memory,
            };
            expr.apply(&ctx)
        })
        .collect();

    let pid_to_idx: HashMap<u32, usize> = processes
        .iter()
        .enumerate()
        .map(|(i, p)| (p.pid, i))
        .collect();

    let mut result: Vec<(classify::ProcessClass, &ProcessInfo)> = filtered
        .into_iter()
        .map(|p| (classify::classify_process(p), p))
        .collect();

    result.sort_by(|a, b| match sort_field {
        SortField::Cpu => {
            b.1.cpu_usage
                .partial_cmp(&a.1.cpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.pid.cmp(&b.1.pid))
        }
        SortField::Memory => b.1.memory.cmp(&a.1.memory).then(a.1.pid.cmp(&b.1.pid)),
        SortField::Pid => a.1.pid.cmp(&b.1.pid),
        SortField::Name => {
            a.1.name_lower
                .cmp(&b.1.name_lower)
                .then(a.1.pid.cmp(&b.1.pid))
        }
        SortField::Security => {
            let sa = security_scores
                .get(&a.1.pid)
                .map(|s| s.score)
                .unwrap_or(100);
            let sb = security_scores
                .get(&b.1.pid)
                .map(|s| s.score)
                .unwrap_or(100);
            sa.cmp(&sb).then(a.1.pid.cmp(&b.1.pid))
        }
        SortField::DiskRead => {
            b.1.disk_read_speed
                .cmp(&a.1.disk_read_speed)
                .then(a.1.pid.cmp(&b.1.pid))
        }
        SortField::DiskWrite => {
            b.1.disk_write_speed
                .cmp(&a.1.disk_write_speed)
                .then(a.1.pid.cmp(&b.1.pid))
        }
        SortField::NetSent => {
            b.1.net_sent_rate
                .cmp(&a.1.net_sent_rate)
                .then(a.1.pid.cmp(&b.1.pid))
        }
        SortField::NetRecv => {
            b.1.net_recv_rate
                .cmp(&a.1.net_recv_rate)
                .then(a.1.pid.cmp(&b.1.pid))
        }
    });

    result
        .iter()
        .map(|(class, p)| (*pid_to_idx.get(&p.pid).unwrap_or(&0), *class))
        .collect()
}

/// 构造 fake security scores：让 pid % 10 == 0 的进程是 50 分（命中
/// `security_score < 80`），其余 100 分（不命中）。
fn fake_security_scores(processes: &[ProcessInfo]) -> HashMap<u32, SecurityScore> {
    processes
        .iter()
        .map(|p| {
            let score = if p.pid % 10 == 0 { 50 } else { 100 };
            (
                p.pid,
                SecurityScore {
                    score,
                    factors: Vec::new(),
                    signature: SignatureStatus::default(),
                },
            )
        })
        .collect()
}

fn bench_rebuild_sorted_cache(c: &mut Criterion) {
    let sizes = [100_usize, 500, 1000];

    // Substring 路径：query "chrome" 命中约 1/5 进程。
    {
        let mut group = c.benchmark_group("rebuild_sorted_cache_substring");
        for &size in &sizes {
            let processes = make_processes(size);
            group.throughput(Throughput::Elements(size as u64));
            group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
                b.iter(|| {
                    let result = rebuild_substring(
                        black_box(&processes),
                        black_box("chrome"),
                        black_box("chrome"),
                        black_box(SortField::Cpu),
                    );
                    black_box(result);
                });
            });
        }
        group.finish();
    }

    // FilterExpr 路径：`cpu > 5 AND name =~ /chrome/`。
    {
        let mut group = c.benchmark_group("rebuild_sorted_cache_filter_expr");
        let expr = make_filter_expr();
        for &size in &sizes {
            let processes = make_processes(size);
            let scores = fake_security_scores(&processes);
            let total_memory = 16 * 1024 * 1024 * 1024;
            group.throughput(Throughput::Elements(size as u64));
            group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
                b.iter(|| {
                    let result = rebuild_filter_expr(
                        black_box(&processes),
                        black_box(&expr),
                        black_box(&scores),
                        black_box(total_memory),
                        black_box(SortField::Cpu),
                    );
                    black_box(result);
                });
            });
        }
        group.finish();
    }
}

criterion_group!(benches, bench_rebuild_sorted_cache);
criterion_main!(benches);
