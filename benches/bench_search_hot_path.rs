//! v0.13 阶段 1：搜索按键 → rebuild_sorted_cache 全链路 benchmark。
//!
//! stage doc 任务 4.6 关注点：搜索按键到 rebuild 的全链路开销，
//! 验证 v0.6 stage 6 增量 lowercase 优化在 query 长度 1 / 10 / 50 下的开销。
//!
//! 流程：query 输入 → query_lower 计算 → name_lower.contains → 过滤。
//! 本 bench 不调真实 SearchState（它是状态ful的），直接测底层算法。

mod common;

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};

use common::make_processes;

fn bench_search_hot_path(c: &mut Criterion) {
    let n = 500_usize;
    let processes = make_processes(n);

    // 3 档 query 长度：1 / 10 / 50
    let queries: &[(&str, &str)] = &[
        ("len_1", "c"),
        ("len_10", "cccccccccc"),        // 10 字符全部命中 chrome 前缀 'c'
        ("len_50", &"c".repeat(50)[..]), // 50 字符（不会命中，测长 query 开销）
    ];

    let mut group = c.benchmark_group("search_substring_filter");
    group.throughput(Throughput::Elements(n as u64));
    for &(label, raw_query) in queries {
        group.bench_with_input(
            BenchmarkId::from_parameter(label),
            &raw_query,
            |b, raw_query| {
                b.iter(|| {
                    // 模拟 SearchState::handle_input 路径：
                    // 1. query.to_lowercase() （query_lower 增量在 ASCII 下 O(1)，
                    //    这里测整体重算——保守上界）
                    let q_lower = raw_query.to_lowercase();
                    // 2. filter: name_lower.contains(q_lower) || pid.contains(query)
                    let count = processes
                        .iter()
                        .filter(|p| {
                            p.name_lower.contains(black_box(&q_lower))
                                || p.pid.to_string().contains(black_box(raw_query))
                        })
                        .count();
                    black_box(count);
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_search_hot_path);
criterion_main!(benches);
