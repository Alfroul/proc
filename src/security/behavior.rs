use super::score::{RiskCategory, RiskFactor};
use crate::collect::ProcessInfo;

const HIGH_VALUE_NAMES: &[&str] = &[
    "svchost.exe",
    "lsass.exe",
    "csrss.exe",
    "smss.exe",
    "winlogon.exe",
    "services.exe",
    "explorer.exe",
    "dwm.exe",
    "taskhostw.exe",
    "runtimebroker.exe",
];

const KNOWN_TYPOS: &[&str] = &[
    "scvhost.exe",
    "svhost.exe",
    "svchosts.exe",
    "lsasss.exe",
    "csrss.exe.exe",
    "explorer.exe.exe",
    "svch0st.exe",
    "svchost.exc",
];

#[must_use]
pub fn check_name_spoofing(name: &str) -> Option<RiskFactor> {
    let name_lower = name.to_lowercase();

    if KNOWN_TYPOS.iter().any(|t| t.to_lowercase() == name_lower) {
        return Some(RiskFactor {
            category: RiskCategory::FilePath,
            name: "name_typosquat".to_string(),
            weight: 30,
            description: format!("进程名 {} 疑似仿冒系统进程", name),
        });
    }

    for target in HIGH_VALUE_NAMES {
        if is_near_match(&name_lower, target) {
            return Some(RiskFactor {
                category: RiskCategory::FilePath,
                name: "name_near_match".to_string(),
                weight: 25,
                description: format!("进程名 {} 与系统进程 {} 高度相似", name, target),
            });
        }
    }

    None
}

fn is_near_match(a: &str, b: &str) -> bool {
    if a == b {
        return false;
    }
    let a_len = a.len();
    let b_len = b.len();
    if a_len.abs_diff(b_len) > 1 {
        return false;
    }

    let (shorter, longer) = if a_len <= b_len {
        (a.as_bytes(), b.as_bytes())
    } else {
        (b.as_bytes(), a.as_bytes())
    };
    let mut diff = 0;
    let mut si = 0;
    let mut li = 0;

    while si < shorter.len() && li < longer.len() {
        if shorter[si] == longer[li] {
            si += 1;
            li += 1;
        } else {
            diff += 1;
            if diff > 1 {
                return false;
            }
            if a_len == b_len {
                si += 1;
                li += 1;
            } else {
                li += 1;
            }
        }
    }
    diff + (longer.len() - li) + (shorter.len() - si) <= 1
}

#[must_use]
pub fn check_resource_anomaly(proc: &ProcessInfo) -> Option<RiskFactor> {
    let cpu = proc.cpu_usage;
    let mem_mb = proc.memory as f64 / (1024.0 * 1024.0);

    if cpu > 80.0 && mem_mb > 100.0 && mem_mb < 2048.0 {
        let compute_apps = [
            "miner",
            "xmrig",
            "cgminer",
            "bminer",
            "ethminer",
            "nbminer",
            "t-rex",
            "phoenixminer",
        ];
        let name_lower = proc.name.to_lowercase();
        let is_known_compute = compute_apps.iter().any(|&k| name_lower.contains(k))
            || name_lower.contains("compile")
            || name_lower.contains("build")
            || name_lower.contains("render");

        if !is_known_compute {
            return Some(RiskFactor {
                category: RiskCategory::CommandLine,
                name: "resource_anomaly".to_string(),
                weight: 10,
                description: format!("高资源占用异常 (CPU {:.0}%, MEM {:.0}MB)", cpu, mem_mb),
            });
        }
    }

    None
}

#[must_use]
pub fn check_child_explosion(proc: &ProcessInfo, all_procs: &[ProcessInfo]) -> Option<RiskFactor> {
    let child_count = all_procs
        .iter()
        .filter(|p| p.parent_pid == Some(proc.pid))
        .count();

    if child_count > 20 {
        let legit_multi = [
            "svchost.exe",
            "services.exe",
            "explorer.exe",
            "code.exe",
            "devenv.exe",
            "chrome.exe",
            "msedge.exe",
            "firefox.exe",
            "idea64.exe",
            "javaw.exe",
        ];
        let name_lower = proc.name.to_lowercase();
        if !legit_multi.contains(&name_lower.as_str()) {
            return Some(RiskFactor {
                category: RiskCategory::ParentChain,
                name: "child_explosion".to_string(),
                weight: 15,
                description: format!("产生 {} 个子进程", child_count),
            });
        }
    }

    None
}

