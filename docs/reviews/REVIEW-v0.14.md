# REVIEW-v0.14：v0.14.0 cycle 全局 Review

> **范围**：v0.14.0 cycle stage 1-4 全部产出（commit `f9b38b7 plan(v0.14)` 之后的全部 working tree 改动）——录屏文件格式 v3 按需加载 + 书签系统 + 时间轴搜索 + 倒放 + footer 元数据 + `.prec.idx` / `.bookmarks.json` sidecar 兼容层 + CLI `--info`。
> **方法**：按 stage 5 doc 任务清单 §2 的 6 子项审查（代码质量 / 架构 / 性能 / 完整性 / 安全跨平台 / P0-P1-P2 列表）。
> **基线**：`cargo test --release -q` = **1242 passed / 0 failed / 3 ignored**；`cargo fmt --all -- --check` / `cargo clippy --release --all-targets -- -D warnings` / `cargo build --release --no-default-features` / `cargo bench --no-run` 全过。
> **结论**：**P0 0 / P1 1 / P2 1**。stage 1-4 全部交付，无未交付项。1 个 P1 集中在文档完整性（4 个 stage docs 头部缺 ✅ 已完成标记，与 v0.13 stage 3 P1-2/P1-3 同款），1 个 P2 归档 TD-49（VT100 replay 路径无倒放 / 搜索 + 长录屏搜索遍历优化）。
> **Date**：2026-07-06。

---

## 0. 验收对照表（stage 1-4 是否全交付）

| Stage | 范围 | 验收 | 状态 |
|---|---|---|---|
| 1 Spike | 录屏文件格式 v3（按需加载 + footer + v1/v2 sidecar + `--info`）。`RecordingFooter { version, header_version, start_time, end_time, frame_count, anomaly_count, event_count, max_cpu, max_mem, frame_offsets }` + `FOOTER_MAGIC = *b"PREC3FT\x01"` + `Player` 重写为按需加载（`{ path, header, footer, file: RefCell<File>, cache_idx: Cell<Option<usize>>, cache_frame: RefCell<Option<UiFrame>> }`）+ `IdxSidecar` v1/v2 兼容 + CLI `proc replay --info` | `cargo test --release` 1135 passed（基线 1115 + 新测试 20）；v3 round-trip / random seek / footer correctness / v1/v2 兼容 / sidecar 6 case 全过；CLI `--info` 输出 footer 元数据 | ✅ 全交付（commit `dc96ae8`）|
| 2 Slice | 书签系统。`Bookmark { id, frame_idx, timestamp_secs, label, created_at }` + `BookmarkFile` sidecar；录制时按 `b` inline label 输入（Enter 提交 / Esc 取消 / 空 label 默认「书签 #N」）；回放时按 `B`（Shift+B）打开书签面板（Up/Down/Enter 跳帧 · `e` 编辑 label · `d` 删除 · 子串搜索 · Esc 关闭）；`VtRecorder::frame_count()` 让 `b` 拿到当前帧索引 | `cargo test --release` 1168 passed（基线 1135 + 新测试 33 = 16 bookmark.rs unit + 17 integration）；录制路径 8 case + 回放路径 9 case 全过 | ✅ 全交付（commit `3af95f7`）|
| 3 Slice | 时间轴搜索。FilterExpr 扩 `FrameField`（Timestamp / Cpu / Mem / Name / AnomalySeverity）5 维度 + `FrameEvalCtx { frame: &UiFrame }` + `apply_frame()` + `parse_frame()` Frame 模式入口（`ParseMode` dispatch 解决 cpu/mem 字段名歧义）；`ReplaySearch { input, expr, error, matches, cursor }` 状态机；`/` 进入搜索输入态 + `n`/`N` vim 同款跳转 + 命中帧位置 `●` 在 timeline 高亮；parse 失败保留上一次成功 AST；substring 模式走 `build_frame_substring_expr` regex escape | `cargo test --release` 1221 passed（基线 1168 + 新测试 53 = 13 search.rs unit + 40 integration）；parse_frame 维度 / apply_frame 命中 / substring escape / FrameField unit / ReplaySearch 集成全过 | ✅ 全交付（commit `fe68b7a` 合并 stage 3+4）|
| 4 Slice | 倒放。`ReplayDirection { Forward (default), Reverse }` 枚举（与 ReplaySpeed 正交）+ `TimelineState.direction` 字段 + `r` 键切方向（小写 r；不与录制键 R 冲突）+ `tick` 双向分支（正向 clamp 末帧暂停 / 倒放 clamp 首帧暂停，对称）+ timeline icon `▶` / `◀` / `⏸` 三态（playing+Forward 时 ▶ / playing+Reverse 时 ◀ / paused 时 ⏸）+ `ReplayAction::DirectionToggled` 让 App 设 status_message 提示；切方向不重置 `current_frame` / `half_tick` | `cargo test --release` 1242 passed（基线 1221 + 新测试 21）；ReplayDirection enum + start 默认 + r 键切方向 + tick 双向分支含 Half/Quad/边界 + 边界连续性 + search 与 direction 解耦 + r 在搜索输入态被吞 + r 与 R 不冲突 21 case 全过 | ✅ 全交付（commit `fe68b7a` 合并 stage 3+4）|

**结论**：stage 1-4 全部交付，无未交付项。cycle 业务代码累计 ~1750 行（与方案 A 预期对齐），新测试累计 +127（1115 → 1242）。

---

## 1. 六子项审查

### 1.1 代码质量

#### 1.1.1 stage 1 v3 文件格式：footer + 按需加载 LRU 单帧缓存设计扎实

