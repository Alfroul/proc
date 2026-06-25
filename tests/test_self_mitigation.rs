//! v0.6.0 阶段 2 — self_mitigation 集成测试。
//!
//! 验收点：
//! - 调 apply_self_mitigations() 不 panic，返回 Vec<&'static str>
//! - 第二次调用幂等（部分策略拒绝是预期行为）
//! - 非 Windows 平台返回空 Vec（API 兼容性占位）
//!
//! 说明：Windows 上某些 mitigation（特别是 DEP Permanent）如果镜像已通过
//! `/NXCOMPAT` 链接器标志默认开启，运行时再次 SetProcessMitigationPolicy 会
//! 返回错误。这是预期行为 — 失败项进 `failed` 列表但函数不 panic，
//! 上层 main.rs 通过 eprintln! 提示用户。本测试只验证调用语义。

use proc::security::self_mitigation::apply_self_mitigations;

#[test]
fn apply_self_mitigations_no_panic_returns_vec() {
    let _failed = apply_self_mitigations();
    // 不 panic 即可；具体失败项视 Windows 版本 / 镜像 default mitigation 而定
}

#[test]
fn apply_self_mitigations_idempotent_second_call_no_panic() {
    // 第一次调用可能成功应用；第二次因 Permanent 标记拒绝，但函数本身不 panic。
    let _ = apply_self_mitigations();
    let failed = apply_self_mitigations();
    let _ = failed;
}

#[test]
fn non_windows_returns_empty_vec() {
    #[cfg(windows)]
    {
        // 仅做编译路径占位 — Windows 上跳过断言
    }
    #[cfg(not(windows))]
    {
        let failed = apply_self_mitigations();
        assert!(
            failed.is_empty(),
            "non-Windows should return empty: {failed:?}"
        );
    }
}
