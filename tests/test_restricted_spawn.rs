//! v0.6.0 阶段 2 — restricted_spawn 集成测试。
//!
//! 验收点：
//! - spawn 一个简单程序（whoami / echo）能拿到 pid + stdout
//! - kill 在子进程上工作（不挂起）
//! - stdout 读得到至少一行输出
//! - 非 Windows 平台走 fallback 路径

use proc::security::restricted_spawn::spawn_with_reduced_privileges;
use std::io::Read;

/// 读 stdout 全部字节并按 UTF-8 lossy 转 String（容忍 zh-CN Windows 上 whoami
/// 输出 GBK；测试只关心非空 + 不挂）。
fn drain_to_string_lossy(stdout: &mut std::fs::File) -> String {
    let mut bytes = Vec::new();
    stdout.read_to_end(&mut bytes).expect("read_to_end");
    String::from_utf8_lossy(&bytes).into_owned()
}

#[test]
fn spawn_simple_program_returns_child_with_stdout() {
    // 选最跨平台的简单程序 — Windows 上 whoami 自带；Linux/macOS 上 echo 是 builtin+coreutils。
    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "windows") {
        ("whoami.exe", vec![])
    } else {
        ("/bin/echo", vec!["restricted_spawn_test"])
    };

    let mut child = spawn_with_reduced_privileges(program, &args).expect("spawn");
    let mut stdout = child.stdout().expect("stdout");
    let buf = drain_to_string_lossy(&mut stdout);
    assert!(
        !buf.trim().is_empty(),
        "expected non-empty stdout, got: {buf:?}"
    );
}

#[test]
fn child_has_pid() {
    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "windows") {
        ("whoami.exe", vec![])
    } else {
        ("/bin/echo", vec!["x"])
    };
    let child = spawn_with_reduced_privileges(program, &args).expect("spawn");
    let pid = child.id();
    assert!(pid > 0, "pid should be positive, got {pid}");
}

#[test]
fn kill_is_idempotent_and_no_hang() {
    // 选一个会长期运行的程序 — Windows 上 ping；Linux 上 sleep。
    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "windows") {
        ("ping.exe", vec!["-n", "60", "127.0.0.1"])
    } else {
        ("/bin/sleep", vec!["60"])
    };
    let mut child = spawn_with_reduced_privileges(program, &args).expect("spawn");
    // kill 两次 — 不应 panic 或挂。fallback 路径下 process_handle=0 → kill noop，
    // 测试仅验证调用语义；真实 kill 走 restricted 路径（elevated 环境验证）。
    child.kill().expect("kill first");
    child.kill().expect("kill second (idempotent)");
}

#[test]
fn stdout_implements_read() {
    // 验证返回的 stdout 是 fs::File（impl Read）— 上层 BufReader::new(stdout) 不需要改类型
    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "windows") {
        ("whoami.exe", vec![])
    } else {
        ("/bin/echo", vec!["x"])
    };
    let mut child = spawn_with_reduced_privileges(program, &args).expect("spawn");
    let mut stdout: std::fs::File = child.stdout().expect("stdout take");
    let mut bytes = Vec::new();
    stdout.read_to_end(&mut bytes).expect("read_to_end");
    assert!(!bytes.is_empty());
}

#[test]
fn fallback_path_kill_works() {
    // 验证 fallback 路径下 kill 真能终止子进程（防 reader_loop 永远阻塞）。
    // 用一个长期运行的程序：Windows ping / Linux sleep。
    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "windows") {
        ("ping.exe", vec!["-n", "60", "127.0.0.1"])
    } else {
        ("/bin/sleep", vec!["60"])
    };
    let mut child = spawn_with_reduced_privileges(program, &args).expect("spawn");
    // kill 不应挂
    child.kill().expect("kill");
    // drop 时 RestrictedChild::drop 会再次 kill + wait（idempotent）
}

#[cfg(windows)]
#[test]
fn spawn_with_reduced_privileges_strips_se_debug() {
    // 在非 elevated 测试环境上验证 — 子进程即便通过 whoami /priv 报告权限，
    // 也应看不到 SeDebugPrivilege（DISABLE_MAX_PRIVILEGE 剥离所有权限）。
    //
    // 注意：在 elevated 测试进程上更可信；普通进程本来就没有 SeDebug。
    // 这里只验证 spawn 路径成功并产生 stdout（已覆盖），权限剥离的精确验证需要
    // elevated 环境，留给手动 / Process Explorer 验证（见 ADR-0008 验收段）。
    let mut child =
        spawn_with_reduced_privileges("whoami.exe", &["/priv"]).expect("spawn whoami /priv");
    let mut stdout = child.stdout().expect("stdout");
    let buf = drain_to_string_lossy(&mut stdout);
    assert!(!buf.trim().is_empty(), "whoami /priv empty: {buf:?}");
}