#[must_use]
pub fn check_privilege_escalation(proc: &ProcessInfo) -> Option<RiskFactor> {
    let user = proc.user_id.as_deref()?;
    let is_system = user.contains("SYSTEM") || user.contains("LocalSystem");
    let is_service = user.contains("NETWORK SERVICE") || user.contains("LOCAL SERVICE");

    if !is_system && !is_service {
        return None;
    }

    let exe_lower = proc.exe.as_deref().unwrap_or("").to_lowercase();
    let user_writable = exe_lower.contains("\\users\\")
        && !exe_lower.contains("\\program files")
        && !exe_lower.contains("\\windows\\");

    if user_writable {
        return Some(RiskFactor {
            category: RiskCategory::Privilege,
            name: "privilege_escalation".to_string(),
            weight: 30,
            description: format!("以 {} 身份运行用户目录程序", user),
        });
    }

    None
}

/// Check: svchost.exe must be launched by services.exe with -k flag
#[must_use]
pub fn check_svchost_integrity(
    proc: &ProcessInfo,
    all_procs: &[ProcessInfo],
) -> Option<RiskFactor> {
    if proc.name.to_lowercase() != "svchost.exe" {
        return None;
    }

    let exe_lower = proc.exe.as_deref().unwrap_or("").to_lowercase();
    let in_system_dir =
        exe_lower.contains("\\windows\\system32\\") || exe_lower.contains("\\windows\\syswow64\\");

    if !in_system_dir {
        return Some(RiskFactor {
            category: RiskCategory::FilePath,
            name: "svchost_wrong_path".to_string(),
            weight: 30,
            description: "svchost.exe 不在系统目录".to_string(),
        });
    }

    let mut violations = Vec::new();

    let cmd_str = proc.cmd.join(" ").to_lowercase();
    if !cmd_str.contains("-k ") && !cmd_str.contains("/k ") {
        violations.push("缺少 -k 参数");
    }

    let parent_is_services = proc
        .parent_pid
        .and_then(|ppid| all_procs.iter().find(|p| p.pid == ppid))
        .map(|p| p.name.to_lowercase() == "services.exe")
        .unwrap_or(false);

    if !parent_is_services {
        violations.push("父进程非 services.exe");
    }

    if !violations.is_empty() {
        return Some(RiskFactor {
            category: RiskCategory::ParentChain,
            name: "svchost_integrity".to_string(),
            weight: 20,
            description: format!("svchost.exe 异常: {}", violations.join(", ")),
        });
    }

    None
}

/// Check: process name does not match exe filename (renamed binary)
#[must_use]
pub fn check_name_path_mismatch(proc: &ProcessInfo) -> Option<RiskFactor> {
    let exe = proc.exe.as_deref()?;
    let exe_filename = exe.rsplit('\\').next().unwrap_or("").to_lowercase();
    let proc_name = proc.name.to_lowercase();

    if exe_filename.is_empty() || exe_filename == proc_name {
        return None;
    }

    // Known legitimate mismatches
    const KNOWN_MISMATCHES: &[(&str, &str)] = &[
        ("windowsterminal.exe", "wt.exe"),
        ("wt.exe", "windowsterminal.exe"),
    ];
    if KNOWN_MISMATCHES.iter().any(|(a, b)| {
        (proc_name == *a && exe_filename == *b) || (proc_name == *b && exe_filename == *a)
    }) {
        return None;
    }

    Some(RiskFactor {
        category: RiskCategory::FilePath,
        name: "name_path_mismatch".to_string(),
        weight: 15,
        description: format!("进程名 {} 与文件名 {} 不一致", proc.name, exe_filename),
    })
}
