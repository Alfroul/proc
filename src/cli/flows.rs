//! `proc flows [--limit N] [--json]` — v0.7 阶段 8：列出 ProcessFlow（ADR-0016）。
//!
//! Linux + `ebpf` feature 才有真实数据。其它平台 / 未启用 feature 时给降级提示。

use colored::Colorize;

use crate::ebpf::EBPF_ENABLED;

/// 默认等待 ebpf worker attach + 收集首批事件的时间。
/// 2s 与 `proc diag` 同款兜底；典型 connect 事件 < 1s 出现。
const WARM_UP_SECS: u64 = 2;

pub fn run_flows(limit: Option<usize>, json: bool) {
    if !EBPF_ENABLED {
        eprintln!(
            "{}",
            "eBPF flow graph 需要 Linux + `ebpf` feature（`cargo build --features ebpf`）".yellow()
        );
        return;
    }

    let mut app = match crate::app::App::new() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{} {}", "错误:".red(), e);
            std::process::exit(1);
        }
    };

    if app.workers.ebpf_worker.is_none() {
        eprintln!(
            "{}",
            "eBPF worker 启动失败：需要 root 或 CAP_BPF，内核 ≥ 5.10。详见日志。".yellow()
        );
        return;
    }

    // 让 ebpf worker 收集首批事件 + DNS 关联。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(WARM_UP_SECS);
    while std::time::Instant::now() < deadline {
        app.tick();
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let flows: Vec<&crate::ebpf::flow::ProcessFlow> = match limit {
        Some(n) => app.flows.iter().take(n).collect(),
        None => app.flows.iter().collect(),
    };

    if json {
        // 序列化前转 owned（serde_json 处理 Vec<&T> 麻烦，直接 clone）。
        let owned: Vec<&crate::ebpf::flow::ProcessFlow> = flows;
        let snapshot: Vec<crate::ebpf::flow::ProcessFlow> = owned.into_iter().cloned().collect();
        match serde_json::to_string_pretty(&snapshot) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("{} 序列化失败: {}", "错误:".red(), e),
        }
        return;
    }

    if flows.is_empty() {
        println!(
            "{}",
            "当前暂无活跃 flow（启动浏览器或 curl 触发 connect）".yellow()
        );
        return;
    }

    println!(
        "{}",
        format!("ProcessFlow（{} 条 · connect + DNS 关联）", flows.len()).cyan()
    );
    println!(
        "  {:<7} {:<16} {:<16} {:<6} {:<24} {:<10}",
        "PID", "进程名", "远端", "端口", "域名", "首次见到"
    );
    for f in flows {
        let dns = f.dns_name.clone().unwrap_or_else(|| "—".into());
        let comm = if f.comm.is_empty() {
            "?".to_string()
        } else {
            f.comm.clone()
        };
        let pid_str = if f.is_ghost() {
            format!("👻{}", f.pid)
        } else {
            f.pid.to_string()
        };
        let ts = format_system_time(f.first_seen);
        println!(
            "  {:<7} {:<16} {:<16} {:<6} {:<24} {:<10}",
            pid_str, comm, f.remote_addr, f.remote_port, dns, ts
        );
    }
}

/// 把 SystemTime 格式化成 `HH:MM:SS`（本地时区）。
fn format_system_time(t: std::time::SystemTime) -> String {
    let dur = match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d,
        Err(_) => return "?".into(),
    };
    let secs = dur.as_secs();
    let offset = crate::local_offset_hours() * 3600;
    let local = (secs as i64 + offset).max(0) as u64;
    let h = ((local % 86_400) / 3600) as u32;
    let m = ((local % 3600) / 60) as u32;
    let s = (local % 60) as u32;
    format!("{h:02}:{m:02}:{s:02}")
}
