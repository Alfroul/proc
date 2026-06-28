//! Windows ETW NT Kernel Logger + DiskIo_TypeGroup1 实装。
//!
//! 设计：
//! - **NT Kernel Logger**：固定 session name "NT Kernel Logger" + 固定 GUID
//!   `{9e814aad-3204-11d2-9a82-006008a869e3}`。优势：API 简单，Windows 7+ 全支持。
//!   劣势：单实例（同时只能一个进程用），失败时 fallback sysinfo。
//! - **EVENT_TRACE_FLAG_DISK_IO**：启用 DiskIo provider（事件 ID 12/13/15）。
//! - **DiskIo_TypeGroup1 schema（event ID 15）**：Win10+ x64 偏移如下（pointer-size
//!   敏感，32-bit Windows 不支持——见下方 cfg-gate）：
//!   ```text
//!   offset  type   field
//!   0       u32    TransferSize
//!   4       u32    DiskNumber
//!   8       u64    Irp           (pointer-sized)
//!   16      u64    FileObject    (pointer-sized)
//!   24      u64    HighResResponseTime
//!   32      u32    IssuingThreadId
//!   ```
//!   schema 自 Win8 起稳定。Win11 可能有额外字段，但只读上面 6 个。
//! - **read/write 区分**：EVENT_HEADER.EventDescriptor.Opcode 在 TypeGroup1 事件里
//!   通常是 2（read）或 3（write）。schema 已稳定多年但仍按 best-effort 处理：
//!   未识别的 opcode 一律按 read 计（保守）。
//! - **thread → pid map**：见 `super::thread_map`，sysinfo 5s 全量刷新。
//!
//! 失败模式（全部降级到 sysinfo）：
//! - `StartTraceW` 失败：非管理员 / NT Kernel Logger 已被占用（资源监视器、
//!   另一个 proc 实例）/ Windows Event Log 服务未启动
//! - `OpenTraceW` 失败：极少见，通常内存不足
//! - ProcessTrace 启动后 panic：catch_unwind 兜底，主线程 UI 继续可用

#![cfg(target_os = "windows")]

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::Win32::System::Diagnostics::Etw::{
    CONTROLTRACE_HANDLE, CloseTrace, ControlTraceW, EVENT_RECORD, EVENT_TRACE_CONTROL_STOP,
    EVENT_TRACE_FLAG_DISK_IO, EVENT_TRACE_LOGFILEW, EVENT_TRACE_PROPERTIES,
    EVENT_TRACE_REAL_TIME_MODE, OpenTraceW, PEVENT_RECORD_CALLBACK,
    PROCESS_TRACE_MODE_EVENT_RECORD, PROCESS_TRACE_MODE_RAW_TIMESTAMP,
    PROCESS_TRACE_MODE_REAL_TIME, PROCESSTRACE_HANDLE, ProcessTrace, StartTraceW,
    WNODE_FLAG_TRACED_GUID,
};
use windows::core::PCWSTR;

use crate::disk_io_etw::thread_map::ThreadToPidMap;
use crate::disk_io_etw::{DiskIoEtwWorker, DiskIoMap, DiskIoStats};
use crate::metrics::crash::WorkerCrash;
use crate::worker::{SnapshotWorker, run_poll_loop};

/// 1s flush：与 v0.6 per-process net_rate 同款节奏。
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// DiskIo_TypeGroup1 在 EVENT_HEADER.EventDescriptor.Id == 15。
const DISKIO_TYPE_GROUP1_ID: u16 = 15;

/// Opcode 值（来自 Microsoft-Windows-Kernel-Disk manifest）：
/// - 0x02 = read completion
/// - 0x03 = write completion
const DISKIO_OPCODE_WRITE: u8 = 3;

/// 累加 buffer：每 PID 的 (read_bytes, write_bytes) since last flush。
type AccumMap = HashMap<u32, (u64, u64)>;

