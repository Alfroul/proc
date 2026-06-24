# 阶段 5：架构拆分 Slice — App 上帝对象 + main.rs 1571 行

> **独立会话指令**：阅读 CONTEXT.md 和 docs/stages/stage-5.md，完成所有任务后确认完成
>
> ⚠️ **本阶段容量预警**：预估 ~1400 行（接近 1500 上限）。如上下文消耗 > 600K 必须触发 Checkpoint，按 plan.md Checkpoint 流程执行。

**目标**：把 1857 行的 `App` 上帝对象拆为「App + 3 个 Controller」；把 1571 行的 `main.rs` 拆为「main + 12 个 CLI 子模块」。

**前置依赖**：阶段 4 已完成（ProcessInfo 字段已稳定，避免拆分时和字段变更冲突）。

**依赖测试**（开工时跑这些测试的详情）：
- `cargo test --release --tb=no -q`（全量回归 summary，应 ~681）
- `cargo test --release test_process_info_arc test_search_perf test_search_correctness --tb=no -q`（阶段 4 新增性能测试详情）
- `cargo test --release test_inspector --tb=no -q`（既有 inspector 测试，拆分后必须不破坏）

**预期代码量**：~1400 行（搬迁为主，新代码主要是 Controller 的封装 boilerplate）

**拆分原则**（来自 CONTEXT.md surgical 原则）：
- 纯搬迁，不改业务逻辑
- 字段名 / 函数名保持不变（避免大规模改名）
- 测试不动（除少数 import 路径调整）
- 拆完后 0.5.0 / v0.6.0 阶段 1-4 的功能全部回归通过

**任务清单**：

### 任务 1：InspectorController（最大块，优先做）

**新模块**：`src/inspect/controller.rs`

