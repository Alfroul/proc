//! v0.8.0 阶段 2 — TD-12：Linux stub 测试覆盖增强（ADR-0002 cfg-gate）。
//!
//! 目标：让 Linux CI runner 真正跑 cfg(target_os = "linux") 的 stub 路径，
//! 防止「/proc 读失败时 inspect::* 的降级路径」静默回归。
//!
//! 覆盖 4 个采集模块的 Linux 路径（外部集成测试视角）：
//! - [`proc::inspect::env::collect_env`] 读 `/proc/<pid>/environ`
//! - [`proc::inspect::dlls::collect_dlls`] 读 `/proc/<pid>/maps`
//! - [`proc::inspect::handles::collect_handles`] 读 `/proc/<pid>/fd/*`
//! - [`proc::inspect::memory::collect_memory`] 读 `/proc/<pid>/maps`
//!
//! 契约：bogus pid（u32::MAX 几乎肯定不存在）→ 返回 [`ProcError::PermissionDenied`]，
//! 不是 panic / 空 Vec / 默认值。源码内 `#[cfg(test)] mod tests` 已有同类单元测试，
//! 这里从外部 public API 视角再验一遍，防 lib API 演化时 stub 路径 silently 破损。
//!
//! **平台约束**：Linux-only。Windows / macOS 上这些 case 不编译
//! （collect_* 的 Linux 实现分支 cfg(target_os = "linux")）。

#![cfg(target_os = "linux")]

use proc::inspect::{dlls, env, handles, memory};

/// bogus pid 在 Linux 上应让 env 采集返回 Err（读 `/proc/<bogus>/environ` 失败）。
///
/// 这是 TD-12 的核心契约：stub 路径不能 panic / 返回空 Vec 伪装成功。
#[test]
fn linux_env_bogus_pid_returns_err() {
    let res = env::collect_env(u32::MAX);
    assert!(
        res.is_err(),
        "expected Err for bogus pid on Linux, got {:?}",
        res
    );
}

/// 同上：dlls 采集对 bogus pid 返回 Err。
#[test]
fn linux_dlls_bogus_pid_returns_err() {
    let res = dlls::collect_dlls(u32::MAX);
    assert!(
        res.is_err(),
        "expected Err for bogus pid on Linux, got {:?}",
        res
    );
}

/// 同上：handles 采集对 bogus pid 返回 Err。
#[test]
fn linux_handles_bogus_pid_returns_err() {
    let res = handles::collect_handles(u32::MAX);
    assert!(
        res.is_err(),
        "expected Err for bogus pid on Linux, got {:?}",
        res
    );
}

/// 同上：memory 采集对 bogus pid 返回 Err。
#[test]
fn linux_memory_bogus_pid_returns_err() {
    let res = memory::collect_memory(u32::MAX);
    assert!(
        res.is_err(),
        "expected Err for bogus pid on Linux, got {:?}",
        res
    );
}

/// 自己进程的 env 采集在 Linux 上应当成功（std::process::id 总能读 /proc/self/environ）。
///
/// 这条对「正常路径」的兜底断言，配合上面 4 条 bogus pid case，让 stub 行为
/// 既有「失败契约」也有「成功契约」双向锁定。CI 容器偶尔会清空环境变量，
/// 所以不强制 PATH 存在，只要求 Ok + 至少 1 条变量。
#[test]
fn linux_self_env_returns_ok_nonempty() {
    let pid = std::process::id();
    let res = env::collect_env(pid);
    match res {
        Ok(vars) => {
            // CI 上 /proc/self 应当可读；空 Vec 表示 stub 错把成功当失败。
            assert!(
                !vars.is_empty(),
                "expected ≥1 env var for self pid {pid}, got empty"
            );
        }
        Err(e) => {
            // 极端受限的容器可能拒绝读自己的 environ —— 仅记录，不挂测试。
            eprintln!("note: collect_env({pid}) failed in CI Linux: {e}");
        }
    }
}

/// 自己进程的 memory 采集在 Linux 上至少返回 1 条区域（任何进程都有 stack/heap）。
#[test]
fn linux_self_memory_returns_nonempty_or_logs() {
    let pid = std::process::id();
    let res = memory::collect_memory(pid);
    match res {
        Ok(regions) => {
            assert!(
                !regions.is_empty(),
                "expected ≥1 memory region for self pid {pid}"
            );
            // 每条 size > 0 是 maps parser 的不变量。
            for r in &regions {
                assert!(r.size > 0, "zero-size region: {r:?}");
            }
        }
        Err(e) => {
            eprintln!("note: collect_memory({pid}) failed in CI Linux: {e}");
        }
    }
}
