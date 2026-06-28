//! Linux + feature `ebpf` 的 [`EbisuBpfWorker`] 实装。
//!
//! **Part A 状态**：best-effort 骨架。Windows 上无法编译/验证；Linux 会话
//! 需验证：
//! 1. tracepoint attach API（`TracePoint::attach(category, name)`）
//! 2. ring_buf reader API（`RingBuf::next()`）
//! 3. ELF 加载（`aya::Ebpf::load`）
//! 4. include_bytes! 路径（开发者先 `cargo +nightly build --target
//!    bpfel-unknown-none -p proc-ebpf --release` 后才能编译 userspace）
//!
//! 一切 attach 失败时 [`try_spawn_impl`] 返回 `None`，UI 走 fallback。

#![cfg(all(target_os = "linux", feature = "ebpf"))]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use aya::{Ebpf, maps::RingBuf, programs::TracePoint};
use aya_log::EbpfLogger;

use crate::ebpf::elf_loader::EBPF_ELF;
use crate::ebpf::flow::{FlowEvent, RawEvent, parse_raw_event};
use crate::metrics::crash::WorkerCrash;

/// ring_buf poll 间隔：低频（100ms）足够 connect/exit 这类事件；
/// TCP 字节计数（Part B 接 tcp_sendmsg）会更密，到时候调小。
const RING_BUF_POLL: Duration = Duration::from_millis(100);

/// ring_buf → mpsc channel 的缓冲：高频事件积压时主线程 1s tick 也能跟上。
const EVENT_CHANNEL_BOUND: usize = 1024;

pub struct EbisuBpfWorker {
    flow_rx: Receiver<FlowEvent>,
    ring_buf_thread: Option<JoinHandle<()>>,
    stop_flag: Arc<AtomicBool>,
    /// aya::Ebpf 持有 ELF + map / program 句柄；ring_buf reader 线程的
    /// 数据源 `RingBuf` 由 `bpf.take_map("EVENTS")` 拿出，但 bpf 本身
    /// 必须存活到 Drop，否则 attach link 会失效。
    _bpf: Option<Ebpf>,
}

impl EbisuBpfWorker {
    /// 拉取所有已 buffer 的 FlowEvent（主线程 1s tick 调）。
    #[must_use]
    pub fn try_recv_events(&self) -> Vec<FlowEvent> {
        let mut events = Vec::new();
        while let Ok(ev) = self.flow_rx.try_recv() {
            events.push(ev);
        }
        events
    }
}

impl Drop for EbisuBpfWorker {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(h) = self.ring_buf_thread.take() {
            let _ = h.join();
        }
    }
}

/// 启动 eBPF worker。任何步骤失败（load / attach / spawn）都返回 `None`，
/// 上游 UI 走 fallback。错误日志通过 `tracing::warn!` 输出。
#[must_use]
pub fn try_spawn_impl(_crash_tx: Option<Sender<WorkerCrash>>) -> Option<EbisuBpfWorker> {
    // 1. 加载内核态 ELF。失败常见原因：ELF 未编译 / 内核版本不兼容。
    let mut bpf = Ebpf::load(EBPF_ELF)
        .map_err(|e| {
            tracing::warn!(error = %e, "eBPF ELF load 失败，flows 走 fallback");
        })
        .ok()?;

    // aya-log 可选；失败仅丢失内核侧 debug 日志，不阻塞。
    let _ = EbpfLogger::init(&mut bpf);

    // 2. attach sys_enter_connect tracepoint
    let prog1: &mut TracePoint = bpf
        .program_mut("proc_sys_enter_connect")
        .ok()??
        .try_into()
        .ok()?;
    prog1
        .load()
        .map_err(|e| {
            tracing::warn!(error = %e, "proc_sys_enter_connect load 失败");
        })
        .ok()?;
    prog1.attach("syscalls", "sys_enter_connect").map_err(|e| {
        tracing::warn!(error = %e, "proc_sys_enter_connect attach 失败（无权限？内核 < 5.10？）");
    }).ok()?;

    // 3. attach sched_process_exit tracepoint
    let prog2: &mut TracePoint = bpf
        .program_mut("proc_sched_process_exit")
        .ok()??
        .try_into()
        .ok()?;
    prog2
        .load()
        .map_err(|e| {
            tracing::warn!(error = %e, "proc_sched_process_exit load 失败");
        })
        .ok()?;
    prog2
        .attach("sched", "sched_process_exit")
        .map_err(|e| {
            tracing::warn!(error = %e, "proc_sched_process_exit attach 失败");
        })
        .ok()?;

    // 4. 拿 EVENTS ring_buf。`take_map` 把所有权移出来，bpf 内部不再持有。
    let ring_buf_map = bpf.take_map("EVENTS")?;
    let mut ring_buf: RingBuf<_> = RingBuf::try_from(ring_buf_map)
        .map_err(|e| {
            tracing::warn!(error = %e, "EVENTS ring_buf 绑定失败");
        })
        .ok()?;

    // 5. spawn reader 线程
    let stop_flag = Arc::new(AtomicBool::new(false));
    let (flow_tx, flow_rx) = mpsc::sync_channel::<FlowEvent>(EVENT_CHANNEL_BOUND);
    let stop_clone = Arc::clone(&stop_flag);
    let handle = thread::Builder::new()
        .name("ebpf-ring-buf-reader".into())
        .spawn(move || run_reader_loop(ring_buf, flow_tx, stop_clone))
        .ok()?;

    Some(EbisuBpfWorker {
        flow_rx,
        ring_buf_thread: Some(handle),
        stop_flag,
        _bpf: Some(bpf),
    })
}

/// reader 线程主循环：drain ring_buf → parse → send mpsc。
///
/// 出错时 `tracing::warn!` 后继续（单个事件损坏不影响后续）；channel 满
/// (`WouldBlock`) 时直接丢，与 v0.7 disk_io_etw 同款"最新"语义。
fn run_reader_loop(
    mut ring_buf: RingBuf<i32>,
    flow_tx: Sender<FlowEvent>,
    stop_flag: Arc<AtomicBool>,
) {
    while !stop_flag.load(Ordering::Relaxed) {
        // 单轮消费所有可用事件；RingBuf::next() 返回 None 表示当前无数据。
        while let Some(item) = ring_buf.next() {
            let bytes = item.to_vec();
            if bytes.len() >= std::mem::size_of::<RawEvent>() {
                // SAFETY: bytes.len() >= sizeof RawEvent，结构 #[repr(C)]
                // 字段顺序对齐。读 raw pointer 之前确认 size_of。
                let raw: &RawEvent = unsafe { &*(bytes.as_ptr() as *const RawEvent) };
                if let Some(ev) = parse_raw_event(raw) {
                    if flow_tx.send(ev).is_err() {
                        // 主线程已 drop worker / 退出；停止读。
                        return;
                    }
                }
            }
        }
        thread::sleep(RING_BUF_POLL);
    }
}
