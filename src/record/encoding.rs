//! v0.17 stage 3 TD-45：record 模块 bincode 选项层。
//!
//! ## 背景
//!
//! brainstorm 决策 3 用户拍板 TD-45 全实装「bincode varint vs fixint 切换 + 旧
//! `.prec` 文件兼容层」。stage 3 落地评估后发现 varint 切换 ROI 极低（详见
//! `docs/stages/v0.17-stage-3.md` 决策 4 段）：
//!
//! - **replay 偶发触发**，165 µs 单帧 seek 完全无感（PERF-BASELINE-v0.13 §4 实测）；
//!   30 min × 30 FPS × 1000 进程连续 seek 才有感知（agent 实际场景罕见）
//! - **varint vs fixint 性能差异**：varint 让小数字占少 byte（u64=1 vs 8 byte），
//!   但 parse 时需 condition branch（每 byte 检查 high bit）；fixint 直接 memcpy。
//!   bincode 1.x varint 实测比 fixint 慢 1.5-2x（serialize + deserialize 都慢）
//! - **影响 stage 5 VT100 转码**：v0.17 stage 5 落地 VT100 → UiFrame 转换器
//!   写入 v3 文件（fixint），如 stage 3 bump 到 v4 varint，stage 5 需协调版本号
//! - **breaking change**：v0.17 写的 v4 varint 文件 v0.16 proc 读不了
//!
//! ## 决策：选项层 + 评估文档化（**不切 varint**）
//!
//! 当前所有版本（v1 / v2 / v3）返 fixint 配置（与 `bincode::serialize` 默认等价）。
//! record 模块的所有 `bincode::serialize` / `bincode::deserialize` 调用改走
//! [`options_for_version`]，让未来 v0.18+ cycle 评估「网络传输录屏文件」场景时
//! （如远程 agent / Web dashboard），只需改本文件 + bump `RECORDING_VERSION` 即可。
//!
//! ## 行为等价性
//!
//! `bincode::DefaultOptions::new().with_no_limit().with_little_endian().with_fixint_encoding()`
//! 与 `bincode::serialize` / `bincode::deserialize` 默认配置字节级等价（bincode 1.x
//! 默认就是 fixint + little-endian + no_limit）。既有 `.prec` 文件零迁移。
//!
//! ## 演进路径（v0.18 cycle 项 2 落地）
//!
//! **v0.18 stage 1 Spike**：加 `version >= 4` 分支 stub（**暂仍走 fixint**），
//! 让 stage 2 切 varint 时只需替换 stage 1 Spike 标记的分支。当前实现：
//!
//! ```ignore
//! pub fn options_for_version(version: u16) -> impl Options {
//!     let base = bincode::DefaultOptions::new().with_no_limit().with_little_endian();
//!     if version >= 4 {
//!         // v0.18 stage 1 Spike stub：暂仍走 fixint，stage 2 切到 varint
//!         // base.with_varint_encoding()
//!         base.with_fixint_encoding()
//!     } else {
//!         base.with_fixint_encoding()
//!     }
//! }
//! ```
//!
//! **v0.18 stage 2 实装**：bump `RECORDING_VERSION` 3 → 4 + writer 写新文件用
//! varint（替换 stage 1 Spike 标记的分支为 `base.with_varint_encoding()`）+
//! reader 按 `header.version` 选 config（旧 v1/v2/v3 走 fixint 兼容层，新 v4+
//! 走 varint）。详见 ADR-0027 § Migration path + REVIEW-v0.17 §1 Findings P2-A1。

use bincode::Options;

