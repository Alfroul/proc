# 阶段 6：UX 打磨 + 测试增强 Slice

> **独立会话指令**：阅读 CONTEXT.md 和 docs/stages/stage-6.md，完成所有任务后确认完成

**目标**：键位 'r' / 'c' 去歧义（#12/#13）；引入 proptest + criterion + Linux stub 测试。

**前置依赖**：阶段 5 已完成（Controller 已抽出，键位改在 controller 内更容易）。

**依赖测试**（开工时跑这些测试的详情）：
- `cargo test --release --tb=no -q`（全量回归 summary，应 ~691）
- `cargo test --release test_inspector test_workers --tb=no -q`（阶段 5 拆分后必须不破坏的测试）
- 在 Linux CI runner 上跑 `cargo test --release test_inspect_stub test_eject_stub test_gpu_stub`（阶段 6 新增 Linux stub 测试）

**预期代码量**：~800 行（含测试）

**任务清单**：

### 任务 1：'r' 三语义去歧义（项 #12）

**改 `src/inspect/controller.rs::handle_key`**（阶段 5 已搬迁）：

```rust
// 原: KeyCode::Char('r') => self.dirty = true,
// 新:
KeyCode::F(5) => {  // F5 刷新（替代 'r'）
    self.dirty = true;
    return InspectorAction::StatusMsg("刷新中...".into());
}
// 保留 'r' 但显示 deprecation warning（v0.6.0 兼容期）
KeyCode::Char('r') => {
    self.dirty = true;
    return InspectorAction::StatusMsg("⚠ 'r' 将在 v0.7.0 移除，请用 F5".into());
}
```

**改 `src/view_models/docker_panel.rs::handle_key`**：

```rust
// 原: KeyCode::Char('r') => self.restart_container(...),
// 新: Shift+R 重启
KeyCode::Char('R') => {  // 大写 R = Shift+r
    if let Some(id) = self.selected_container_id() {
        return self.handle_restart(ctx, id);
    }
}
// 保留小写 'r' 显示 deprecation warning
KeyCode::Char('r') => {
    return PanelAction::StatusMsg("⚠ docker 'r' 将在 v0.7.0 改为 Shift+R".into());
}
```

**改 `src/tui/help_panel.rs`**：
- 详情页段：`r 刷新` → `F5 刷新详情`
- Docker 段：`r 重启` → `Shift+R 重启容器`
- USB 段：`r 刷新设备`（保留不变）

**测试**：在 `tests/test_inspector.rs` 加 case：
- 详情页按 F5 → `dirty = true`
- 详情页按 'r' → `dirty = true` + status_message 含 deprecation
- Docker 面板按 Shift+R → restart 调用
- Docker 面板按 'r' → status_message 含 deprecation

---

### 任务 2：'c' 双语义去歧义（项 #13）

**改 `src/inspect/controller.rs::handle_key`**：

```rust
// 原: KeyCode::Char('c') => copy_process_info(),
// 新: 'y' yank（vim 风格）
KeyCode::Char('y') => {
    return InspectorAction::CopyToClipboard;
}
// 保留 'c' 显示 deprecation warning
KeyCode::Char('c') => {
    return InspectorAction::StatusMsg("⚠ 详情页 'c' 将在 v0.7.0 改为 'y'".into());
}
```

**InspectorAction 加变体**：
```rust
pub enum InspectorAction {
    // ...
    CopyToClipboard,
}
```

**App 层处理**：
```rust
match self.inspector.handle_key(key, self.recording) {
    InspectorAction::CopyToClipboard => {
        let _ = arboard::Clipboard::new().and_then(|mut c| {
            c.set_text(self.format_process_info_for_clipboard())
        });
        self.status_message = "已复制到剪贴板".into();
    }
    // ...
}
```

**改 `src/tui/help_panel.rs`**：
- 详情页段：`c 复制` → `y 复制进程信息（vim yank）`
- 全局段：`c 切换侧边栏折叠/展开`（保留不变，不再冲突）

**改 `src/tui/detail_view.rs::draw_summary`**：底部快捷键提示 `y 复制`。

**测试**：详情页按 'y' 后剪贴板有内容（mock `arboard::Clipboard`）。

---

### 任务 3：proptest 引入（项 #18）

