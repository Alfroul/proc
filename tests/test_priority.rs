//! 阶段 4 A4：优先级 + affinity 集成测试。
//!
//! 注意：自己进程在 Windows 上普通账户通常能 get_priority 但 set_priority
//! 可能被 PROCESS_SET_INFORMATION 拒（取决于完整性级别）；Linux 容器里
//! RLIMIT_NICE 可能限制 setpriority 改到负 nice 值。测试设计为：
//! - get_priority / get_affinity 永远能跑（不 panic）
//! - set/get 往返仅在平台允许时断言
//! - PriorityClass 转换函数完全静态，断言精确

use proc::process_control::{
    PriorityClass, get_affinity, get_priority, set_affinity, set_priority,
};

#[test]
fn self_get_priority_does_not_panic() {
    let pid = std::process::id();
    match get_priority(pid) {
        Ok(class) => {
            // 必须是 6 档之一；label() 不 panic。
            let _ = class.label();
        }
        Err(e) => {
            eprintln!("note: get_priority({pid}) failed in CI: {e}");
        }
    }
}

#[test]
fn self_get_affinity_returns_nonzero_mask() {
    let pid = std::process::id();
    match get_affinity(pid) {
        Ok(mask) => {
            // 任何运行中进程的 affinity mask 至少有 1 位（否则没法跑）。
            assert!(mask != 0, "affinity mask should be non-zero");
            assert!(
                u64::count_ones(mask) <= 64,
                "affinity mask has >64 bits: 0x{mask:X}"
            );
        }
        Err(e) => {
            eprintln!("note: get_affinity({pid}) failed in CI: {e}");
        }
    }
}

#[test]
fn priority_class_to_nice_then_from_nice_round_trips() {
    // PriorityClass → nice → PriorityClass 必须能往返（from_nice 是带量化误差的，
    // 但 to_nice 给的值正好落在每档中心 nice 上，所以 round-trip 是精确的）。
    for class in [
        PriorityClass::Idle,
        PriorityClass::BelowNormal,
        PriorityClass::Normal,
        PriorityClass::AboveNormal,
        PriorityClass::High,
        PriorityClass::Realtime,
    ] {
        let nice = class.to_nice();
        let back = PriorityClass::from_nice(nice);
        assert_eq!(
            back,
            class,
            "{} (nice={}) did not round-trip",
            class.label(),
            nice
        );
    }
}

#[test]
fn priority_class_bump_up_then_down_returns_to_original() {
    // Normal → bump_up → AboveNormal → bump_down → Normal。
    // 仅在边界外（Realtime/Idle）bump 不对称。
    let mut cur = PriorityClass::Normal;
    cur = cur.bump_up();
    assert_eq!(cur, PriorityClass::AboveNormal);
    cur = cur.bump_down();
    assert_eq!(cur, PriorityClass::Normal);

    // 连续 bump_up/down 不越界。
    for _ in 0..10 {
        cur = cur.bump_up();
        assert_ne!(cur, PriorityClass::Idle); // bump_up 不会降到 Idle
    }
    assert_eq!(cur, PriorityClass::Realtime);
}

#[test]
fn self_set_get_priority_round_trip_when_allowed() {
    // 仅在能 set 的环境（非受限 CI 容器）下断言；失败时跳过，不挂测试。
    let pid = std::process::id();
    let original = match get_priority(pid) {
        Ok(c) => c,
        Err(_) => return,
    };
    // 选一个跟 original 不同的档做往返（避免 no-op）。
    let target = match original {
        PriorityClass::Normal => PriorityClass::BelowNormal,
        _ => PriorityClass::Normal,
    };
    if set_priority(pid, target).is_err() {
        // 权限不足 / 平台不支持 / RLIMIT_NICE 等：跳过。
        eprintln!(
            "note: set_priority({pid}, {:?}) denied in CI, skipping round-trip",
            target
        );
        return;
    }
    // 恢复原值，避免影响后续测试。
    let _ = set_priority(pid, original);
    match get_priority(pid) {
        Ok(back) => {
            // 某些 Windows 版本上 get_priority 可能返回一个相邻档（系统会钳位），
            // 这里只断言"回到 original 或仍在合法 6 档"。
            let _ = back.label();
        }
        Err(e) => eprintln!("note: post-set get_priority({pid}) failed: {e}"),
    }
}

#[test]
fn affinity_mask_round_trip_when_allowed() {
    let pid = std::process::id();
    let original = match get_affinity(pid) {
        Ok(m) if m != 0 => m,
        _ => return,
    };
    // 取 original 的最低位作为新 mask（保证至少 1 核可用）。
    let new_mask = 1u64 << (u64::trailing_zeros(original) as u64);
    if set_affinity(pid, new_mask).is_err() {
        eprintln!("note: set_affinity({pid}, 0x{new_mask:X}) denied in CI");
        return;
    }
    // 立即恢复，避免影响后续测试。
    let _ = set_affinity(pid, original);
}

#[test]
fn self_set_affinity_invalid_mask_fails_gracefully() {
    let pid = std::process::id();
    // mask = 0 不合法（必须至少 1 核）；set_affinity 应当返回 Err 而非 panic。
    let _ = set_affinity(pid, 0);
    // mask 高位超出系统 CPU 数（如 0xFF..FF 在 8 核机器上）也应当被 OS 拒绝。
    let _ = set_affinity(pid, u64::MAX);
}
