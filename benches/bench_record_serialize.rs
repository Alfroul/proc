//! v0.13 阶段 1：录屏 UiFrame bincode 序列化 benchmark。
//!
//! 测 `bincode::serialize(&ui_frame)` 单帧序列化开销。
//! stage doc 任务 4.4 关注点：「长 session（30min+ = 1800 frames × 1000 进程）
//! 的累计 IO 成本」。
//!
//! 3 档 fixture：100 / 500 / 1000 进程的 UiFrame（含 cpu_history + mem_history +
//! processes）。UiFrame 是 pub，可直接构造，不依赖 App。

mod common;

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};

use common::make_ui_frame;

fn bench_record_serialize(c: &mut Criterion) {
    let sizes = [100_usize, 500, 1000];

    let mut group = c.benchmark_group("record_serialize_ui_frame");
    for &size in &sizes {
        let frame = make_ui_frame(size);
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &frame, |b, frame| {
            b.iter(|| {
                let bytes = bincode::serialize(black_box(frame)).expect("serialize");
                black_box(bytes);
            });
        });
    }
    group.finish();

    // 反序列化对照（reader 路径 hot path）。
    let mut group = c.benchmark_group("record_deserialize_ui_frame");
    for &size in &sizes {
        let frame = make_ui_frame(size);
        let bytes = bincode::serialize(&frame).expect("serialize");
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &bytes, |b, bytes| {
            b.iter(|| {
                let frame = bincode::deserialize::<proc::record::frame::UiFrame>(black_box(bytes))
                    .expect("deserialize");
                black_box(frame);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_record_serialize);
criterion_main!(benches);