**Cargo.toml**：
```toml
[dev-dependencies]
tempfile = "3"
proptest = "1"
criterion = { version = "0.5", features = ["html_reports"] }

[[bench]]
name = "sort_cache"
harness = false

[[bench]]
name = "refresh_heavy"
harness = false
```

**新文件**：`tests/proptest_vt100.rs`

```rust
//! VT100 parser 不 panic on arbitrary bytes
//! 见 docs/stages/stage-6.md 阶段 6 任务 3

use proptest::prelude::*;

proptest! {
    #[test]
    fn vt100_parser_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..10000)) {
        let mut parser = vt100::Parser::new(80, 24, 0);
        parser.process(&bytes);
        // 必须 not panic
    }
    
    #[test]
    fn vt100_parser_handles_resize_after_feed(
        bytes in prop::collection::vec(any::<u8>(), 0..1000),
        new_cols in 1u16..200,
        new_rows in 1u16..100,
    ) {
        let mut parser = vt100::Parser::new(80, 24, 0);
        parser.process(&bytes);
        parser.set_size(new_cols, new_rows);
    }
}
```

**新文件**：`tests/proptest_parsers.rs`

```rust
//! 所有外部字节流解析器不 panic

use proptest::prelude::*;
use proc::port_map::{parse_proc_net_snmp_tcp, TcpState};
use proc::dns_log::{parse_query_type, parse_query_results};

proptest! {
    #[test]
    fn parse_query_type_resilient(s in ".{0,50}") {
        let _ = parse_query_type(&s);
    }
    
    #[test]
    fn parse_query_results_resilient(s in ".{0,500}") {
        let _ = parse_query_results(&s);
    }
    
    #[test]
    fn tcp_state_from_any_str(hex in "[0-9A-Fa-f]{1,8}") {
        let _ = TcpState::from_state_str(&hex);
    }
    
    #[test]
    fn snmp_parser_resilient(content in ".{0,5000}") {
        let _ = parse_proc_net_snmp_tcp(&content);
    }
    
    #[test]
    fn maps_parser_resilient(content in ".{0,5000}") {
        let _ = proc::inspect::memory::parse_proc_maps(&content);
    }
}
```

**注意**：proptest 默认 256 case，CI 跑可能 ~30s。

---

### 任务 4：criterion benchmark（项 #19）

**新文件**：`benches/sort_cache.rs`

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use proc::collect::{ProcessInfo, SortField, sort_processes};

fn mock_process(i: usize) -> ProcessInfo {
    use std::sync::Arc;
    ProcessInfo {
        pid: i as u32,
        ppid: Some((i / 2) as u32),
        name: Arc::from(format!("proc_{}", i % 50).as_str()),
        cmd: Arc::from(vec![format!("/bin/proc_{}", i)]),
        // ... 其他字段 default
        cpu_usage: (i as f32) * 0.1,
        memory_bytes: (i as u64) * 1024,
        ..Default::default()
    }
}

fn bench_sort_500(c: &mut Criterion) {
    let procs: Vec<ProcessInfo> = (0..500).map(mock_process).collect();
    
    c.bench_function("sort 500 procs by cpu", |b| {
        b.iter(|| {
            let mut cloned = black_box(procs.clone());
            sort_processes(&mut cloned, SortField::Cpu);
        });
    });
    
    c.bench_function("sort 500 procs by memory", |b| {
        b.iter(|| {
            let mut cloned = black_box(procs.clone());
            sort_processes(&mut cloned, SortField::Mem);
        });
    });
    
    c.bench_function("sort 500 procs by name", |b| {
        b.iter(|| {
            let mut cloned = black_box(procs.clone());
            sort_processes(&mut cloned, SortField::Name);
        });
    });
}

criterion_group!(benches, bench_sort_500);
criterion_main!(benches);
```

**新文件**：`benches/refresh_heavy.rs`

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use proc::collect::HeavySnapshot;

fn bench_refresh_heavy(c: &mut Criterion) {
    c.bench_function("heavy refresh (single shot)", |b| {
        b.iter(|| {
            let snap = HeavySnapshot::refresh_once();
            black_box(snap);
        });
    });
}

criterion_group!(benches, bench_refresh_heavy);
criterion_main!(benches);
```

