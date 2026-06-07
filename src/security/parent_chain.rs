use crate::collect::ProcessInfo;
use super::score::{RiskCategory, RiskFactor};

const OFFICE_APPS: &[&str] = &[
    "winword.exe", "excel.exe", "powerpnt.exe", "outlook.exe",
    "msaccess.exe", "mspub.exe", "visio.exe", "project.exe",
    "onenote.exe", "onenotem.exe",
];

const SHELL_PROCESSES: &[&str] = &[
    "cmd.exe", "powershell.exe", "pwsh.exe",
    "wscript.exe", "cscript.exe", "mshta.exe",
];

pub fn analyze_parent_chain(proc: &ProcessInfo, all_procs: &[ProcessInfo]) -> Vec<RiskFactor> {
    let mut factors = Vec::new();

    // Build PID -> ProcessInfo lookup
    let pid_map: std::collections::HashMap<u32, &ProcessInfo> = all_procs
        .iter()
        .map(|p| (p.pid, p))
        .collect();

    let proc_name_lower = proc.name.to_lowercase();

    // Check: shell process spawned by office app
    if SHELL_PROCESSES.contains(&proc_name_lower.as_str()) {
        if let Some(ppid) = proc.parent_pid {
            if let Some(parent) = pid_map.get(&ppid) {
                let parent_lower = parent.name.to_lowercase();
                if OFFICE_APPS.contains(&parent_lower.as_str()) {
                    factors.push(RiskFactor {
                        category: RiskCategory::ParentChain,
                        name: "office_spawning_shell".to_string(),
                        weight: 30,
                        description: format!("Office 程序 {} 启动了 {}", parent.name, proc.name),
                    });
                }
            }
        }
    }

    // Check: orphan process (parent exited)
    if let Some(ppid) = proc.parent_pid {
        if ppid > 0 && !pid_map.contains_key(&ppid) {
            factors.push(RiskFactor {
                category: RiskCategory::ParentChain,
                name: "orphan".to_string(),
                weight: 5,
                description: "父进程已退出（孤儿进程）".to_string(),
            });
        }
    }

    // Check: deep chain (> 6 levels)
    let depth = chain_depth(proc.pid, &pid_map);
    if depth > 6 {
        factors.push(RiskFactor {
            category: RiskCategory::ParentChain,
            name: "deep_chain".to_string(),
            weight: 5,
            description: format!("进程链深度 {}（超过 6 层）", depth),
        });
    }

    // Check: explorer.exe spawning unexpected children via unknown chain
    if proc_name_lower == "explorer.exe" {
        // Not typically a risk factor for explorer itself
    } else {
        // Check if there's a suspicious chain from explorer -> unknown -> shell
        if SHELL_PROCESSES.contains(&proc_name_lower.as_str()) {
            if let Some(risk) = check_suspicious_ancestor(proc.pid, &pid_map) {
                factors.push(risk);
            }
        }
    }

    factors
}

fn chain_depth(pid: u32, pid_map: &std::collections::HashMap<u32, &ProcessInfo>) -> usize {
    let mut depth = 0;
    let mut current = pid;
    let mut visited = std::collections::HashSet::new();
    while let Some(proc) = pid_map.get(&current) {
        if visited.contains(&current) {
            break;
        }
        visited.insert(current);
        depth += 1;
        match proc.parent_pid {
            Some(ppid) if ppid > 0 && ppid != current => current = ppid,
            _ => break,
        }
    }
    depth
}

fn check_suspicious_ancestor(
    pid: u32,
    pid_map: &std::collections::HashMap<u32, &ProcessInfo>,
) -> Option<RiskFactor> {
    let mut current = pid;
    let mut visited = std::collections::HashSet::new();
    while let Some(proc) = pid_map.get(&current) {
        if visited.contains(&current) {
            break;
        }
        visited.insert(current);
        if let Some(ppid) = proc.parent_pid {
            if ppid > 0 && ppid != current {
                if let Some(parent) = pid_map.get(&ppid) {
                    let parent_lower = parent.name.to_lowercase();
                    // If parent is a known legitimate launcher, it's fine
                    if parent_lower == "explorer.exe"
                        || parent_lower == "cmd.exe"
                        || parent_lower == "powershell.exe"
                        || parent_lower == "pwsh.exe"
                        || parent_lower == "windowsterminal.exe"
                        || parent_lower == "wt.exe"
                        || parent_lower == "conhost.exe"
                    {
                        return None;
                    }
                }
                current = ppid;
            } else {
                break;
            }
        } else {
            break;
        }
    }
    // If we got here without finding a known legitimate ancestor
    None
}