```rust
//! v0.6.0 阶段 5: 详情页状态 + 数据加载逻辑封装。
//! 从 App 上帝对象拆出。见 CONTEXT.md。

use crate::inspect::{EnvVar, HandleInfo, MemoryRegion, InspectionData};
use crate::inspect::env_mask;
use crate::process_control::{PriorityClass, get_priority, get_affinity};
use crate::search::SearchState;
use crate::tui::detail_view::InspectionTab;
use crate::error::Result;

pub struct InspectorController {
    pub target_pid: Option<u32>,
    pub target_start_time: u64,
    pub tab: InspectionTab,
    pub data: Option<InspectionData>,
    pub handles_data: Option<Vec<HandleInfo>>,
    pub memory_data: Option<Vec<MemoryRegion>>,
    pub search: SearchState,
    pub scroll: usize,
    
    // 缓存（避免每帧 syscall）
    pub priority_cache: Option<PriorityClass>,
    pub affinity_cache: Option<u64>,
    
    // v0.6.0 阶段 2 新增字段
    pub env_reveal: bool,
    
    pub dirty: bool,   // r 触发，下一 tick 重采集
}

impl InspectorController {
    pub fn new() -> Self {
        Self {
            target_pid: None,
            target_start_time: 0,
            tab: InspectionTab::Summary,
            data: None,
            handles_data: None,
            memory_data: None,
            search: SearchState::default(),
            scroll: 0,
            priority_cache: None,
            affinity_cache: None,
            env_reveal: false,
            dirty: false,
        }
    }
    
    pub fn open(&mut self, pid: u32, start_time: u64) {
        self.target_pid = Some(pid);
        self.target_start_time = start_time;
        self.tab = InspectionTab::Summary;
        self.search = SearchState::default();
        self.scroll = 0;
        self.priority_cache = None;
        self.affinity_cache = None;
        self.dirty = true;   // 触发首采
    }
    
    pub fn close(&mut self) {
        self.target_pid = None;
        self.data = None;
        self.handles_data = None;
        self.memory_data = None;
    }
    
    pub fn is_open(&self) -> bool { self.target_pid.is_some() }
    
    /// 同步采集（r 触发 / open 触发）
    pub fn refresh(&mut self) -> Result<()> {
        let pid = self.target_pid.ok_or_else(|| crate::error::ProcError::internal("inspector not open"))?;
        self.data = Some(crate::inspect::inspect(pid)?);
        self.handles_data = Some(crate::inspect::collect_handles(pid).unwrap_or_default());
        self.memory_data = Some(crate::inspect::collect_memory(pid).unwrap_or_default());
        self.priority_cache = get_priority(pid).ok();
        self.affinity_cache = get_affinity(pid).ok();
        self.dirty = false;
        Ok(())
    }
    
    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent, recording: bool) -> InspectorAction {
        use crossterm::event::KeyCode;
        
        // 搜索 active 优先
        if self.search.active {
            match key.code {
                KeyCode::Esc => { self.search = SearchState::default(); return InspectorAction::Consumed; }
                KeyCode::Tab => return InspectorAction::Consumed,  // 不让 Tab 切走
                _ => { /* 输入字符到 search */ return InspectorAction::Consumed; }
            }
        }
        
        match key.code {
            KeyCode::Tab => { self.tab = self.tab.next(); self.scroll = 0; }
            KeyCode::BackTab => { self.tab = self.tab.prev(); self.scroll = 0; }
            KeyCode::Esc => return InspectorAction::Close,
            KeyCode::Char('r') => self.dirty = true,    // 阶段 6 会改 F5
            KeyCode::Char('v') => {
                // v0.6.0 阶段 2: env reveal toggle
                if recording {
                    return InspectorAction::StatusMsg("录屏中禁止 reveal env secret".into());
                }
                self.env_reveal = !self.env_reveal;
                return InspectorAction::StatusMsg(
                    if self.env_reveal { "Env: 显示真值".into() }
                    else { "Env: 已 mask secret".into() }
                );
            }
            KeyCode::Char('+') | KeyCode::('=') => return InspectorAction::BumpPriority(true),
            KeyCode::Char('-') => return InspectorAction::BumpPriority(false),
            // ... 其他既有详情页键位（保持原 handle_detail_key 行为）
            _ => {}
        }
        InspectorAction::Consumed
    }
    
    /// draw_env_tab 用：reveal 计算考虑录屏
    pub fn env_render_reveal(&self, recording: bool) -> bool {
        self.env_reveal && !recording
    }
}

#[derive(Debug)]
pub enum InspectorAction {
    Consumed,
    Close,
    StatusMsg(String),
    BumpPriority(bool),   // true=up, false=down
}
```

**搬迁过程**：

1. `App::handle_detail_key` 内的所有逻辑 → `InspectorController::handle_key`
2. `App::switch_mode(ProcessDetail)` 内的 inspection 加载逻辑 → `InspectorController::open` + `refresh`
3. `App` 14 个 inspection_* 字段 → `App::inspector: InspectorController`
4. `App::detail_priority` / `detail_affinity` 删除（迁到 controller）
5. `App::env_reveal` 删除（迁到 controller）
6. `App::inspection_search` / `inspection_scroll` 删除

App 中保留访问器：
```rust
impl App {
    pub fn inspector(&self) -> &InspectorController { &self.inspector }
    pub fn inspector_mut(&mut self) -> &mut InspectorController { &mut self.inspector }
}
```

---

### 任务 2：ReplayController

**新模块**：`src/replay/controller.rs`

```rust
//! v0.6.0 阶段 5: 录屏回放状态机封装。

use crate::record::reader::Player;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReplaySpeed { Half, Normal, Double, Quad }

impl ReplaySpeed {
    pub fn step(self) -> f32 {
        match self { Self::Half => 0.5, Self::Normal => 1.0, Self::Double => 2.0, Self::Quad => 4.0 }
    }
    pub fn cycle(self) -> Self {
        match self {
            Self::Half => Self::Normal,
            Self::Normal => Self::Double,
            Self::Double => Self::Quad,
            Self::Quad => Self::Half,
        }
    }
}

pub struct ReplayController {
    pub player: Option<Player>,
    pub current_frame_idx: usize,
    pub total_frames: usize,
    pub playing: bool,
    pub speed: ReplaySpeed,
    pub sub_step_accum: f32,   // half 速度下累积 0.5 步长
}

impl ReplayController {
    pub fn new() -> Self { /* ... */ }
    pub fn open(&mut self, path: &Path) -> Result<()> { /* ... */ }
    pub fn tick(&mut self) -> ReplayAction { /* half/normal/double/quad 步进 */ }
    pub fn handle_key(&mut self, key: KeyEvent) -> ReplayAction { /* 原 replay 键位 */ }
    pub fn close(&mut self) { /* ... */ }
}
```

