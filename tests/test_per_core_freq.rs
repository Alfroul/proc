//! 阶段 2 — B2 per-core CPU 频率/温度集成测试。
//!
//! 三类用例：
//! 1. `SystemSnapshot::per_core_freq()` 在 Windows/Linux/macOS 上至少返回 1 个频率
//!    （sysinfo 0.34 在三个平台都有 `Cpu::frequency()` 实现）。
//! 2. `parse_scaling_cur_freq` 纯函数的跨平台 sanity（详细边界 case 在 src/collect.rs
//!    的内嵌单元测试里覆盖）。
//! 3. App 状态机：按 `c` 切换 `sidebar_expanded`，状态正确 + 高度随之变化 +
//!    持久化到 ui.toml。
//!
//! 注意：`sidebar_expanded` 默认从 ui.toml 加载。下面把 HOME / USERPROFILE 指向
//! 临时目录，并在单个测试 fn 内串行驱动，避免 cargo 默认并行跑测试时多个用例
//! 抢同一个 ui.toml。

use std::sync::OnceLock;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use proc::app::App;
use proc::collect::{SystemSnapshot, parse_scaling_cur_freq};
use proc::error::Result;

// 一次性把 HOME / USERPROFILE 重定向到临时目录，让 ui_state::path() 落在沙箱里。
// 子进程级修改会持续到测试结束，所以用 OnceLock 保证只 set 一次。
static ENV_SANDBOX: OnceLock<tempfile::TempDir> = OnceLock::new();

fn sandbox_home() -> &'static std::path::Path {
    let dir = ENV_SANDBOX.get_or_init(|| {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let path = tmp.path();
        // Windows 用 USERPROFILE，Linux/macOS 用 HOME。
        // SAFETY: 测试是单线程跑（这个文件所有 HOME-touching 测试集中在
        // app_sidebar_toggle_state_machine 一个 fn 里）+ 测试进程退出后环境不影响他人。
        unsafe {
            std::env::set_var("USERPROFILE", path);
            std::env::set_var("HOME", path);
        }
        tmp
    });
    dir.path()
}

// ── per-core 频率采集 ──────────────────────────────────────────────────────────

#[test]
fn snapshot_returns_at_least_one_core_freq() -> Result<()> {
    let snap = SystemSnapshot::new()?;
    // worker 启动时已 recv_first，缓存里有至少一帧。这里直接读 last-known。

    let freq = snap.per_core_freq();
    // sysinfo 0.34 在 Windows（注册表 ~MHz）、Linux（sysfs / /proc/cpuinfo）、
    // macOS（sysctl hw.cpufrequency）都有实现；CI 跑在正常 x86_64 上至少 1 核。
    assert!(
        !freq.is_empty(),
        "per_core_freq 应至少返回 1 个频率，实际为空"
    );
    // 频率值不验证范围：sysinfo 在 Windows 注册表路径偶尔返回 0（Hypervisor /
    // 某些虚拟化场景），Linux sysfs 在无 cpufreq 驱动时也是 0。这些是平台
    // 差异，不是采集 bug；sidebar 会显示 0MHz 让用户知道数据有但不可信。
    // 温度 Vec 长度应与 freq 对齐（即使全 None）。
    assert_eq!(
        snap.per_core_temp().len(),
        freq.len(),
        "per_core_temp 长度应与 per_core_freq 对齐"
    );
    Ok(())
}

// ── parse_scaling_cur_freq 纯函数 ──────────────────────────────────────────────

#[test]
fn parse_scaling_cur_freq_basic() {
    assert_eq!(parse_scaling_cur_freq("3400000\n"), Some(3400));
    assert_eq!(parse_scaling_cur_freq("2500000"), Some(2500));
}

#[test]
fn parse_scaling_cur_freq_rejects_garbage() {
    assert_eq!(parse_scaling_cur_freq(""), None);
    assert_eq!(parse_scaling_cur_freq("not a number"), None);
}

// ── App sidebar_expanded 状态机（串行，单 fn 避免并行文件竞争） ──────────────────

fn press_c(app: &mut App) {
    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
}

#[test]
fn app_sidebar_toggle_state_machine() {
    let home = sandbox_home().to_path_buf();

    // 1) 默认折叠
    let mut app = App::new().expect("App::new");
    assert!(
        !app.sidebar_expanded,
        "ui.toml 不存在时 sidebar_expanded 默认 false"
    );
    assert_eq!(app.sidebar_height(), 13, "折叠高度 = 13");

    // 2) 按 c 展开
    press_c(&mut app);
    assert!(app.sidebar_expanded, "按 c 后 sidebar_expanded 应为 true");
    assert_eq!(
        app.sidebar_height(),
        13 + 1 + 8 + 1,
        "展开高度 = 13 + 表头(1) + 8 核 + 间隔(1)"
    );

    // 3) 写盘验证：按 c 时已经调用了 save_sidebar_expanded
    let ui_toml = home.join(".config").join("proc").join("ui.toml");
    let raw = std::fs::read_to_string(&ui_toml)
        .unwrap_or_else(|_| panic!("ui.toml 应在按 c 后被写盘: {}", ui_toml.display()));
    assert!(
        raw.contains("sidebar_expanded = true"),
        "写盘内容应包含 sidebar_expanded = true，实际：\n{raw}"
    );

    // 4) 再按 c 回到折叠
    press_c(&mut app);
    assert!(!app.sidebar_expanded, "再按 c 应回到折叠");
    assert_eq!(app.sidebar_height(), 13);

    // 5) 再次展开，写盘，然后 drop App 重新构造验证往返
    press_c(&mut app);
    assert!(app.sidebar_expanded);
    drop(app);
    let app2 = App::new().expect("App::new 重载");
    assert!(
        app2.sidebar_expanded,
        "ui.toml 写入后重载 App 应恢复 sidebar_expanded=true"
    );
}
