# 阶段 4：性能 Slice — ProcessInfo Arc<str> + ProcessStatus 枚举 + rebuild_sorted 优化

> **独立会话指令**：阅读 CONTEXT.md 和 docs/stages/stage-4.md，完成所有任务后确认完成

**目标**：消除 HeavyWorker 每秒数千次堆分配；搜索框逐字符输入消除可感知延迟。

**前置依赖**：阶段 3 已完成。

**依赖测试**（开工时跑这些测试的详情）：
- `cargo test --release -q`（全量回归 summary，应 ~666）
- `cargo test --release test_log_rotate test_crash_report test_worker_metrics -q`（阶段 3 新增可观测性测试详情）

**预期代码量**：~600 行（含测试）

**任务清单**：

### 任务 0：影响范围扫描（必须先做）

`ProcessInfo` 字段类型变更会影响所有构造点。开工时先扫描定位：

```bash
cd D:/terminal/tool/proc
grep -n "ProcessInfo {" src/ tests/ -r | grep -v "target/" > /tmp/processinfo_uses.txt
cat /tmp/processinfo_uses.txt
```

预期 ~14 处构造点（CHANGELOG v0.5.0 阶段 7 D1 已记录）：
- `src/collect.rs` × 2
- `src/eject/locks.rs` × 1
- `src/record/conversions.rs` × 1
- 测试代码 × ~10

逐处标注变更类型：
- A 类（仅改字段名）：`name: String` → `name: p.name().to_string_lossy().into()`
- B 类（pattern matching 解构）：需要适配 `Arc<str>` 不能用 `&str` 直接 match
- C 类（serde round-trip）：派生 Clone 后 `Arc<str>` 序列化等价于 `String`

把扫描结果填到本文件「影响范围扫描」段（开工时回填）。

---

### 任务 1：定义 `ProcessStatus` 枚举（替代 `format!("{:?}")`）

**改 `src/collect.rs`**：

```rust
/// v0.6.0 阶段 4: Copy 枚举替代 String（避免每进程 format! 分配）
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProcessStatus {
    Run,
    Sleep,
    Idle,
    Stop,
    Zombie,
    Traced,
    Dead,
    DeadLock,
    Wakekill,
    Unknown,
}

impl Default for ProcessStatus {
    fn default() -> Self { Self::Unknown }
}

impl From<sysinfo::ProcessStatus> for ProcessStatus {
    fn from(s: sysinfo::ProcessStatus) -> Self {
        match s {
            sysinfo::ProcessStatus::Run => Self::Run,
            sysinfo::ProcessStatus::Sleep => Self::Sleep,
            sysinfo::ProcessStatus::Idle => Self::Idle,
            sysinfo::ProcessStatus::Stop => Self::Stop,
            sysinfo::ProcessStatus::Zombie => Self::Zombie,
            sysinfo::ProcessStatus::Traced => Self::Traced,
            sysinfo::ProcessStatus::Dead => Self::Dead,
            sysinfo::ProcessStatus::Deadlock => Self::DeadLock,
            sysinfo::ProcessStatus::Wakekill => Self::Wakekill,
            _ => Self::Unknown,
        }
    }
}

impl ProcessStatus {
    pub fn badge(self) -> &'static str {
        match self {
            Self::Run => "R",
            Self::Sleep => "S",
            Self::Idle => "I",
            Self::Stop => "T",
            Self::Zombie => "Z",
            Self::Traced => "Tr",
            Self::Dead => "D",
            Self::DeadLock => "Dl",
            Self::Wakekill => "Wk",
            Self::Unknown => "?",
        }
    }
    
    pub fn tooltip(self) -> &'static str {
        match self {
            Self::Run => "Running",
            Self::Sleep => "Sleeping",
            Self::Idle => "Idle",
            Self::Stop => "Stopped",
            Self::Zombie => "Zombie",
            Self::Traced => "Traced",
            Self::Dead => "Dead",
            Self::DeadLock => "Deadlock",
            Self::Wakekill => "Wakekill",
            Self::Unknown => "Unknown",
        }
    }
}
```

注意：`sysinfo::ProcessStatus` 变体名根据 sysinfo 0.34 实际枚举核对（可能 `Deadlock` vs `DeadLock`、`UninterruptibleSleep` 等）。

**测试**：模块内嵌单测覆盖 `From` 映射 + `badge` / `tooltip` 一一对应。

---

### 任务 2：`ProcessInfo` 字段类型升级

**改 `src/collect.rs::ProcessInfo`**：

