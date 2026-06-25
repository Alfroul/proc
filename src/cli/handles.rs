//! `proc handles` / `proc who` — 句柄枚举与「谁占用这文件」反查（阶段 4 A1）。

use colored::Colorize;

/// 用 sysinfo 反查 PID → 进程名。失败时返回 "?"。
fn pid_to_name(pid: u32) -> String {
    crate::collect::sysinfo_with(|sys| {
        sys.process(sysinfo::Pid::from_u32(pid))
            .map(|p| p.name().to_string_lossy().to_string())
            .unwrap_or_else(|| "?".to_string())
    })
}

pub fn run_who(path: &std::path::Path) {
    let handles = match crate::inspect::handles::find_lockers(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{} {}", "反查失败:".red(), e);
            std::process::exit(1);
        }
    };
    if handles.is_empty() {
        // filelocksmith 在非管理员账户下看不到系统进程句柄；空结果绝大多数是这个原因。
        println!(
            "{}",
            "未发现占用此路径的进程（提示：枚举系统进程句柄需要管理员权限）".yellow()
        );
        return;
    }
    let mut table = comfy_table::Table::new();
    table.set_header(vec!["PID", "进程名", "类型", "路径"]);
    for h in &handles {
        // find_lockers 反查路径下 raw_handle 字段被借用来存 PID（见模块注释）。
        let pid = h.raw_handle;
        let name = pid_to_name(pid as u32);
        table.add_row(vec![
            pid.to_string(),
            name,
            h.kind.label().to_string(),
            h.name.clone(),
        ]);
    }
    println!("{table}");
}

pub fn run_handles(pid: &Option<u32>, file: &Option<std::path::PathBuf>) {
    match (pid, file) {
        (Some(pid), None) => run_handles_pid(*pid),
        (None, Some(path)) => run_who(path),
        (Some(_), Some(_)) => {
            eprintln!("{}", "--pid 与 --file 互斥，请二选一".red());
            std::process::exit(1);
        }
        (None, None) => {
            eprintln!(
                "{}",
                "用法: proc handles --pid <PID>   或   proc handles --file <PATH>".red()
            );
            std::process::exit(1);
        }
    }
}

fn run_handles_pid(pid: u32) {
    let handles = match crate::inspect::handles::collect_handles(pid) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{} {}", "枚举句柄失败:".red(), e);
            std::process::exit(1);
        }
    };
    if handles.is_empty() {
        println!(
            "{}",
            format!("PID {} 当前无可见句柄（权限不足或进程已退出）", pid).yellow()
        );
        return;
    }
    let mut table = comfy_table::Table::new();
    table.set_header(vec!["类型", "名称", "句柄", "访问"]);
    for h in &handles {
        let name = if h.name.is_empty() {
            "-".to_string()
        } else {
            h.name.clone()
        };
        let access = if h.granted_access == 0 {
            "-".to_string()
        } else {
            format!("0x{:08X}", h.granted_access)
        };
        table.add_row(vec![
            h.kind.label().to_string(),
            name,
            format!("0x{:X}", h.raw_handle),
            access,
        ]);
    }
    println!("{table}");
}
