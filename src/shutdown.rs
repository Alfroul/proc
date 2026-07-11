use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

static FLAG: OnceLock<Arc<AtomicBool>> = OnceLock::new();

/// Registers the global Ctrl+C / SIGINT handler. Safe to call multiple times;
/// only the first call installs the handler.
pub fn init() {
    FLAG.get_or_init(|| {
        let flag = Arc::new(AtomicBool::new(false));
        let flag_for_handler = flag.clone();
        // Best-effort registration. ctrlc may fail on some platforms; we
        // still return the flag so callers can poll it (it just won't flip).
        let _ = ctrlc::set_handler(move || {
            flag_for_handler.store(true, Ordering::SeqCst);
        });
        flag
    });
}

/// Returns true if Ctrl+C / SIGINT has been received since process start.
/// Always false if [`init`] was never called.
pub fn requested() -> bool {
    FLAG.get()
        .map(|f| f.load(Ordering::SeqCst))
        .unwrap_or(false)
}

/// 主动触发 shutdown（v0.18 stage 2 项 4 落地）。
///
/// flip 全局 flag 让 `requested()` 返 true，主循环（如 `run_record_headless`）
/// 检测后干净退出。用于 record auto-stop timer thread（`run_record_headless` 内
/// spawn `std::thread::spawn(move || { sleep N secs; shutdown::request(); })`）。
///
/// 与 Ctrl+C handler 走同一 flag，主循环无需区分 shutdown 来源（用户 Ctrl+C
/// 还是 timer 触发）— 都走相同的干净退出路径（VtRecorder::stop + flush）。
///
/// # Panics
///
/// 不会 panic。如 [`init`] 未调用（FLAG 未初始化），本函数 no-op（与
/// [`requested`] 在 FLAG 未初始化时返 false 一致）。
pub fn request() {
    if let Some(flag) = FLAG.get() {
        flag.store(true, Ordering::SeqCst);
    }
}
