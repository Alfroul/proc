//! 进程自我加固 — 见 CONTEXT.md / ADR-0008。
//!
//! v0.6.0 阶段 2 开启 4 项 mitigation policy（外加 ImageLoad 防 UNC 注入）：
//!
//! | Policy | 位 | 含义 |
//! |---|---|---|
//! | DEP (Permanent) | Enable + Permanent | 数据段不可执行；永久不可逆 |
//! | ASLR HighEntropy | EnableHighEntropyASLR + EnableBottomUpRandomization | 64-bit ASLR 用 24-bit 熵 |
//! | ProhibitDynamicCode | ProhibitDynamicCode | 禁止 VirtualAlloc(PAGE_EXECUTE_*) / 写后执行 |
//! | DisableExtensionPoints | DisableExtensionPoints | 禁 AppInit_DLLs / 全局钩子 |
//! | ImageLoad | NoRemoteMftImages + NoLowMftImages + PreferSystem32Images | 禁从 UNC / Low Mandatory 加载 |
//!
//! **不开启 ProcessSignaturePolicy** — 会让 nvml-wrapper 等 native 依赖因未签名挂掉（ADR-0008）。
//!
//! 失败语义：失败时把策略名加进返回的 `failed` Vec，**不 panic**。
//! 启动健壮性 > 完美加固（如已经在 mitigation-enabled 进程中再次调用会拒绝）。

use std::ffi::c_void;

/// 启动时调一次，把 5 项 mitigation policy 应用到当前进程。
///
/// 返回失败策略名列表（空 = 全部成功）。调用方负责 eprintln!/tracing。
///
/// 跨版本稳定性：windows 0.57→0.61 把 bitfield 字段名封进 `_bitfield`，
/// 这里通过 union 的 `Flags: u32` 直接写位值，不依赖具体字段名。
#[cfg(windows)]
pub fn apply_self_mitigations() -> Vec<&'static str> {
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetProcessMitigationPolicy, ProcessASLRPolicy, ProcessDEPPolicy,
        ProcessDynamicCodePolicy, ProcessExtensionPointDisablePolicy, ProcessImageLoadPolicy,
        SetProcessMitigationPolicy,
    };

    // SAFETY: 我们只对自己进程调用，传 ptr 都指向本函数栈上结构；size_of 与类型一致。
    unsafe {
        let mut failed: Vec<&'static str> = Vec::new();

        // ── 1. DEP (Permanent) ───────────────────────────────────────────────
        // PROCESS_MITIGATION_DEP_POLICY { union { Flags }, Permanent: bool }
        // bit0 = Enable, bit1 = DisableAtlThunkEmulation (附带打开)
        #[repr(C)]
        #[derive(Clone, Copy, Default)]
        struct DepPolicy {
            flags: u32,
            permanent: bool,
        }
        // v0.11 后修复：Rust 二进制默认带 /NXCOMPAT linker flag，PE header 已声明
        // DEP Enable + Permanent。运行时再调 SetProcessMitigationPolicy 会被拒绝
        // （Permanent 不可改），导致 warning 误报。先 GetProcessMitigationPolicy 预检，
        // 已经 Enable + Permanent 时视为成功跳过；其他状态（off / 非 Permanent）才调
        // SetProcessMitigationPolicy 强制设到 Permanent。
        let mut current = DepPolicy::default();
        let already_ok = GetProcessMitigationPolicy(
            GetCurrentProcess(),
            ProcessDEPPolicy,
            &mut current as *mut _ as *mut c_void,
            std::mem::size_of::<DepPolicy>(),
        )
        .is_ok()
            && current.permanent
            && (current.flags & 0b001) != 0; // Enable bit

        if !already_ok {
            let dep = DepPolicy {
                flags: 0b011, // Enable | DisableAtlThunkEmulation
                permanent: true,
            };
            if SetProcessMitigationPolicy(
                ProcessDEPPolicy,
                &dep as *const _ as *const c_void,
                std::mem::size_of::<DepPolicy>(),
            )
            .is_err()
            {
                failed.push("DEP");
            }
        }

        // ── 2. ASLR High Entropy ─────────────────────────────────────────────
        // PROCESS_MITIGATION_ASLR_POLICY { union { Flags } }
        // bit0 = EnableBottomUpRandomization, bit2 = EnableHighEntropyASLR
        #[repr(C)]
        #[derive(Clone, Copy, Default)]
        struct AslrPolicy {
            flags: u32,
        }
        let aslr = AslrPolicy {
            flags: 0b101, // EnableBottomUpRandomization | EnableHighEntropyASLR
        };
        if SetProcessMitigationPolicy(
            ProcessASLRPolicy,
            &aslr as *const _ as *const c_void,
            std::mem::size_of::<AslrPolicy>(),
        )
        .is_err()
        {
            failed.push("ASLR");
        }

        // ── 3. ProhibitDynamicCode ───────────────────────────────────────────
        // PROCESS_MITIGATION_DYNAMIC_CODE_POLICY { union { Flags } }
        // bit0 = ProhibitDynamicCode
        #[repr(C)]
        #[derive(Clone, Copy, Default)]
        struct DynCodePolicy {
            flags: u32,
        }
        let dyn_code = DynCodePolicy { flags: 0b1 };
        if SetProcessMitigationPolicy(
            ProcessDynamicCodePolicy,
            &dyn_code as *const _ as *const c_void,
            std::mem::size_of::<DynCodePolicy>(),
        )
        .is_err()
        {
            failed.push("DynamicCode");
        }

        // ── 4. DisableExtensionPoints ────────────────────────────────────────
        // PROCESS_MITIGATION_EXTENSION_POINT_DISABLE_POLICY { union { Flags } }
        // bit0 = DisableExtensionPoints
        #[repr(C)]
        #[derive(Clone, Copy, Default)]
        struct ExtPointPolicy {
            flags: u32,
        }
        let ext = ExtPointPolicy { flags: 0b1 };
        if SetProcessMitigationPolicy(
            ProcessExtensionPointDisablePolicy,
            &ext as *const _ as *const c_void,
            std::mem::size_of::<ExtPointPolicy>(),
        )
        .is_err()
        {
            failed.push("ExtensionPoint");
        }

        // ── 5. ImageLoad (NoRemote + NoLow + PreferSystem32) ─────────────────
        // PROCESS_MITIGATION_IMAGE_LOAD_POLICY { union { Flags } }
        // bit0 = NoRemoteMftImages, bit1 = NoLowMftImages, bit2 = PreferSystem32Images
        #[repr(C)]
        #[derive(Clone, Copy, Default)]
        struct ImageLoadPolicy {
            flags: u32,
        }
        let img = ImageLoadPolicy { flags: 0b111 };
        if SetProcessMitigationPolicy(
            ProcessImageLoadPolicy,
            &img as *const _ as *const c_void,
            std::mem::size_of::<ImageLoadPolicy>(),
        )
        .is_err()
        {
            failed.push("ImageLoad");
        }

        failed
    }
}

#[cfg(not(windows))]
pub fn apply_self_mitigations() -> Vec<&'static str> {
    // Linux/macOS 等价物（prctl/seccomp）见 ADR-0008 后续工作，v0.6.0 暂不实现。
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_self_mitigations_returns_without_panic() {
        // 测试进程也跑一次。Windows 上 DEP Permanent 等策略如果镜像已通过 /NXCOMPAT
        // 默认开启，运行时再次 SetProcessMitigationPolicy 会拒绝 — 失败项进 Vec，
        // 但函数本身不 panic。这是预期行为（启动健壮性 > 完美加固，ADR-0008）。
        let _failed = apply_self_mitigations();
    }
}
