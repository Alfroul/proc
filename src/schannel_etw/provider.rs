//! Windows `Microsoft-Windows-Schannel-Events` ETW session + TDH SNI 解析（阶段 2）。
//!
//! 阶段 2 实测结论（管理员 + logman/tracerpt，2026-06-28）：
//! - **Provider GUID**：`{91CC1150-71AA-47E2-AE18-C96E61736B6F}`
//!   （`Microsoft-Windows-Schannel-Events`，**不是** ADR-0018 §3 阶段 1 推测的
//!   `{37D2C3CD-...}` —— 原 GUID `Security: SChannel` 实测对 curl TLS handshake
//!   不 fire 任何 event；新 GUID 在同一管理员 + logman 流程下抓到 1793 等 events）
//! - **SNI event ID = 1793**（DeleteSecurityContext Start, opcode=1, Task=28672）
//! - **SNI 字段 = `TargetName`**（UTF-16 LE null-terminated string）
//! - 字段 layout：`ContextHandle` (u64 pointer) + `TargetName` (UTF-16 LE)。
//!   parser 走 TDH 动态遍历，**不硬编码字段顺序或偏移**——manifest 加字段时
//!   `TargetName` 仍能找到（ADR-0018 §3 「TDH 路线」决定）。
//!
//! PID 关联：Schannel event 自带 `EVENT_HEADER.ProcessId`，**不复用**
//! disk_io_etw 的 thread→pid map（后者是 NT Kernel Logger 才需要）。
//!
//! 失败模式（全部降级，返回 None）：
//! - `StartTraceW` 失败：非管理员 / session name 已被占用
//! - `EnableTraceEx2` 失败：provider GUID 错误 / Schannel 服务未启动
//! - `OpenTraceW` 失败：极少见，通常内存不足
//! - x86 (32-bit)：cfg-gate 直接拒绝（与 disk_io_etw 一致）

#![cfg(target_os = "windows")]

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
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

use crate::metrics::crash::WorkerCrash;
use crate::schannel_etw::SchannelEtwWorker;
use crate::schannel_etw::parser::{SniRecord, read_utf16_le_until_null};
use crate::worker::{SnapshotWorker, run_poll_loop};

/// 1s flush：与 disk_io_etw 同款节奏（drain accum → push channel）。
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// **阶段 2 实测修订**：SNI event ID = 1793（DeleteSecurityContext Start）。
///
/// 阶段 1 ADR-0018 §3 推测的 196 实测完全不出现（logman/tracerpt 在管理员下
/// 触发 3 个 curl https:// TLS handshake 后未抓到任何 196 event）。新版 provider
/// GUID `{91CC1150-...}` 下，SNI 字段（`TargetName`）只出现在 1793 opcode=1
/// 事件里。
const SCHANNEL_EVENT_SNI_ID: u16 = 1793;

/// `Microsoft-Windows-Schannel-Events` ETW provider GUID。
///
/// **阶段 2 实测修订**：阶段 1 ADR-0018 §3 推测的 `{37D2C3CD-...}`
/// （`Security: SChannel`）实测对 curl TLS handshake **不 fire 任何 event**。
/// 真正 fire 1793 / 257 / 258 / 1025-1028 / 1537-1538 等 Schannel 事件的是
/// `Microsoft-Windows-Schannel-Events` provider，GUID `{91CC1150-...}`。
const SCHANNEL_PROVIDER_GUID: GUID = GUID::from_values(
    0x91CC1150,
    0x71AA,
    0x47E2,
    [0xAE, 0x18, 0xC9, 0x6E, 0x61, 0x73, 0x6B, 0x6F],
);

/// session name：自定义（非 NT Kernel Logger —— Schannel 是用户态 provider）。
/// 末尾 NUL 满足 PCWSTR 要求。
const SESSION_NAME: &str = "proc-schannel-sni\0";

/// TRACE_LEVEL_VERBOSE = 5：抓全级别事件（1793 是 Level 4 Informational）。
const TRACE_LEVEL_VERBOSE: u8 = 5;

