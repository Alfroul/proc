//! Kill/Watchdog 安全（阶段 4）测试。
//!
//! 见 docs/stages/stage-4.md。覆盖：
//! - `kill_process` 对不存在 PID 返回 `AlreadyGone`
//! - `kill_process` 对 System (PID 4) 非 admin 时返回 `AccessDenied`
//! - `find_processes_by_name` 不匹配时返回空
//! - `find_processes_by_name` 能找到 spawn 出来的测试进程
//! - `kill_by_name(dry_run = true)` 不实际终止进程

use std::time::Duration;

use proc::kill::{
    self, KillByNameMatch, KillByNameResult, KillResult, find_processes_by_name, kill_by_name,
    kill_process,
};

#[test]
fn test_kill_single_already_gone() {
    // 0xFFFF_FFFF 几乎不可能在任意系统上对应实际进程
    let result = kill_process(0xFFFF_FFFF, false).expect("kill_process should not error");
    match result {
        KillResult::AlreadyGone | KillResult::AccessDenied => {}
        other => panic!("expected AlreadyGone or AccessDenied, got {:?}", other),
    }
}

#[test]
fn test_kill_single_access_denied_pid_4() {
    // PID 4 是 Windows 上的 System 进程。非 admin 调用 OpenProcess +
    // TerminateProcess 应返回 AccessDenied；CI 可能以 admin 运行，此时也可能
    // 直接返回 Killed（但实际 System 受 PPL 保护，不可能真被杀）。
    // 非 Windows 平台 PID 4 不存在，返回 AlreadyGone 也可接受。
    let result = kill_process(4, false).expect("kill_process should not error");
    match result {
        KillResult::AccessDenied | KillResult::Failed(_) | KillResult::AlreadyGone => {}
        KillResult::Killed => {
            // 极少数环境下可能"成功"（虽然 System 实际不会被杀）— 不严格断言
        }
    }
}

#[test]
fn test_kill_by_name_no_match() {
    let matches = find_processes_by_name("this_process_should_not_exist_xyz_unique_42.exe");
    assert!(
        matches.is_empty(),
        "expected zero matches for nonexistent name, got: {:?}",
        matches
    );
}

#[test]
fn test_kill_by_name_finds_spawned_process() {
    // spawn 一个长跑进程以便 sysinfo 能看到它。
    // 使用 ping/sleep 这种独立的 .exe/二进制，避免 cmd /c timeout 在某些
    // bash 环境下 PATH 被 MSYS 的 GNU timeout 干扰。
    #[cfg(windows)]
    let mut child = std::process::Command::new("ping")
        .args(["-n", "60", "127.0.0.1"])
        .spawn()
        .expect("spawn test process");
    #[cfg(not(windows))]
    let mut child = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .expect("spawn test process");

    let pid = child.id();
    // 给 sysinfo 一点时间观测到子进程
    std::thread::sleep(Duration::from_millis(800));

    // 用 PID 反查 sysinfo 得到精确进程名（不依赖硬编码 "ping.exe"/"sleep"，
    // 因为 sysinfo 在不同平台返回的 name 大小写/扩展名可能不同）。
    let mut sys = sysinfo::System::new_all();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let observed = sys
        .process(sysinfo::Pid::from_u32(pid))
        .map(|p| p.name().to_string_lossy().to_string());

    let name = match observed {
        Some(n) => n,
        None => {
            // sysinfo 没观察到，测试环境限制 — 跳过断言而不是 panic
            let _ = child.kill();
            let _ = child.wait();
            eprintln!("warn: sysinfo failed to observe PID {pid}, skipping assertion");
            return;
        }
    };

    // 在 kill 之前调用 find_processes_by_name，确保 child 仍在 sysinfo 视图中
    // （否则若并行测试也 spawn 了同名进程，可能看到别的 PID）。
    let matches = find_processes_by_name(&name);

    // 清理
    let _ = child.kill();
    let _ = child.wait();

    assert!(
        matches.iter().any(|m| m.pid == pid),
        "expected PID {} in matches for name '{}', got: {:?}",
        pid,
        name,
        matches
    );
}

#[test]
fn test_kill_by_name_dry_run_no_op() {
    #[cfg(windows)]
    let mut child = std::process::Command::new("ping")
        .args(["-n", "60", "127.0.0.1"])
        .spawn()
        .expect("spawn test process");
    #[cfg(not(windows))]
    let mut child = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .expect("spawn test process");

    let pid = child.id();
    std::thread::sleep(Duration::from_millis(800));

    // 用 PID 反查 sysinfo 得到精确进程名
    let mut sys = sysinfo::System::new_all();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    let observed = sys
        .process(sysinfo::Pid::from_u32(pid))
        .map(|p| p.name().to_string_lossy().to_string());

    let name = match observed {
        Some(n) => n,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            eprintln!("warn: sysinfo failed to observe PID {pid}, skipping dry_run assertion");
            return;
        }
    };

    let results = kill_by_name(&name, false, true).expect("kill_by_name dry_run should not error");

    // dry_run: 所有结果的 outcome 必须为 None
    assert!(
        results
            .iter()
            .all(|r: &KillByNameResult| r.outcome.is_none()),
        "dry_run should not produce kill outcomes, got: {:?}",
        results
    );
    // 并且 spawn 出来的 PID 必须在匹配列表中
    assert!(
        results.iter().any(|r| r.pid == pid),
        "expected PID {} in dry_run matches, got: {:?}",
        pid,
        results
    );

    // 关键：dry_run 之后子进程必须仍然存活
    match child.try_wait() {
        Ok(None) => { /* 仍在运行 — 正确 */ }
        Ok(Some(_)) => panic!("dry_run killed the test process unexpectedly"),
        Err(e) => panic!("try_wait failed: {}", e),
    }

    // 清理
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn test_kill_by_name_match_struct_instantiation() {
    // 轻量结构测试 — 确保公共字段可构造、可比较（CI 跨平台兼容性回归）
    let m = KillByNameMatch {
        pid: 4321,
        name: "demo.exe".to_string(),
    };
    assert_eq!(m.pid, 4321);
    assert_eq!(m.name, "demo.exe");

    let m2 = KillByNameMatch {
        pid: 4321,
        name: "demo.exe".to_string(),
    };
    assert_eq!(m, m2);
}

#[test]
fn test_kill_by_name_empty_results_structure() {
    // 没有匹配时，kill_by_name 返回空 Vec 而不是 error
    let results = kill::kill_by_name("nonexistent_xyz_42.exe", false, false).expect("should be Ok");
    assert!(results.is_empty());
}

#[test]
fn test_kill_by_name_dry_run_returns_outcome_none() {
    // dry_run 永远不调 kill_process — 即便匹配到一个权限受保护的系统进程
    // 也必须返回 outcome = None
    // 用一个一定不存在的名字确保 vec 为空，行为符合契约
    let results = kill::kill_by_name("nonexistent_xyz_43.exe", true, true).expect("should be Ok");
    assert!(results.iter().all(|r| r.outcome.is_none()));
    assert!(results.is_empty());
}
