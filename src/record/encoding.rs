//! record 模块 bincode 序列化 helper（v0.18 stage 2 切 varint 后的统一入口）。
//!
//! ## 历史
//!
//! - **v0.17 stage 3 TD-45 落地**：加 `options_for_version(version) -> impl Options`
//!   函数让 record 模块的所有 bincode serialize/deserialize 调用走统一配置层。
//!   当时所有版本走 fixint（与 `bincode::serialize` 默认等价），评估 varint ROI
//!   极低（详见 `docs/stages/v0.17-stage-3.md` 决策 4 段）。
//! - **v0.18 stage 1 Spike**：加 `version >= 4` 分支 stub（暂仍走 fixint），
//!   让 stage 2 切 varint 时只需替换 stub 标记的分支。
//! - **v0.18 stage 2**：bump `RECORDING_VERSION` 3 → 4 + `version >= 4` 走 varint
//!   （新文件 size 更小 ~10-15%）+ 旧文件 v1/v2/v3 走 fixint 兼容层。**接口变更**：
//!   `options_for_version` 返回 `impl Options` 在两分支不同类型时编译失败
//!   （bincode 1.x `Options` trait 有 `Sized` bound 不 object-safe，无法
//!   `Box<dyn Options>`），改为 `serialize_with_version` / `deserialize_with_version`
//!   两个 helper 函数把 dispatch 收敛到函数内部。
//!
//! ## 行为
//!
//! | version | header | frame/footer |
//! |---|---|---|
//! | 1 / 2 / 3 | fixint | fixint（兼容层，旧文件可读）|
//! | 4+（v0.18+ 新文件）| fixint | varint（新文件 size 更小）|
//!
//! **关键**：header 永远走 fixint（reader.rs:69 `bincode::deserialize(&header_buf)`
//! 不走本 helper），让 reader 拿到 `header.version` 后再分支选 config。
//! `RecordingFooter::header_version` 字段镜像 header.version 用于 sanity check。
//!
//! ## 行为等价性（旧文件）
//!
//! `bincode::DefaultOptions::new().with_no_limit().with_little_endian().with_fixint_encoding()`
//! 与 `bincode::serialize` / `bincode::deserialize` 默认配置字节级等价（bincode 1.x
//! 默认就是 fixint + little-endian + no_limit）。既有 v1/v2/v3 `.prec` 文件零迁移。

use bincode::Options;
use serde::{Serialize, de::DeserializeOwned};

/// bincode base 配置（no_limit + little_endian），varint/fixint 分支共用。
fn base_options() -> impl Options {
    bincode::DefaultOptions::new()
        .with_no_limit()
        .with_little_endian()
}

/// 返回 fixint 配置（v0.17 stage 3 接口，v0.18 stage 2 保留兼容）。
///
/// **v0.18 stage 2 起**：record v3/v4 文件改走 [`serialize_with_version`] /
/// [`deserialize_with_version`] 按 version 分支选 varint/fixint。本函数仅返
/// fixint 配置，供 vt100 / sidecar 等**不参与 version 分支**的模块继续使用
/// （VT100 是 v2 fixint / sidecar 是 v1 fixint，都不切 varint）。
///
/// 既有 `tests/test_mcp_v0_17.rs` 也用本函数验证 v0.17 stage 3 落地的 fixint
/// 兼容层（与本函数语义一致：所有版本走 fixint）。
///
/// # Parameters
///
/// - `version`：仅用于文档化，函数内不分支（永远 fixint）
///
/// # Returns
///
/// `impl bincode::Options`（fixint + little-endian + no_limit，与 `bincode::serialize`
/// 默认字节级等价）。
#[must_use]
pub fn options_for_version(_version: u16) -> impl Options {
    base_options().with_fixint_encoding()
}

/// 按 version 序列化（v0.18 stage 2 切 varint 后的统一入口）。
///
/// - `version >= 4`：varint 编码（小数字占 byte 少，u64=1 vs 8），新文件 size 更小 ~10-15%
/// - `version <= 3`：fixint 编码（与 `bincode::serialize` 默认等价，旧文件零迁移）
///
/// # Parameters
///
/// - `version`：`RecordingHeader.version` 字段值（v1 / v2 / v3 / v4+）
/// - `value`：要序列化的 serde::Serialize 对象
///
/// # Errors
///
/// bincode 序列化失败时返 `bincode::Error`（如 IO 错误 / 序列化错误）。
pub fn serialize_with_version<S: Serialize>(
    version: u16,
    value: &S,
) -> Result<Vec<u8>, bincode::Error> {
    let base = base_options();
    if version >= 4 {
        base.with_varint_encoding().serialize(value)
    } else {
        base.with_fixint_encoding().serialize(value)
    }
}