/// Win32 ERROR_INSUFFICIENT_BUFFER（122）：TdhGetEventInformation 第一调预期返回。
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

/// 累加 buffer：自上次 1s flush 以来 callback 解出的 SniRecord 列表。
type AccumVec = Vec<SniRecord>;

/// 尝试启动 Schannel ETW worker。失败时返回 None，调用方走降级
/// （阶段 3 才会消费 `SniRecord` 填 ProcessFlow.sni，阶段 2 worker
/// 启停失败不影响其它路径）。
pub(super) fn try_spawn_windows(
    crash_tx: Option<Sender<WorkerCrash>>,
) -> Option<SchannelEtwWorker> {
    // 安全护栏：x86 (32-bit) Windows 拒绝（pointer-size 与 manifest 偏移不稳）。
    #[cfg(not(target_pointer_width = "64"))]
    {
        tracing::warn!("Schannel ETW 仅支持 x64 Windows（32-bit 偏移不稳），降级");
        return None;
    }

    #[cfg(target_pointer_width = "64")]
    {
        let accum: Arc<Mutex<AccumVec>> = Arc::new(Mutex::new(Vec::new()));

        // 1) StartTraceW 开 session
        let session_handle = match start_schannel_session() {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Schannel ETW StartTraceW 失败（非管理员？session 已被占用？），降级"
                );
                return None;
            }
        };

        // 2) EnableTraceEx2 启用 Schannel provider（level=verbose, keyword=all）
        if let Err(e) = enable_schannel_provider(session_handle) {
            tracing::warn!(error = %e, "Schannel ETW EnableTraceEx2 失败，降级");
            let _ = stop_session(session_handle);
            return None;
        }

        // 3) OpenTraceW + 注册 callback
        let trace_handle = match open_trace_with_callback() {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(error = %e, "Schannel ETW OpenTraceW 失败，降级");
                let _ = stop_session(session_handle);
                return None;
            }
        };

        // 4) ProcessTrace 阻塞线程：ETW callback 在此线程上触发。
        //    **关键**：thread_local 必须在 ProcessTrace 调用前于**同一线程**设置，
        //    否则 callback 拿不到 accum 引用（thread_local 不跨线程传递）。
        //
        //    **v0.10 阶段 4（REVIEW-11 P1-2）**：spawn 失败时必须清理已启的
        //    session_handle + trace_handle——session 名 `"proc-schannel-sni\0`
        //    被占用会让后续 proc 重启也失败，trace_handle 泄漏影响其它 ETW
        //    消费者（xperf / perfmon）。
        let accum_for_thread = Arc::clone(&accum);
        let trace_thread = match std::thread::Builder::new()
            .name("schannel-etw-process".into())
            .spawn(move || {
                CALLBACK_ACCUM.with(|cell| {
                    *cell.borrow_mut() = Some(Arc::clone(&accum_for_thread));
                });
                let handles = [trace_handle];
                // SAFETY: trace_handle 来自 OpenTraceW，VALID_HANDLE；handles 数组
                // 生命周期覆盖整个 ProcessTrace 调用。None start/end = 不限时间窗。
                let _ = unsafe { ProcessTrace(&handles, None, None) };
            }) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Schannel ETW ProcessTrace 线程 spawn 失败（线程 ulimit？OOM？），降级 + 清理已启 handle"
                );
                let _ = stop_session(session_handle);
                // SAFETY: trace_handle 来自 OpenTraceW 成功返回；ProcessTrace
                // 未调用过（trace_thread 没 spawn 起来），CloseTrace 仍合法
                // （幂等——重复调用返回 ERROR_INVALID_HANDLE 但不 panic）。
                unsafe {
                    let _ = CloseTrace(trace_handle);
                }
                return None;
            }
        };

        // 5) SnapshotWorker body：run_poll_loop 1s drain accum → push channel；
        //    shutdown 信号触发后停止 ETW session + join ProcessTrace 线程
        let worker = SnapshotWorker::spawn(
            "schannel-etw-worker",
            crash_tx,
            move |snap_tx, shutdown_rx, metrics| {
                run_poll_loop(&snap_tx, &shutdown_rx, &metrics, POLL_INTERVAL, || {
                    let drained = std::mem::take(&mut *accum.lock().expect("accum poisoned"));
                    if drained.is_empty() {
                        None
                    } else {
                        Some(drained)
                    }
                });
                tracing::debug!("Schannel ETW worker shutting down, stopping session");
                let _ = stop_session(session_handle);
                // stop_session 后 ProcessTrace 自然返回
                let _ = trace_thread.join();
                // SAFETY: trace_handle 已被 ProcessTrace 消费完毕（join 已返回），
                // 此处 CloseTrace 是幂等的——重复调用仅返回 ERROR_INVALID_HANDLE。
                unsafe {
                    let _ = CloseTrace(trace_handle);
                }
            },
        );

        Some(worker)
    }
}

