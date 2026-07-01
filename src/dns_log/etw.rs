//! DNS ETW provider：`Microsoft-Windows-DNS-Client` 实时 session 抓 event 3008/3010。
//!
//! v0.11 阶段 2 落地，对应 [ADR-0020](../../docs/adr/0020-dns-etw-provider.md)。
//!
//! # 设计要点（与 `schannel_etw/provider.rs` 同款模板）
//!
//! - **Provider GUID** `{1C95126E-7EEA-49A9-A3FE-A9FB58F46014}`
//!   （`Microsoft-Windows-DNS-Client`，Microsoft 公开文档 + logman 实证）
//! - **Event fast-filter**：3008（QueryResponseEx）/ 3010（QueryCompletedEx）
//!   都含 `QueryName` / `QueryType` / `QueryStatus` / `QueryResults` 字段。
//!   两者都抓保证完整性——3008 在某些 Win 版本对 NXDOMAIN/Timeout 不触发，
//!   3010 是「查询完成」语义更全；去重靠主线程 `App::dns_log_recent` FIFO + 时间戳。
//! - **TDH 动态 schema**（同 ADR-0018 §3 路线）：`TdhGetEventInformation` 拉
//!   `TRACE_EVENT_INFO` → 遍历 top-level properties 按 name 找字段 → 用
//!   `TdhGetPropertySize` 累计 offset → 从 `UserData` 读 UTF-16 LE 串 / UInt32。
//!   **不硬编码字段顺序 / 偏移** —— manifest 加字段时仍能找到。
//! - **PID 来源**：`EVENT_HEADER.ProcessId`（用户态 provider 自带 PID，**不复用**
//!   disk_io_etw 的 thread→pid map）。Win10 1607+ Dnscache service 已记录
//!   originating PID 到 EVENT_HEADER；早期版本可能指向 svchost。
//! - **process_name / start_time**：callback 只填 query 字段（process_name 留空），
//!   `drain()` 在 worker 线程 lookup（与 PowerShell 路径同款 `PidNameLookup`）。
//!
//! # 失败模式（全部降级，返回 `Err` 让 `detect_collector` fallback PowerShell）
//!
//! - `StartTraceW` 失败：非管理员 / session name 已被占用
//! - `EnableTraceEx2` 失败：provider GUID 错误 / DNS-Client service 未启动
//! - `OpenTraceW` / ProcessTrace spawn 失败：罕见，通常内存不足或线程 ulimit
//! - x86 (32-bit)：cfg-gate 直接拒绝（pointer-size 与 manifest 偏移不稳）
//! - 非 Windows：cfg-gate 编译 stub，整个模块不存在 `EtwDnsCollector`

#![cfg(target_os = "windows")]

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use windows::Win32::System::Diagnostics::Etw::{
    CONTROLTRACE_HANDLE, CloseTrace, ControlTraceW, ENABLE_TRACE_PARAMETERS,
    EVENT_CONTROL_CODE_ENABLE_PROVIDER, EVENT_PROPERTY_INFO, EVENT_RECORD,
    EVENT_TRACE_CONTROL_STOP, EVENT_TRACE_FLAG, EVENT_TRACE_LOGFILEW, EVENT_TRACE_PROPERTIES,
    EVENT_TRACE_REAL_TIME_MODE, EnableTraceEx2, OpenTraceW, PEVENT_RECORD_CALLBACK,
    PROCESS_TRACE_MODE_EVENT_RECORD, PROCESS_TRACE_MODE_RAW_TIMESTAMP,
    PROCESS_TRACE_MODE_REAL_TIME, PROCESSTRACE_HANDLE, PROPERTY_DATA_DESCRIPTOR, ProcessTrace,
    StartTraceW, TRACE_EVENT_INFO, TdhGetEventInformation, TdhGetPropertySize,
    WNODE_FLAG_TRACED_GUID,
};
use windows::core::GUID;
use windows::core::PCWSTR;

use crate::dns_log::windows_dns::PidNameLookup;
use crate::dns_log::{DnsLogCollector, DnsQuery, DnsResult, parse_query_type};
use crate::error::{ProcError, Result};