搬迁过程：把 `App::replay_*` 8 字段 + `replay_tick` / `replay_load_current_frame` 全部迁过去。

---

### 任务 3：WorkerManager

**新模块**：`src/workers/manager.rs`

```rust
//! v0.6.0 阶段 5: 所有后台 worker 句柄的统一持有者 + metrics 聚合。

pub struct WorkerManager {
    pub light: SnapshotWorker<LightSnapshot>,
    pub heavy: SnapshotWorker<HeavySnapshot>,
    pub smart: Option<SmartWorker>,
    pub dns_log: Option<DnsLogWorker>,
    pub net_flow: Option<NetFlowWorker>,
    pub port: PortWorker,
}

impl WorkerManager {
    pub fn new() -> Self { /* 启动所有 worker */ }
    
    pub fn metrics(&self) -> Vec<(&'static str, WorkerStats)> {
        vec![
            ("light", self.light.metrics()),
            ("heavy", self.heavy.metrics()),
            ("dns_log", self.dns_log.as_ref().map(|w| w.metrics())),
            ("net_flow", self.net_flow.as_ref().map(|w| w.metrics())),
            ("smart", self.smart.as_ref().map(|w| w.metrics())),
            ("port", self.port.metrics()),
        ].into_iter()
        .filter_map(|(n, opt)| opt.map(|s| (n, s)))
        .collect()
    }
    
    pub fn restart(&mut self, name: &str) -> Result<()> {
        match name {
            "light" => { /* drop + respawn */ }
            "dns_log" => { /* drop + detect_collector + spawn */ }
            // ...
            _ => bail!("unknown worker: {}", name),
        }
    }
}
```

搬迁过程：`App` 中所有 `*_worker` 字段 → `App::workers: WorkerManager`。

---

### 任务 4：App 瘦身

`src/app.rs` 目标从 1857 行 → ~600 行。完成后保留：
- App 结构体（持 `inspector / replay / workers / panels / alert / theme / record / ui_state / ...`）
- App::new / tick / handle_key / draw 调度
- 各 panel 的 PanelContext 组装（不可拆，与 ratatui render 紧耦合）

搬迁工作量统计（估算）：
- `InspectorController` 搬迁：~400 行代码 + ~200 行字段定义 = ~600 行
- `ReplayController` 搬迁：~250 行代码 + ~50 行字段 = ~300 行
- `WorkerManager` 搬迁：~150 行代码 + ~100 行字段 = ~250 行
- 总计：~1150 行从 app.rs 移走

**测试**：保持原有 `tests/test_inspector.rs` / `test_workers.rs` / 集成测试全部不破坏。

---

### 任务 5：main.rs 拆 src/cli/ 子模块

**新模块**：`src/cli/mod.rs` + 12 个子模块

```
src/cli/
├── mod.rs              # re-export run_subcommand
├── ls.rs               # run_ls
├── kill.rs             # run_kill / run_pkill
├── port.rs             # run_port (+ --stats)
├── handles.rs          # run_handles / run_who
├── priority.rs         # run_priority / run_affinity
├── smart.rs            # run_smart
├── dns.rs              # run_dns
├── monitor.rs          # run_monitor
├── docker_cmd.rs       # 11 个 docker 子分发
├── record.rs           # run_record / run_replay
├── export.rs           # run_export
├── eject.rs            # run_eject
└── diag.rs             # run_diag（阶段 3 已新建，本阶段归位）
```

