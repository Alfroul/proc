//! `proc ls` 与 `proc tree` — 进程列表 / 进程树两种展示形态。

use colored::Colorize;

use crate::classify;
use crate::collect::{self, SortField};
use crate::format::format_bytes;

pub fn run_ls(sort: &str, limit: &Option<usize>) {
    let mut snapshot = match collect::SystemSnapshot::new() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{} {}", "错误:".red(), e);
            std::process::exit(1);
        }
    };

    if let Err(e) = snapshot.refresh() {
        eprintln!("{} {}", "刷新失败:".red(), e);
        std::process::exit(1);
    }

    let _ = snapshot.refresh_heavy_incremental();
    let mut processes = snapshot.cached_processes_vec();
    let sort_field = match sort {
        "mem" | "memory" => SortField::Memory,
        "name" => SortField::Name,
        "pid" => SortField::Pid,
        "disk_read" | "diskread" => SortField::DiskRead,
        "disk_write" | "diskwrite" => SortField::DiskWrite,
        "net_sent" | "netsent" => SortField::NetSent,
        "net_recv" | "netrecv" => SortField::NetRecv,
        _ => SortField::Cpu,
    };

    crate::collect::sort_processes(&mut processes, sort_field);

    if let Some(n) = limit {
        processes.truncate(*n);
    }

    let show_net = matches!(sort_field, SortField::NetSent | SortField::NetRecv);

    let mut table = comfy_table::Table::new();
    if show_net {
        table.set_header(vec![
            "PID",
            "CPU%",
            "MEM%",
            "内存",
            "↑网络/s",
            "↓网络/s",
            "分类",
            "名称",
        ]);
    } else {
        table.set_header(vec!["PID", "CPU%", "MEM%", "内存", "分类", "名称"]);
    }

    for proc in &processes {
        let class = classify::classify_process(proc);
        let mem_str = format_bytes(proc.memory);
        let cpu_str = format!("{:.1}", proc.cpu_usage);
        let mem_pct = if proc.virtual_memory > 0 {
            format!(
                "{:.1}",
                proc.memory as f64 / proc.virtual_memory as f64 * 100.0
            )
        } else {
            "0.0".to_string()
        };

        if show_net {
            table.add_row(vec![
                proc.pid.to_string(),
                cpu_str,
                mem_pct,
                mem_str,
                crate::format::format_speed(proc.net_sent_rate),
                crate::format::format_speed(proc.net_recv_rate),
                class.label().to_string(),
                proc.name.to_string(),
            ]);
        } else {
            table.add_row(vec![
                proc.pid.to_string(),
                cpu_str,
                mem_pct,
                mem_str,
                class.label().to_string(),
                proc.name.to_string(),
            ]);
        }
    }

    println!("{table}");
}

pub fn run_tree() {
    let mut snapshot = match collect::SystemSnapshot::new() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{} {}", "错误:".red(), e);
            std::process::exit(1);
        }
    };

    if let Err(e) = snapshot.refresh() {
        eprintln!("{} {}", "刷新失败:".red(), e);
        std::process::exit(1);
    }

    let _ = snapshot.refresh_heavy_incremental();
    let processes = snapshot.cached_processes_vec();
    let (_, total_mem) = snapshot.memory_usage();
    let tree_nodes = crate::tree::build_process_tree(&processes, total_mem);
    let output = crate::tree::format_tree_text(&tree_nodes);
    println!("{output}");
}