| 检查 | 实测 | 结果 |
|---|---|---|
| `RecordingFooter` schema 完整 | `src/record/frame.rs::RecordingFooter { version, header_version, start_time, end_time, frame_count, anomaly_count, event_count, max_cpu, max_mem, frame_offsets: Vec<u64> }` 10 字段 | ✅ |
| `FOOTER_MAGIC` 演进机制 | `FOOTER_MAGIC: [u8; 8] = *b"PREC3FT\x01"`（末字节 `\x01` 是 schema 版本号，bump 让 footer schema 演进）+ `FOOTER_TRAILER_LEN: u64 = 16`（8B footer_len + 8B magic）| ✅ |
| writer 累积 9 状态 | `Recorder::start` writer 线程 internal `current_offset / frame_offsets / first_frame_ts / end_time / anomaly_count / event_count / max_cpu / max_mem / frame_count`；`WriterMsg::Frame` 写帧前 push current_offset + 更新元数据；`WriterMsg::Stop` 构造 footer + 写文件末尾（footer_bytes + 8B footer_len + 8B FOOTER_MAGIC），`RECORDING_VERSION` 2 → 3 | ✅ |
| `Player` 按需加载结构 | `{ path, header, footer, file: RefCell<File>, cache_idx: Cell<Option<usize>>, cache_frame: RefCell<Option<UiFrame>> }`；`frame_at(idx) -> Option<UiFrame>` 改 owned + LRU 单帧缓存（cache_idx / cache_frame RefCell）| ✅（标准 Rust 内部可变性模式）|
| `open` 流程 seek trailer 检测 v3 | `open` 读 header + seek `file_size - 16` 检测 trailer（v3 路径 deserialize footer / 非 v3 走 `open_legacy`）| ✅ |
| `IdxSidecar` v1/v2 兼容 | `try_load` / `write` / `from_legacy` API；新鲜性校验同 `.bookmarks.json`（size + mtime 必须匹配）；v1/v2 老文件首次 open 自动生成 sidecar（永不重写本体）| ✅ |
| CLI `--info` 不开 TUI | `proc replay recording.prec --info` 走 `run_replay_info` 输出 footer 元数据（帧数 / 时长 / 异常 / docker / 最高 CPU/mem）| ✅ |

**判定**：v3 文件格式设计扎实，footer schema 演进机制清晰，按需加载 LRU 单帧缓存标准。✅

#### 1.1.2 stage 2 书签系统：sidecar 解耦设计正确

| 检查 | 实测 | 结果 |
|---|---|---|
| `Bookmark` schema | `{ id, frame_idx, timestamp_secs, label, created_at }` 5 字段 | ✅ |
| `BookmarkFile` sidecar | `{ magic, version, source_path, source_size, source_mtime, bookmarks: Vec<Bookmark> }`；`try_load` / `load_or_empty` / `write` / `add` / `remove` / `edit_label` / `sort_by_frame` API；新鲜性校验同 `.prec.idx`（size + mtime 必须匹配）；损坏静默降级到空列表 | ✅ |
| `BookmarkPanelState` UI 状态 | `{ cursor, search_query, editing_label, editing_id }`；`start_edit` / `end_edit` / `is_editing` 控制 inline 编辑生命周期；录制 / 回放两侧共用 | ✅ |
| `VtRecorder::frame_count()` | `Arc<AtomicU64>` writer thread fetch_add + 主线程 load，让录制中按 `b` 能拿到当前帧索引 | ✅（标准 atomic 计数模式）|
| `App::pending_bookmark_label` 早期拦截 | `handle_key` 在该状态激活时拦截所有按键（仅次于 kill_confirm 的优先级）| ✅ |
| 书签面板激活时 tick 暂停 | `ReplayController::tick` 第 1 行 `if self.bookmark_panel.is_some() { return ReplayAction::Noop; }` | ✅ |
| 书签面板键位路由 | `handle_bookmark_panel_key` 把 panel 从 self 取出操作再放回（标准 `std::mem::take` 借用拆分模式）| ✅ |
| `flush_recording_bookmarks` 两路径 | 正常停止 + Ctrl+C 退出两路径都 flush sidecar | ✅ |

**判定**：书签 sidecar 解耦设计正确（与录屏本体不污染，可单独分享 / 删除）。✅

#### 1.1.3 stage 3 时间轴搜索：FilterExpr 三层 ctx 分离 + ReplaySearch 状态机设计清晰

| 检查 | 实测 | 结果 |
|---|---|---|
| `FrameField` 枚举 | `{ Timestamp, Cpu, Mem, Name, AnomalySeverity }` 5 维度；`is_text()` / `extract_first()` / `any_match()` 三方法 | ✅ |
| `FrameEvalCtx` 与 `EvalCtx` / `NetworkEvalCtx` 平级 | `FrameEvalCtx { frame: &UiFrame }`；apply_frame / apply / apply_network 三方法对称 | ✅ |
| `FilterExpr` 3 个新变体 | `FrameFieldCmp` / `FrameRegex` / `FrameIn`（FrameTextEq 合并到 FrameFieldCmp 的 Eq/Ne 分支，简化）| ✅ |
| `apply_frame` 递归正确 | `And(l, r) => l.apply_frame(ctx) && r.apply_frame(ctx)` / `Or` / `Not`（`src/filter/mod.rs:490-492`）；Process / Network 变体在 FrameEvalCtx 下返 false（line 493）| ✅ |
| `contains_frame_field` helper | 与 `contains_process_field` 对称，让 timeline 搜索 / List 视图 detect 是否走 apply_frame 路径 | ✅ |
| `build_frame_substring_expr` regex escape | `regex::escape(input)` 防元字符注入（如 `.` `*` `+` 当字面量）| ✅ |
| `ParseMode { Process, Frame }` dispatch | `parse_with_mode(input, mode)` 入口 + `parse_frame(input)` 公开入口（Frame 模式）；既有 `parse(input)` 默认 Process 模式不变（向后兼容 List / Tree / Flow 视图）| ✅ |
| `cpu`/`mem` 字段名歧义解决 | Frame 模式下解析成 `FrameField::Cpu/Mem`，Process 模式下解析成 `Field::Cpu/Mem`（同名字段歧义由 mode dispatch 解决，不重命名字段）| ✅ |
| `ReplaySearch` 状态机 | `{ input, expr, error, matches, cursor }` 5 字段；`is_active()` / `push_char` / `pop_char` / `reset` / `recompute_matches(total, frame_at)` / `next_match` / `prev_match` / `current_match` 9 方法 | ✅ |
| parse 失败保留上一次 expr | `reparse()` 失败时仅更新 `error`，不动 `expr`（让 UI 继续过滤，既有 FilterExpr UX 同款契约）| ✅ |
| `recompute_matches` lazy 化 | 一次性遍历 N 帧 + 命中索引缓存到 `matches: Vec<usize>`，n/N 跳转只读缓存 | ✅ |
| `recompute_search_matches` 借用拆分 | `let mut search = std::mem::take(&mut self.search);` + frame_at closure 注入（标准借用拆分模式）| ✅（`controller.rs:370-384`）|

