//! `proc flows [--limit N] [--json]` — v0.7 阶段 8：列出 ProcessFlow。
//!
//! v0.12 阶段 2：Windows-only 后 Schannel ETW 是唯一来源。非管理员 /
//! x86 / Schannel worker 未起来 → 退到提示。

use colored::Colorize;

/// 默认等待 worker attach + 收集首批事件的时间。
/// 2s 与 `proc diag` 同款兜底；典型 TLS handshake < 1s 出现。
const WARM_UP_SECS: u64 = 2;

pub fn run_flows(limit: &Option<usize>, json: bool, filter: Option<&str>) {
    let mut app = match crate::app::App::new() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{} {}", "错误:".red(), e);
            std::process::exit(1);
        }
    };

    // Schannel worker 未起来（非管理员 / x86 / session 占用）→ 退到提示。
    if app.workers.schannel_etw_worker.is_none() {
        eprintln!(
            "{}",
            "Flow graph 需要 Windows 管理员权限（Schannel ETW session 启动失败）。详见日志。"
                .yellow()
        );
        return;
    }

    // 让 worker 收集首批事件 + SNI 关联。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(WARM_UP_SECS);
    while std::time::Instant::now() < deadline {
        app.tick();
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // v0.11 阶段 3：先 collect 全部，再过滤，最后截断 limit（典型「找前 N 个
    // 命中 X 的 flow」语义）。filter 走 FilterExpr v2 apply_network。
    let mut flows: Vec<&crate::flow::ProcessFlow> = app.flows.iter().collect();
    if let Some(expr_str) = filter {
        match crate::filter::parse(expr_str) {
            Ok(expr) => {
                // v0.11 阶段 8 REVIEW-13 P1-2：检测纯 process 字段表达式。
                // process 字段在 Flow 视图（apply_network ctx）下永远 false，
                // 用户写 `cpu > 5` 会把所有 flow 都过滤掉，体验反直觉。
                // 给出 warn + 退出 1，提示用户改用 network 字段。
                if expr.contains_process_field() {
                    eprintln!(
                        "{} filter 表达式只含 process 字段（cpu/mem/name/...），\
                        在 Flow 视图下永远不命中。\n\
                        Flow 字段：sni / dns_name / remote_addr / remote_port / \
                        bytes_out / bytes_in。详见 ADR-0011。",
                        "提示:".yellow()
                    );
                    std::process::exit(1);
                }
                flows.retain(|f| {
                    let ctx = crate::filter::NetworkEvalCtx { flow: f };
                    expr.apply_network(&ctx)
                })
            }
            Err(e) => {
                eprintln!("{} {}", "filter 语法错误:".red(), e);
                std::process::exit(1);
            }
        }
    }
    if let Some(n) = limit {
        flows.truncate(*n);
    }

    if json {
        // 序列化前转 owned（serde_json 处理 Vec<&T> 麻烦，直接 clone）。
        let snapshot: Vec<crate::flow::ProcessFlow> = flows.into_iter().cloned().collect();
        match serde_json::to_string_pretty(&snapshot) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("{} 序列化失败: {}", "错误:".red(), e),
        }
        return;
    }

    if flows.is_empty() {
        println!(
            "{}",
            "当前暂无活跃 flow（启动浏览器或 curl 触发 TLS handshake）".yellow()
        );
        return;
    }

    let n = flows.len();
    println!(
        "{}",
        format!("ProcessFlow（{n} 条 · Schannel TLS handshake SNI）").cyan()
    );
    println!(
        "  {:<7} {:<16} {:<24} {:<10}",
        "PID", "进程名", "SNI/域名", "首次见到"
    );
    for f in flows {
        let name = f
            .sni
            .clone()
            .or_else(|| f.dns_name.clone())
            .unwrap_or_else(|| "—".into());
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
        println!("  {:<7} {:<16} {:<24} {:<10}", pid_str, comm, name, ts);
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