```rust
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub ppid: Option<u32>,
    pub name: Arc<str>,           // 原 String，v0.6.0 阶段 4 改
    pub cmd: Arc<[String]>,       // 原 Vec<String>，v0.6.0 阶段 4 改
    pub exe: Option<Arc<str>>,    // 原 Option<String>
    pub cwd: Option<Arc<str>>,    // 原 Option<String>
    pub user: Option<Arc<str>>,   // 原 Option<String>
    
    pub status: ProcessStatus,    // 原 String，v0.6.0 阶段 4 改
    
    /// v0.6.0 阶段 4 新增：预计算的 lowercase name，搜索匹配用
    #[serde(skip)]
    pub name_lower: Arc<str>,
    
    pub cpu_usage: f32,
    pub memory_bytes: u64,
    pub start_time: u64,
    pub disk_read: u64,
    pub disk_write: u64,
    pub net_sent_rate: u64,
    pub net_recv_rate: u64,
    pub security_score: Option<u8>,
    // ... 既有其他字段
}
```

注意：`#[serde(skip)] name_lower` 不序列化（重计算生成），减少 `.prec` 录屏文件大小。

**HeavyWorker 构造点改造**：

```rust
// src/collect.rs HeavyWorker::refresh
let name: Arc<str> = Arc::from(p.name().to_string_lossy().to_string().as_str());
let name_lower: Arc<str> = Arc::from(p.name().to_string_lossy().to_lowercase().as_str());
let cmd: Arc<[String]> = Arc::from(p.cmd().iter().map(|s| s.to_string()).collect::<Vec<_>>());
let status: ProcessStatus = p.status().into();

ProcessInfo {
    pid: p.pid().as_u32(),
    name: Arc::clone(&name),
    name_lower: Arc::clone(&name_lower),
    cmd,
    status,
    // ...
}
```

注意：`Arc::from(string.as_str())` 触发一次分配（从 String 转 Arc<str>），但**构造一次后所有读取/clone 都是 atomic increment**，没有堆分配。

**测试**：`tests/test_process_info_arc.rs`（新）：
- `Arc<str>` Clone 是原子计数，不分配堆
- `ProcessInfo::default()` 等价于空字符串字段
- serde round-trip：`name_lower` skip 序列化
- `ProcessStatus::from(sysinfo::ProcessStatus::Run)` == `ProcessStatus::Run`

---

### 任务 3：`rebuild_sorted_cache` 缓存 name_lower + PID idx（项 #14）

**改 `src/view_models/process_panel.rs::SearchState`**：

```rust
#[derive(Clone, Debug, Default)]
pub struct SearchState {
    pub query: String,
    /// v0.6.0 阶段 4 新增：缓存的 lowercase query，避免每字符 to_lowercase
    pub query_lower: String,
    pub active: bool,
}

impl SearchState {
    pub fn set_query(&mut self, q: String) {
        self.query_lower = q.to_lowercase();
        self.query = q;
        self.active = !self.query.is_empty();
    }
    
    pub fn matches(&self, name_lower: &str) -> bool {
        if !self.active { return true; }
        name_lower.contains(&self.query_lower)
    }
}
```

**改 `src/app.rs::rebuild_sorted_cache`**：

```rust
pub fn rebuild_sorted_cache(&mut self) {
    let procs = &self.cached_processes;
    
    // 1. 排序（一次性，不再每按键重建）
    let mut indices: Vec<usize> = (0..procs.len()).collect();
    indices.sort_by(|&a, &b| {
        match self.sort_field {
            SortField::Cpu => procs[b].cpu_usage.partial_cmp(&procs[a].cpu_usage),
            SortField::Mem => procs[b].memory_bytes.cmp(&procs[a].memory_bytes),
            SortField::Name => procs[a].name.cmp(&procs[b].name),
            // ...
        }.unwrap_or(std::cmp::Ordering::Equal)
    });
    
    // 2. 搜索过滤（直接用 name_lower，无 to_lowercase）
    let search = &self.process_panel.search;
    let filtered: Vec<usize> = if search.active {
        indices.into_iter()
            .filter(|&i| search.matches(&procs[i].name_lower))
            .collect()
    } else {
        indices
    };
    
    self.cached_sorted = filtered;
}
```

**关键优化**：
- 每按键不再重建 `HashMap<Pid, usize>`
- 每按键不再 `to_string_lossy().to_lowercase()` 每进程
- 搜索 query 的 to_lowercase 也只算一次（在 `SearchState::set_query` 时）

