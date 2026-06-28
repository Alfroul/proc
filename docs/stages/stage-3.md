# 阶段 3：可观测性 Slice — 日志 rotate + crash report + worker metrics

> **独立会话指令**：阅读 CONTEXT.md 和 docs/stages/stage-3.md，完成所有任务后确认完成

**目标**：proc 崩溃后能拿到 backtrace + 上次崩溃前的日志；worker 慢/丢帧有可视化的 metrics。

**前置依赖**：阶段 2 已完成。

**依赖测试**（开工时跑这些测试的详情）：
- `cargo test --release -q`（全量回归 summary，应 ~641）
- `cargo test --release test_env_mask test_self_mitigation test_record_protection test_restricted_spawn -q`（阶段 2 新增安全测试详情）

**预期代码量**：~780 行（含测试）

**任务清单**：

### 任务 1：日志 append + daily rotate（项 #15）

**Cargo.toml 加依赖**：
```toml
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }
tracing-appender = "0.2"
```

**改 `src/main.rs::init_tracing`**：

```rust
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;

// 修改返回签名：返回 guard（main 持有到程序退出）
fn init_tracing() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let config_dir = proc::dirs_config_dir();
    if let Err(e) = std::fs::create_dir_all(&config_dir) {
        eprintln!("警告: 创建配置目录失败: {} (日志不可用)", e);
        return None;
    }
    
    // v0.6.0 阶段 3: daily rotate，保留 7 天
    cleanup_old_logs(&config_dir, 7);
    
    let file_appender = RollingFileAppender::new(
        Rotation::DAILY,
        &config_dir,
        "proc.log",   // 实际文件名: proc.YYYY-MM-DD.log
    );
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_writer(non_blocking)
        .with_ansi(false)
        .finish();
    
    if tracing::subscriber::set_global_default(subscriber).is_err() {
        eprintln!("警告: 初始化日志失败 (日志不可用)");
        return None;
    }
    
    Some(guard)  // 必须 keep alive 到程序退出
}

fn cleanup_old_logs(dir: &std::path::Path, keep_days: u32) {
    use std::time::{Duration, SystemTime};
    let cutoff = SystemTime::now() - Duration::from_secs(keep_days as u64 * 86400);
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            // 匹配 proc.YYYY-MM-DD.log 或 proc.log
            let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if !name.starts_with("proc") || !name.ends_with(".log") {
                continue;
            }
            let modified = entry.metadata()
                .ok()
                .and_then(|m| m.modified().ok());
            if modified.map(|t| t < cutoff).unwrap_or(false) {
                let _ = std::fs::remove_file(&path);
                eprintln!("清理旧日志: {}", path.display());
            }
        }
    }
}

fn main() {
    // v0.6.0 阶段 2: self-mitigation 最早调用
    let failed = proc::security::self_mitigation::apply_self_mitigations();
    if !failed.is_empty() {
        eprintln!("warning: self-mitigation policies failed: {}", failed.join(", "));
    }
    
    // v0.6.0 阶段 3: tracing init 返回 guard，必须 hold 到 main 结束
    let _log_guard = init_tracing();
    
    // v0.6.0 阶段 3: panic hook 早注册（init_tracing 之后，业务逻辑之前）
    proc::metrics::crash::install_panic_hook();
    
    // ... 既有 main 逻辑（保持 _log_guard 在 scope 内）
}
```

**注意**：`_log_guard` drop 时会 flush；main 函数结束前不要让它提前 drop。

**测试**：`tests/test_log_rotate.rs`（新）：
- 启动 proc 测试 binary → 看 `~/.config/proc/proc.YYYY-MM-DD.log` 存在
- 模拟 8 天前的 log 文件 → 下次启动 cleanup 删除
- 多次启动 → 日志追加不覆盖（同一天同一文件）

---

### 任务 2：panic → crash report + worker catch_unwind（项 #16）

**新模块**：`src/metrics/crash.rs`