/// `Microsoft-Windows-DNS-Client` ETW provider GUID。
///
/// 来源：Microsoft _MSNT_DNSClientManifest manifest + logman create trace 实证。
/// 与 ADR-0020 §决策 1 一致。
const DNS_CLIENT_PROVIDER_GUID: GUID = GUID::from_values(
    0x1C95126E,
    0x7EEA,
    0x49A9,
    [0xA3, 0xFE, 0xA9, 0xFB, 0x58, 0xF4, 0x60, 0x14],
);

/// event 3008 (`QueryResponseEx`)：查询响应到达时触发。
/// 含完整 QueryResults 字段，与 3010 schema 相同。
const DNS_EVENT_QUERY_RESPONSE: u16 = 3008;

/// event 3010 (`QueryCompletedEx`)：查询完成（成功/失败均可）。
/// 与 PowerShell 路径 `Id=3010` 同款契约，fallback 时数据形态一致。
const DNS_EVENT_QUERY_COMPLETED: u16 = 3010;

/// 自定义 session name + 末尾 NUL 满足 PCWSTR（非 NT Kernel Logger）。
const SESSION_NAME: &str = "proc-dns-client\0";

/// TRACE_LEVEL_VERBOSE = 5：抓全级别事件（DNS-Client events 多在 Informational 级）。
const TRACE_LEVEL_VERBOSE: u8 = 5;

/// Win32 ERROR_INSUFFICIENT_BUFFER（122）：TdhGetEventInformation 第一调预期返回。
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

/// 累加 buffer：自上次 drain 以来 callback 解出的 DnsQuery 列表（process_name 留空，
/// drain 时由 `PidNameLookup` 填实）。
type AccumVec = Vec<DnsQuery>;
type SharedAccum = Arc<Mutex<AccumVec>>;

thread_local! {
    /// callback 内访问的累加 buffer（与 schannel_etw / disk_io_etw 同款模式）。
    /// thread_local 绑定在 `EtwDnsCollector::new` 内 spawn 的 trace_thread 闭包里设置，
    /// **不能**在 `open_trace_with_callback` 里做——后者在 spawning 线程调用，
    /// callback 实际在 ProcessTrace 线程触发。
    static CALLBACK_ACCUM: std::cell::RefCell<Option<SharedAccum>> =
        const { std::cell::RefCell::new(None) };
}

/// DNS ETW collector：实现 [`DnsLogCollector`]，与 [`super::windows_dns::PowershellDnsCollector`]
/// 同接口。`detect_collector()` 先尝试 ETW → 失败 fallback PowerShell。
pub struct EtwDnsCollector {
    accum: SharedAccum,
    session_handle: CONTROLTRACE_HANDLE,
    trace_handle: PROCESSTRACE_HANDLE,
    trace_thread: Option<JoinHandle<()>>,
    /// PID → (name, start_time) lookup，drain 时填到 DnsQuery。
    /// 在 worker 线程跑（drain 在 worker body 内调用），不需 Sync。
    pid_lookup: PidNameLookup,
}

