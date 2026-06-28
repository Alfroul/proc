//! v0.7.0 阶段 1 TD-14：panic hook chain 时序验证（integration test）。
//!
//! 验证三个场景（见 `docs/tech-debt.md` TD-14 / `docs/stages/v0.7-stage-1.md` 任务 9）：
//!
//! 1. **TUI 模式链式**：先模拟 `tui::setup_terminal` 注册的 terminal restore hook，
//!    再调 `install_panic_hook_with_dir`，触发 panic 后验证：
//!    - crash report 文件出现在指定目录，文件名 `crash-{ts}.txt`
//!    - 文件内容含 panic message + proc 版本
//!    - terminal restore hook 被调用（计数器 +1）
//! 2. **catch_unwind 与 panic hook 的关系**：catch_unwind 捕获 panic 时
//!    panic hook **会被调用**（Rust 标准库实际语义；与 worker 注释里的
//!    旧猜测不同 — worker 仍显式调 `write_worker_crash_report` 是为了
//!    文件名带 worker 前缀 + 不与主线程 report 冲突，不是因为 hook 没调）。
//! 3. **CLI 模式（无 TUI hook）**：只装 panic hook，不装 terminal restore，
//!    触发 panic 验证 crash report 仍写盘 + restore 计数器为 0。
//!
//! **隔离**：所有 panic hook 测试用 `STATIC_MUTEX` 串行执行，因为全局 panic
//! hook 是 process-wide 状态，并发跑会互相覆盖 hook。每个测试结束时 `take_hook`
//! 清理自己注册的 hook，避免污染后续测试。

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use proc::metrics::crash;

/// 强制本文件所有测试串行执行（panic hook 是全局状态）。
static TEST_MUTEX: Mutex<()> = Mutex::new(());

/// 模拟 tui::setup_terminal 注册的 terminal restore hook 计数器。
/// 每次被调用 +1，验证 panic 链是否走到 restore 步骤。
static RESTORE_CALLS: AtomicUsize = AtomicUsize::new(0);

/// 测试用的 panic message — 同时是 grep 文件内容的锚点。
const TEST_PANIC_MSG: &str = "td-14 synthetic panic";

fn read_single_crash_file(dir: &std::path::Path) -> (PathBuf, String) {
    let entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read_dir {dir:?}: {e}"))
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("crash-"))
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "expected exactly one crash-* file in {dir:?}, got {entries:?}"
    );
    let path = entries[0].path();
    let content = std::fs::read_to_string(&path).unwrap();
    (path, content)
}

#[test]
fn tui_chain_writes_report_and_invokes_restore_hook() {
    let _guard = TEST_MUTEX.lock().unwrap();
    RESTORE_CALLS.store(0, Ordering::SeqCst);

    let tmp = tempfile::tempdir().unwrap();

    // 1. 模拟 tui::setup_terminal 注册的 restore hook（先于 panic hook 安装）。
    //    语义与 src/tui/mod.rs:59-64 一致：take_hook → 包一层 restore → set_hook。
    let pre_existing_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        RESTORE_CALLS.fetch_add(1, Ordering::SeqCst);
        pre_existing_hook(info);
    }));

    // 2. 装 crash hook（会 take_hook 上面那层，链式调用）。
    crash::install_panic_hook_with_dir(tmp.path().to_path_buf());

    // 3. 触发 panic。catch_unwind 阻止 unwind 杀测试线程，但 panic hook
    //    **仍会被调用**（Rust 标准库实际行为）。
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        panic!("{TEST_PANIC_MSG}");
    }));
    assert!(result.is_err(), "catch_unwind should capture the panic");

    // 4. 清理：把整个 chain 拆掉，恢复默认 hook，避免污染后续测试。
    let _ = std::panic::take_hook();

    // 5. 验证 crash report 文件 + 内容。
    let (path, content) = read_single_crash_file(tmp.path());
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    assert!(
        name.starts_with("crash-") && name.ends_with(".txt"),
        "filename pattern crash-*.txt, got {name}"
    );
    assert!(
        content.contains(TEST_PANIC_MSG),
        "crash report should embed panic message; got:\n{content}"
    );
    assert!(
        content.contains(env!("CARGO_PKG_VERSION")),
        "crash report should embed proc version; got:\n{content}"
    );

    // 6. 验证 restore hook 被调用恰好一次（panic 链终点）。
    assert_eq!(
        RESTORE_CALLS.load(Ordering::SeqCst),
        1,
        "restore hook should be invoked exactly once at end of panic chain"
    );
}

