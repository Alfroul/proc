//! 跨平台兼容性测试。
//!
//! v0.12 阶段 2（ADR-0022）：proc 转为 Windows-only 后此文件保留 Windows
//! 端契约测试。Linux / macOS stub 行为已不再适用。
//!
//! 覆盖：
//! - `local_offset_hours` 在 Windows 上返回合理值（-12 .. 14）
//! - `ProcessInfo` 可构造并完整序列化/反序列化

use proc::collect::ProcessInfo;
use proc::local_offset_hours;

#[test]
fn test_local_offset_hours_in_range() {
    let offset = local_offset_hours();
    assert!(
        (-12..=14).contains(&offset),
        "local_offset_hours() returned {offset}, expected -12 .. 14"
    );
}

#[test]
fn test_lib_functions_compile() {
    // 构造一个最小可用的 ProcessInfo，确保结构体能落地。
    let info = sample_process();
    assert_eq!(info.pid, 4242);
    assert_eq!(info.name.as_ref(), "compat_probe");
}

#[test]
fn test_process_info_construction_cross_platform() {
    let info = sample_process();
    let json = serde_json::to_string(&info).expect("serialize ProcessInfo");
    let back: ProcessInfo = serde_json::from_str(&json).expect("deserialize ProcessInfo");
    // name_lower 是 #[serde(skip)]，反序列化后默认空字符串，不参与 round-trip。
    // 这里把原对象的 name_lower 也清零，让等价比较聚焦于持久化字段。
    let mut info_no_lower = info.clone();
    info_no_lower.name_lower = std::sync::Arc::from("");
    assert_eq!(info_no_lower, back);
    assert_eq!(
        back.cmd.as_ref(),
        &["--demo".to_string(), "value".to_string()][..]
    );
}

fn sample_process() -> ProcessInfo {
    let name: std::sync::Arc<str> = std::sync::Arc::from("compat_probe");
    ProcessInfo {
        pid: 4242,
        name: std::sync::Arc::clone(&name),
        cpu_usage: 12.5,
        memory: 64 * 1024 * 1024,
        virtual_memory: 256 * 1024 * 1024,
        disk_usage: (1024, 2048),
        disk_read_speed: 100,
        disk_write_speed: 200,
        net_sent_rate: 0,
        net_recv_rate: 0,
        status: proc::collect::ProcessStatus::Run,
        exe: Some(std::sync::Arc::from("/usr/bin/compat_probe")),
        cmd: std::sync::Arc::from(vec!["--demo".to_string(), "value".to_string()]),
        cwd: Some(std::sync::Arc::from("/tmp")),
        parent_pid: Some(1),
        session_id: Some(0),
        user_id: Some(std::sync::Arc::from("0")),
        start_time: 1_700_000_000,
        run_time: 3600,
        name_lower: std::sync::Arc::from(name.to_lowercase().as_str()),
        throttled: proc::throttle::EcoQoSState::default(),
        signature_status: proc::security::SignatureStatus::default(),
        parent_chain: Vec::new(),
    }
}

// ===========================================================================
// v0.8.0 阶段 2 — TD-12：inspect::* 跨平台契约（所有平台编译运行）
// ===========================================================================
//
// bogus pid 在 Windows / Linux / macOS 上都应让 4 个 inspect 函数返回 Err：
// - Windows：OpenProcess 失败（pid 不存在）→ Err(PermissionDenied)
// - Linux：/proc/<bogus>/* 读失败 → Err(PermissionDenied)
// - macOS：cfg-gate stub 直接 Err
//
// 这条跨平台 case 锁住「失败路径不 panic」的契约，配合 test_linux_stubs.rs
// 的 Linux-only case，覆盖 stub 行为的两面。

#[test]
fn inspect_env_bogus_pid_returns_err_cross_platform() {
    use proc::inspect::env;
    let res = env::collect_env(u32::MAX);
    assert!(res.is_err(), "expected Err for bogus pid, got {:?}", res);
}

#[test]
fn inspect_dlls_bogus_pid_returns_err_cross_platform() {
    use proc::inspect::dlls;
    let res = dlls::collect_dlls(u32::MAX);
    assert!(res.is_err(), "expected Err for bogus pid, got {:?}", res);
}

#[test]
fn inspect_handles_bogus_pid_returns_err_cross_platform() {
    use proc::inspect::handles;
    let res = handles::collect_handles(u32::MAX);
    assert!(res.is_err(), "expected Err for bogus pid, got {:?}", res);
}

#[test]
fn inspect_memory_bogus_pid_returns_err_cross_platform() {
    use proc::inspect::memory;
    let res = memory::collect_memory(u32::MAX);
    assert!(res.is_err(), "expected Err for bogus pid, got {:?}", res);
}

#[test]
fn inspect_top_level_bogus_pid_returns_empty_data_cross_platform() {
    // inspect(pid) 顶层函数对任一子模块失败用 unwrap_or_default() 兜底，
    // bogus pid 在所有平台都应返回空 InspectionData（不 panic）。
    // 这是 UI 详情页的降级契约：失败 = 空数据，不是 crash。
    let data = proc::inspect::inspect(u32::MAX);
    assert!(
        data.env.is_empty(),
        "expected empty env, got {} vars",
        data.env.len()
    );
    assert!(
        data.dlls.is_empty(),
        "expected empty dlls, got {} items",
        data.dlls.len()
    );
    assert!(
        data.net.is_empty(),
        "expected empty net, got {} entries",
        data.net.len()
    );
}