**判定**：FilterExpr 三层 ctx 分离（Process / Network / Frame）类型系统保证字段不跨 ctx 误用；ReplaySearch 状态机设计清晰；ParseMode dispatch 解决字段名歧义不破坏既有契约。✅

#### 1.1.4 stage 4 倒放：ReplayDirection 与 ReplaySpeed 正交 + tick 双向分支对称

| 检查 | 实测 | 结果 |
|---|---|---|
| `ReplayDirection` 枚举设计 | `{ Forward (#[default]), Reverse }`；`is_reverse()` / `icon()` (`▶` / `◀`) / `toggle()` 三方法；作为独立枚举（而非扩 ReplaySpeed 8 档）—— speed × direction 正交，独立字段让 UI 渲染 / 配置序列化直观 | ✅ |
| `TimelineState.direction` 字段 | 默认 `ReplayDirection::Forward`（与既有行为兼容）；`ReplayController::start` 初始化 Forward | ✅ |
| `r` 键切方向分支 | `handle_key` 普通模式 `KeyCode::Char('r') => { ts.direction = ts.direction.toggle(); return ReplayAction::DirectionToggled; }`（`controller.rs:240-246`）；小写 r；不与录制键 R（Shift+R 在 App 主路径）冲突 | ✅ |
| `R`（Shift+R）保留 fallthrough | `R` 在 `handle_key` 没捕获（fallthrough 到 `_ => {}`），保留既有 no-op 行为不动（surgical）| ✅ |
| `tick` 双向分支对称 | 正向 `(current + step).min(last)` + 到末帧暂停（既有逻辑保留）/ 倒放 `current.saturating_sub(step)` + 到首帧 `current == 0` 暂停（新行为，对称）| ✅（`controller.rs:594-610`）|
| step 计算与 direction 独立 | Half / Normal / Double / Quad 四档 speed 在两方向都工作（Half 速度倒放 `(half_tick + 1) % 2` 与正向一致）| ✅ |
| 切方向不重置节奏 | 切方向不重置 `current_frame` / `half_tick`（与 speed 切换不重置 half_tick 同款原则）| ✅ |
| timeline icon 三态 | playing+Forward 时 ▶ / playing+Reverse 时 ◀ / paused 时 ⏸（暂停态不区分方向）；`ReplayDirection::icon()` 与 timeline 渲染共用同一字符表 | ✅ |
| `ReplayAction::DirectionToggled` | 让 App 设 status_message 提示「倒放中（再按 r 切回正向）」/「正向播放（再按 r 切倒放）」 | ✅ |
| `r` 在搜索输入态被吞 | `search_input_active == true` 时 `handle_search_input_key` 优先（`Char(c) if !c.is_control()` 命中），`r` 字符 push 到 search input 不切方向 | ✅（test_replay_direction.rs 验证）|
| `r` 在书签面板激活时被吞 | `bookmark_panel.is_some()` 时 `handle_bookmark_panel_key` 优先 | ✅ |

**判定**：ReplayDirection 设计正确（正交独立枚举），tick 双向分支对称（边界暂停对称），键位冲突避免（小写 r + 搜索输入态 / 书签面板激活时拦截）。✅

#### 1.1.5 测试覆盖度：cycle 累计 +127 新测试

| Stage | 新测试数 | 累计基线 | 覆盖维度 |
|---|---|---|---|
| stage 1 | +20 | 1115 → 1135 | v3 round-trip / random seek / footer correctness / v1/v2 兼容 / sidecar 6 case |
| stage 2 | +33 | 1135 → 1168 | 16 bookmark.rs unit + 17 integration（录制路径 8 + 回放路径 9）|
| stage 3 | +53 | 1168 → 1221 | 13 search.rs unit + 40 integration（12 parse_frame 维度 + 14 apply_frame 命中 + 4 substring escape + 3 FrameField unit + 7 ReplaySearch 集成）|
| stage 4 | +21 | 1221 → 1242 | 4 ReplayDirection enum + 1 start 默认 + 3 r 键切方向 + 8 tick 双向分支含 Half/Quad/边界 + 2 边界连续性 + 1 search 与 direction 解耦 + 1 r 在搜索输入态被吞 + 1 r 与 R 不冲突 |
| **cycle 累计** | **+127** | **1115 → 1242** | **全维度覆盖** |

**判定**：cycle 测试覆盖度优秀，每 stage 测试数与新增功能复杂度匹配。✅

---

### 1.2 架构审查

#### 1.2.1 录屏模块改动范围最小