impl EtwDnsCollector {
    /// 启动 ETW session + callback 线程。失败返 `Err` 让调用方 fallback。
    pub fn new() -> Result<Self> {
        // x86 (32-bit) 拒绝：pointer-size 与 manifest 偏移不稳。
        #[cfg(not(target_pointer_width = "64"))]
        {
            return Err(ProcError::monitor(
                "DNS ETW 仅支持 x64 Windows（32-bit 偏移不稳）",
            ));
        }

        #[cfg(target_pointer_width = "64")]
        {
            let accum: SharedAccum = Arc::new(Mutex::new(Vec::new()));

            // 1) StartTraceW 开 session
            let session_handle = start_dns_session().map_err(|e| {
                tracing::warn!(
                    error = %e,
                    "DNS ETW StartTraceW 失败（非管理员？session 已被占用？）"
                );
                ProcError::monitor(format!("DNS ETW StartTraceW 失败: {e}"))
            })?;

            // 2) EnableTraceEx2 启用 DNS-Client provider
            if let Err(e) = enable_dns_provider(session_handle) {
                tracing::warn!(error = %e, "DNS ETW EnableTraceEx2 失败");
                let _ = stop_session(session_handle);
                return Err(ProcError::monitor(format!(
                    "DNS ETW EnableTraceEx2 失败: {e}"
                )));
            }

            // 3) OpenTraceW + 注册 callback
            let trace_handle = match open_trace_with_callback() {
                Ok(h) => h,
                Err(e) => {
                    tracing::warn!(error = %e, "DNS ETW OpenTraceW 失败");
                    let _ = stop_session(session_handle);
                    return Err(ProcError::monitor(format!("DNS ETW OpenTraceW 失败: {e}")));
                }
            };

            // 4) ProcessTrace 阻塞线程：callback 在此线程触发。
            //    thread_local 必须在 ProcessTrace 调用前于**同一线程**设置。
            //    spawn 失败时清理 session + trace handle（同 schannel_etw REVIEW-11 P1-2 修复）。
            let accum_for_thread = Arc::clone(&accum);
            let trace_thread = std::thread::Builder::new()
                .name("dns-etw-process".into())
                .spawn(move || {
                    CALLBACK_ACCUM.with(|cell| {
                        *cell.borrow_mut() = Some(Arc::clone(&accum_for_thread));
                    });
                    let handles = [trace_handle];
                    // SAFETY: trace_handle 来自 OpenTraceW，VALID_HANDLE；handles 数组
                    // 生命周期覆盖整个 ProcessTrace 调用。
                    let _ = unsafe { ProcessTrace(&handles, None, None) };
                })
                .map_err(|e| {
                    tracing::warn!(
                        error = %e,
                        "DNS ETW ProcessTrace 线程 spawn 失败，清理已启 handle"
                    );
                    let _ = stop_session(session_handle);
                    // SAFETY: trace_handle 来自 OpenTraceW 成功返回；ProcessTrace
                    // 未调用过（trace_thread 没 spawn 起来），CloseTrace 仍合法（幂等）。
                    unsafe {
                        let _ = CloseTrace(trace_handle);
                    }
                    ProcError::monitor(format!("DNS ETW ProcessTrace 线程 spawn 失败: {e}"))
                })?;

            Ok(Self {
                accum,
                session_handle,
                trace_handle,
                trace_thread: Some(trace_thread),
                pid_lookup: PidNameLookup::new(),
            })
        }
    }
}

impl DnsLogCollector for EtwDnsCollector {
    fn drain(&mut self) -> Vec<DnsQuery> {
        let mut queries: Vec<DnsQuery> =
            std::mem::take(&mut *self.accum.lock().expect("dns accum poisoned"));
        // callback 留空 process_name；drain 时（worker 线程）lookup 填实。
        // 与 PowerShell 路径 reader_loop 同款逻辑（start_time 一致性检查 + 10s 刷新）。
        for q in &mut queries {
            let (name, start_time) = self.pid_lookup.lookup(q.pid);
            q.process_name = name;
            q.start_time = start_time;
        }
        queries
    }

    fn provider_name(&self) -> &'static str {
        "windows-etw"
    }
}