**新文件**：`.github/workflows/bench.yml`

```yaml
name: bench
on:
  pull_request:
    branches: [master]
  workflow_dispatch:

jobs:
  bench:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
        with: { key: bench }
      - name: Install criterion
        run: |
          cargo install critcmp --locked || true
      - name: Save baseline (master)
        if: github.event_name == 'pull_request'
        run: |
          git checkout origin/master
          cargo bench --bench sort_cache -- --save-baseline master
          git checkout -
      - name: Run PR bench
        if: github.event_name == 'pull_request'
        run: |
          cargo bench --bench sort_cache -- --baseline master 2>&1 | tee bench-output.txt
      - name: Check regression (>20%)
        if: github.event_name == 'pull_request'
        run: |
          # criterion 输出中找 "regressed" 标记
          if grep -q "regressed" bench-output.txt; then
            echo "⚠ Performance regression detected"
            # 不阻断 PR，只警告（实际项目可改 exit 1）
            cat bench-output.txt
          fi
```

---

### 任务 5：Linux stub 测试（项 #20）

**新文件**：`tests/test_inspect_stub.rs`

```rust
//! v0.6.0 阶段 6: Windows-only 模块的 Linux stub 测试
//! 见 docs/stages/stage-6.md 任务 5

#[cfg(not(target_os = "windows"))]
mod stub_tests {
    use proc::inspect;
    use proc::error::ProcError;
    
    #[test]
    fn env_returns_permission_denied_on_linux() {
        let pid = std::process::id();
        let result = inspect::collect_env(pid);
        assert!(matches!(result, Err(ProcError::PermissionDenied { .. }) | Err(ProcError::Unsupported { .. })),
            "Linux stub should return PermissionDenied or Unsupported, got: {:?}", result);
    }
    
    #[test]
    fn handles_returns_permission_denied_on_linux() {
        let pid = std::process::id();
        let result = inspect::collect_handles(pid);
        // Linux 实际上 /proc/self/fd 能读，所以可能返回 Ok 或 Err，关键是 not panic
        let _ = result;
    }
    
    #[test]
    fn memory_returns_permission_denied_on_linux() {
        let pid = std::process::id();
        let result = inspect::collect_memory(pid);
        // Linux /proc/self/maps 可读，可能返回 Ok；not panic 即可
        let _ = result;
    }
}

#[cfg(target_os = "windows")]
mod windows_tests {
    use proc::inspect;
    
    #[test]
    fn env_succeeds_for_self_on_windows() {
        let pid = std::process::id();
        let result = inspect::collect_env(pid);
        assert!(result.is_ok(), "self env should be readable: {:?}", result);
        let env = result.unwrap();
        // 应该至少有 PATH / SYSTEMROOT 之一
        assert!(env.iter().any(|v| v.key.to_uppercase() == "PATH") ||
                env.iter().any(|v| v.key.to_uppercase() == "SYSTEMROOT"),
            "expected PATH or SYSTEMROOT in env");
    }
    
    #[test]
    fn handles_succeeds_for_self_on_windows() {
        let pid = std::process::id();
        let result = inspect::collect_handles(pid);
        // 自身句柄数应该 > 0
        assert!(result.map(|h| !h.is_empty()).unwrap_or(false),
            "self should have handles: {:?}", result);
    }
}
```

**新文件**：`tests/test_eject_stub.rs`

```rust
#[cfg(not(target_os = "windows"))]
mod stub_tests {
    use proc::eject;
    
    #[test]
    fn list_devices_returns_empty_on_linux() {
        // Linux 不支持 USB 占用检测，应该返回空 Vec 或 Err
        let result = eject::list_removable_devices();
        assert!(result.is_empty() || result.is_err());
    }
    
    #[test]
    fn find_lockers_returns_empty_on_linux() {
        let result = eject::find_lockers("/tmp");
        // Linux stub 应该返回空 Vec
        assert!(result.map(|l| l.is_empty()).unwrap_or(true));
    }
}

#[cfg(target_os = "windows")]
mod windows_tests {
    use proc::eject;
    
    #[test]
    fn list_devices_does_not_panic() {
        let _ = eject::list_removable_devices();
    }
}
```

**新文件**：`tests/test_gpu_stub.rs`