**测试**：
- `tests/test_search_perf.rs`（新）：500 进程下 `rebuild_sorted_cache` 100 次调用平均耗时 < 100µs（基线 1ms，10x 提升）
- `tests/test_search_correctness.rs`（新）：大小写混合 query 匹配正确

---

### 任务 4：同步既有调用点

扫描所有 `process.name() == "xxx"` 或 `process.name().contains("xxx")` 模式，确认改成 `&process.name`（Arc<str> deref 到 &str）能正确工作。

```bash
grep -n "\.name()" src/ -r | grep -v "name: " | grep -v "name_lower"
# 这些是访问 name 字段的点，看是否有 .to_string() / .clone() 调用
```

**典型适配**：
```rust
// 旧（String）
let name: &str = &process_info.name;
println!("{}", process_info.name);

// 新（Arc<str>）—— 行为完全相同，Deref 自动生效
let name: &str = &process_info.name;
println!("{}", process_info.name);

// 旧（要传 owned）
fn foo(name: String) { ... }
foo(process_info.name.clone());

// 新（Arc::clone 不分配堆）
fn foo(name: Arc<str>) { ... }
foo(Arc::clone(&process_info.name));
// 或者改成传 &str
fn foo(name: &str) { ... }
foo(&process_info.name);
```

**`record/conversions.rs::FrameProcess`** 适配：FrameProcess 用 String（序列化兼容），转换时 `Arc::as_ref().to_string()`。

---

### 任务 5：更新 CHANGELOG + CONTEXT.md

CHANGELOG Unreleased 段追加：
```markdown
### 阶段 4 — ProcessInfo 性能优化

- Changed (#11): `ProcessInfo` 字段类型升级 — `name: String → Arc<str>` / `cmd: Vec<String> → Arc<[String]>` / `exe / cwd / user: Option<String> → Option<Arc<str>>`。消除 HeavyWorker 每秒数千次堆分配（500 进程 × 1.5s 重采）。
- Added (#11): `ProcessStatus` Copy 枚举替代 `format!("{:?}", sysinfo::ProcessStatus)` String 分配；含 10 个变体 + `badge()` / `tooltip()` 方法。
- Added (#14): `ProcessInfo::name_lower: Arc<str>` 预计算字段（heavy worker 一次性算好，serde skip 不序列化）；`SearchState::query_lower` 缓存；`rebuild_sorted_cache` 不再每按键重建 HashMap + lowercase Vec。
- Performance: 500 进程基准下搜索框逐字符输入延迟从 ~1ms 降到 ~100µs（10x 提升）；heavy refresh 平均耗时下降 ~30%（预期）。
- Note: serde round-trip 测试覆盖；`#[serde(skip)] name_lower` 减少录屏文件大小。
```

CONTEXT.md：术语演进历史段已经预填 ProcessStatus / name_lower / query_lower 条目（本阶段实施时确认 propagate 完成）。

---

### 验收命令

```bash
cargo test --release -q    # 阶段 3 完工后 ~666 → 阶段 4 新增 ~15 → ~681
cargo clippy --release --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build --release --no-default-features

# 特殊验证：
# 1. 启动 proc 500+ 进程 → 搜索框逐字符输入无卡顿
# 2. criterion benchmark（阶段 6 引入后才能跑，本阶段先记录基线数字）
#    单测内置 Instant 测量: heavy refresh 耗时前后对比
```

**验收标准**：
- 全量回归通过（~681）
- clippy / fmt / no-default-features 编译通过
- `ProcessInfo` 字段升级不破坏 serde round-trip（既有 .prec 录屏文件可重放）
- 搜索性能提升（单测 builtin 计时验证）
- CHANGELOG + CONTEXT.md 更新

**主修改区域**：
- `src/collect.rs`（ProcessInfo 字段 + HeavyWorker 构造点 + ProcessStatus 枚举）
- `src/app.rs`（rebuild_sorted_cache 重写）
- `src/view_models/process_panel.rs`（SearchState）
- `src/record/conversions.rs`（Arc → String 转换）
- `src/eject/locks.rs` / `src/format.rs` / `src/classify.rs` / 其他 ProcessInfo 访问点（适配 &str deref）
- `tests/test_process_info_arc.rs(新)` + `tests/test_search_perf.rs(新)` + `tests/test_search_correctness.rs(新)`
- `CHANGELOG.md` / `CONTEXT.md`

**容量预警**：本阶段代码量 ~600 行，但**影响范围大**（14+ 构造点），如编译错误堆积触发 Checkpoint。