/// 返回指定文件版本对应的 bincode 配置。
///
/// 当前所有版本（v1 / v2 / v3 / v4+ stub）返 fixint 配置（与 `bincode::serialize`
/// 默认等价）。**v0.18 stage 1 Spike 加 `version >= 4` 分支 stub 暂仍走 fixint**，
/// stage 2 实装时切换为 varint（bump `RECORDING_VERSION` 3 → 4 + writer 写新文件
/// varint + reader 按 `header.version` 选 config）。
///
/// # Parameters
///
/// - `version`：`RecordingHeader.version` 字段值（v1 / v2 / v3 / v4+ stub）
///
/// # Returns
///
/// `impl bincode::Options`（具体类型由 bincode 派生的 wrapper chain，编译期已知；
/// bincode config wrapper 都 `Copy + Clone`，但 `impl Options` opaque type 编译器
/// 不自动推断 Copy——调用方在 loop 中需每次调本函数创建新实例，或用 `&opts` + ref
/// 调用）。
///
/// **关键**：bincode 1.3.3 的 `DefaultOptions::new()` 默认是 **varint** 编码
/// （不是 fixint！），与 `bincode::serialize` 默认 fixint 行为**不一致**。本函数
/// 显式 `.with_fixint_encoding()` 确保 v1/v2/v3 文件用 fixint 编码，与既有
/// `.prec` 文件字节级等价。
#[must_use]
pub fn options_for_version(version: u16) -> impl Options {
    let base = bincode::DefaultOptions::new()
        .with_no_limit()
        .with_little_endian();
    if version >= 4 {
        // v0.18 stage 1 Spike stub：version >= 4 分支暂仍走 fixint，stage 2 切换为
        // `base.with_varint_encoding()` 让新文件 size 更小（30 min × 30 FPS × 1000
        // 进程录屏 ~10-15% size 下降）。详见 REVIEW-v0.17 §1 Findings P2-A1 +
        // 本文件 doc comment §演进路径。
        base.with_fixint_encoding()
    } else {
        base.with_fixint_encoding()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::frame::{RECORDING_MAGIC, RECORDING_VERSION, RecordingHeader};
    use bincode::Options;

    #[test]
    fn options_for_version_3_equivalent_to_default_serialize() {
        // v3 文件 options_for_version 与 bincode::serialize 默认配置字节级等价
        let header = RecordingHeader {
            magic: *RECORDING_MAGIC,
            version: RECORDING_VERSION,
            start_time: 1_700_000_000,
            hostname: "test-host".to_string(),
        };
        let bytes_via_opts = options_for_version(RECORDING_VERSION)
            .serialize(&header)
            .expect("serialize via opts");
        let bytes_default = bincode::serialize(&header).expect("default serialize");
        assert_eq!(bytes_via_opts, bytes_default, "v3 bytes mismatch");
    }

    #[test]
    fn options_for_version_round_trip_v3() {
        let header = RecordingHeader {
            magic: *RECORDING_MAGIC,
            version: RECORDING_VERSION,
            start_time: 1_700_000_000,
            hostname: "round-trip-test".to_string(),
        };
        // impl Options 不是 Copy，每次调用创建新实例
        let bytes = options_for_version(RECORDING_VERSION)
            .serialize(&header)
            .expect("serialize");
        let back: RecordingHeader = options_for_version(RECORDING_VERSION)
            .deserialize(&bytes)
            .expect("deserialize");
        assert_eq!(back.magic, header.magic);
        assert_eq!(back.version, header.version);
        assert_eq!(back.start_time, header.start_time);
        assert_eq!(back.hostname, header.hostname);
    }

    #[test]
    fn options_for_version_legacy_v1_v2_equivalent_to_default() {
        // v1 / v2 旧文件也走 fixint 默认配置（与 stage 3 前行为一致）
        for v in [1_u16, 2_u16] {
            let header = RecordingHeader {
                magic: *RECORDING_MAGIC,
                version: v,
                start_time: 1_700_000_000,
                hostname: format!("legacy-v{v}"),
            };
            let bytes_via_opts = options_for_version(v)
                .serialize(&header)
                .expect("serialize via opts");
            let bytes_default = bincode::serialize(&header).expect("default serialize");
            assert_eq!(bytes_via_opts, bytes_default, "v{v} bytes mismatch");
        }
    }

    #[test]
    fn options_for_version_v4_stub_still_uses_fixint() {
        // v0.18 stage 1 Spike：version >= 4 分支暂仍走 fixint（与 bincode::serialize
        // 默认字节级等价）。stage 2 切换为 varint 后本测试改为对比 varint 字节流。
        let header = RecordingHeader {
            magic: *RECORDING_MAGIC,
            version: 4, // v0.18 stage 1 Spike 占位版本号（RECORDING_VERSION 仍为 3）
            start_time: 1_700_000_000,
            hostname: "v4-stub".to_string(),
        };
        let bytes_via_opts = options_for_version(4)
            .serialize(&header)
            .expect("serialize via opts");
        let bytes_default = bincode::serialize(&header).expect("default serialize");
        assert_eq!(
            bytes_via_opts, bytes_default,
            "v4 stage 1 Spike stub 应与 fixint 默认字节级等价"
        );
    }

    #[test]
    fn options_for_version_v4_stub_round_trip() {
        // v0.18 stage 1 Spike：version 4 stub round-trip 验证（stage 2 切 varint
        // 后此测试需保留——varint round-trip 也是合法的，只是字节流不同）。
        let header = RecordingHeader {
            magic: *RECORDING_MAGIC,
            version: 4,
            start_time: 1_700_000_000,
            hostname: "v4-stub-round-trip".to_string(),
        };
        let bytes = options_for_version(4)
            .serialize(&header)
            .expect("serialize");
        let back: RecordingHeader = options_for_version(4)
            .deserialize(&bytes)
            .expect("deserialize");
        assert_eq!(back.version, header.version);
        assert_eq!(back.hostname, header.hostname);
    }
}
