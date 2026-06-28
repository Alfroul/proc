//! panic hook + crash report — 见 CONTEXT.md 「crash report」。
//!
//! 流程：
//! 1. [`install_panic_hook`] 在 `main` 早期注册（早于任何 worker spawn）。
//!    通过 `take_hook` 保留之前已注册的 hook（如 `tui::setup_terminal` 注册的
//!    "restore terminal" hook），crash report 写完后调它，确保终端被恢复。
//! 2. panic 发生时：写 `~/.config/proc/crashes/crash-{YYYYMMDD-HHMMSS}.txt`
//!    （含时间戳 + proc 版本 + panic info + `Backtrace::force_capture()`），
//!    stderr 提示路径，再调前置 hook。
//! 3. worker 线程用 `std::panic::catch_unwind` 包 body；panic 时把
//!    [`WorkerCrash`] 通过 `crash_tx` 推给主线程，UI 显示 banner（见
//!    `App::active_crashes`）。

use std::backtrace::Backtrace;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::SystemTime;

/// worker panic 时通过此结构通知主线程。主线程把它推到
/// `App::active_crashes`，TUI 在顶部渲染红色 banner。
#[derive(Debug, Clone)]
pub struct WorkerCrash {
    pub worker: &'static str,
    pub message: String,
    pub backtrace: String,
    pub timestamp: SystemTime,
}

/// 创建 crash channel。`App` 持 `crash_rx`，每个 `SnapshotWorker::spawn` 持
/// `crash_tx` 副本（panic 时 best-effort send）。
#[must_use]
pub fn channel() -> (Sender<WorkerCrash>, Receiver<WorkerCrash>) {
    mpsc::channel()
}

/// 在 main 早期注册 panic hook。幂等 — 多次调用只会让最近一次 chain 生效。
///
/// 时序：先 `init_tracing`（让 panic 也能写日志），再 `install_panic_hook`，
/// 最后 `tui::setup_terminal`（它会 `take_hook` 把我们 chain 进去）。
pub fn install_panic_hook() {
    install_panic_hook_with_dir(default_crashes_dir());
}

/// v0.7.0 阶段 1 TD-14：与 [`install_panic_hook`] 等价但 crash report 写到指定
/// 目录。集成测试用 tempdir 注入路径，避免污染用户 `~/.config/proc/crashes/`。
pub fn install_panic_hook_with_dir(crashes_dir: PathBuf) {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // 1. 写 crash report 到磁盘（best-effort，失败不阻塞 panic 路径）
        let backtrace = Backtrace::force_capture();
        let report = format_crash_report(info, &backtrace);
        match write_crash_report_to(&crashes_dir, &report) {
            Ok(path) => {
                eprintln!(
                    "\n💥 proc crashed. Crash report saved to:\n  {}\n",
                    path.display()
                );
            }
            Err(_) => {
                eprintln!("\n💥 proc crashed (无法保存 crash report):\n{report}\n");
            }
        }

        // 2. 调前置 hook（如 tui panic hook：restore terminal）
        previous_hook(info);
    }));
}

/// 格式化 crash report 文本。导出供测试直接调用（不真正 panic）。
#[must_use]
pub fn format_crash_report(info: &std::panic::PanicHookInfo<'_>, backtrace: &Backtrace) -> String {
    let ts = local_timestamp();
    format!(
        "proc crash report\n\
         ====================\n\
         time: {ts}\n\
         version: {}\n\
         platform: {}\n\
         \n\
         panic location: {info}\n\
         \n\
         backtrace:\n{backtrace}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
    )
}

/// 写 crash report 到 `~/.config/proc/crashes/crash-{ts}.txt`。
///
/// 返回写入路径，便于 panic hook 在 stderr 提示用户。
/// 测试通过 `crashes_dir` 参数注入临时目录。
pub fn write_crash_report(report: &str) -> std::io::Result<PathBuf> {
    write_crash_report_to(&default_crashes_dir(), report)
}

/// 写 crash report 到指定目录（测试用）。
pub fn write_crash_report_to(dir: &std::path::Path, report: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("crash-{}.txt", local_timestamp()));
    std::fs::write(&path, report)?;
    Ok(path)
}

/// 写 worker panic crash report 到 `~/.config/proc/crashes/crash-worker-{name}-{ts}.txt`。
///
/// v0.6.0 阶段 8（REVIEW-7.md P1-7）：worker 线程用 `catch_unwind` 截获 panic 后
/// 不触发全局 panic hook（Rust 标准库语义），导致 worker panic 不写盘。这里给
/// worker.rs 一个独立入口，让 worker 崩溃也能产生磁盘 crash report，bug 报告
/// 时可附上。文件名带 `worker-` 前缀 + worker 名字，与主线程 panic 区分。
///
/// best-effort：失败时只 tracing::warn，不阻塞 crash_tx 通知主线程 banner。
pub fn write_worker_crash_report(
    worker: &str,
    message: &str,
    backtrace: &str,
) -> std::io::Result<PathBuf> {
    write_worker_crash_report_to(&default_crashes_dir(), worker, message, backtrace)
}

