//! v0.6.0 阶段 3：日志滚动测试。
//!
//! 验证 `proc::cleanup_old_logs`：
//! - 7 天内的 `proc*.log` 保留
//! - 超过 7 天的 `proc*.log` 删除
//! - 非 `proc*.log`（如 `crash-*.txt`）不动
//!
//! init_tracing 整体行为（RollingFileAppender + WorkerGuard flush）走人工验收
//! （启动 proc → 看 ~/.config/proc/proc.YYYY-MM-DD.log 存在 + 内容追加）。

use std::fs;
use std::path::Path;
use std::time::SystemTime;

fn write_with_mtime(path: &Path, content: &str, age_days: u64) {
    fs::write(path, content).unwrap();
    let new_mtime = SystemTime::now() - std::time::Duration::from_secs(age_days * 86400 + 60);
    let _ = filetime::set_file_mtime(path, filetime::FileTime::from_system_time(new_mtime));
}

#[test]
fn cleanup_removes_only_old_proc_logs() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // 8 天前的 proc.log → 应删
    write_with_mtime(&dir.join("proc.2026-06-01.log"), "old", 8);
    // 3 天前的 proc.log → 应保留
    write_with_mtime(&dir.join("proc.2026-06-21.log"), "recent", 3);
    // 今天的 proc.log → 应保留
    write_with_mtime(&dir.join("proc.log"), "today", 0);

    let removed = proc::cleanup_old_logs(dir, 7);
    assert_eq!(removed, 1);
    assert!(!dir.join("proc.2026-06-01.log").exists());
    assert!(dir.join("proc.2026-06-21.log").exists());
    assert!(dir.join("proc.log").exists());
}

#[test]
fn cleanup_does_not_touch_crash_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();

    // crash 文件即使 30 天前也保留 — cleanup_old_logs 只看 proc*.log。
    write_with_mtime(&dir.join("crash-20260501-120000.txt"), "old crash", 30);
    write_with_mtime(&dir.join("proc.old.log"), "old log", 30);

    let removed = proc::cleanup_old_logs(dir, 7);
    assert_eq!(removed, 1);
    assert!(dir.join("crash-20260501-120000.txt").exists());
    assert!(!dir.join("proc.old.log").exists());
}

#[test]
fn cleanup_handles_missing_dir_gracefully() {
    let nonexistent = std::path::PathBuf::from("/nonexistent/dir/for/proc/tests");
    // 不应 panic — read_dir 失败时返回 0。
    let removed = proc::cleanup_old_logs(&nonexistent, 7);
    assert_eq!(removed, 0);
}

#[test]
fn cleanup_with_keep_0_removes_all_proc_logs() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    write_with_mtime(&dir.join("proc.2026-06-23.log"), "1 day", 1);
    write_with_mtime(&dir.join("proc.log"), "today", 0);

    // keep_days=0 → cutoff = now - 0 = now → 任何 mtime <= now 的都删
    // （mtime 是过去时间，必早于 cutoff = now）。
    let removed = proc::cleanup_old_logs(dir, 0);
    assert!(removed >= 1);
}

#[test]
fn epoch_to_ymdhsm_round_trip() {
    // 2026-06-13 00:00:00 UTC = 1_781_308_800
    let (y, m, d, h, min, s) = proc::epoch_to_ymdhms(1_781_308_800);
    assert_eq!((y, m, d, h, min, s), (2026, 6, 13, 0, 0, 0));
    // +1 hour
    let (y, m, d, h, min, s) = proc::epoch_to_ymdhms(1_781_308_800 + 3600);
    assert_eq!((y, m, d, h, min, s), (2026, 6, 13, 1, 0, 0));
    // +1 day + 12:34:56
    let (y, m, d, h, min, s) = proc::epoch_to_ymdhms(1_781_308_800 + 86400 + 45296);
    assert_eq!((y, m, d, h, min, s), (2026, 6, 14, 12, 34, 56));
}