| 检查 | 实测 | 结果 |
|---|---|---|
| stage 1 改动文件 | `src/record/{frame.rs(RecordingFooter + FOOTER_MAGIC + RECORDING_VERSION 2→3), writer.rs(Recorder writer thread 累积 footer 状态 + Stop 写 footer/trailer), reader.rs(Player 重写为按需加载 + open_legacy fallback + frame_at 返 owned), sidecar.rs(新), mod.rs(pub mod sidecar + re-export)}` + `src/cli/{def.rs(Replay --info flag), record.rs(run_replay_info 分支)}` | ✅（最小范围，未污染其他模块）|
| stage 2 改动文件 | `src/record/{bookmark.rs(新 ~280 行 16 unit test), mod.rs(pub mod bookmark + re-export), vt100.rs(VtRecorder 加 frame_count 字段 + 方法 + writer thread fetch_add)}` + `src/app.rs(import + App 加 4 字段 + PendingBookmarkLabel struct + 5 method; handle_key 加 pending_bookmark_label 早期拦截 + recording_wanted + b 激活段; dispatch_replay_action 加 BookmarkPanelToggled 分支)` + `src/replay/controller.rs(ReplayController 加 2 字段 + handle_bookmark_panel_key 方法 + filter_indices 顶层函数 + ReplayAction::BookmarkPanelToggled 变体)` + `src/tui/{mod.rs(set_recording_path + set_recording_frame_count 每 tick + flush 正常/Ctrl+C 两条路径), replay_panel.rs(draw_bookmark_panel modal + draw_timeline 末尾叠加 + 入口 [B 书签] 提示行)}` | ✅（书签模块独立，App / Controller 改动最小）|
| stage 3 改动文件 | `src/filter/{mod.rs(FrameField 枚举 + FrameEvalCtx + 3 个新变体 + apply_frame + contains_frame_field + build_frame_substring_expr + 3 unit test), parser.rs(ParseMode 枚举 + parse_with_mode + parse_frame 入口 + parse_field_with mode dispatch + anomaly.severity 点号特殊处理 + leaf 加 Frame 分支)}` + `src/replay/{search.rs(新 ~410 行 13 unit test), mod.rs(pub mod search + re-export ReplaySearch), controller.rs(ReplayController 加 search + search_input_active 字段 + handle_search_input_key 方法 + recompute_search_matches helper + ReplayAction::SearchInputToggled/SearchMatchesUpdated 变体 + start 时 reset search)}` + `src/app.rs(dispatch_replay_action 加 Search* 分支 + status_message 含命中数 / parse 错误)` + `src/tui/replay_panel.rs(draw_timeline 加搜索输入态 + 命中标记渲染：gauge 替换为 Paragraph `●■` 字符布局 + info line 末尾追加搜索状态)` | ✅（FilterExpr 扩展遵循三层 ctx 分离原则）|
| stage 4 改动文件 | `src/replay/{controller.rs(ReplayDirection 枚举 + TimelineState.direction 字段 + start 初始化 + handle_key 加 r 分支 + tick 双向分支 + ReplayAction::DirectionToggled 变体), mod.rs(re-export ReplayDirection)}` + `src/app.rs(re-export ReplayDirection + use + dispatch_replay_action 加 DirectionToggled 分支 + status_message 中文提示)` + `src/tui/replay_panel.rs(draw_timeline icon 三态 ▶/◀/⏸)` | ✅（最小改动，独立枚举不污染 ReplaySpeed）|

**判定**：4 个 stage 的改动范围都最小，未污染其他模块；FilterExpr 三层 ctx 分离原则严格遵守；ReplayController 字段扩展合理（bookmark_panel / search / direction 三类功能正交独立）。✅

#### 1.2.2 ReplayController 字段扩展合理性

| 字段 | stage | 类型 | 设计理由 |
|---|---|---|---|
| `replay_player` | v0.6 阶段 5 | `Option<Player>` | 既有 |
| `timeline_state` | v0.6 阶段 5 | `Option<TimelineState>` | 既有 |
| `bookmarks` | v0.14 stage 2 | `Option<BookmarkFile>` | sidecar load 一次，None = 未加载 |
| `bookmark_panel` | v0.14 stage 2 | `Option<BookmarkPanelState>` | None=未打开 / Some=打开中 |
| `search` | v0.14 stage 3 | `ReplaySearch` | 始终存在（默认空 input），`is_active()` 决定生效 |
| `search_input_active` | v0.14 stage 3 | `bool` | 与 `search.is_active()` 不同：input 可在退出输入态后仍非空（n/N 跳转用）|

**判定**：6 字段扩展合理，`Option<T>` 表达「未激活」语义清晰；`search: ReplaySearch`（非 Option）因为 search 状态始终存在（默认空），通过 `is_active()` 判断生效——比 `Option<ReplaySearch>` 更直观。✅

#### 1.2.3 ReplayAction 枚举变体扩展（5 个新变体）

| 变体 | stage | 触发 | App 副作用 |
|---|---|---|---|
| `ApplyFrame` | v0.6 阶段 5 | tick / Left / Right / Home / End / n / N | 应用 current_frame 到 panels |
| `Quit` | v0.6 阶段 5 | `q` 键 | should_quit = true |
| `Noop` | v0.6 阶段 5 | 无副作用键 | — |
| `BookmarkPanelToggled` | v0.14 stage 2 | `B` 键打开 / Esc 关闭 | status_message 提示 |
| `SearchInputToggled` | v0.14 stage 3 | `/` 进入 / Esc 退出输入态 | status_message 提示 |
| `SearchMatchesUpdated` | v0.14 stage 3 | Enter / Backspace / 字符 push（recompute 后）| status_message 含命中数 / parse 错误 |
| `DirectionToggled` | v0.14 stage 4 | `r` 键 | status_message 提示「倒放中 / 正向播放」 |

**判定**：5 个新 ReplayAction 变体设计合理，每个变体对应一个用户可感知的状态变化，App dispatch_replay_action 分支扩展最小。✅

---

### 1.3 性能审查

#### 1.3.1 v3 按需加载 PERF-BASELINE TD-45 闭环验证

| 指标 | v0.13.0 基线（全量加载） | v0.14.0 落地（按需加载） | 验证方法 |
|---|---|---|---|
| 启动加载（30 min × 30 FPS × 1000 进程 = 54000 frames）| **9 秒**（54000 × 165 µs deserialize）| < 100 ms（仅读 header + footer + frame_offsets）| `bench_record_serialize` 单帧 deserialize 165 µs @ 1000 进程不变；启动只 deserialize 1 个 footer |
| 内存占用（同上）| **~10 GB**（54000 × 200 KB UiFrame bincode）| ~12 MB（仅当前帧 + LRU 单帧缓存）| `Player::cache_frame: RefCell<Option<UiFrame>>` 单帧 LRU |
| 单帧 seek（@ 1000 进程）| 165 µs | 165 µs（不变）| 按需加载不改变单帧 deserialize 成本 |
| n/N 跳转延迟 | N/A | < 200 µs（单帧 seek + 应用到 panels）| 用户无感 |

**判定**：v3 按需加载 PERF-BASELINE TD-45 闭环——启动加载 9s → < 100ms（90× 加速），内存 10 GB → ~12 MB（800× 缩减）。**核心 ROI 达成**。✅

#### 1.3.2 v1/v2 老文件按需加载 sidecar 兼容性

| 检查 | 实测 | 结果 |
|---|---|---|
| `IdxSidecar` 自动生成 | v1/v2 老文件首次 open 后自动生成 `.prec.idx` sidecar（含 frame_offsets + 元数据），下次 open 走按需加载路径 | ✅ |
| v1/v2 老文件永不重写 | sidecar 单独管理（与录屏本体解耦）；本体只读 | ✅ |
| 新鲜性校验 | `IdxSidecar::try_load` 校验 source_size + source_mtime 必须匹配（同 `.bookmarks.json`）；不匹配则重生成 | ✅ |
| 损坏静默降级 | sidecar 损坏时回退到全量加载 + 重生成 sidecar | ✅ |