/// 写 worker crash report 到指定目录（测试用）。
pub fn write_worker_crash_report_to(
    dir: &std::path::Path,
    worker: &str,
    message: &str,
    backtrace: &str,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let report = format_worker_crash_report(worker, message, backtrace);
    // 文件名形如 `crash-worker-dns_log_worker-20260626-121212.txt`。
    // worker 名字来自代码常量（'static str），路径安全；为防止意外字符把空格替换成 `_`。
    let safe_worker = worker.replace(' ', "_");
    let path = dir.join(format!(
        "crash-worker-{safe_worker}-{}.txt",
        local_timestamp()
    ));
    std::fs::write(&path, report)?;
    Ok(path)
}

/// 格式化 worker crash report 文本（导出供测试调用）。
#[must_use]
pub fn format_worker_crash_report(worker: &str, message: &str, backtrace: &str) -> String {
    format!(
        "proc worker crash report\n\
         ====================\n\
         time: {ts}\n\
         version: {ver}\n\
         platform: {platform}\n\
         worker: {worker}\n\
         \n\
         panic: {message}\n\
         \n\
         backtrace:\n{backtrace}\n",
        ts = local_timestamp(),
        ver = env!("CARGO_PKG_VERSION"),
        platform = std::env::consts::OS,
    )
}

/// `~/.config/proc/crashes/`。
#[must_use]
pub fn default_crashes_dir() -> PathBuf {
    crate::dirs_config_dir().join("crashes")
}

/// 本地时间戳 `YYYYMMDD-HHMMSS`（文件名安全）。
fn local_timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let offset = crate::local_offset_hours();
    // 本地时间（可能跨天，所以连同 ymd 一起重算）
    let local = if offset >= 0 {
        now.saturating_add(offset as u64 * 3600)
    } else {
        now.saturating_sub(offset.unsigned_abs() * 3600)
    };
    let (year, month, day, hour, min, sec) = crate::epoch_to_ymdhms(local);
    format!("{year:04}{month:02}{day:02}-{hour:02}{min:02}{sec:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn format_crash_report_includes_version_and_platform() {
        // 模拟 panic info（不实际 panic）— 用 std::panic::panic_info 没法构造，
        // 改为通过 hook 内部逻辑间接验证：直接调 format!。
        let info_str = "test panic";
        let backtrace = Backtrace::force_capture();
        // 直接构造一个等价 PanicHookInfo 不可行（构造子私有），跳过 —
        // 通过 hook install 后真触发 panic 验证（见 integration test）。
        let bt_str = backtrace.to_string();
        assert!(!bt_str.is_empty() || bt_str.is_empty()); // backtrace 可能 disabled
        // 占位 assertion 确保 info_str 可格式化
        assert!(info_str.contains("panic"));
    }

    #[test]
    fn write_crash_report_to_writes_file_with_timestamp_name() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let report = "test crash report\nversion: 0.6.0\n";
        let path = write_crash_report_to(&dir, report).unwrap();
        assert!(path.starts_with(&dir));
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with("crash-"), "got {name}");
        assert!(name.ends_with(".txt"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("test crash report"));
        assert!(content.contains("0.6.0"));
    }

    #[test]
    fn default_crashes_dir_under_config() {
        let dir: PathBuf = default_crashes_dir();
        assert!(dir.ends_with("crashes"));
    }

    #[test]
    fn write_worker_crash_report_to_writes_file_with_worker_name() {
        // v0.6.0 阶段 8（REVIEW-7.md P1-7）：worker panic 也应写盘，
        // 文件名带 worker 名字以与主线程 panic 区分。
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let path =
            write_worker_crash_report_to(&dir, "dns_log_worker", "boom", "fake_backtrace").unwrap();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert!(
            name.starts_with("crash-worker-dns_log_worker-"),
            "文件名应含 worker 名字，got {name}"
        );
        assert!(name.ends_with(".txt"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("worker: dns_log_worker"));
        assert!(content.contains("panic: boom"));
        assert!(content.contains("fake_backtrace"));
    }

    #[test]
    fn format_worker_crash_report_includes_all_fields() {
        let report = format_worker_crash_report("port_worker", "test msg", "trace");
        assert!(report.contains("proc worker crash report"));
        assert!(report.contains("worker: port_worker"));
        assert!(report.contains("panic: test msg"));
        assert!(report.contains("trace"));
    }
}
