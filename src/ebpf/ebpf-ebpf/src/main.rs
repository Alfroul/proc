//! v0.7 阶段 8：eBPF 内核态程序（ADR-0016）。
//!
//! **Part A 状态**：骨架占位，含 3 类事件的 tracepoint / kprobe 模板 +
//! Event union（与 userspace `src/ebpf/flow.rs::FlowEvent` 二进制兼容）+
//! RingBuf 推送。Linux 验证 / 真实 attach 在 Part B / Linux 会话完成。
//!
//! 监听事件：
//! - `sys_enter_connect`：socket connect() 入口（fd + sockaddr_in 指针）
//! - `sched_process_exit`：进程退出（exit-accounting）
//! - （Part B）kprobe `tcp_connect` / `__tcp_connect`：完整 TCP 流建立
//!
//! 编译：
//! ```bash
//! cd src/ebpf/ebpf-ebpf
//! cargo +nightly build --target bpfel-unknown-none
//! ```
//!
//! 数据通路：
//! 内核态 ring_buf → userspace reader 线程 → FlowAggregator → App::flows

#![no_std]
#![no_main]

use aya_ebpf::{
    helpers::gen::{bpf_get_current_pid_tgid, bpf_get_current_task, bpf_ktime_get_ns},
    macros::{map, tracepoint},
    maps::RingBuf,
    programs::TracePointContext,
};

// RingBuf 容量：4096 字节（aya 最小要求）。低频事件足够；高频场景调大。
#[map]
static EVENTS: RingBuf = RingBuf::pinned(64 * 1024);

/// Event kind 标签（与 userspace `FlowEventKind` 对齐，u8 ABI）。
#[repr(u8)]
pub enum EventKind {
    /// socket connect() 入口（sa_family + pid + daddr + dport）。
    Connect = 1,
    /// 进程退出（pid + start_time）。
    Exit = 2,
}

/// 内核态 → 用户态单条事件。
///
/// **ABI 稳定**：userspace `src/ebpf/flow.rs::RawEvent` 必须 byte-for-byte
/// 与此结构兼容（含 `#[repr(C)]`、字段顺序、padding）。改字段必须双边同步。
#[repr(C)]
pub struct Event {
    pub kind: u8,
    pub _pad: [u8; 3],
    pub pid: u32,
    pub start_time: u64,
    pub ts_ns: u64,
    /// IPv4 远端地址（network byte order）。`sched_process_exit` 不用，置 0。
    pub remote_addr: u32,
    /// 远端端口（host byte order）。`sched_process_exit` 不用，置 0。
    pub remote_port: u16,
    pub _pad2: [u8; 6],
}

/// `sys_enter_connect` tracepoint。
///
/// tracepoint context 拿不到 sockaddr 指针的 typed API；用 `ctx.read_at::<u64>(offset)`
/// 按 `/sys/kernel/debug/tracing/events/syscalls/sys_enter_connect/format` 的字段
/// 偏移读 `struct sockaddr __user *` 指针，再 `bpf_probe_read_user` 读真正 sockaddr。
///
/// 偏移量在不同内核版本上稳定（tracepoint 格式声明固定）：
/// - offset 8：`__syscall_meta` 之后的 common 字段后第一个 syscall-specific 字段
/// - 实际偏移需在 Linux 会话用 `cat /sys/kernel/debug/tracing/events/syscalls/sys_enter_connect/format`
///   核对；这里先按典型 x86_64 layout 写 16。
#[tracepoint]
pub fn proc_sys_enter_connect(ctx: TracePointContext) -> u32 {
    match try_handle_connect(&ctx) {
        Ok(_) => 0,
        Err(_) => 1,
    }
}