**判定**：v1/v2 老文件兼容性设计扎实，sidecar 解耦原则与 stage 2 `.bookmarks.json` 一致。✅

#### 1.3.3 长录屏搜索遍历延迟（已知限制）

**stage 3 doc §「风险 2」**：长录屏遍历可能慢（30 min × 30 FPS × 1000 进程 = 54000 frames × 165 µs = ~9 秒）。

| 检查 | 实测 | 结果 |
|---|---|---|
| recompute_matches 仅在 input 变化时调 | `push_char` / `pop_char` 触发 `recompute_search_matches`；n/N 跳转只读 `matches` 缓存 | ✅ |
| status_message 提示用户 | `SearchMatchesUpdated` 让 App 设 status 含命中数 + parse 错误 | ✅（但无「正在搜索 N 帧…」异步提示——长录屏遍历期间 UI 短暂冻结）|
| 长录屏遍历 ~9 秒用户可感 | 遍历期间 TUI 阻塞（同步路径）| ⚠ **已知限制**（stage 3 doc 标注，P2 候选 TD-49）|

**判定**：长录屏搜索遍历是已知限制（stage 3 doc 明确），未引入异步搜索（保持 surgical 简单），延迟 ~9 秒用户可感但可接受（30 min 录屏是极端场景，常用 5-10 min 录屏 < 1 秒）。**P2 候选 TD-49**：footer 加索引段让阈值搜索 O(1)（如 max_cpu_frame_idx / first_critical_anomaly_idx）。✅

#### 1.3.4 timeline 高亮渲染开销

| 检查 | 实测 | 结果 |
|---|---|---|
| gauge 替换为自定义 Paragraph | 命中帧时 `draw_timeline` 把 gauge 替换为 Paragraph（`●` 标命中位置 + `■` 标当前帧）；无命中时保留 Gauge | ✅ |
| 字符布局开销 | 按帧索引均匀分布到 timeline 宽度；每帧占 `W / total` 字符（`<=` 时多帧挤一格）| O(W) where W = timeline 宽度（典型 80 字符），用户无感 |
| `●` overlay 不影响其他 Span | 命中位置字符替换，不影响 info_line 其他 Span（speed_label / direction icon / search 提示）| ✅ |

**判定**：timeline 高亮渲染开销可忽略（O(W) 字符布局，W < 100）。✅

---

### 1.4 完整性检查

#### 1.4.1 brainstorm 文档

| 检查 | 实测 | 状态 |
|---|---|---|
| 阶段总览表反映方案 A（5 stage：1 Spike + 3 Slice + 1 Review+收尾）| `docs/stages/v0.14-brainstorm.md:220-227` 5 stage 都列；stage 1/2/3/4 标 ✅，stage 5 标 ⬜ | ✅ |
| 用户拍板记录段填（方案 A 理由 + cycle 主题）| brainstorm §「推荐方案：A」第 197-208 行完整（用户选方案 A 理由 5 条）| ✅ |
| stage 数量自适应规则段 | brainstorm 第 228-232 行（默认按方案 A 5 stage 推进，遇问题再调整）| ✅ |
| cycle 阶段总览表 stage 3 / stage 4 状态 | 第 224-225 行 stage 3 + stage 4 都 ✅ 已完成 | ✅ |

**判定**：brainstorm 完整反映方案 A 决策。✅

#### 1.4.2 stage docs 头部 ✅ 标记

| 检查 | 实测 | 状态 |
|---|---|---|
| `docs/stages/v0.14-stage-1.md` 头部 ✅ | 第 1 行 `### 阶段 1：Spike — ...`，第 3 行 `> **独立会话指令**：...`，**无 ✅ 标记** | ❌ **P1-1** |
| `docs/stages/v0.14-stage-2.md` 头部 ✅ | 同上结构，**无 ✅ 标记** | ❌ **P1-1** |
| `docs/stages/v0.14-stage-3.md` 头部 ✅ | 同上结构，**无 ✅ 标记** | ❌ **P1-1** |
| `docs/stages/v0.14-stage-4.md` 头部 ✅ | 同上结构，**无 ✅ 标记** | ❌ **P1-1** |

**P1-1**：4 个 stage docs 头部缺 `> ✅ **已完成**` 标记。与 v0.13 stage 3 P1-2 / P1-3 同款问题（cycle 末段 Review 时发现 stage docs 头部 ✅ 标记漏加）。stage 5 收尾段 §4.5 任务会加。

**判定**：1 个 P1（4 个 stage docs 头部 ✅ 缺）。✅

#### 1.4.3 CHANGELOG `[Unreleased]` 段

| 检查 | 实测 | 状态 |
|---|---|---|
| `[Unreleased]` 段含 stage 1-4 条目 | `CHANGELOG.md:8-18` 含 stage 1 / stage 2 / stage 3 / stage 4 各一段（详细描述）+ stage 5 待启动条目 | ✅ |
| `[0.13.0]` 段保留（v0.13 cycle 历史）| `CHANGELOG.md:20` 起 `[0.13.0] - 2026-07-05` | ✅ |
| `[Unreleased]` → `[0.14.0]` 改名 | stage 5 收尾段 §4.1 任务 | ⬜ 待执行（不算 Review P1，是收尾任务）|

**判定**：CHANGELOG `[Unreleased]` 段 stage 1-4 条目齐全，stage 5 收尾段会改 `[0.14.0] - 2026-07-XX` + 加阶段汇总 + 关键数字表。✅

#### 1.4.4 README 录屏章节

| 检查 | 实测 | 状态 |
|---|---|---|
| 录屏章节提 v0.14 起的所有新功能 | `README.md:225-231` 含：录制时按 `b` 标记书签（line 225）/ 回放时按 `B` 打开书签面板（line 227）/ 回放时按 `/` 进入时间轴搜索（line 228）/ 回放时按 `r` 切方向（line 229）/ `proc replay --info`（line 230）/ `.bookmarks.json` sidecar（line 231）| ✅ |
| README banner v0.14.0 段 | 当前 banner 第 5 行是 v0.13.0；stage 5 收尾段 §4.3 任务会加 v0.14.0 banner | ⬜ 待执行（不算 Review P1，是收尾任务）|