```rust
//! panic → crash report — 见 CONTEXT.md。
//! panic hook 写 ~/.config/proc/crashes/crash-{YYYYMMDD-HHMMSS}.txt
//! worker 线程用 catch_unwind 包装，panic 时通知主线程显示 banner。

use std::backtrace::Backtrace;
use std::path::PathBuf;

pub fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // 1. 恢复终端（如果是 TUI 模式）
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(std::io::stderr(), crossterm::terminal::LeaveAlternateScreen);
        
        // 2. 写 crash report
        let backtrace = Backtrace::force_capture();
        let report = format_crash_report(info, &backtrace);
        if let Some(path) = write_crash_report(&report) {
            eprintln!("\n💥 proc crashed. Crash report saved to:\n   {}\n", path.display());
        } else {
            eprintln!("\n💥 proc crashed (无法保存 crash report):\n{}\n", report);
        }
        
        // 3. 调用 default hook（输出到 stderr）
        default_hook(info);
    }));
}

fn format_crash_report(info: &std::panic::PanicHookInfo<'_>, backtrace: &Backtrace) -> String {
    let ts = current_timestamp();
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

fn current_timestamp() -> String {
    // 复用 lib.rs 的 epoch_secs_to_ymd
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (year, month, day, hour, min, sec) = proc::epoch_to_ymdhms(now);
    format!("{year:04}-{month:02}-{day:02}-{hour:02}{min:02}{sec:02}")
}

fn write_crash_report(report: &str) -> Option<PathBuf> {
    let config_dir = proc::dirs_config_dir()?;
    let crashes_dir = config_dir.join("crashes");
    std::fs::create_dir_all(&crashes_dir).ok()?;
    let path = crashes_dir.join(format!("crash-{}.txt", current_timestamp()));
    std::fs::write(&path, report).ok()?;
    Some(path)
}
```

**注意**：`proc::epoch_to_ymdhms` 需要在 lib.rs 新增（基于既有 `epoch_secs_to_ymd` 扩展时分秒）。或者用更简单的本地时间获取（既有 `local_offset_hours`）。

**改 `src/worker.rs::run_poll_loop`**：worker 主循环外包 `catch_unwind`

```rust
use std::panic::{catch_unwind, AssertUnwindSafe};

pub fn run_with_crash_recovery<F>(name: &'static str, crash_tx: &Sender<WorkerCrash>, body: F)
where
    F: FnOnce() + Send + 'static,
{
    let result = catch_unwind(AssertUnwindSafe(body));
    if let Err(payload) = result {
        let msg = payload.downcast_ref::<&str>().map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic>".into());
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!(worker = name, panic = %msg, "worker panicked");
        let _ = crash_tx.send(WorkerCrash {
            name,
            message: msg,
            backtrace: backtrace.to_string(),
            timestamp: std::time::SystemTime::now(),
        });
    }
}

#[derive(Debug, Clone)]
pub struct WorkerCrash {
    pub name: &'static str,
    pub message: String,
    pub backtrace: String,
    pub timestamp: std::time::SystemTime,
}
```

**改 `App`**：加 crash_rx + UI banner

```rust
pub struct App {
    // ...
    pub crash_rx: Option<Receiver<WorkerCrash>>,
    pub active_crashes: Vec<WorkerCrash>,
}
```

UI banner：在 `tui/mod.rs::draw_main_panel` 顶部，如果有 `active_crashes` 显示红色 banner：
```
⚠ Worker 'dns-log-reader' 崩溃 (5 min ago): panic message
  按 R 重启 worker / D 关闭提示
```

**测试**：`tests/test_crash_report.rs`（新）：
- 调用 `format_crash_report` 不 panic
- `write_crash_report` 写文件到 tmp dir，内容包含 version / panic / backtrace
- worker crash 时 `crash_tx` 收到 `WorkerCrash`

---

### 任务 3：worker metrics 暴露（项 #17）

**新模块**：`src/metrics/mod.rs`

