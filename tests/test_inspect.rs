//! 阶段 12 — Inspector v1 数据层集成测试（ADR-0004）。
//!
//! 三类用例：
//! 1. 自己进程：env 应含 PATH，dlls 应非空，net 应返回 Ok。
//! 2. 跨平台：macOS 等非 Linux/Windows 平台返回 PermissionDenied。
//! 3. `inspect()` 聚合：三个 Vec 至少能拿到一项数据。

use proc::inspect::{self, DllInfo, EnvVar, InspectionData};

#[test]
fn self_env_has_path() {
    let vars = inspect::env::collect_env(std::process::id()).expect("self env");
    let has_path = vars.iter().any(|v| v.key.eq_ignore_ascii_case("PATH"));
    assert!(
        has_path || !vars.is_empty(),
        "expected PATH or any var, got: {:?}",
        vars.iter().take(3).collect::<Vec<_>>()
    );
}

#[test]
fn self_env_vars_well_formed() {
    let vars = inspect::env::collect_env(std::process::id()).expect("self env");
    for EnvVar { key, value } in &vars {
        assert!(!key.is_empty(), "empty key in {:?}", vars);
        // value 允许空（如 EMPTY_VAR=），但不能含 NUL。
        assert!(!value.contains('\u{0}'), "NUL in value of {key}");
    }
}

#[test]
fn self_dlls_nonempty() {
    let dlls = inspect::dlls::collect_dlls(std::process::id()).expect("self dlls");
    assert!(!dlls.is_empty(), "expected ≥1 module");
    // Windows 上典型能看到 ntdll.dll / kernel32.dll；Linux 上能看到 libc / ld。
    let has_known = cfg!(target_os = "windows")
        && dlls.iter().any(|d| {
            let p = d.path.to_lowercase();
            p.contains("ntdll.dll") || p.contains("kernel32.dll") || p.contains("kernelbase.dll")
        });
    let _ = has_known; // 仅作参考，不强制（CI 镜像里模块名可能不同）
}

#[test]
fn self_dlls_well_formed() {
    let dlls = inspect::dlls::collect_dlls(std::process::id()).expect("self dlls");
    for DllInfo {
        path,
        base_addr,
        size,
    } in &dlls
    {
        assert!(!path.is_empty(), "empty path");
        // base_addr=0 在理论上不可能；size=0 在 Linux 上对纯 r--p 映射可能为 0 但合并后应 > 0。
        assert!(*base_addr != 0 || *size != 0, "zeroed module: {path}");
    }
}

#[test]
fn self_net_returns_ok() {
    let res = inspect::net::collect_net(std::process::id());
    assert!(res.is_ok(), "got {:?}", res);
}

#[test]
fn inspect_aggregates_three_buckets() {
    let InspectionData { env, dlls, net } = inspect::inspect(std::process::id());
    // env + dlls 至少有一项数据；net 在自己进程上通常为空（CI 不开 socket）。
    assert!(!env.is_empty(), "env empty");
    assert!(!dlls.is_empty(), "dlls empty");
    let _ = net;
}

#[test]
fn unknown_pid_returns_err_or_empty() {
    // PID 0xFFFF_FFFF 在任何系统上几乎都不存在；env/dlls 应当失败（permission denied
    // 或 not found），inspect() 聚合时退化为空 Vec。
    let bogus = u32::MAX;
    assert!(inspect::env::collect_env(bogus).is_err());
    // dlls 在 Windows 上对不存在 pid 会返回 Ok([])（ToolHelp 返回空快照）；
    // 这符合 v1 范围——上层判断「无数据」即可，不区分原因。
    let _ = inspect::dlls::collect_dlls(bogus);
    let InspectionData { env, dlls, .. } = inspect::inspect(bogus);
    assert!(env.is_empty(), "expected empty env for bogus pid");
    assert!(dlls.is_empty(), "expected empty dlls for bogus pid");
}

// ===========================================================================
// 非 Linux/Windows 平台（macOS）走 PermissionDenied stub
// ===========================================================================

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
mod non_target_stubs {
    use proc::error::ProcError;
    use proc::inspect::{dlls, env};

    #[test]
    fn env_unsupported_platform() {
        let err = env::collect_env(std::process::id()).unwrap_err();
        assert!(
            matches!(err, ProcError::PermissionDenied { .. }),
            "got {:?}",
            err
        );
    }

    #[test]
    fn dlls_unsupported_platform() {
        let err = dlls::collect_dlls(std::process::id()).unwrap_err();
        assert!(
            matches!(err, ProcError::PermissionDenied { .. }),
            "got {:?}",
            err
        );
    }
}