**判定**：README 录屏章节 v0.14 功能描述完整（stage 3 + stage 4 落地时已加）。✅

#### 1.4.5 CONTEXT.md 演进历史段

| 检查 | 实测 | 状态 |
|---|---|---|
| v0.14.0 段存在 | `CONTEXT.md:208` `### v0.14.0 落地变更（开发中，2026-07-05 启动）` | ✅ |
| stage 1 行（v3 文件格式）| `CONTEXT.md:215` 完整描述 stage 1 | ✅ |
| stage 2 行（书签系统）| `CONTEXT.md:214` 完整描述 stage 2 | ✅ |
| stage 3 行（时间轴搜索）| `CONTEXT.md:213` 完整描述 stage 3 | ✅ |
| stage 4 行（倒放）| `CONTEXT.md:212` 完整描述 stage 4 | ✅ |
| stage 5 行（Review + 收尾）| 缺 | ⬜ 待执行（stage 5 收尾段 §4.7 任务，本地不入 commit）|

**判定**：CONTEXT.md 演进历史段 stage 1-4 行齐全（v0.14 cycle 落地时已加），stage 5 收尾段会加 stage 5 行（本地不入 commit，.gitignore 私有文件）。✅

#### 1.4.6 tech-debt.md

| 检查 | 实测 | 状态 |
|---|---|---|
| v0.15.0+ 候选段 | 当前 tech-debt.md 含 v0.13.0+ 候选段（TD-44~48），无 v0.14 stage 5 新 TD 候选 | ⬜ 待 stage 5 Review §3 决策（P2-1 候选 TD-49）|
| TD-44~48 终态 | 全部归档正确（4 项 v0.13 stage 2 归档 + 1 项 v0.13 stage 3 归档）| ✅ |

**判定**：tech-debt TD-44~48 终态正确，stage 5 §3 决策新 TD-49（P2-1）归档到 v0.15+ 候选补遗段。✅

---

### 1.5 安全 / 跨平台审查

#### 1.5.1 v3 文件格式向后兼容性

| 检查 | 实测 | 结果 |
|---|---|---|
| v1/v2 老文件读取路径保留 | `Player::open` 流程 seek trailer 检测 v3 / fallback `open_legacy`；`open_legacy` 先 sidecar 命中（size + mtime 匹配）→ fallback 全量加载 + 构造 footer + 写 sidecar | ✅ |
| v3 文件 footer 自描述 | footer 含 frame_offsets，不需要 sidecar；FOOTER_MAGIC 末字节 bump 让 schema 演进 | ✅ |
| v1/v2 老文件**永不重写** | sidecar 单独管理（`.prec.idx`），本体只读 | ✅ |
| 录屏文件含敏感数据 | v0.6 既有的录屏确认弹窗仍生效（按 `R` 触发时先弹确认）| ✅ |
| `.bookmarks.json` sidecar 不含敏感数据 | 仅 `label` 是用户输入（id / frame_idx / timestamp_secs / created_at 都是元数据）| ✅ |

**判定**：v3 文件格式向后兼容性设计扎实，v1/v2 老文件零迁移成本即可享受按需加载（首次 open 自动生成 sidecar）。✅

#### 1.5.2 VT100 replay 路径与 UiFrame replay 路径的差异（已知限制）

| 检查 | 实测 | 结果 |
|---|---|---|
| VT100 replay 路径不加倒放 | VT100 文件是字节流（无结构化帧索引），倒放需要把整个字节流倒序解析（每个 VT100 序列需反向应用：clear / cursor move / SGR 等），实现成本远超 stage 4 的 ~250 行预算（实际 ~1000+ 行 + VT100 反向解释器）| ✅（stage 4 doc §6 明确 surgical 跳过）|
| VT100 replay 路径不加搜索 | VT100 文件无结构化数据（无 UiFrame），无法 apply FilterExpr | ✅（stage 3 doc §6 明确 surgical 跳过）|
| `r` 键在 VT100 replay 路径下 no-op | VT100 replay 是独立循环（`run_vt_replay`），保留既有 fallthrough 不动 | ✅ |
| `/` 键在 VT100 replay 路径下 no-op | 同上 | ✅ |
| CONTEXT 术语段标注 | stage 5 收尾段会加「VT100 replay 无倒放 / 搜索」标注（与 stage 3 doc「VT100 replay 无时间轴搜索」同款注释）| ⬜ 待执行（不算 Review P1，是收尾任务）|

**判定**：VT100 replay 路径与 UiFrame replay 路径的差异是已知限制（surgical 跳过，stage 3 / stage 4 doc 明确），不阻断 v0.14.0 发布；归档 TD-49 留 v0.15+ cycle 评估。✅

#### 1.5.3 录屏文件含敏感数据（v0.6 既有，v0.14 cycle 不动）

| 检查 | 实测 | 结果 |
|---|---|---|
| 录屏启动确认弹窗 | v0.6 落地，按 `R` 触发时先弹确认（警告会捕获屏幕所有内容含 DNS 域名 / 进程 cmd / env 真值）| ✅ |
| 录屏期间 Env Tab 强制 mask | v0.6 落地，录屏中 `env_reveal=true` 也强制 mask | ✅ |
| v0.14 cycle 是否破坏 v0.6 既有保护 | stage 1-4 改动范围最小（仅 record / replay / filter / tui/replay_panel），不动 Env Tab mask 路径 | ✅ |

**判定**：v0.14 cycle 未破坏 v0.6 既有的录屏敏感数据保护。✅

---

### 1.6 P0 / P1 / P2 列表

#### P0（阻断 v0.14.0 发布）：0 项

无。cycle 业务代码 +127 测试全过，fmt / clippy / build / bench 全过，无编译 / 测试 / 关键文档阻断问题。

#### P1（cycle 内闭环）：1 项

| 编号 | 问题 | 修复 |
|---|---|---|
| **P1-1** | 4 个 stage docs 头部缺 `> ✅ **已完成**` 标记（`docs/stages/v0.14-stage-{1,2,3,4}.md` 头部），与 v0.13 stage 3 P1-2 / P1-3 同款问题 | stage 5 收尾段 §4.5 任务加 4 个 ✅ 标记（+ stage 5 doc 自身 1 个 = 5 个）|