```rust
//! WorkerMetrics — atomic counters，无锁采集 + 快照查询。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

#[derive(Default)]
pub struct WorkerMetrics {
    pub poll_count: AtomicU64,
    pub poll_total_us: AtomicU64,       // 累计微秒
    pub poll_max_us: AtomicU64,         // 单次最大
    pub channel_full_count: AtomicU64,  // try_send Full 次数
    pub last_error: Mutex<Option<(SystemTime, String)>>,
}

impl WorkerMetrics {
    pub fn record_poll(&self, elapsed: Duration) {
        let us = elapsed.as_micros() as u64;
        self.poll_count.fetch_add(1, Ordering::Relaxed);
        self.poll_total_us.fetch_add(us, Ordering::Relaxed);
        // CAS 更新 max
        let mut current_max = self.poll_max_us.load(Ordering::Relaxed);
        while us > current_max {
            match self.poll_max_us.compare_exchange_weak(
                current_max, us, Ordering::Relaxed, Ordering::Relaxed
            ) {
                Ok(_) => break,
                Err(now) => current_max = now,
            }
        }
    }
    
    pub fn record_channel_full(&self) {
        self.channel_full_count.fetch_add(1, Ordering::Relaxed);
    }
    
    pub fn record_error(&self, msg: impl Into<String>) {
        if let Ok(mut last) = self.last_error.lock() {
            *last = Some((SystemTime::now(), msg.into()));
        }
    }
    
    pub fn snapshot(&self) -> WorkerStats {
        let count = self.poll_count.load(Ordering::Relaxed);
        let total = self.poll_total_us.load(Ordering::Relaxed);
        WorkerStats {
            poll_count: count,
            avg_us: if count > 0 { total / count } else { 0 },
            max_us: self.poll_max_us.load(Ordering::Relaxed),
            channel_full: self.channel_full_count.load(Ordering::Relaxed),
            last_error: self.last_error.lock()
                .ok()
                .and_then(|g| g.as_ref().map(|(t, m)| (*t, m.clone()))),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkerStats {
    pub poll_count: u64,
    pub avg_us: u64,
    pub max_us: u64,
    pub channel_full: u64,
    pub last_error: Option<(SystemTime, String)>,
}

impl WorkerStats {
    pub fn health_badge(&self) -> &'static str {
        if self.channel_full > 10 { return "⚠"; }
        if self.max_us > 500_000 { return "⚠"; }  // > 500ms 单次
        if self.last_error.is_some() { return "⚠"; }
        "✓"
    }
}
```

**改 `src/worker.rs::SnapshotWorker<T>`**：

加 `metrics: Arc<WorkerMetrics>` 字段。`run_poll_loop` 内：
```rust
let t0 = Instant::now();
let snapshot = (self.collect_fn)();
self.metrics.record_poll(t0.elapsed());

match self.cmd_tx.try_send(...) {
    Ok(_) => {}
    Err(TrySendError::Full(_)) => {
        self.metrics.record_channel_full();
        tracing::warn!(target: "metrics", "channel full, dropping frame");
    }
    Err(e) => {
        self.metrics.record_error(format!("{e:?}"));
    }
}
```

**改 `App`**：聚合所有 worker metrics

```rust
impl App {
    pub fn worker_metrics(&self) -> Vec<(&'static str, WorkerStats)> {
        vec![
            ("light", self.snapshot_worker.metrics()),
            ("heavy", self.snapshot_worker.heavy_metrics()),
            ("dns_log", self.dns_log_worker.as_ref().map(|w| w.metrics())),
            ("net_flow", self.net_flow_worker.as_ref().map(|w| w.metrics())),
            ("smart", self.smart_worker.metrics()),
            ("port", self.port_worker.metrics()),
        ].into_iter()
        .filter_map(|(n, opt)| opt.map(|s| (n, s)))
        .collect()
    }
}
```

**改 `src/tui/help_panel.rs`**：在 `?` 帮助页加 "Workers" 区段