impl Drop for EtwDnsCollector {
    fn drop(&mut self) {
        // 1) stop session 让 ProcessTrace 返回
        let _ = stop_session(self.session_handle);
        // 2) join ProcessTrace 线程
        if let Some(h) = self.trace_thread.take() {
            let _ = h.join();
        }
        // 3) CloseTrace 幂等（重复调用仅返 ERROR_INVALID_HANDLE）
        // SAFETY: trace_handle 来自 OpenTraceW 成功返回；ProcessTrace 已返回（join 已完成），
        // 此处 CloseTrace 是合法清理。
        unsafe {
            let _ = CloseTrace(self.trace_handle);
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────
// StartTraceW + EnableTraceEx2 + OpenTraceW
// ──────────────────────────────────────────────────────────────────────────

/// `SESSION_NAME` 的 UTF-16 wide 版本（已含末尾 NUL）。
fn session_name_wide() -> Vec<u16> {
    SESSION_NAME.encode_utf16().collect()
}

/// 启动 DNS-Client ETW session（与 schannel_etw 同款 EVENT_TRACE_PROPERTIES 布局）。
fn start_dns_session() -> windows::core::Result<CONTROLTRACE_HANDLE> {
    let logger_name_wide = session_name_wide();
    let logger_name_bytes = logger_name_wide.len() * 2;
    let file_name_bytes = 2; // 空 wide string（仅 NUL）
    let buf_size =
        std::mem::size_of::<EVENT_TRACE_PROPERTIES>() + logger_name_bytes + file_name_bytes;
    let mut buf: Vec<u8> = vec![0u8; buf_size];

    // SAFETY: buf 是我们刚才分配的连续内存；EVENT_TRACE_PROPERTIES 是 #[repr(C)]
    // POD，对齐满足。props 只在我们 drop buf 之前有效。
    let props = unsafe { &mut *(buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES) };
    props.Wnode.BufferSize = buf_size as u32;
    props.Wnode.Flags = WNODE_FLAG_TRACED_GUID;
    props.Wnode.ClientContext = 1; // QPC 时间戳
    props.LoggerNameOffset = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32;
    props.LogFileNameOffset = props.LoggerNameOffset + logger_name_bytes as u32;
    // DNS-Client 是用户态 provider，EnableFlags 不用（NT Kernel Logger 专用）。
    props.EnableFlags = EVENT_TRACE_FLAG(0);
    props.LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
    props.BufferSize = 64; // KB
    props.MinimumBuffers = 20;
    props.MaximumBuffers = 100;

    let mut session_handle = CONTROLTRACE_HANDLE::default();
    let logger_name_pcwstr = PCWSTR(logger_name_wide.as_ptr());
    // SAFETY: 见上述 props 安全说明；session_handle 是 out 参数。
    let err = unsafe {
        StartTraceW(
            &mut session_handle,
            logger_name_pcwstr,
            buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES,
        )
    };
    err.ok()?;
    Ok(session_handle)
}

/// `EnableTraceEx2` 启用 DNS-Client provider（level=verbose, keyword=all）。
fn enable_dns_provider(session_handle: CONTROLTRACE_HANDLE) -> windows::core::Result<()> {
    let params = ENABLE_TRACE_PARAMETERS {
        Version: 1,
        ..Default::default()
    };
    // SAFETY: EnableTraceEx2 是 standard ETW API；session_handle 来自 StartTraceW。
    // MatchAnyKeyword=全 1：与 schannel_etw 同款（DNS-Client event keyword 文档不完整，
    // 显式全 1 最稳，避免漏抓）。
    let err = unsafe {
        EnableTraceEx2(
            session_handle,
            &DNS_CLIENT_PROVIDER_GUID,
            EVENT_CONTROL_CODE_ENABLE_PROVIDER.0,
            TRACE_LEVEL_VERBOSE,
            0xFFFF_FFFF_FFFF_FFFF, // MatchAnyKeyword
            0,                     // MatchAllKeyword
            0,                     // Timeout
            Some(&params),
        )
    };
    err.ok()?;
    Ok(())
}

/// OpenTraceW 注册 callback。callback 收到 DNS event 3008/3010 时用 TDH 解字段。
fn open_trace_with_callback() -> windows::core::Result<PROCESSTRACE_HANDLE> {
    let logger_name_wide = session_name_wide();
    let mut logfile: EVENT_TRACE_LOGFILEW = unsafe { std::mem::zeroed() };
    logfile.LoggerName = windows::core::PWSTR(logger_name_wide.as_ptr() as *mut u16);
    logfile.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME
        | PROCESS_TRACE_MODE_EVENT_RECORD
        | PROCESS_TRACE_MODE_RAW_TIMESTAMP;
    let callback: PEVENT_RECORD_CALLBACK = Some(dns_event_callback);
    logfile.Anonymous2.EventRecordCallback = callback;
    logfile.Context = std::ptr::null_mut();

    // SAFETY: logfile 字段已正确设置；返回 INVALID_PROCESSTRACE_HANDLE 表示失败。
    let trace_handle = unsafe { OpenTraceW(&mut logfile) };
    if trace_handle.Value == u64::MAX {
        return Err(windows::core::Error::from_win32());
    }
    Ok(trace_handle)
}

/// 停止 session（drop 时调用，让 ProcessTrace 返回）。
fn stop_session(session_handle: CONTROLTRACE_HANDLE) -> windows::core::Result<()> {
    let logger_name_wide = session_name_wide();
    let logger_name_bytes = logger_name_wide.len() * 2;
    let file_name_bytes = 2;
    let buf_size =
        std::mem::size_of::<EVENT_TRACE_PROPERTIES>() + logger_name_bytes + file_name_bytes;
    let mut buf: Vec<u8> = vec![0u8; buf_size];

    // SAFETY: 同 start_dns_session。
    let props = unsafe { &mut *(buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES) };
    props.Wnode.BufferSize = buf_size as u32;
    props.LoggerNameOffset = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32;
    props.LogFileNameOffset = props.LoggerNameOffset + logger_name_bytes as u32;

    let logger_name_pcwstr = PCWSTR(logger_name_wide.as_ptr());
    // SAFETY: 同上。
    let err = unsafe {
        ControlTraceW(
            session_handle,
            logger_name_pcwstr,
            buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES,
            EVENT_TRACE_CONTROL_STOP,
        )
    };
    err.ok()?;
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────
// ETW callback + TDH parser
// ──────────────────────────────────────────────────────────────────────────

/// DNS-Client ETW callback：每 fire 一个 DNS event 调一次。
///
/// - **Fast filter**：只处理 3008 / 3010，其它直接 return（TDH 解析有 10-50μs/event
///   开销，不应每 event 都调）。
/// - **TDH 解析**：3008/3010 → `parse_dns_via_tdh` → `DnsQuery`（push 到 accum）。
/// - **PID**：来自 `EVENT_HEADER.ProcessId`（用户态 provider 自带）。
///
/// **v0.11 阶段 8 REVIEW-13 P1-1**：整段 parse + push 包 `catch_unwind`，
/// 避免 panic 跨 FFI 边界 UB（Rust 标准库语义：`extern "system"` callback
/// 内 unwind 是 undefined behavior，windows-rs ProcessTrace 不保证捕获）。
/// 同时 `accum.lock()` 改 `if let Ok(...)` 防 Mutex poison panic。与 v0.6
/// 阶段 3 worker.rs::run_poll_loop catch_unwind 同款原则——best-effort drop
/// event 而非 propagate panic。
unsafe extern "system" fn dns_event_callback(record: *mut EVENT_RECORD) {
    if record.is_null() {
        return;
    }
    // SAFETY: caller (ETW) 保证 record 非空时有效；我们已检查。
    let record = unsafe { &*record };

    let event_id = record.EventHeader.EventDescriptor.Id;
    if event_id != DNS_EVENT_QUERY_RESPONSE && event_id != DNS_EVENT_QUERY_COMPLETED {
        return;
    }

    // catch_unwind 包裹：parse + push 任何 panic 都被吞掉（best-effort drop event），
    // 避免 panic 跨 FFI 边界 UB。AssertUnwindSafe 安全——callback 内无共享可变状态
    // 除 accum（Mutex 保护）；query 是 owned DnsQuery。
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let Some(query) = parse_dns_via_tdh(record) else {
            return;
        };
        let accum_opt = CALLBACK_ACCUM.with(|cell| cell.borrow().clone());
        if let Some(accum) = accum_opt {
            // 不用 expect：drain 端 panic 时 Mutex 会 poison，下一次 lock().expect() 会
            // 再次 panic → 跨 FFI 边界 UB。poison 时静默丢这条 event。
            if let Ok(mut acc) = accum.lock() {
                acc.push(query);
            }
        }
    }));
}

/// 用 TDH 动态 schema 解 DNS event 3008/3010 的 4 个字段。
///
/// 步骤：
/// 1. `TdhGetEventInformation` 拉 TRACE_EVENT_INFO buffer（含 property 数组）。
/// 2. 遍历 top-level properties，对每个 property：
///    - 读 name（buffer[`NameOffset`..] 当 UTF-16 LE 串）。
///    - name == `QueryName` / `QueryResults` → 从 UserData[offset..offset+size] 读 UTF-16 LE 串。
///    - name == `QueryType` / `QueryStatus` → 读 4 字节 UInt32（LE）。
///    - 累计 size 到 offset（用 `TdhGetPropertySize`）。
///
/// **不硬编码字段顺序 / 偏移** —— manifest 加字段时仍能找到（Win10 1809 vs 22H2 兼容）。
fn parse_dns_via_tdh(record: &EVENT_RECORD) -> Option<DnsQuery> {
    let info_buf = tdh_get_event_info_buffer(record)?;
    let info_ptr = info_buf.as_ptr() as *const TRACE_EVENT_INFO;
    // SAFETY: info_buf 来自 TdhGetEventInformation 成功返回，按 TRACE_EVENT_INFO 对齐。
    let info = unsafe { &*info_ptr };

    let user_len = record.UserDataLength as usize;
    if record.UserData.is_null() || user_len == 0 {
        return None;
    }
    // SAFETY: ETW 保证 UserData 在 callback 期间非空 + 长度 UserDataLength bytes。
    let user_data: &[u8] =
        unsafe { std::slice::from_raw_parts(record.UserData as *const u8, user_len) };

    let mut query_name = String::new();
    let mut query_type_raw = String::new();
    let mut query_status: u32 = 0;
    let mut query_results = String::new();

    let mut offset_in_user_data = 0usize;
    let top_count = info.TopLevelPropertyCount as usize;
    for i in 0..top_count {
        let prop = property_at_index(info_ptr, i)?;
        let name = read_wide_string_at_offset(&info_buf, prop.NameOffset as usize);
        let size = tdh_get_property_size_for_index(record, &info_buf, i)?;

        let start = offset_in_user_data.min(user_data.len());
        let end = (offset_in_user_data + size).min(user_data.len());

        match name.as_str() {
            "QueryName" => {
                query_name = read_utf16_le_until_null(&user_data[start..end]);
            }
            "QueryType" if end - start >= 4 => {
                let v = u32::from_le_bytes([
                    user_data[start],
                    user_data[start + 1],
                    user_data[start + 2],
                    user_data[start + 3],
                ]);
                query_type_raw = v.to_string();
            }
            "QueryStatus" if end - start >= 4 => {
                query_status = u32::from_le_bytes([
                    user_data[start],
                    user_data[start + 1],
                    user_data[start + 2],
                    user_data[start + 3],
                ]);
            }
            "QueryResults" => {
                query_results = read_utf16_le_until_null(&user_data[start..end]);
            }
            _ => {}
        }

        offset_in_user_data = offset_in_user_data.saturating_add(size);
    }

    if query_name.is_empty() {
        return None;
    }

    let pid = record.EventHeader.ProcessId;
    if pid == 0 {
        // PID 0 = System Idle（噪声），与 PowerShell 路径同款 drop。
        return None;
    }

    let result = DnsResult::from_windows_status(query_status, &query_results);
    let ts = filetime_to_systemtime(record.EventHeader.TimeStamp);

    Some(DnsQuery {
        timestamp: ts,
        pid,
        start_time: 0, // drain 时 lookup 填
        process_name: String::new(),
        query_name,
        query_type: parse_query_type(&query_type_raw),
        result,
    })
}

/// `TdhGetEventInformation` 拉 TRACE_EVENT_INFO buffer（growing buffer 模式）。
fn tdh_get_event_info_buffer(record: &EVENT_RECORD) -> Option<Vec<u8>> {
    let mut buf_size: u32 = 0;
    // SAFETY: record 是 callback 给的有效指针；buf_size 是 out 参数；buffer=None。
    let status =
        unsafe { TdhGetEventInformation(record as *const EVENT_RECORD, None, None, &mut buf_size) };
    if status != ERROR_INSUFFICIENT_BUFFER || buf_size == 0 {
        return None;
    }

    let mut buffer: Vec<u8> = vec![0u8; buf_size as usize];
    loop {
        // SAFETY: buffer 大小已按 TDH 第一调的 required size 分配。
        let status = unsafe {
            TdhGetEventInformation(
                record as *const EVENT_RECORD,
                None,
                Some(buffer.as_mut_ptr() as *mut TRACE_EVENT_INFO),
                &mut buf_size,
            )
        };
        if status == 0 {
            return Some(buffer);
        }
        if status != ERROR_INSUFFICIENT_BUFFER {
            return None;
        }
        buffer.resize(buf_size as usize, 0);
    }
}

/// 从 TRACE_EVENT_INFO 的 property 数组取第 `idx` 个 EVENT_PROPERTY_INFO。
///
/// windows-rs 的 `TRACE_EVENT_INFO.EventPropertyInfoArray` 是 `[EVENT_PROPERTY_INFO; 1]`
/// 占位字段（C 端是 trailing flexible array），实际长度由 `PropertyCount` 决定。
fn property_at_index(
    info_ptr: *const TRACE_EVENT_INFO,
    idx: usize,
) -> Option<&'static EVENT_PROPERTY_INFO> {
    if info_ptr.is_null() {
        return None;
    }
    // SAFETY: info_ptr 来自成功的 TdhGetEventInformation；PropertyCount 是 buffer
    // 内合法 property 数。我们只读 idx < PropertyCount 的 entry。
    let info = unsafe { &*info_ptr };
    if idx as u32 >= info.PropertyCount {
        return None;
    }
    // SAFETY: TDH 保证 buffer 内 PropertyCount 个 EVENT_PROPERTY_INFO 连续可读。
    let prop_ptr = unsafe { info.EventPropertyInfoArray.as_ptr().add(idx) };
    Some(unsafe { &*prop_ptr })
}