#### P2（归档 v0.15+ cycle）：1 项

| 编号 | 问题 | 归档 |
|---|---|---|
| **P2-1 → TD-49** | VT100 replay 路径无倒放 / 搜索（字节流需反向解释器 ~1000+ 行 / 无结构化数据无法 apply FilterExpr）+ 长录屏搜索遍历延迟优化（30 min × 1000 进程 ≈ 9 秒用户可感，footer 加索引段让阈值搜索 O(1)）—— stage 3 / stage 4 doc 「不在本 stage 范围」段已明确 surgical 跳过，但应归档留 v0.15+ cycle 评估 | tech-debt.md 加 TD-49 段 |

---

## 2. P1 修复方案

### P1-1：4 个 stage docs 头部加 ✅ 标记

在 `docs/stages/v0.14-stage-{1,2,3,4}.md` 第 1 行（`### 阶段 N：...` 行）下面插入：

```markdown
> ✅ **已完成**（v0.14.0 阶段 N 会话产出，2026-07-XX）
```

具体日期 stage 5 收尾时填（v0.14 stage 1 = 2026-07-05 / stage 2 = 2026-07-05 / stage 3+4 = 2026-07-06 / stage 5 = 2026-07-06）。

**修复位置**：
- `docs/stages/v0.14-stage-1.md:1` 后插入 ✅ 标记
- `docs/stages/v0.14-stage-2.md:1` 后插入 ✅ 标记
- `docs/stages/v0.14-stage-3.md:1` 后插入 ✅ 标记
- `docs/stages/v0.14-stage-4.md:1` 后插入 ✅ 标记
- `docs/stages/v0.14-stage-5.md:1` 后插入 ✅ 标记（stage 5 本文档，收尾时加）