```
Workers (avg/max/polls/drops):
  light    ✓ avg=2ms  max=15ms  polls=12345  drops=0
  heavy    ✓ avg=45ms max=80ms  polls=1234   drops=0
  net_flow ⚠ avg=8ms  max=22ms  polls=600    drops=2
  dns_log  ✓ avg=1ms  max=5ms   polls=1200   drops=0
  smart    ✓ avg=120ms max=300ms polls=20    drops=0
```

**新 CLI `proc diag`**：在 `src/cli.rs::Command` 加 `Diag` 变体，`src/main.rs::run_diag` 输出 JSON。

实际归位阶段 6（CLI 拆分）后是 `src/cli/diag.rs`。

阶段 3 先简单放到 `src/main.rs` 末尾即可，阶段 6 搬迁。

```rust
// src/cli.rs
#[derive(clap::Args, Debug)]
pub struct DiagArgs {
    /// 输出 JSON（默认 human-readable）
    #[arg(long)]
    pub json: bool,
}

// src/main.rs
fn run_diag(json: bool) {
    let app = proc::App::new();
    let metrics = app.worker_metrics();
    if json {
        let json = serde_json::to_string_pretty(&metrics).unwrap();
        println!("{}", json);
    } else {
        println!("Worker diagnostics:\n");
        for (name, stats) in &metrics {
            println!("  {:10} {} avg={}μs max={}μs polls={} drops={}",
                name, stats.health_badge(), stats.avg_us, stats.max_us,
                stats.poll_count, stats.channel_full);
        }
    }
}
```

注：`App::new()` 在 CLI 模式下不启动 TUI，只启动 worker 拿一次 metrics。可能需要拆出一个 `App::workers_only()` 构造方法不启 TUI。

**测试**：`tests/test_worker_metrics.rs`（新）：
- `WorkerMetrics::record_poll` 多次后 snapshot 正确
- 并发 record_poll 不死锁（crossbeam 10 线程并发）
- channel_full + last_error 字段同步更新

---

### 任务 4：更新 CHANGELOG + CONTEXT.md

CHANGELOG.md Unreleased 段追加阶段 3 内容。

CONTEXT.md：在「当前术语」段补 `WorkerStats::health_badge` / `WorkerCrash` / `crashes/` 路径等。

### 验收命令

```bash
cargo test --release -q    # 阶段 2 完工后 ~641 → 阶段 3 新增 ~25 → ~666
cargo clippy --release --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build --release --no-default-features

# 特殊验证：
# 1. 启动 proc → 退出 → 重启 → proc.YYYY-MM-DD.log 有上次启动的内容（追加不覆盖）
# 2. 故意制造 panic（debug 加 panic!("test")）→ crashes/crash-{ts}.txt 文件存在
# 3. 启动后按 ? 看 Workers 区段显示
# 4. proc diag → 输出 metrics
```

**验收标准**：
- 全量回归通过（~666）
- clippy / fmt / no-default-features 编译通过
- CHANGELOG + CONTEXT.md 更新
- 5 个新模块（`src/metrics/{mod.rs, crash.rs}` + `tests/test_log_rotate.rs / test_crash_report.rs / test_worker_metrics.rs`）入仓
- 真实环境验证：日志 rotate + crash report + worker metrics

**主修改区域**：
- `Cargo.toml`（加 tracing-appender）
- `src/main.rs`（init_tracing 改造 + panic hook + main 持 _log_guard）
- `src/metrics/{mod.rs(新), crash.rs(新)}`
- `src/worker.rs`（加 metrics + catch_unwind + WorkerCrash）
- `src/app.rs`（加 crash_rx / active_crashes / worker_metrics 聚合）
- `src/cli.rs`（加 Diag 变体）
- `src/tui/{help_panel.rs, mod.rs}`（Workers 区段 + crash banner）
- `tests/test_log_rotate.rs(新)` + `tests/test_crash_report.rs(新)` + `tests/test_worker_metrics.rs(新)`
- `CHANGELOG.md` / `CONTEXT.md`