/// 尝试启动 ETW disk IO worker。失败时返回 None，调用方走 sysinfo fallback。
pub(super) fn try_spawn_windows(crash_tx: Option<Sender<WorkerCrash>>) -> Option<DiskIoEtwWorker> {
    // 安全护栏：x86 (32-bit) Windows 上 pointer-size 是 4 字节，硬编码偏移失效。
    // Windows 11 x64-only，但 Windows 10 还有 x86 版本——直接拒绝。
    #[cfg(not(target_pointer_width = "64"))]
    {
        tracing::warn!("ETW disk IO 仅支持 x64 Windows（32-bit 偏移不同），降级到 sysinfo");
        return None;
    }

    #[cfg(target_pointer_width = "64")]
    {
        let accum: Arc<Mutex<AccumMap>> = Arc::new(Mutex::new(HashMap::new()));
        let thread_map = ThreadToPidMap::spawn_refresh_thread();

        // 1) 启动 NT Kernel Logger session
        let session_handle = match start_kernel_logger_session() {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "ETW NT Kernel Logger 启动失败（非管理员？session 已被占用？），降级到 sysinfo"
                );
                return None;
            }
        };

        // 2) Open real-time consumer trace + 注册 callback
        let accum_cb = Arc::clone(&accum);
        let thread_map_handle = thread_map.clone_handle();
        let trace_handle = match open_trace_with_callback(accum_cb, thread_map_handle) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!(error = %e, "ETW OpenTraceW 失败，降级到 sysinfo");
                let _ = stop_kernel_logger_session(session_handle);
                return None;
            }
        };

        // 3) ProcessTrace 阻塞线程：ETW callback 在此线程上触发
        let trace_thread = std::thread::Builder::new()
            .name("disk-io-etw-process".into())
            .spawn(move || {
                let handles = [trace_handle];
                // SAFETY: trace_handle 来自 OpenTraceW，VALID_HANDLE；handles 数组
                // 生命周期覆盖整个 ProcessTrace 调用。None start/end = 不限时间窗。
                let _ = unsafe { ProcessTrace(&handles, None, None) };
            })
            .ok()?;

        // 4) SnapshotWorker body：run_poll_loop 1s drain accum → push channel；
        //    shutdown 信号触发后停止 ETW session + join ProcessTrace 线程
        let worker = SnapshotWorker::spawn(
            "disk-io-etw-worker",
            crash_tx,
            move |snap_tx, shutdown_rx, metrics| {
                run_poll_loop(&snap_tx, &shutdown_rx, &metrics, POLL_INTERVAL, || {
                    let drained = std::mem::take(&mut *accum.lock().expect("accum poisoned"));
                    let map: DiskIoMap = drained
                        .into_iter()
                        .map(|(pid, (r, w))| {
                            (
                                pid,
                                DiskIoStats {
                                    read_bps: r,
                                    write_bps: w,
                                },
                            )
                        })
                        .collect();
                    Some(map)
                });
                // shutdown_rx triggered —— 清理 ETW 资源
                tracing::debug!("ETW disk IO worker shutting down, stopping session");
                let _ = stop_kernel_logger_session(session_handle);
                // stop_kernel_logger_session 后 ProcessTrace 自然返回
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
// NT Kernel Logger session 启停
// ──────────────────────────────────────────────────────────────────────────

/// "NT Kernel Logger\0" wide string（16 chars + NUL = 17 u16）。
const KERNEL_LOGGER_NAME_WIDE: &[u16] = &[
    b'N' as u16,
    b'T' as u16,
    b' ' as u16,
    b'K' as u16,
    b'e' as u16,
    b'r' as u16,
    b'n' as u16,
    b'e' as u16,
    b'l' as u16,
    b' ' as u16,
    b'L' as u16,
    b'o' as u16,
    b'g' as u16,
    b'g' as u16,
    b'e' as u16,
    b'r' as u16,
    0, // NUL terminator
];

/// NT Kernel Logger session 启动。失败时返回 Error。
fn start_kernel_logger_session() -> windows::core::Result<CONTROLTRACE_HANDLE> {
    // EVENT_TRACE_PROPERTIES + logger_name(17 u16) + file_name(空 NUL = 1 u16)
    let logger_name_bytes = KERNEL_LOGGER_NAME_WIDE.len() * 2;
    let file_name_bytes = 2; // 空 wide string
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
    props.EnableFlags = EVENT_TRACE_FLAG_DISK_IO;
    props.LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
    props.BufferSize = 64; // KB
    props.MinimumBuffers = 20;
    props.MaximumBuffers = 100;

    let mut session_handle = CONTROLTRACE_HANDLE::default();
    let logger_name_pcwstr = PCWSTR(KERNEL_LOGGER_NAME_WIDE.as_ptr());
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

/// 停止 NT Kernel Logger session。drop 时调用，让 ProcessTrace 线程返回。
fn stop_kernel_logger_session(session_handle: CONTROLTRACE_HANDLE) -> windows::core::Result<()> {
    let logger_name_bytes = KERNEL_LOGGER_NAME_WIDE.len() * 2;
    let file_name_bytes = 2;
    let buf_size =
        std::mem::size_of::<EVENT_TRACE_PROPERTIES>() + logger_name_bytes + file_name_bytes;
    let mut buf: Vec<u8> = vec![0u8; buf_size];

    // SAFETY: 同 start_kernel_logger_session。
    let props = unsafe { &mut *(buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES) };
    props.Wnode.BufferSize = buf_size as u32;
    props.LoggerNameOffset = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32;
    props.LogFileNameOffset = props.LoggerNameOffset + logger_name_bytes as u32;

    let logger_name_pcwstr = PCWSTR(KERNEL_LOGGER_NAME_WIDE.as_ptr());
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
// Open trace + callback registration
// ──────────────────────────────────────────────────────────────────────────

/// 打开 real-time consumer trace，注册 EventRecordCallback。
fn open_trace_with_callback(
    accum: Arc<Mutex<AccumMap>>,
    thread_map: Arc<Mutex<HashMap<u32, u32>>>,
) -> windows::core::Result<PROCESSTRACE_HANDLE> {
    // 通过 thread_local 把 accum / thread_map 传给 callback。
    // EVENT_TRACE_LOGFILEW.Context 是 per-event-record `UserContext`，不适合
    // 传 trace-level shared state（被 ETW 内部复用）。thread_local 同 ProcessTrace
    // 线程独占，正好契合「callback 与 OpenTraceW 在同一线程」的语义。
    CALLBACK_ACCUM.with(|cell| *cell.borrow_mut() = Some(Arc::clone(&accum)));
    CALLBACK_THREAD_MAP.with(|cell| *cell.borrow_mut() = Some(Arc::clone(&thread_map)));

    let mut logfile: EVENT_TRACE_LOGFILEW = unsafe { std::mem::zeroed() };
    logfile.LoggerName = windows::core::PWSTR(KERNEL_LOGGER_NAME_WIDE.as_ptr() as *mut u16);
    logfile.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME
        | PROCESS_TRACE_MODE_EVENT_RECORD
        | PROCESS_TRACE_MODE_RAW_TIMESTAMP;
    let callback: PEVENT_RECORD_CALLBACK = Some(disk_io_event_callback);
    logfile.Anonymous2.EventRecordCallback = callback;
    logfile.Context = std::ptr::null_mut();

    // SAFETY: logfile 字段已正确设置；返回 INVALID_PROCESSTRACE_HANDLE 表示失败。
    let trace_handle = unsafe { OpenTraceW(&mut logfile) };
    if trace_handle.Value == u64::MAX {
        return Err(windows::core::Error::from_win32());
    }
    Ok(trace_handle)
}

type SharedAccum = Arc<Mutex<AccumMap>>;
type SharedThreadMap = Arc<Mutex<HashMap<u32, u32>>>;

thread_local! {
    /// callback 内访问的累加 buffer。
    static CALLBACK_ACCUM: std::cell::RefCell<Option<SharedAccum>> =
        const { std::cell::RefCell::new(None) };
    /// callback 内访问的 thread→pid map。
    static CALLBACK_THREAD_MAP: std::cell::RefCell<Option<SharedThreadMap>> =
        const { std::cell::RefCell::new(None) };
}

/// ETW callback：每 fire 一个 DiskIo 事件调一次。
///
/// 解析 EVENT_RECORD.UserData 取 TransferSize + IssuingThreadId，查 thread_map
/// 拿 PID，按 opcode 加到 accum[pid] 的 read 或 write 列。
///
/// **性能关键**：callback 在 ETW 内部线程跑，每秒可能调用数千次。锁占用必须
/// 极短（hash insert + u64 add）。
unsafe extern "system" fn disk_io_event_callback(record: *mut EVENT_RECORD) {
    if record.is_null() {
        return;
    }
    // SAFETY: caller (ETW) 保证 record 非空时有效；我们已检查。
    let record = unsafe { &*record };

    if record.EventHeader.EventDescriptor.Id != DISKIO_TYPE_GROUP1_ID {
        return;
    }

    let user_data = record.UserData;
    let user_len = record.UserDataLength as usize;
    // DiskIo_TypeGroup1 在 x64 至少 36 bytes（见 mod.rs 注释）
    if user_data.is_null() || user_len < 36 {
        return;
    }

    // SAFETY: user_data 是非空指针，长度 >= 36，可以读 6 个字段。
    let bytes = unsafe { std::slice::from_raw_parts(user_data as *const u8, 36) };
    let transfer_size = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let issuing_thread_id = u32::from_le_bytes([bytes[32], bytes[33], bytes[34], bytes[35]]);
    let opcode = record.EventHeader.EventDescriptor.Opcode;

    let accum_opt = CALLBACK_ACCUM.with(|cell| cell.borrow().clone());
    let thread_map_opt = CALLBACK_THREAD_MAP.with(|cell| cell.borrow().clone());
    let (Some(accum), Some(thread_map)) = (accum_opt, thread_map_opt) else {
        return;
    };

    let pid = {
        let map = thread_map.lock().expect("thread_map poisoned");
        map.get(&issuing_thread_id).copied()
    };
    let Some(pid) = pid else {
        return;
    };

    let is_write = opcode == DISKIO_OPCODE_WRITE;
    let mut acc = accum.lock().expect("accum poisoned");
    let entry = acc.entry(pid).or_insert((0, 0));
    if is_write {
        entry.1 += u64::from(transfer_size);
    } else {
        entry.0 += u64::from(transfer_size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 仅编译通过性测试：常量值正确。
    #[test]
    fn diskio_constants() {
        assert_eq!(DISKIO_TYPE_GROUP1_ID, 15);
        assert_eq!(DISKIO_OPCODE_WRITE, 3);
        assert_eq!(KERNEL_LOGGER_NAME_WIDE.len(), 17); // 16 chars + NUL
    }
}