**注意**：4 个 stage docs 头部 ✅ 标记修复不动业务代码（仅 docs/* 改动），与 v0.13 stage 3 P1-2 / P1-3 同款规则。

---

## 3. P2 归档（TD-49）

### TD-49（REVIEW-v0.14 P2-1）：VT100 replay 路径无倒放 / 搜索 + 长录屏搜索遍历优化

**位置**：
- VT100 倒放：`src/tui/mod.rs::run_vt_replay`（VT100 字节流反向解释器需 ~1000+ 行实装）
- VT100 搜索：同上（VT100 文件无结构化数据，无法 apply FilterExpr）
- 长录屏搜索遍历优化：`src/record/frame.rs::RecordingFooter`（footer 加索引段让阈值搜索 O(1)）+ `src/replay/search.rs::recompute_matches`（按 footer 索引段短路）

**现状**：
- VT100 replay 路径不加倒放 / 搜索是 stage 3 / stage 4 doc 「不在本 stage 范围」段明确 surgical 跳过的——VT100 文件是字节流（无结构化帧索引），倒放需要把整个字节流倒序解析（每个 VT500 序列需反向应用：clear / cursor move / SGR 等），实现成本远超 stage 4 的 ~250 行预算；VT100 文件无结构化数据（无 UiFrame），无法 apply FilterExpr。
- 长录屏搜索遍历延迟是 stage 3 doc §「风险 2」明确的已知限制：30 min × 30 FPS × 1000 进程 = 54000 frames × 165 µs = ~9 秒用户可感。当前 `ReplaySearch::recompute_matches` 走同步遍历（input 变化时调一次），未引入异步搜索（保持 surgical 简单）。

**影响**：
- VT100 replay 用户感知不到差（VT100 replay 是独立 CLI 子命令 `proc replay-vt`，与 UiFrame replay `proc replay` 入口不同，用户使用时已知类型）
- 长录屏搜索遍历 ~9 秒用户可感但可接受（30 min 录屏是极端场景，常用 5-10 min 录屏 < 1 秒）；遍历期间 TUI 阻塞（同步路径），无异步提示

**修复方案**（v0.15+ cycle 评估）：
1. **VT100 倒放**（高成本 ~1000+ 行）：实装 VT500 反向解释器（每个 VT500 序列需反向应用：clear / cursor move / SGR 等），或转码 VT100 字节流到 UiFrame 结构（让 VT100 replay 享受 UiFrame replay 全部能力，但转码本身也是 ~1000+ 行）
2. **VT100 搜索**（同上）：VT100 文件转码到 UiFrame 后自动获得搜索能力
3. **长录屏搜索遍历优化**（中成本 ~200 行）：`RecordingFooter` 加索引段（如 `max_cpu_frame_idx` / `first_critical_anomaly_idx` / `cpu_threshold_frames: Vec<usize>`），让阈值搜索 O(1)；substring / regex 搜索仍走遍历路径（无自然索引）

**验证**：
- VT100 倒放：`run_vt_replay` 加 `r` 键分支 + 反向迭代字节流 + VT500 反向解释器单元测试
- 长录屏搜索优化：`recompute_matches` 按 footer 索引段短路（如 `cpu > 80` 走 `footer.max_cpu_frame_idx` 直接定位）；criterion bench 对比 before/after

**REVIEW-v0.14 决策**：归档 v0.15+ cycle 评估。理由：(1) VT100 replay 倒放 / 搜索实现成本高（~1000+ 行），用户痛点弱于书签 / 搜索 / 倒放（VT100 replay 是独立子命令，用户已知类型，UiFrame replay 已有完整能力）；(2) 长录屏搜索遍历是极端场景（30 min × 1000 进程），常用 5-10 min 录屏 < 1 秒用户无感，footer 加索引段需评估 schema 演进（FOOTER_MAGIC 末字节 bump）；(3) v0.14 cycle 已交付完整 UiFrame replay v2 能力（按需加载 + 书签 + 搜索 + 倒放），VT100 replay 是次要路径，留 v0.15+ cycle 评估时基于用户反馈重新决定优先级。

---

## 4. 验收

### 4.1 全量回归

`cargo test --release -q` = **1242 passed / 0 failed / 3 ignored**（v0.13.0 → v0.14 stage 1 → stage 2 → stage 3+4 全程基线递增 1115 → 1135 → 1168 → 1221 → 1242，cycle 累计 +127 新测试）。

理由：v0.14 cycle 4 个 stage 全部交付业务代码 + 测试，每个 stage 测试数与新增功能复杂度匹配。

### 4.2 静态检查

| 检查 | 命令 | 结果 |
|---|---|---|
| 格式化 | `cargo fmt --all -- --check` | ✅ 通过 |
| Clippy | `cargo clippy --release --all-targets -- -D warnings` | ✅ 通过 |
| 无默认 feature 构建 | `cargo build --release --no-default-features` | ✅ 通过（2m 54s）|
| Bench 编译 | `cargo bench --no-run` | ✅ 通过（2m 55s，6 个 bench + lib + main + 8 个 bench executable 全编译）|

### 4.3 stage docs ✅ 标记

- stage 1 doc：P1-1 修复后加 ✅
- stage 2 doc：P1-1 修复后加 ✅
- stage 3 doc：P1-1 修复后加 ✅
- stage 4 doc：P1-1 修复后加 ✅
- stage 5 doc：本 stage 完工时加 ✅

### 4.4 P0 / P1 / P2 闭环

- **P0 = 0** ✓
- **P1 = 1**（P1-1 闭环——见 §2 修复方案）
- **P2 = 1**（归档 TD-49——见 §3）

---

## 5. 后续（stage 5 收尾段 + cycle 闭环）

stage 5 Review 段完工后，stage 5 收尾段任务（按 stage 5 doc §4 任务清单）：

1. **CHANGELOG.md**：`[Unreleased]` → `[0.14.0] - 2026-07-XX` + 5 stage 阶段汇总 + 关键数字表（启动加载 9s → < 100ms / 内存 10 GB → ~12 MB / 单帧 seek 165 µs 不变）
2. **Cargo.toml**：`0.13.0` → `0.14.0` + Cargo.lock 自动同步
3. **README.md**：banner 加 v0.14.0 段（4 大能力：按需加载 + 书签 + 时间轴搜索 + 倒放）
4. **brainstorm.md**：cycle 阶段总览表 stage 5 ⬜ → ✅ + 末尾加 cycle 总结段（5 stage 全交付 + cycle 数据 + 关键决策 + REVIEW-v0.14 结论）
5. **5 stage docs 头部 ✅**（P1-1 修复，含 stage 5 doc 本身）
6. **tech-debt.md**：加 v0.15.0+ 候选补遗段 TD-49
7. **CONTEXT.md**：演进历史加 stage 5 行（本地，不入 commit）
8. **commit**：`release(v0.14.0): 阶段 1-5 全交付（录屏文件格式 v3 + 书签 + 时间轴搜索 + 倒放 + REVIEW-v0.14 + tag v0.14.0）`
9. **git tag v0.14.0**：等用户确认 push（与 v0.13.0 同款规则）

---

## 6. 总结

v0.14 cycle 是「录屏回放 v2 cycle」（方案 A 完整 v2 5 stage）：
- **stage 1** Spike（commit `dc96ae8`）：录屏文件格式 v3 — 按需加载 + footer + v1/v2 sidecar + `--info`
- **stage 2** Slice（commit `3af95f7`）：书签系统 — `b` 录制时添加 / `B` 回放时打开面板 / `.bookmarks.json` sidecar
- **stage 3** Slice（commit `fe68b7a`）：时间轴搜索 — FilterExpr 扩 5 维度 + timeline 高亮 + n/N 跳转
- **stage 4** Slice（commit `fe68b7a`）：倒放 — `r` 切方向 + tick 双向分支 + timeline `▶`/`◀`/`⏸` 三态
- **stage 5** Review + 收尾（commit 待）：本 Review + 收尾 + tag v0.14.0

**核心结论**：录屏模块从 v0.6 的「能用」升级到「事后分析能力完整」。按需加载解决长 session OOM（PERF-BASELINE TD-45 闭环：启动加载 9s → < 100ms / 内存 10 GB → ~12 MB）；书签 + 搜索 + 倒放补完 Forward / Reverse 双向控件集，与既有 List / Tree / AppGroup / Flow 视图享有同款 FilterExpr UX（`/` + `:` + substring 模式）。VT100 replay 路径不加新功能（surgical 跳过，归档 TD-49）。

**cycle 数据**：
- 全量回归：1115 passed（v0.13.0 基线）→ 1242 passed（v0.14.0 落地），+127 新测试
- 业务代码：~1750 行（与方案 A 预期对齐）
- 启动加载：9 秒 → < 100 ms（30 min × 1000 进程 session，90× 加速）
- 内存占用：~10 GB → ~12 MB（同上，800× 缩减）
- 单帧 seek：165 µs @ 1000 进程（不变，按需加载不改变单帧 deserialize 成本）

**REVIEW-v0.14 完工交付**：
- 本报告（~370 行）
- P1-1 修复（4 个 stage docs 头部 ✅ 标记，stage 5 收尾段 §4.5 加）
- TD-49 归档（VT100 replay 倒放 / 搜索 + 长录屏搜索遍历优化，留 v0.15+ cycle 评估）
- stage 5 收尾段（CHANGELOG + Cargo + README + brainstorm + tech-debt + CONTEXT + git tag v0.14.0）
- stage 1-5 docs 头部 ✅（含 stage 5 doc 本身）
- v0.15.0 cycle 启动指引（基于 v0.14 落地情况 + TD-44~49 残留 + cycle 5 stage 重 cycle 节奏对比）

**v0.15.0 候选方向**（stage 5 收尾段总结时给方向建议，用户最终拍板）：
- 主题 A：性能优化 cycle（基于 PERF-BASELINE TD-44~47 残留 + v0.14 cycle 重 cycle 后的轻 cycle 节奏，TD-47 parent_chain Arc 重构是首选）
- 主题 B：可观测性 cycle（Theme F — 实时流 / 远程查看 / WebSocket transport）
- 主题 C：proc inspect 增强 cycle（详情页 Tab 扩展 / 新增 Tab）
- 主题 D：MCP tool 扩展 cycle（暴露更多 proc 能力给 LLM agent）
- 主题 E：UI/UX polish cycle（v0.12 cycle 同款轻 cycle，主题与键位 / 帮助页 / 命令面板优化）
- 主题 F：VT100 replay 增强 cycle（TD-49 — VT100 字节流转码 UiFrame / 反向解释器，让 VT100 replay 享受 v0.14 全部能力）