fn try_handle_connect(ctx: &TracePointContext) -> Result<(), u64> {
    // 从 ctx 拿 syscall 第二参数（sockaddr 指针）。tracepoint arg offset = 16
    // （common header 8 bytes + syscall id 8 bytes）。详见上方注释。
    let sockaddr_ptr: u64 = ctx.read_at::<u64>(16).map_err(|_| 1u64)?;

    // 先读 sa_family（前 2 字节）判 IPv4 / IPv6
    let mut family_buf = [0u8; 16];
    aya_ebpf::helpers::gen::bpf_probe_read_user(
        family_buf.as_mut_ptr() as *mut _,
        16,
        sockaddr_ptr as *const _,
    )
    .map_err(|_| 2u64)?;

    let family = u16::from_ne_bytes([family_buf[0], family_buffer_get(&family_buf, 1)]);
    if family != 2 {
        // AF_INET = 2；AF_INET6 = 10 留 Part B。非 IPv4/6 直接放行。
        return Ok(());
    }

    // sockaddr_in layout：family(2) + port(2) + addr(4)
    let port_be = u16::from_ne_bytes([
        family_buffer_get(&family_buf, 2),
        family_buffer_get(&family_buf, 3),
    ]);
    let addr_be = u32::from_ne_bytes([
        family_buffer_get(&family_buf, 4),
        family_buffer_get(&family_buf, 5),
        family_buffer_get(&family_buf, 6),
        family_buffer_get(&family_buf, 7),
    ]);

    let pid_tgid = bpf_get_current_pid_tgid();
    let pid = (pid_tgid & 0xFFFF_FFFF) as u32;
    let start_time = bpf_current_task_start_time();
    let ts_ns = bpf_ktime_get_ns();

    push_event(Event {
        kind: EventKind::Connect as u8,
        _pad: [0; 3],
        pid,
        start_time,
        ts_ns,
        remote_addr: addr_be,
        remote_port: port_be,
        _pad2: [0; 6],
    });

    Ok(())
}

/// `sched:sched_process_exit` tracepoint（exit-accounting）。
#[tracepoint(category = "sched")]
pub fn proc_sched_process_exit(ctx: TracePointContext) -> u32 {
    match try_handle_exit(&ctx) {
        Ok(_) => 0,
        Err(_) => 1,
    }
}

fn try_handle_exit(ctx: &TracePointContext) -> Result<(), u64> {
    // sched_process_exit format: common 8 bytes + `comm[16]` + `pid(4)` + `priority(4)`。
    // 偏移 24 = 8 + 16（comm）= pid 起始位置。
    let pid = ctx.read_at::<u32>(24).map_err(|_| 1u64)?;
    let start_time = bpf_current_task_start_time();
    let ts_ns = bpf_ktime_get_ns();

    push_event(Event {
        kind: EventKind::Exit as u8,
        _pad: [0; 3],
        pid,
        start_time,
        ts_ns,
        remote_addr: 0,
        remote_port: 0,
        _pad2: [0; 6],
    });

    Ok(())
}

/// 把事件推到 RingBuf。预留 reserved 后写 header + payload。
fn push_event(event: Event) {
    if let Some(mut entry) = EVENTS.reserve::<Event>(0) {
        entry.write(event);
        entry.submit(0);
    }
}

/// 取 `family_buf` 第 idx 字节。helper 因为 no_std 不能直接 slice index（BPF 验证器
/// 无法证明 idx < len）—— 单独函数让 verifier 友好。
fn family_buffer_get(buf: &[u8; 16], idx: usize) -> u8 {
    if idx < buf.len() {
        buf[idx]
    } else {
        0
    }
}

/// 通过 `bpf_get_current_task` 拿 `task_struct`，再读 `start_boot_ns` 字段。
///
/// **注意**：`task_struct` 字段偏移内核版本敏感（CO-RE 应该用 BTF，但 aya-ebpf
/// 的 BTF helper 当前 API 不稳定）。Part A 这里返回 0 占位；Part B 在 Linux 会话
/// 用 `aya-tool` 生成 BTF binding 后补完真实 start_time。
fn bpf_current_task_start_time() -> u64 {
    let _ = bpf_get_current_task();
    0
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