/// 从 buffer 字节偏移 `byte_offset` 读 UTF-16 LE null-terminated 串。
fn read_wide_string_at_offset(buffer: &[u8], byte_offset: usize) -> String {
    if byte_offset >= buffer.len() {
        return String::new();
    }
    let mut units: Vec<u16> = Vec::new();
    let mut i = byte_offset;
    while i + 1 < buffer.len() {
        let code_unit = u16::from_le_bytes([buffer[i], buffer[i + 1]]);
        if code_unit == 0 {
            break;
        }
        units.push(code_unit);
        i += 2;
    }
    String::from_utf16_lossy(&units)
}

/// 用 `TdhGetPropertySize` 拿第 `idx` 个 property 的字节 size。
fn tdh_get_property_size_for_index(
    record: &EVENT_RECORD,
    info_buf: &[u8],
    idx: usize,
) -> Option<usize> {
    let info_ptr = info_buf.as_ptr() as *const TRACE_EVENT_INFO;
    let prop = property_at_index(info_ptr, idx)?;
    let name_offset = prop.NameOffset as usize;
    if name_offset + 2 > info_buf.len() {
        return None;
    }

    let mut name_buf: Vec<u16> = Vec::new();
    let mut i = name_offset;
    while i + 1 < info_buf.len() {
        let code_unit = u16::from_le_bytes([info_buf[i], info_buf[i + 1]]);
        name_buf.push(code_unit);
        i += 2;
        if code_unit == 0 {
            break;
        }
    }
    if name_buf.last() != Some(&0) {
        name_buf.push(0);
    }

    let descriptors = [PROPERTY_DATA_DESCRIPTOR {
        PropertyName: name_buf.as_ptr() as u64,
        ArrayIndex: 0,
        Reserved: 0,
    }];
    let mut size: u32 = 0;
    // SAFETY: record 是 callback 给的有效指针；descriptors 含合法 PCWSTR。
    let status =
        unsafe { TdhGetPropertySize(record as *const EVENT_RECORD, None, &descriptors, &mut size) };
    if status == 0 {
        Some(size as usize)
    } else {
        None
    }
}