#[test]
fn catch_unwind_invokes_panic_hook_before_catching() {
    // 验证 Rust 标准库实际语义：catch_unwind 之前 panic hook 已被调用。
    // 这条事实让 worker.rs 的 catch_unwind + 显式 write_worker_crash_report
    // 路径有意义 —— worker panic 实际上会写"主线程风格"crash report，
    // 紧接着 worker 又显式写"worker 风格"crash-worker-* report。两份文件
    // 文件名前缀不同，便于人工区分。
    let _guard = TEST_MUTEX.lock().unwrap();
    RESTORE_CALLS.store(0, Ordering::SeqCst);

    let tmp = tempfile::tempdir().unwrap();
    crash::install_panic_hook_with_dir(tmp.path().to_path_buf());

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        panic!("{TEST_PANIC_MSG}-worker");
    }));
    assert!(result.is_err());

    let _ = std::panic::take_hook();

    let entries: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("crash-"))
        .collect();
    assert!(
        !entries.is_empty(),
        "panic hook should have written at least one crash report even though catch_unwind caught the panic"
    );
    // 内容应含 panic message。
    let any_contains_msg = entries.iter().any(|e| {
        std::fs::read_to_string(e.path())
            .map(|c| c.contains(&format!("{TEST_PANIC_MSG}-worker")))
            .unwrap_or(false)
    });
    assert!(
        any_contains_msg,
        "at least one crash report should contain the panic message"
    );
}

#[test]
fn cli_only_chain_writes_report_without_restore_hook() {
    // CLI 模式：只装 crash hook，不装 tui restore hook（setup_terminal 未调用）。
    // 链只有一层：crash hook 写盘 + 调前置（默认）hook。
    let _guard = TEST_MUTEX.lock().unwrap();
    RESTORE_CALLS.store(0, Ordering::SeqCst);

    let tmp = tempfile::tempdir().unwrap();
    // 不模拟 setup_terminal —— 直接装 crash hook。
    crash::install_panic_hook_with_dir(tmp.path().to_path_buf());

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        panic!("{TEST_PANIC_MSG}-cli");
    }));
    assert!(result.is_err());

    let _ = std::panic::take_hook();

    // CLI 模式 restore hook 不存在，计数器应为 0。
    assert_eq!(
        RESTORE_CALLS.load(Ordering::SeqCst),
        0,
        "no tui restore hook installed in CLI mode"
    );

    let (_path, content) = read_single_crash_file(tmp.path());
    assert!(
        content.contains(&format!("{TEST_PANIC_MSG}-cli")),
        "CLI panic crash report missing panic message; got:\n{content}"
    );
    assert!(
        content.contains(env!("CARGO_PKG_VERSION")),
        "CLI crash report missing version"
    );
}

#[test]
fn worker_crash_report_helper_writes_separate_file() {
    // 验证 worker panic 的"补充"路径：catch_unwind handler 内显式调
    // write_worker_crash_report（v0.6.0 阶段 8 P1-7）→ 写 crash-worker-*.txt。
    // 与上面的 catch_unwind_invokes_panic_hook 测试合起来，说明真实 worker
    // panic 在生产环境会产生 2 个文件：crash-*.txt（hook 写）+ crash-worker-*.txt
    // （worker 显式写）。bug 报告时附 worker 那份更精准。
    let _guard = TEST_MUTEX.lock().unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let path = crash::write_worker_crash_report_to(
        tmp.path(),
        "dns_log_worker",
        "synthetic worker panic",
        "<synthetic backtrace>",
    )
    .unwrap();

    let name = path.file_name().unwrap().to_string_lossy().to_string();
    assert!(
        name.starts_with("crash-worker-dns_log_worker-"),
        "worker crash filename pattern crash-worker-<name>-*, got {name}"
    );
    assert!(name.ends_with(".txt"));

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("worker: dns_log_worker"));
    assert!(content.contains("synthetic worker panic"));
    assert!(content.contains("<synthetic backtrace>"));
}
