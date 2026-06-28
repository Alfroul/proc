//! Schannel ETW SNI 解析器：v0.10 阶段 2 实装。
//!
//! 阶段 2 实测结论（管理员 + logman/tracerpt 探测，2026-06-28）：
//! - **真实 provider**：`Microsoft-Windows-Schannel-Events`
//!   GUID `{91CC1150-71AA-47E2-AE18-C96E61736B6F}`
//!   （**不是** ADR-0018 §3 阶段 1 推测的 `{37D2C3CD-...}`，原 GUID
//!   `Security: SChannel` 实测对 curl TLS handshake **不 fire 任何 event**）
//! - **SNI event ID = 1793**（DeleteSecurityContext Start, Task=28672, Opcode=1）
//!   （**不是** ADR-0018 §3 阶段 1 推测的 196；实测 196 完全不出现）
//! - **SNI 字段名 = `TargetName`**（不是 `ServerName`）
//! - 字段 layout：2 个 top-level property：`ContextHandle` (u64 pointer) + `TargetName`
//!   (UTF-16 LE null-terminated string)
//! - **PID 来源**：`EVENT_HEADER.ProcessId`（Schannel 是用户态 provider，自带 PID，
//!   **不复用** disk_io_etw 的 thread→pid map）
//!
//! 解析路径（ADR-0018 §3 TDH 动态 schema）：用 `TdhGetEventInformation` 拉
//! TRACE_EVENT_INFO → 遍历 property 数组找 `TargetName` → 用
//! `TdhGetPropertySize` 累加算 offset → 从 UserData 读 UTF-16 LE 串。
//! 不硬编码字段顺序 / 偏移，跨 Win10/Win11 版本兼容（manifest 可扩字段）。
//!
//! 跨平台布局：本文件只放 `SniRecord` 数据结构 + 纯 UTF-16 LE 读串辅助函数
//! （单测覆盖）；Windows-specific TDH 调用在 `provider.rs`。

use std::time::SystemTime;

/// Schannel ETW worker → 主线程的 SNI 嗅探记录。
///
/// 对应 v0.10 stage 2 doc 任务 2 的 `SchannelEvent`（实际命名为 `SniRecord`
/// 与 disk_io_etw::DiskIoStats 风格一致）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SniRecord {
    /// 发起 TLS handshake 的进程 PID（来自 EVENT_HEADER.ProcessId）。
    pub pid: u32,
    /// TLS SNI 明文（Schannel event 1793 `TargetName` 字段；UTF-16 LE → String）。
    pub sni: String,
    /// 事件时间戳（来自 EVENT_HEADER.TimeStamp → SystemTime）。
    pub ts: SystemTime,
}

/// 从字节切片读 UTF-16 LE 串，直到首个 null terminator（u16 == 0）或切片耗尽。
///
/// 单测覆盖：构造 `[c, 0, c, 0, ..., 0, 0]` → 正确 String。供 `provider.rs`
/// 的 TDH 路径在拿到 `TargetName` 的 data offset + size 后调本函数解串。
///
/// 输入字节长度必须为偶数；若为奇数末尾补 0x00（防御性，正常 TDH 路径
/// 不会出现奇数长度）。
#[must_use]
pub fn read_utf16_le_until_null(bytes: &[u8]) -> String {
    let mut u16s: Vec<u16> = Vec::with_capacity(bytes.len() / 2 + 1);
    let mut i = 0;
    while i + 1 < bytes.len() {
        let code_unit = u16::from_le_bytes([bytes[i], bytes[i + 1]]);
        if code_unit == 0 {
            break;
        }
        u16s.push(code_unit);
        i += 2;
    }
    String::from_utf16_lossy(&u16s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SniRecord struct 编译契约：可构造 + 可 Clone + 可 Serialize。
    #[test]
    fn sni_record_is_constructible() {
        let rec = SniRecord {
            pid: 1234,
            sni: "example.com".into(),
            ts: SystemTime::UNIX_EPOCH,
        };
        let json = serde_json::to_string(&rec).expect("serialize");
        assert!(json.contains("\"sni\":\"example.com\""));
        assert!(json.contains("\"pid\":1234"));
    }

    /// read_utf16_le_until_null：基础 ASCII 域名 + null terminator。
    #[test]
    fn read_utf16_ascii_with_null() {
        // "example.com" + NUL
        let mut bytes: Vec<u8> = Vec::new();
        for c in "example.com".encode_utf16() {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0]); // null terminator
        assert_eq!(read_utf16_le_until_null(&bytes), "example.com");
    }

    /// 不带 null terminator：读到切片耗尽为止（防御性，正常 TDH 路径
    /// 通常包含 null）。
    #[test]
    fn read_utf16_without_null_terminator() {
        let mut bytes: Vec<u8> = Vec::new();
        for c in "www.bing.com".encode_utf16() {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
        assert_eq!(read_utf16_le_until_null(&bytes), "www.bing.com");
    }

    /// 空 / 单字节 / 仅有 null：返回空 String（不 panic）。
    #[test]
    fn read_utf16_empty_and_short_inputs() {
        assert_eq!(read_utf16_le_until_null(&[]), "");
        assert_eq!(read_utf16_le_until_null(&[0xAB]), ""); // 奇数长度单字节
        assert_eq!(read_utf16_le_until_null(&[0, 0]), ""); // 只剩 null
    }

    /// 含 CJK / 非 ASCII code unit（验证 UTF-16 LE 路径不丢字节）。
    /// 虽然 SNI 实际只有 ASCII，但 `String::from_utf16_lossy` 必须正确处理。
    #[test]
    fn read_utf16_cjk_code_units() {
        // "中文" = [0x4E2D, 0x6587]
        let bytes: Vec<u8> = vec![0x2D, 0x4E, 0x87, 0x65, 0, 0];
        assert_eq!(read_utf16_le_until_null(&bytes), "中文");
    }
}