/// 按 version 反序列化（v0.18 stage 2 切 varint 后的统一入口）。
///
/// - `version >= 4`：varint 解码
/// - `version <= 3`：fixint 解码（与 `bincode::deserialize` 默认等价）
///
/// # Parameters
///
/// - `version`：`RecordingHeader.version` 字段值（v1 / v2 / v3 / v4+）
/// - `bytes`：要反序列化的字节切片
///
/// # Errors
///
/// bincode 反序列化失败时返 `bincode::Error`（如字节流损坏 / 类型不匹配）。
pub fn deserialize_with_version<D: DeserializeOwned>(
    version: u16,
    bytes: &[u8],
) -> Result<D, bincode::Error> {
    let base = base_options();
    if version >= 4 {
        base.with_varint_encoding().deserialize(bytes)
    } else {
        base.with_fixint_encoding().deserialize(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::frame::{RECORDING_MAGIC, RECORDING_VERSION, RecordingHeader};

    #[test]
    fn legacy_v1_v2_v3_equivalent_to_default_serialize() {
        // v1 / v2 / v3 旧文件走 fixint 默认配置（兼容层，与 bincode::serialize 字节级等价）
        for v in [1_u16, 2_u16, 3_u16] {
            let header = RecordingHeader {
                magic: *RECORDING_MAGIC,
                version: v,
                start_time: 1_700_000_000,
                hostname: format!("legacy-v{v}"),
            };
            let bytes_via_helper =
                serialize_with_version(v, &header).expect("serialize via helper");
            let bytes_default = bincode::serialize(&header).expect("default serialize");
            assert_eq!(bytes_via_helper, bytes_default, "v{v} bytes mismatch");
        }
    }

    #[test]
    fn options_for_version_returns_fixint_for_compat() {
        // v0.18 stage 2：options_for_version 保留为 fixint-only 兼容函数
        // （vt100 / sidecar / v0.17 test 继续用，不参与 version 分支）
        let header = RecordingHeader {
            magic: *RECORDING_MAGIC,
            version: 3,
            start_time: 1_700_000_000,
            hostname: "compat".to_string(),
        };
        let bytes_via_opts = options_for_version(3)
            .serialize(&header)
            .expect("serialize via opts");
        let bytes_default = bincode::serialize(&header).expect("default serialize");
        assert_eq!(
            bytes_via_opts, bytes_default,
            "options_for_version 应返 fixint"
        );
    }

    #[test]
    fn v4_uses_varint() {
        // v0.18 stage 2：version >= 4 走 varint，与 bincode::serialize 默认 fixint
        // 字节流不同（同 RecordingHeader round-trip 字段一致但字节流不同）
        let header = RecordingHeader {
            magic: *RECORDING_MAGIC,
            version: RECORDING_VERSION, // = 4（v0.18 stage 2 bump）
            start_time: 1_700_000_000,
            hostname: "v4-varint".to_string(),
        };
        let bytes_via_helper =
            serialize_with_version(RECORDING_VERSION, &header).expect("serialize via helper");
        let bytes_default_fixint = bincode::serialize(&header).expect("default serialize");
        assert_ne!(
            bytes_via_helper, bytes_default_fixint,
            "v4 varint 字节流应与 fixint 默认不同"
        );
    }

    #[test]
    fn v4_varint_round_trip() {
        // v0.18 stage 2：v4 varint round-trip 验证（字段一致但字节流与 v3 fixint 不同）
        let header = RecordingHeader {
            magic: *RECORDING_MAGIC,
            version: RECORDING_VERSION,
            start_time: 1_700_000_000,
            hostname: "v4-varint-round-trip".to_string(),
        };
        let bytes = serialize_with_version(RECORDING_VERSION, &header).expect("serialize");
        let back: RecordingHeader =
            deserialize_with_version(RECORDING_VERSION, &bytes).expect("deserialize");
        assert_eq!(back.version, header.version);
        assert_eq!(back.hostname, header.hostname);
    }

    #[test]
    fn v4_varint_smaller_than_fixint_for_small_numbers() {
        // v0.18 stage 2：varint 对小数字占 byte 少（u64=1 byte vs fixint 8 byte）
        let header_small = RecordingHeader {
            magic: *RECORDING_MAGIC,
            version: RECORDING_VERSION,
            start_time: 100, // 小数字 → varint 占 1 byte / fixint 占 8 byte
            hostname: "small".to_string(),
        };
        let bytes_varint =
            serialize_with_version(RECORDING_VERSION, &header_small).expect("varint serialize");
        let bytes_fixint = bincode::serialize(&header_small).expect("fixint serialize");
        assert!(
            bytes_varint.len() < bytes_fixint.len(),
            "varint ({}) 应比 fixint ({}) 短",
            bytes_varint.len(),
            bytes_fixint.len()
        );
    }

    #[test]
    fn v3_fixint_round_trip() {
        // v3 旧文件 fixint round-trip 验证（与 v4 varint 字节流不同但 round-trip 一致）
        let header = RecordingHeader {
            magic: *RECORDING_MAGIC,
            version: 3,
            start_time: 1_700_000_000,
            hostname: "v3-fixint-round-trip".to_string(),
        };
        let bytes = serialize_with_version(3, &header).expect("serialize");
        let back: RecordingHeader = deserialize_with_version(3, &bytes).expect("deserialize");
        assert_eq!(back.version, header.version);
        assert_eq!(back.hostname, header.hostname);
    }
}