```rust
use proc::gpu::{detect_providers, GpuProvider};

#[test]
fn detect_providers_does_not_panic() {
    let providers = detect_providers();
    // 至少返回 Vec（可能为空，比如 macOS）
    assert!(providers.iter().all(|p| !p.provider_name().is_empty()));
}

#[test]
fn no_provider_panics_on_refresh() {
    let providers = detect_providers();
    for mut p in providers {
        p.refresh();   // 必须 not panic
        let _ = p.list_gpus();
    }
}

#[cfg(not(target_os = "windows"))]
#[test]
fn nvml_provider_not_constructed_on_linux() {
    // detect_providers 在 Linux 不应该返回 NvmlProvider
    let providers = detect_providers();
    for p in &providers {
        assert_ne!(p.provider_name(), "NvmlProvider", 
            "NvmlProvider should not be constructed on Linux");
    }
}
```

---

### 任务 6：更新 CHANGELOG + CONTEXT.md

CHANGELOG Unreleased 段追加：
```markdown
### 阶段 6 — UX 打磨 + 测试增强

- Changed (#12): 'r' 三语义去歧义 — 详情页改为 F5（保留 'r' v0.7.0 移除）；Docker 改为 Shift+R；USB 保留 'r'。
- Changed (#13): 'c' 双语义去歧义 — 详情页复制改为 'y'（vim yank 风格，保留 'c' v0.7.0 移除）；全局 'c' 保留侧边栏折叠。
- Added (#18): proptest 引入 — `tests/proptest_vt100.rs` + `tests/proptest_parsers.rs` 覆盖 VT100 parser / parse_query_type / from_state_str / parse_proc_net_snmp_tcp / parse_proc_maps 5 个外部字节流入口。
- Added (#19): criterion benchmark — `benches/sort_cache.rs` + `benches/refresh_heavy.rs`；`.github/workflows/bench.yml` 自动回归比对（>20% 警告）。
- Added (#20): Windows-only 模块的 Linux stub 测试 — `tests/test_inspect_stub.rs` + `tests/test_eject_stub.rs` + `tests/test_gpu_stub.rs` 覆盖 collect_env/handles/memory + eject + gpu 在 Linux/macOS 的降级路径。
- Docs: help_panel.rs 同步更新快捷键表；CHANGELOG 记录 deprecation 警告。
```

CONTEXT.md：「术语演进历史」段 r/c 键位变更条目标「阶段 6 已落地」。

---

### 验收命令

```bash
cargo test --release --tb=no -q    # 阶段 5 完工后 ~691 → 阶段 6 新增 ~50 → ~741
cargo clippy --release --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build --release --no-default-features

# 阶段 6 特殊验证：
# 1. proptest: cargo test --release --test proptest_vt100 --test proptest_parsers
#    默认每个 proptest 跑 256 case
# 2. criterion: cargo bench --bench sort_cache
#    生成 target/criterion/reports/sort_cache/index.html
# 3. Linux stub: 在 ubuntu-latest CI runner 上跑（已有 check-linux job）
# 4. 键位: TUI 手动验证 F5 刷新 / Shift+R docker restart / y 详情页复制
```

**验收标准**：
- 全量回归通过（~741）
- clippy / fmt / no-default-features 编译通过
- proptest 全部通过（不 panic）
- criterion benchmark 报告生成
- Linux CI 上 stub 测试全绿
- TUI 手动验证：F5 / Shift+R / 'y' 全部工作
- CHANGELOG + CONTEXT.md 更新

**主修改区域**：
- `src/inspect/controller.rs`（F5 / 'y' 键位）
- `src/view_models/docker_panel.rs`（Shift+R）
- `src/tui/help_panel.rs`（同步快捷键表）
- `src/tui/detail_view.rs`（'y 复制' 提示）
- `Cargo.toml`（加 proptest / criterion dev-deps + bench 段）
- `benches/{sort_cache.rs, refresh_heavy.rs}(新)`
- `tests/{proptest_vt100.rs, proptest_parsers.rs, test_inspect_stub.rs, test_eject_stub.rs, test_gpu_stub.rs}(新)`
- `.github/workflows/bench.yml(新)`
- `CHANGELOG.md` / `CONTEXT.md`
