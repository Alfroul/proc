//! v0.13 阶段 1：HeavyWorker 单轮 hot path benchmark。
//!
//! 测 HeavyWorker 单轮「Arc clone + parent_chain 批量 + 写回」链路
//! （`src/collect.rs::HeavyWorker::start` 中的 worker body 段）。
//! **不调真实 sysinfo**——sysinfo 是 IO + 系统调用，依赖运行机器进程数；
//! mock 一组 fake ProcessInfo 后跑可重现的 CPU-bound 部分。
//!
//! 关注点（stage doc 任务 4.2）：
//! - 每周期堆分配次数（parent_chain Vec clone 是已知疑点 5）
//! - 批量 build_parent_chain 性能（v0.11 阶段 5 落地）
//!
//! v0.17 stage 2 TD-47：parent_chain 改 `Vec<(u32, Arc<str>)>` 后，
//! build_parent_chain body 用 `Arc::clone` 替换 `String::to_string`，预期
//! alloc 数字下降 ~3x（仅 Vec 自身分配，元素字符串走 Arc refcount 共享）。
//!
//! 3 档 fixture：100 / 500 / 1000 进程。

mod common;

use std::collections::HashMap;

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use proc::collect::ProcessInfo;
use proc::security::lineage::build_parent_chain;

use common::make_processes_map;

/// 模拟 HeavyWorker 单轮 hot path 的「parent_chain 批量 + 写回」段。
///
/// 与 src/collect.rs:949-966 一致：先 collect 所有 chain 到独立 HashMap
/// （绕开 Rust 借用规则），再 iter_mut 写入 ProcessInfo.parent_chain。
fn heavy_parent_chain_pass(
    processes: &mut HashMap<u32, ProcessInfo>,
) -> HashMap<u32, Vec<(u32, std::sync::Arc<str>)>> {
    let pid_to_chain: HashMap<u32, Vec<(u32, std::sync::Arc<str>)>> = processes
        .keys()
        .map(|&pid| (pid, build_parent_chain(pid, processes)))
        .collect();
    for (pid, proc) in processes.iter_mut() {
        if let Some(chain) = pid_to_chain.get(pid) {
            proc.parent_chain = chain.clone();
        }
    }
    pid_to_chain
}

fn bench_refresh_heavy(c: &mut Criterion) {
    let sizes = [100_usize, 500, 1000];

    let mut group = c.benchmark_group("refresh_heavy_parent_chain");
    for &size in &sizes {
        let base = make_processes_map(size);
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &base, |b, base| {
            // Each iter clones the base map so mutations don't accumulate.
            b.iter_batched(
                || base.clone(),
                |mut processes| {
                    let chains = heavy_parent_chain_pass(black_box(&mut processes));
                    black_box(chains);
                    black_box(processes);
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(benches, bench_refresh_heavy);
criterion_main!(benches);
