//! v0.13 阶段 1：FilterExpr apply benchmark。
//!
//! 测 `FilterExpr::apply(&EvalCtx)` / `apply_network(&NetworkEvalCtx)` 各种
//! 表达式（stage doc 任务 4.5）：
//! - (a) `cpu > 5`：单 field 比较
//! - (b) `name =~ /chrome/i`：regex
//! - (c) `cpu > 5 AND mem > 100mb`：复合
//! - (d) `sni in ("a", "b", ...)`：HashSet lookup（v0.12 阶段 5 TD-29 落地）
//!
//! fixture：500 进程 / 500 flows。

mod common;

use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use proc::filter::{EvalCtx, parse};
use proc::security::score::SecurityScore;
use proc::security::signature::SignatureStatus;

use common::{make_flows, make_processes, network_eval_ctx};

fn bench_filter_expr_apply(c: &mut Criterion) {
    let n = 500_usize;
    let processes = make_processes(n);
    let total_memory = 16 * 1024 * 1024 * 1024_u64;

    // fake security scores（与 bench_rebuild_sorted_cache 同款）
    let scores: std::collections::HashMap<u32, SecurityScore> = processes
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
        .collect();

    // (a) cpu > 5
    let expr_a = parse("cpu > 5").expect("parse");
    {
        let mut group = c.benchmark_group("filter_expr_apply_cpu_gt");
        group.throughput(Throughput::Elements(n as u64));
        group.bench_function("500_processes", |b| {
            b.iter(|| {
                let count = processes
                    .iter()
                    .filter(|p| {
                        let ctx = EvalCtx {
                            process: p,
                            security_score: scores.get(&p.pid).map(|s| s.score),
                            total_memory,
                        };
                        expr_a.apply(black_box(&ctx))
                    })
                    .count();
                black_box(count);
            });
        });
        group.finish();
    }

    // (b) name =~ /chrome/i
    let expr_b = parse("name =~ /chrome/i").expect("parse");
    {
        let mut group = c.benchmark_group("filter_expr_apply_name_regex");
        group.throughput(Throughput::Elements(n as u64));
        group.bench_function("500_processes", |b| {
            b.iter(|| {
                let count = processes
                    .iter()
                    .filter(|p| {
                        let ctx = EvalCtx {
                            process: p,
                            security_score: scores.get(&p.pid).map(|s| s.score),
                            total_memory,
                        };
                        expr_b.apply(black_box(&ctx))
                    })
                    .count();
                black_box(count);
            });
        });
        group.finish();
    }

    // (c) cpu > 5 AND mem > 100mb
    let expr_c = parse("cpu > 5 AND mem > 100mb").expect("parse");
    {
        let mut group = c.benchmark_group("filter_expr_apply_complex");
        group.throughput(Throughput::Elements(n as u64));
        group.bench_function("500_processes", |b| {
            b.iter(|| {
                let count = processes
                    .iter()
                    .filter(|p| {
                        let ctx = EvalCtx {
                            process: p,
                            security_score: scores.get(&p.pid).map(|s| s.score),
                            total_memory,
                        };
                        expr_c.apply(black_box(&ctx))
                    })
                    .count();
                black_box(count);
            });
        });
        group.finish();
    }

    // (d) sni in (...) — HashSet lookup 路径
    let flows = make_flows(n);
    let expr_d = parse(
        "sni in (\"www.google.com\", \"www.github.com\", \"www.microsoft.com\", \"missing.com\")",
    )
    .expect("parse");
    {
        let mut group = c.benchmark_group("filter_expr_apply_sni_in");
        group.throughput(Throughput::Elements(n as u64));
        group.bench_function("500_flows", |b| {
            b.iter(|| {
                let count = flows
                    .iter()
                    .filter(|f| {
                        let ctx = network_eval_ctx(f);
                        expr_d.apply_network(black_box(&ctx))
                    })
                    .count();
                black_box(count);
            });
        });
        group.finish();
    }
}

criterion_group!(benches, bench_filter_expr_apply);
criterion_main!(benches);