/// 把 `EVENT_HEADER.TimeStamp`（FILETIME-compatible，100ns since 1601-01-01）
/// 转 `SystemTime`（UNIX_EPOCH = 1970-01-01）。与 schannel_etw 同款实现。
fn filetime_to_systemtime(filetime: i64) -> SystemTime {
    const FILETIME_UNIX_EPOCH_DELTA_HUNDREDS_NS: i64 = 116_444_736_000_000_000;
    let unix_hundreds_ns = filetime.saturating_sub(FILETIME_UNIX_EPOCH_DELTA_HUNDREDS_NS);
    let unix_ns = unix_hundreds_ns.saturating_mul(100);
    if unix_ns < 0 {
        return SystemTime::UNIX_EPOCH;
    }
    SystemTime::UNIX_EPOCH + Duration::from_nanos(unix_ns as u64)
}

/// 从字节切片读 UTF-16 LE 串，直到首个 null terminator（u16 == 0）或切片耗尽。
/// 与 `schannel_etw::parser::read_utf16_le_until_null` 同款实现（独立复制避免
/// 跨模块 pub 路径耦合，~12 行重复可接受）。
fn read_utf16_le_until_null(bytes: &[u8]) -> String {
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

// ──────────────────────────────────────────────────────────────────────────
// 单元测试
// ──────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// GUID 字面量契约：与 ADR-0020 §决策 1 一致。
    #[test]
    fn provider_guid_matches_adr() {
        assert_eq!(
            DNS_CLIENT_PROVIDER_GUID,
            GUID::from_values(
                0x1C95126E,
                0x7EEA,
                0x49A9,
                [0xA3, 0xFE, 0xA9, 0xFB, 0x58, 0xF4, 0x60, 0x14],
            )
        );
    }

    /// session name 含 NUL 终结符（PCWSTR 要求）。
    #[test]
    fn session_name_has_nul_terminator() {
        assert!(SESSION_NAME.ends_with('\0'), "SESSION_NAME 必须含 NUL");
    }

    /// event ID 契约：3008 / 3010。
    #[test]
    fn dns_event_ids_match_adr() {
        assert_eq!(DNS_EVENT_QUERY_RESPONSE, 3008);
        assert_eq!(DNS_EVENT_QUERY_COMPLETED, 3010);
    }

    /// filetime_to_systemtime：Unix epoch 起点（FILETIME = delta）应得 UNIX_EPOCH。
    #[test]
    fn filetime_at_unix_epoch_is_zero() {
        let unix_epoch_filetime = 116_444_736_000_000_000i64;
        assert_eq!(
            filetime_to_systemtime(unix_epoch_filetime),
            SystemTime::UNIX_EPOCH
        );
    }

    /// filetime_to_systemtime：负数（1601 之前，不可能但防御性）→ UNIX_EPOCH。
    #[test]
    fn filetime_negative_falls_back_to_unix_epoch() {
        assert_eq!(filetime_to_systemtime(-1), SystemTime::UNIX_EPOCH);
    }

    /// read_wide_string_at_offset：从指定字节偏移读 UTF-16 LE 串直到 NUL。
    #[test]
    fn read_wide_string_basic() {
        let mut bytes: Vec<u8> = vec![0xFF, 0xFF, 0xFF, 0xFF];
        for c in "QueryName ".encode_utf16() {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0]);
        assert_eq!(read_wide_string_at_offset(&bytes, 4), "QueryName ");
    }

    /// read_wide_string_at_offset：offset 越界 → 空 String（不 panic）。
    #[test]
    fn read_wide_string_offset_out_of_bounds() {
        let bytes: Vec<u8> = vec![0xAB, 0xCD];
        assert_eq!(read_wide_string_at_offset(&bytes, 100), "");
    }

    /// read_utf16_le_until_null：基础 ASCII 域名 + null terminator。
    #[test]
    fn read_utf16_ascii_with_null() {
        let mut bytes: Vec<u8> = Vec::new();
        for c in "example.com".encode_utf16() {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0]);
        assert_eq!(read_utf16_le_until_null(&bytes), "example.com");
    }

    /// read_utf16_le_until_null：空 / 奇数长度 / 仅 null → 空 String（不 panic）。
    #[test]
    fn read_utf16_empty_and_short_inputs() {
        assert_eq!(read_utf16_le_until_null(&[]), "");
        assert_eq!(read_utf16_le_until_null(&[0xAB]), "");
        assert_eq!(read_utf16_le_until_null(&[0, 0]), "");
    }

    /// read_utf16_le_until_null：CJK code unit（验证 UTF-16 LE 路径不丢字节）。
    #[test]
    fn read_utf16_cjk_code_units() {
        // "中文" = [0x4E2D, 0x6587]
        let bytes: Vec<u8> = vec![0x2D, 0x4E, 0x87, 0x65, 0, 0];
        assert_eq!(read_utf16_le_until_null(&bytes), "中文");
    }

    /// drain 在 accum 为空时返空 Vec（不 panic）。
    /// 仅在 64-bit Windows 上构造 collector（其它场景 cfg-gate）。
    #[test]
    fn drain_empty_accum_returns_empty() {
        let accum: SharedAccum = Arc::new(Mutex::new(Vec::new()));
        // 模拟 drain：take accum
        let queries = std::mem::take(&mut *accum.lock().unwrap());
        assert!(queries.is_empty());
    }
}
