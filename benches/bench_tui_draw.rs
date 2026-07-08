//! v0.13 阶段 1：TUI 单帧渲染 benchmark。
//!
//! 测 `src/tui/process_table.rs::draw` 的 format! 风暴路径——
//! stage doc 任务 4.3 关注点：「每行每帧 5+ 次 format!（cpu / mem /
//! format_bytes / format_speed / name.clone()）」。
//!
//! **不调 App**——App 紧耦合 SystemSnapshot。本 bench 用 ratatui TestBackend
//! 直接渲染 fake Table，复刻生产 format! 路径 + Arc clone 模式。
//!
//! 关注点：
//! - 每行每帧 format! 调用次数（cpu / mem / mem_pct / name_str）
//! - Arc<str> 在渲染层的 deref / format! 开销
//! - ratatui widgets::Table::render 在 100/500/1000 行的 scaling
//!
//! 3 档 fixture：100 / 500 / 1000 进程。

mod common;

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Cell, Row, Table};

use proc::collect::ProcessInfo;
use proc::format::format_bytes;

use common::make_processes;

/// 模拟 process_table::draw 的 format! 风暴：每行跑 5+ 次 format! /
/// Arc clone / format_bytes，与 src/tui/process_table.rs:71-159 路径一致。
fn build_rows(processes: &[ProcessInfo], total_mem: u64) -> Vec<Row<'_>> {
    processes
        .iter()
        .map(|p| {
            let mem_str = format_bytes(p.memory);
            let cpu_str = format!("{:.1}", p.cpu_usage);
            let mem_pct = if total_mem > 0 {
                format!("{:.1}", p.memory as f64 / total_mem as f64 * 100.0)
            } else {
                "0.0".to_string()
            };
            let name_str = format!("{}{}", p.name, p.signature_status.badge());
            let pid_str = format!("{}", p.pid);
            Cells::build(pid_str, cpu_str, mem_pct, mem_str, name_str).into_row()
        })
        .collect()
}

struct Cells {
    pid: String,
    cpu: String,
    mem_pct: String,
    mem: String,
    name: String,
}

impl Cells {
    fn build(pid: String, cpu: String, mem_pct: String, mem: String, name: String) -> Self {
        Self {
            pid,
            cpu,
            mem_pct,
            mem,
            name,
        }
    }

    fn into_row(self) -> Row<'static> {
        Row::new(vec![
            Cell::from(self.pid),
            Cell::from(self.cpu),
            Cell::from(self.mem_pct),
            Cell::from(self.mem),
            Cell::from(self.name),
        ])
        .style(Style::default())
    }
}

fn bench_tui_draw(c: &mut Criterion) {
    let sizes = [100_usize, 500, 1000];
    let total_mem = 16 * 1024 * 1024 * 1024_u64;
    let area = Rect::new(0, 0, 120, 40);

    let mut group = c.benchmark_group("tui_draw_process_table");
    for &size in &sizes {
        let processes = make_processes(size);
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            &processes,
            |b, processes| {
                b.iter(|| {
                    // 1. format! 风暴：每行 5 个 String。
                    let rows = build_rows(black_box(processes), black_box(total_mem));

                    // 2. 构造 Table widget。
                    let header = Row::new(vec![
                        Cell::from("PID"),
                        Cell::from("CPU%"),
                        Cell::from("MEM%"),
                        Cell::from("内存"),
                        Cell::from("名称"),
                    ])
                    .style(Style::default().fg(Color::Yellow));
                    let table = Table::new(
                        rows,
                        [
                            Constraint::Length(8),
                            Constraint::Length(8),
                            Constraint::Length(8),
                            Constraint::Length(10),
                            Constraint::Min(20),
                        ],
                    )
                    .header(header)
                    .block(Block::default().borders(Borders::ALL).title("进程"));

                    // 3. 渲染到 TestBackend（ratatui 内部 buffer 操作）。
                    let backend = TestBackend::new(area.width, area.height);
                    let mut terminal = Terminal::new(backend).expect("test backend");
                    terminal
                        .draw(|f| {
                            f.render_widget(table, area);
                        })
                        .expect("draw");
                });
            },
        );
    }
    group.finish();

    // v0.17 stage 3 TD-44：format_bytes B 档 itoa vs std format! 对比。
    // 预期 itoa 路径 ~50ns vs 旧 std format! 路径 ~150ns（2-3x 降幅）。
    // 仅测 B 档（bytes < 1024），MB/KB/GB 档保留 f64 {:.1} 路径不在 itoa 范围。
    bench_format_bytes_itoa_vs_format(c);
}

fn bench_format_bytes_itoa_vs_format(c: &mut Criterion) {
    let mut group = c.benchmark_group("format_bytes_itoa_vs_format");
    let sizes: &[u64] = &[0, 1, 100, 500, 999, 1023];
    for &size in sizes {
        group.bench_with_input(BenchmarkId::new("itoa", size), &size, |b, &n| {
            b.iter(|| {
                let mut buf = itoa::Buffer::new();
                let _ = black_box(format!("{}B", buf.format(black_box(n))));
            });
        });
        group.bench_with_input(BenchmarkId::new("std_format", size), &size, |b, &n| {
            b.iter(|| {
                let _ = black_box(format!("{}B", black_box(n)));
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_tui_draw);
criterion_main!(benches);
