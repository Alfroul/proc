//! `proc diag [--json]` — v0.6.0 阶段 3 新增。
//!
//! 输出所有后台 worker 的 metrics（avg_us / max_us / polls / drops），
//! 用户报 bug 时附上。
//!
//! v0.11 阶段 2：human-readable 模式末尾追加 `dns_collector` 行，反映 DNS
//! collector 实际选用的类型（etw / powershell / none）。详见 ADR-0020。

use colored::Colorize;

/// 默认 human-readable 表格；`--json` 输出 JSON（用户附 bug report）。
pub fn run_diag(json: bool) {
    let mut app = match crate::app::App::new() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{} {}", "错误:".red(), e);
            std::process::exit(1);
        }
    };
    // 等所有 worker 至少 poll 一次（light 1s / port 3s / dns 500ms）。
    // 拉得 2s 是兜底；用户报 bug 时通常已运行过 proc，重启到 diag 也就 2-3s。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        app.tick();
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let metrics = app.worker_metrics();
    if json {
        match serde_json::to_string_pretty(&metrics) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("{} 序列化失败: {}", "错误:".red(), e),
        }
    } else {
        println!("Worker diagnostics (avg/max 单位 μs)：\n");
        println!(
            "  {:<10} {:<2} {:>12} {:>12} {:>10} {:>8}",
            "name", "✓", "avg_us", "max_us", "polls", "drops"
        );
        for entry in &metrics {
            let s = &entry.stats;
            println!(
                "  {:<10} {}  {:>12} {:>12} {:>10} {:>8}",
                entry.name,
                s.health_badge(),
                s.avg_us,
                s.max_us,
                s.poll_count,
                s.channel_full,
            );
        }
        // v0.11 阶段 2：DNS collector 类型（ADR-0020）。用户报「DNS 日志缺数据」
        // 时附上此行——etw / powershell / none 三态对应不同诊断路径。
        println!("\n  dns_collector: {}", app.workers.dns_collector_kind);
    }
}
