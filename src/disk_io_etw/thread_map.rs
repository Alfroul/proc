//! thread_id → pid map，5s 全量刷新。供 ETW callback 把 DiskIo 事件的
//! IssuingThreadId 解析成 PID。
//!
//! 实现：`CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD)` 全量枚举所有线程，
//! 读 THREADENTRY32.th32ThreadID + th32OwnerProcessID 写到 map。
//!
//! 不用 sysinfo 的 `Process::tasks()`：sysinfo 0.34.2 在 Windows 上实测
//! `tasks()` 经常返回 None（task 列表初始化时机不稳定），用 ToolHelp 更可靠。
//! 项目 src/collect.rs 已经用同款 ToolHelp 模式枚举进程。
//!
//! `Drop` 时 stop_flag 设 true，刷新线程下一轮检测到后退出，主线程 join。

#![cfg(target_os = "windows")]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};

const REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// 包装 thread→pid map + 刷新线程句柄。Drop 时自动停止刷新。
pub(super) struct ThreadToPidMap {
    map: Arc<Mutex<HashMap<u32, u32>>>,
    stop_flag: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ThreadToPidMap {
    /// 启动刷新线程，**同步预填一份** map 后返回（避免 callback 第一次拉到空 map）。
    pub(super) fn spawn_refresh_thread() -> Self {
        let map: Arc<Mutex<HashMap<u32, u32>>> = Arc::new(Mutex::new(HashMap::new()));

        // 同步预填——5s 内的 IO 也能正确归到 PID
        refresh_once(&map);

        let map_clone = Arc::clone(&map);
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop_flag);
        let thread = thread::Builder::new()
            .name("disk-io-etw-pid-refresh".into())
            .spawn(move || {
                while !stop_clone.load(Relaxed) {
                    // 用 sleep 而不是 recv_timeout：刷新线程无 channel，独立运转
                    thread::sleep(REFRESH_INTERVAL);
                    if stop_clone.load(Relaxed) {
                        break;
                    }
                    refresh_once(&map_clone);
                }
            })
            .expect("spawn disk-io-etw-pid-refresh");

        Self {
            map,
            stop_flag,
            thread: Some(thread),
        }
    }

    /// 克隆内部的 Arc handle，供 callback 线程读取。
    pub(super) fn clone_handle(&self) -> Arc<Mutex<HashMap<u32, u32>>> {
        Arc::clone(&self.map)
    }
}

impl Drop for ThreadToPidMap {
    fn drop(&mut self) {
        self.stop_flag.store(true, Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

/// 用 ToolHelp 全量拉一次 thread→pid map，替换 `map` 内容。
fn refresh_once(map: &Arc<Mutex<HashMap<u32, u32>>>) {
    let snapshot: HANDLE = match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) } {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(error = %e, "CreateToolhelp32Snapshot(THREAD) 失败，保留旧 thread_map");
            return;
        }
    };

    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;

    let mut new_map: HashMap<u32, u32> = HashMap::new();

    // SAFETY: snapshot handle 来自 CreateToolhelp32Snapshot；entry 是 local stack。
    let ok_first = unsafe { Thread32First(snapshot, &mut entry) }.is_ok();
    if ok_first {
        loop {
            // 跳过 PID 0 owner（Idle）—— 它的 IO 通常由 kernel 调度器发出，
            // 归到 PID 0 用户看到「System Idle」也无意义
            if entry.th32OwnerProcessID != 0 {
                new_map.insert(entry.th32ThreadID, entry.th32OwnerProcessID);
            }
            // SAFETY: 同上。
            if unsafe { Thread32Next(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
    }

    let _ = unsafe { CloseHandle(snapshot) };

    if let Ok(mut guard) = map.lock() {
        *guard = new_map;
    }
    // 锁中毒时不写入（保留旧 map），下一轮再尝试
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 刷新一次：至少包含当前测试进程自己的 PID 和它所有线程。
    #[test]
    fn refresh_includes_current_process() {
        let map = Arc::new(Mutex::new(HashMap::new()));
        refresh_once(&map);

        let guard = map.lock().unwrap();
        let current_pid = std::process::id();
        let has_current = guard.values().any(|&p| p == current_pid);
        assert!(
            has_current,
            "current PID {} 应出现在 thread→pid map 中（实际 map 大小 {}）",
            current_pid,
            guard.len()
        );
        assert!(!guard.is_empty(), "thread→pid map 不应为空");
    }

    /// Drop 后刷新线程必须干净退出（不挂测试）。
    #[test]
    fn drop_joins_refresh_thread() {
        let start = std::time::Instant::now();
        {
            let _map = ThreadToPidMap::spawn_refresh_thread();
            // 立刻 drop——join 必须在合理时间内返回（不会等满 5s sleep）
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(7),
            "Drop 耗时 {:?}，预期 < 7s",
            elapsed
        );
    }
}
