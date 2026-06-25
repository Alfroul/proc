//! `proc export` — 当前进程快照导出为 JSON / CSV。

use colored::Colorize;

use crate::collect::{self, SortField};

pub fn run_export(
    format: &str,
    output: &Option<std::path::PathBuf>,
    sort: &str,
    limit: &Option<usize>,
) {
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

    let epoch_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let payload = match format.to_lowercase().as_str() {
        "csv" => crate::format::export_processes_as_csv(&processes),
        _ => crate::format::export_processes_as_json(&processes, epoch_secs),
    };

    match output {
        Some(path) => {
            if let Err(e) = std::fs::write(path, &payload) {
                eprintln!("{} {}", "写入失败:".red(), e);
                std::process::exit(1);
            }
            println!(
                "{}",
                format!("✓ 已导出 {} 个进程到 {}", processes.len(), path.display()).green()
            );
        }
        None => println!("{}", payload),
    }
}