**搬迁规则**（严格遵守 surgical 原则）：
- 每个 `run_*` 函数 + 其私有辅助函数原样搬到对应文件
- 改 `use crate::xxx` 为正确的相对路径
- `src/main.rs` 瘦身到 < 200 行，只保留：
  - `fn main()`（self_mitigation + init_tracing + panic_hook + dispatch）
  - `fn init_tracing()`（阶段 3 已改造）
  - `fn run_subcommand(cmd)`（match dispatch）
  - `fn run_tui()`（启动 TUI）

**搬迁验收**：
- 所有 `proc <subcommand>` 行为完全一致
- 没有任何 `run_*` 函数逻辑变更（diff 应只显示文件移动 + import 调整）

---

### 任务 6：模块挂载到 lib.rs

`src/lib.rs` 加 `pub mod cli;` `pub mod inspect { pub mod controller; }` `pub mod replay { pub mod controller; }` `pub mod workers { pub mod manager; }`。

---

### 任务 7：更新 CHANGELOG + CONTEXT.md

CHANGELOG Unreleased 段追加：
```markdown
### 阶段 5 — 架构拆分（App + main.rs）

- Refactor (#9): `App` 1857 行上帝对象拆分为 `App` + `InspectorController` + `ReplayController` + `WorkerManager` 三个组合。App 瘦身到 ~600 行。
  - 新增: `src/inspect/controller.rs`（详情页状态 + 数据加载 + handle_key）
  - 新增: `src/replay/controller.rs`（录屏回放状态机 + ReplaySpeed 枚举）
  - 新增: `src/workers/manager.rs`（worker 句柄统一持有 + metrics 聚合 + restart）
- Refactor (#10): `src/main.rs` 1571 行平铺拆分为 `src/cli/` 12 个子模块（ls / kill / port / handles / priority / smart / dns / monitor / docker_cmd / record / export / eject）。main.rs 瘦身到 ~200 行。
- Note: 纯搬迁无业务逻辑变更；611 + 阶段 2/3/4 新增测试全部回归通过。
- Docs: CONTRIBUTING.md 补「模块组织」段说明新结构。
```

CONTEXT.md：「当前术语」段的 InspectorController / ReplayController / WorkerManager 标注「阶段 5 已落地」。

---

### 验收命令

```bash
cargo test --release --tb=no -q    # 阶段 4 完工后 ~681 → 阶段 5 新增 ~10 → ~691
cargo clippy --release --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build --release --no-default-features

# 拆分行为一致性验证：
# 1. 手动跑全部 CLI 子命令对比拆分前后输出（diff 为空）
#    proc ls --sort cpu --limit 20 > /tmp/after.txt
#    diff /tmp/before.txt /tmp/after.txt   # 应该为空
# 2. TUI 6 面板全部检查快捷键
# 3. 详情页 6 Tab + 录屏回放
```

**验收标准**：
- 全量回归通过（~691）
- clippy / fmt / no-default-features 编译通过
- 6 个新模块（3 个 controller + cli/ 12 个子文件 + mod.rs）入仓
- CLI 行为完全一致（diff 验证）
- TUI 全功能验证
- App 行数从 1857 降到 ~600；main.rs 从 1571 降到 ~200
- CHANGELOG + CONTEXT.md 更新

**主修改区域**：
- `src/inspect/controller.rs(新)` / `src/replay/controller.rs(新)` / `src/workers/manager.rs(新)`
- `src/cli/{mod.rs, ls.rs, kill.rs, port.rs, handles.rs, priority.rs, smart.rs, dns.rs, monitor.rs, docker_cmd.rs, record.rs, export.rs, eject.rs, diag.rs}(新)`
- `src/app.rs`（拆分，瘦身）
- `src/main.rs`（拆分，瘦身）
- `src/lib.rs`（pub mod）
- `CHANGELOG.md` / `CONTEXT.md`

**容量预警**：~1400 行接近上限。如触发 Checkpoint 流程：
1. 优先完成 InspectorController（独立性强，无跨文件依赖）
2. ReplayController + WorkerManager 拆出后立即生成 Checkpoint
3. 下一会话继续做 main.rs 拆分（独立任务，可单独完成）