// ──────────────────────────────────────────────────────────────────────────
// StartTraceW + EnableTraceEx2 + OpenTraceW
// ──────────────────────────────────────────────────────────────────────────

/// `SESSION_NAME` 的 UTF-16 wide 版本。
fn session_name_wide() -> Vec<u16> {
    // SESSION_NAME 含末尾 NUL；encode_utf16 不会再加 NUL，正好满足 PCWSTR 要求。
    SESSION_NAME.encode_utf16().collect()
}

/// 启动 Schannel ETW session。返回 CONTROLTRACE_HANDLE。
fn start_schannel_session() -> windows::core::Result<CONTROLTRACE_HANDLE> {
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
    // Schannel 是用户态 provider，EnableFlags 不用（NT Kernel Logger 专用）；
    // 用 EnableTraceEx2 启用具体 provider GUID。
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

/// `EnableTraceEx2` 启用 Schannel provider（level=verbose, keyword=all →
/// 抓全事件含 SNI candidate 1793）。
fn enable_schannel_provider(session_handle: CONTROLTRACE_HANDLE) -> windows::core::Result<()> {
    // ENABLE_TRACE_PARAMETERS_VERSION = 1（自 Win7 起固定）。其余字段 ZeroInit。
    let params = ENABLE_TRACE_PARAMETERS {
        Version: 1,
        ..Default::default()
    };
    // SAFETY: EnableTraceEx2 是 standard ETW API；session_handle 来自 StartTraceW；
    // params 是 local stack。参数顺序：(tracehandle, providerid, controlcode, level,
    //           matchanykeyword, matchallkeyword, timeout, enableparameters)
    //
    // **MatchAnyKeyword=全 1**：logman/tracerpt 探测时同款（`logman -p {guid}
    // 0xFFFFFFFFFFFFFFFF 255`）。Schannel event 1793 keyword=0x8000000000000000，
    // 文档对「MatchAnyKeyword=0 是否当 all」说法不一致；显式全 1 最稳。
    let err = unsafe {
        EnableTraceEx2(
            session_handle,
            &SCHANNEL_PROVIDER_GUID,
            EVENT_CONTROL_CODE_ENABLE_PROVIDER.0,
            TRACE_LEVEL_VERBOSE,
            0xFFFF_FFFF_FFFF_FFFF, // MatchAnyKeyword：全 1 = 所有 keyword（含 0x8000_0000_0000_0000）
            0,                     // MatchAllKeyword：0 = 不额外过滤
            0,                     // Timeout：0 = 不等 provider 响应
            Some(&params),
        )
    };
    err.ok()?;
    Ok(())
}

/// OpenTraceW 注册 callback。callback 收到 Schannel event 1793 时用 TDH 解 SNI。
///
/// **注意**：accum 的 thread_local 绑定**不能**在此函数里做——此函数在
/// spawning 线程调用，但 callback 实际在 ProcessTrace 线程触发，thread_local
/// 不会跨线程传递。绑定改在 `try_spawn_windows` 内的 spawn 闭包里做
/// （ProcessTrace 调用前）。
fn open_trace_with_callback() -> windows::core::Result<PROCESSTRACE_HANDLE> {
    let logger_name_wide = session_name_wide();
    let mut logfile: EVENT_TRACE_LOGFILEW = unsafe { std::mem::zeroed() };
    logfile.LoggerName = windows::core::PWSTR(logger_name_wide.as_ptr() as *mut u16);
    logfile.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME
        | PROCESS_TRACE_MODE_EVENT_RECORD
        | PROCESS_TRACE_MODE_RAW_TIMESTAMP;
    let callback: PEVENT_RECORD_CALLBACK = Some(schannel_event_callback);
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

    // SAFETY: 同 start_schannel_session。
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

type SharedAccum = Arc<Mutex<AccumVec>>;

thread_local! {
    /// callback 内访问的累加 buffer（与 disk_io_etw CALLBACK_ACCUM 同款模式）。
    static CALLBACK_ACCUM: std::cell::RefCell<Option<SharedAccum>> =
        const { std::cell::RefCell::new(None) };
}

/// Schannel ETW callback：每 fire 一个 Schannel 事件调一次。
///
/// - **Fast filter**：只处理 event 1793（ADR-0018 §3 实测的 SNI candidate）。
///   其它事件直接 return，避免每 event 都调 TdhGetEventInformation。
/// - **TDH 解析**：1793 → `parse_sni_via_tdh` → `SniRecord`（push 到 accum）。
/// - **PID**：来自 `EVENT_HEADER.ProcessId`（用户态 provider 自带，**不**走
///   disk_io_etw 的 thread→pid map）。
unsafe extern "system" fn schannel_event_callback(record: *mut EVENT_RECORD) {
    if record.is_null() {
        return;
    }
    // SAFETY: caller (ETW) 保证 record 非空时有效；我们已检查。
    let record = unsafe { &*record };

    // Fast filter：只对 1793 调 TDH（TDH 解析有 10-50μs/event 开销）。
    // 若 Microsoft 在未来 Windows 版本改 event ID，parser 仍可走 property-by-name
    // 兜底——届时把这条 fast filter 放宽即可。
    if record.EventHeader.EventDescriptor.Id != SCHANNEL_EVENT_SNI_ID {
        return;
    }

    let Some(rec) = parse_sni_via_tdh(record) else {
        return;
    };

    let accum_opt = CALLBACK_ACCUM.with(|cell| cell.borrow().clone());
    if let Some(accum) = accum_opt {
        let mut acc = accum.lock().expect("accum poisoned");
        acc.push(rec);
    }
}

/// 用 TDH 动态 schema 解 Schannel event 1793 的 `TargetName` 字段。
///
/// 步骤：
/// 1. `TdhGetEventInformation` 拉一份 TRACE_EVENT_INFO buffer（含 property 数组）。
/// 2. 遍历 top-level properties，对每个 property：
///    - 读 name（从 buffer[`NameOffset`..] 当 UTF-16 LE 串）。
///    - 若 name == "TargetName"：从 `UserData[offset..]` 读 UTF-16 LE 串。
///    - 否则：累计当前 property size 到 offset（用 `TdhGetPropertySize`）。
///
/// **不硬编码字段顺序 / 偏移** —— manifest 加字段时 `TargetName` 仍能找到。
///
/// 失败返回 `None`（callback 会静默 drop，不影响后续事件）。
fn parse_sni_via_tdh(record: &EVENT_RECORD) -> Option<SniRecord> {
    // 1) 拉 TRACE_EVENT_INFO buffer（growing buffer 模式：第一调拿 required size）
    let info_buf = tdh_get_event_info_buffer(record)?;
    let info_ptr = info_buf.as_ptr() as *const TRACE_EVENT_INFO;
    // SAFETY: info_buf 来自 TdhGetEventInformation 成功返回，按 TRACE_EVENT_INFO
    // 对齐（TDH API 保证 buffer 对齐 ≥ struct 对齐）。info 只读 buffer 时不越界，
    // 因为我们用 PropertyCount 限制 property 数组的访问范围。
    let info = unsafe { &*info_ptr };

    // 2) UserData slice（callback 进入时 record 还活着，UserData 在 record 生命周期内有效）
    let user_len = record.UserDataLength as usize;
    let user_data: &[u8] = if record.UserData.is_null() || user_len == 0 {
        return None;
    } else {
        // SAFETY: ETW 保证 UserData 在 callback 期间非空 + 长度 UserDataLength bytes。
        unsafe { std::slice::from_raw_parts(record.UserData as *const u8, user_len) }
    };

    // 3) 遍历 top-level properties，累计 offset 直到找到 TargetName
    let mut offset_in_user_data = 0usize;
    let top_count = info.TopLevelPropertyCount as usize;
    for i in 0..top_count {
        let prop = property_at_index(&info_buf, i)?;

        // property 名字在 buffer 里以 UTF-16 LE 存（NameOffset 是字节 offset）
        let name = read_wide_string_at_offset(&info_buf, prop.NameOffset as usize);

        if name.eq_ignore_ascii_case("TargetName") {
            // 读 UserData[offset..] 的 UTF-16 LE 串直到 null
            let start = offset_in_user_data.min(user_data.len());
            let sni = read_utf16_le_until_null(&user_data[start..]);
            if sni.is_empty() {
                return None;
            }
            let ts = filetime_to_systemtime(record.EventHeader.TimeStamp);
            return Some(SniRecord {
                pid: record.EventHeader.ProcessId,
                sni,
                ts,
            });
        }

        // 累计 property size 到 offset（用 TdhGetPropertySize by name）
        let size = tdh_get_property_size_for_index(record, &info_buf, i)?;
        offset_in_user_data += size;
    }
    None
}

/// `TdhGetEventInformation` 拉 TRACE_EVENT_INFO buffer（growing buffer 模式）。
fn tdh_get_event_info_buffer(record: &EVENT_RECORD) -> Option<Vec<u8>> {
    let mut buf_size: u32 = 0;
    // 第一调：buffer = None 拿 required size。预期返回 ERROR_INSUFFICIENT_BUFFER。
    // SAFETY: record 是 callback 给的有效指针；buf_size 是 out 参数；buffer=None。
    let status =
        unsafe { TdhGetEventInformation(record as *const EVENT_RECORD, None, None, &mut buf_size) };
    if status != ERROR_INSUFFICIENT_BUFFER || buf_size == 0 {
        // status==0 表示 buffer size 已经够（理论上不可能，因为我们传了 None）；
        // 其它非零错误直接放弃。
        return None;
    }

    let mut buffer: Vec<u8> = vec![0u8; buf_size as usize];
    loop {
        // SAFETY: buffer 大小已按 TDH 第一调的 required size 分配；TRACE_EVENT_INFO
        // 是 #[repr(C)] POD，按 u8 写入再 cast 安全（TDH 写完后我们对齐读）。
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
        // 罕见 race：required size 又变大了，扩容重试
        buffer.resize(buf_size as usize, 0);
    }
}

/// 从 TRACE_EVENT_INFO 的 property 数组取第 `idx` 个 EVENT_PROPERTY_INFO。
///
/// windows-rs 的 `TRACE_EVENT_INFO.EventPropertyInfoArray` 是 `[EVENT_PROPERTY_INFO; 1]`
/// 占位字段（C 端是 trailing flexible array），实际长度由 `PropertyCount` 决定。
///
/// **TD-35（v0.12 阶段 5）lifetime 修正**：返回的引用生命周期绑到 `info_buf`
/// （TdhGetEventInformation 返回的 `Vec<u8>` owner），不再撒谎说 `&'static`。
/// 之前签名声明 `'static` 但实际只在 owner buffer 活着时有效——buffer drop 后
/// 引用悬空。现在 `info_buf: &[u8]` → `Option<&EVENT_PROPERTY_INFO>`（Rust
/// lifetime elision：唯一输入 lifetime 自动 propagate 到输出），借用检查器
/// 自动保证引用不会逃出 owner。callers 之前传 `info_ptr` 改成传 `info_buf`，
/// info_ptr 在函数内派生。
fn property_at_index(info_buf: &[u8], idx: usize) -> Option<&EVENT_PROPERTY_INFO> {
    if info_buf.is_empty() {
        return None;
    }
    let info_ptr = info_buf.as_ptr() as *const TRACE_EVENT_INFO;
    // SAFETY: caller 承诺 info_buf 是 TdhGetEventInformation 成功返回的 buffer，
    // 按 TRACE_EVENT_INFO 对齐且长度足够；我们只读 idx < PropertyCount 的 entry。
    let info = unsafe { &*info_ptr };
    if idx as u32 >= info.PropertyCount {
        return None;
    }

    // EventPropertyInfoArray 是 [EVENT_PROPERTY_INFO; 1] 占位字段；实际有
    // PropertyCount 个 entry。as_ptr() 拿数组首元素地址，.add(idx) 索引。
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
///
/// 需要 property 名（PCWSTR），从 buffer[`NameOffset`..] 取出（含 NUL）。
fn tdh_get_property_size_for_index(
    record: &EVENT_RECORD,
    info_buf: &[u8],
    idx: usize,
) -> Option<usize> {
    let prop = property_at_index(info_buf, idx)?;
    let name_offset = prop.NameOffset as usize;
    if name_offset + 2 > info_buf.len() {
        return None;
    }

    // 取出 property name 到 owned Vec<u16>（含 NUL 终结符）。
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
    // 保证末尾 NUL（即使原 buffer 没写 NUL——理论上 TDH 保证有，但防御性）
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
/// 转 `SystemTime`（UNIX_EPOCH = 1970-01-01）。
fn filetime_to_systemtime(filetime: i64) -> SystemTime {
    // FILETIME → Unix epoch delta：116_444_736_000_000_000 个 100ns 单位。
    const FILETIME_UNIX_EPOCH_DELTA_HUNDREDS_NS: i64 = 116_444_736_000_000_000;
    let unix_hundreds_ns = filetime.saturating_sub(FILETIME_UNIX_EPOCH_DELTA_HUNDREDS_NS);
    // 100ns → ns：× 100
    let unix_ns = unix_hundreds_ns.saturating_mul(100);
    if unix_ns < 0 {
        // 时间早于 1970-01-01（理论上 Schannel 事件不会，防御性 fallback）
        return SystemTime::UNIX_EPOCH;
    }
    SystemTime::UNIX_EPOCH + Duration::from_nanos(unix_ns as u64)
}

// ──────────────────────────────────────────────────────────────────────────
// 单元测试
// ──────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// GUID 字面量契约：与 ADR-0018 §3 阶段 2 实测修订一致。
    #[test]
    fn provider_guid_matches_adr_stage2() {
        // ADR-0018 §3 实测修订：{91CC1150-71AA-47E2-AE18-C96E61736B6F}
        assert_eq!(
            SCHANNEL_PROVIDER_GUID,
            GUID::from_values(
                0x91CC1150,
                0x71AA,
                0x47E2,
                [0xAE, 0x18, 0xC9, 0x6E, 0x61, 0x73, 0x6B, 0x6F],
            )
        );
    }

    /// session name 含 NUL 终结符（PCWSTR 要求）。
    #[test]
    fn session_name_has_nul_terminator() {
        assert!(SESSION_NAME.ends_with('\0'), "SESSION_NAME 必须含 NUL");
    }

    /// SNI event ID 契约：1793（实测修订，原 196 已否决）。
    #[test]
    fn sni_event_id_is_1793() {
        assert_eq!(SCHANNEL_EVENT_SNI_ID, 1793);
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
        // buf: [pad pad pad] [H\0e\0l\0l\0o\0 \0W\0o\0r\0l\0d\0 \0\0\0]
        let mut bytes: Vec<u8> = vec![0xFF, 0xFF, 0xFF, 0xFF]; // 前置 padding
        for c in "Hello World ".encode_utf16() {
            bytes.extend_from_slice(&c.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0]); // null terminator
        assert_eq!(read_wide_string_at_offset(&bytes, 4), "Hello World ");
    }

    /// read_wide_string_at_offset：offset 越界 → 空 String（不 panic）。
    #[test]
    fn read_wide_string_offset_out_of_bounds() {
        let bytes: Vec<u8> = vec![0xAB, 0xCD];
        assert_eq!(read_wide_string_at_offset(&bytes, 100), "");
    }
}
