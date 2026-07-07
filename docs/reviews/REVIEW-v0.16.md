# REVIEW-v0.16：v0.16.0 cycle 全局 Review

> **范围**：v0.16.0 cycle stage 1-3 全部产出（commit `74f2522 plan(v0.16)` 之后的全部 working tree 改动）—— MCP handler 子 module 扩第 5 个文件 `record.rs`（7 tool Args struct + 7 helper 业务实装 + 5 私有辅助函数）+ 7 个新录屏 / 书签 / USB status tool schema 设计 + ADR-0025a（replay_search agent schema）/ 0025b（record 不暴露）+ 36 集成测试。
> **方法**：按 stage 4 doc §任务 2 列出的 6 子项审查（代码质量 / 架构 / 性能 / 完整性 / 安全跨平台 / P0-P1-P2 列表）。
> **基线**：`cargo test --release -q` = **1317 passed / 0 failed / 3 ignored**；`cargo fmt --all -- --check` / `cargo clippy --release --all-targets -- -D warnings` / `cargo build --release --no-default-features` / `cargo bench --no-run` 全过。
> **结论**：**P0 0 / P1 1 / P2 0**。stage 1-3 全部交付，无未交付项。1 个 P1 集中在文档完整性（stage 1 doc 头部缺独立 `> ✅ **已完成**` 标记，与 v0.14 stage 5 P1-1 / v0.15 stage 4 P1-1 同款问题）；0 个 P2（v0.16 cycle 量级偏轻 + brainstorm 决策段已穷尽未来考虑项，无新 TD 候选）。
> **Date**：2026-07-07。

---

## 0. 验收对照表（stage 1-3 是否全交付）

| Stage | 范围 | 验收 | 状态 |
|---|---|---|---|
| 1 Spike | MCP handler 子 module 扩第 5 个文件 `record.rs`（v0.15 4 子 module → 5 子 module，与 ADR-0024 Strategy A 延续——7 个新 `#[tool]` 仍在主 mod.rs impl 块）+ 7 tool stub schema + `ReplayInfoArgs` / `ReplaySearchArgs` / `BookmarksList/Add/Edit/DeleteArgs` / `EjectStatusArgs` 7 个 Args struct + ADR-0025a（proc_replay_search agent schema 设计：limit 默认 100 + truncated + substring/FilterExpr 双入口 + VT100 兜底 + 长录屏性能 ~9s 可接受）+ ADR-0025b（v0.16 cycle 决策不暴露 `proc_record_start/stop`：TTY 限制 + worker 持续采样成本 + confirm 机制待评估）+ CONTEXT.md 加 4 术语（McP RecordCategory / ReplaySearchQuery / BookmarkAction / EjectSuggestion） | `cargo test --release` 1281 passed（基线不变，stage 1 仅加 stub 不动业务代码）；`grep 'name = "proc_' src/mcp/handler/mod.rs` = 39（32 既有 + 7 新）；`grep '"stub": true' src/mcp/handler/record.rs` = 7 处 stub helper 全部注册；4 个新 Args struct `JsonSchema` derive 编译过 | ✅ 全交付（commit `bf71d5d`）|
| 2 Slice | replay + USB status 业务逻辑（3 tool）填充。`record.rs` 3 个 stub helper 替换为真实业务实现：`make_replay_info_json` 走 `is_vt100_file` 双路径分发（VT100 走 `VtPlayer` 返 `format: "vt100"` + header 字段，v3 走 `Player` 返 `format: "uiframe"` + 完整 footer 字段）+ `has_bookmarks_sidecar` 文件存在性检查 / `make_replay_search_json` 走 `parse_frame` + `build_frame_substring_expr` 双入口 + `apply_frame` 全帧遍历 + limit 默认 100 截断 + `truncated` 字段 + `matched_processes` 集合收集（精准匹配 name 约束走 collect_name_matches / 否则全列帧内进程名）+ `anomaly_severity` 取最高档（critical > warning > info）/ `make_eject_status_json` 走 `scan_all_devices` + `scan_device_locks` + 4 档 suggestion 决策树（`unknown_drive` / `unavailable` / `kill_locks` / `eject_now`）+ device 字段裁剪；加 2 个私有辅助函数 `collect_matched_processes` / `highest_anomaly_severity`；`tests/test_mcp_v0_16.rs`（新）18 case（replay_info 5 + replay_search 7 + eject_status 6）| `cargo test --release` 1299 passed（基线 1281 + 新测试 18）；`grep '"stub": true' src/mcp/handler/record.rs` 4 处剩 bookmarks stub（stage 3 范围）；集成测试 `test_replay_info_*` / `test_replay_search_*` / `test_eject_status_*` 18 case 全过 | ✅ 全交付（commit `2e5a1a6`）|
| 3 Slice | bookmarks 业务逻辑（4 tool）填充。`record.rs` 4 个 stub helper 替换为真实业务实现：`make_bookmarks_list_json` 走 `BookmarkFile::try_load` 区分 fresh vs stale sidecar + `sidecar_present` / `source_healthy` 双字段三态区分（无 sidecar / fresh / stale）+ bookmarks[] 字段裁剪 / `make_bookmarks_add_json` 双路径 frame_idx 校验（v3 用 `Player` / VT100 用 `VtPlayer`，VT100 timestamp 走 `time_range_ms` 内插 `start_ms + (end_ms - start_ms) * frame_idx / (total - 1)`，total=1 兜底）+ label 默认「书签 #N」+ dry_run 路径 + `sidecar_written` 字段 / `make_bookmarks_edit_json` 先查后改（保留 old_label 让 agent 看 diff）+ edit_label + write / `make_bookmarks_delete_json` 先查后删（保留 frame_idx + label 让 agent 知道删了什么）+ remove + write；加 2 个私有辅助函数 `validate_frame_idx_and_timestamp` 双路径校验 + timestamp 提取 / `write_sidecar` 替代 `BookmarkFile::write` 静默失败（失败返 false + warning 让 handler 透出）；`tests/test_mcp_v0_16.rs`（扩）18 case（list 5 + add 6 + edit 4 + delete 3） | `cargo test --release` 1317 passed（基线 1299 + 新测试 18）；`grep '"stub": true' src/mcp/handler/record.rs` 全清零（7 helper 全部去 stub）；集成测试 `test_bookmarks_*` 18 case 全过 | ✅ 全交付（commit `e2a36d4`）|

**结论**：stage 1-3 全部交付，无未交付项。cycle 业务代码累计 ~810 行（与主题 D2 预期 ~810-910 行对齐），新测试累计 +36（1281 → 1317）。

---

## 1. 六子项审查

### 1.1 代码质量

#### 1.1.1 stage 1 子 module 扩 record.rs：Strategy A 延续（rmcp 0.11 限制规避）

| 检查 | 实测 | 结果 |
|---|---|---|
| `handler/record.rs` 行数 | 801 行（7 个 Args struct ~70 行 + 7 个 helper ~530 行 + 5 私有辅助函数 ~180 行 + 模块 doc comment ~20 行）| ✅ |
| `handler/mod.rs` 行数 | 1455 行（v0.15 末 1358 + 7 个 #[tool] stub 方法 ~85 行 + use 声明 +1 行）| ✅（stage 1 §决策 1 Strategy A 延续）|
| `#[tool_router]` impl 块位置 | 主 mod.rs 单 impl 块（与 stage 1 §决策 1 一致，v0.15 4 子 module 同款）| ✅ |
| 39 个 `#[tool]` 方法都在主 mod.rs | `grep -c 'name = "proc_' src/mcp/handler/mod.rs` = 39 | ✅ |
| 39 个唯一 tool name | `grep -oE 'name = "proc_[a-z_]+"' mod.rs \| sort -u \| wc -l` = 39 | ✅ |
| 既有 32 tool 行为零回归 | stage 2/3 决策 6「impl 块结构不动」（仅替换 record.rs 内 stub helper，不动 mod.rs `#[tool]` 方法体）| ✅（基线 1281 → 1317 全过）|
| record.rs 子 module 命名与 `crate::record` 业务模块同名 | 巧合，Rust 模块系统天然区分（路径 `crate::mcp::handler::record::make_*` vs `crate::record::Player`），record.rs 内调业务模块用全限定 `crate::record::Player::open(...)` 避免 use 冲突（stage 1 doc 已记录已知风险 3）| ✅ |

**判定**：record.rs 子 module 落地正确，rmcp 0.11 `#[tool_router]` 不跨 module 收集 `#[tool]` 方法的限制延续规避（所有 7 个新 `#[tool]` 都在主 mod.rs impl 块，子 module 只放 Args struct + 业务 helper）。✅

#### 1.1.2 stage 2 replay + USB status 业务逻辑：双路径 + 4 档 suggestion 三大决策落地正确

| 检查 | 实测 | 结果 |
|---|---|---|
| `proc_replay_info` 双路径分发 | `record.rs::make_replay_info_json` 走 `is_vt100_file` 分发（VT100 → `VtPlayer::open + header + time_range_ms` 返 `format: "vt100"` / v3 → `Player::open + header + footer + time_range` 返 `format: "uiframe"`）| ✅ |
| `proc_replay_info` 共通字段 | `has_bookmarks_sidecar`（文件存在性 `<file_path>.bookmarks.json`）/ `path` / `size_bytes` 双路径都返 | ✅ |
| `proc_replay_search` 双入口（substring + FilterExpr）| `record.rs::make_replay_search_json` 走 `query.strip_prefix(':')` 分发：`Some(stripped)` → `parse_frame(stripped)`（FilterExpr 模式，5 维度 timestamp/cpu/mem/name/anomaly.severity）/ `None` → `build_frame_substring_expr(query)`（substring → `name =~ /<escaped>/i`）| ✅ |
| `proc_replay_search` 全帧遍历 + limit 截断 | stage 2 决策 5（默认 100 + truncated + 升序 + match_count / returned / truncated 三字段）| ✅ |
| `proc_replay_search` matched_processes 字段 | stage 2 决策 6（expr 含 name 约束走 `collect_name_matches` 精准匹配 / 否则全列帧内进程名 fallback）| ✅ |
| `proc_replay_search` anomaly_severity 字段 | 走 `highest_anomaly_severity`（critical > warning > info > 其他 / 空），帧无 anomaly 返 null | ✅ |
| `proc_replay_search` VT100 拒绝 | `record.rs` 顶部 `if is_vt100_file(path) → super::err("VT100 录屏不支持 search")`（VT100 无结构化帧，详见 ADR-0025a）| ✅ |
| `proc_eject_status` drive 字符 normalize | `record.rs::make_eject_status_json` 走 `chars().filter(is_ascii_alphabetic).next().to_ascii_uppercase()`（与 brainstorm §决策 9 + 既有 `make_eject_json` 同款，"E" / "E:" / "E:\\" 都接受）| ✅ |
| `proc_eject_status` 4 档 suggestion 决策树 | `unknown_drive`（drive 字符无效 / 找不到 removable device）/ `unavailable`（scan_all_devices / scan_device_locks 调用失败，含 `warning` 字段）/ `kill_locks`（找到设备 + locks 非空）/ `eject_now`（找到设备 + locks 空）| ✅（brainstorm §决策 9）|
| `proc_eject_status` ejectable = lock_count == 0 | stage 2 决策（cache_status 字段砍掉——`flush_write_cache` 阻塞 3s+ 不适合 MCP 一次性 request-response）| ✅ |

**判定**：stage 2 replay + USB status 业务逻辑落地正确，3 大决策（双路径 / 双入口 / 4 档 suggestion）全部生效。✅

#### 1.1.3 stage 3 bookmarks 业务逻辑：4 helper 走 BookmarkFile 业务 API 路径稳定

| 检查 | 实测 | 结果 |
|---|---|---|
| `proc_bookmarks_list` sidecar_present + source_healthy 双字段三态 | stage 3 决策 1（无 sidecar → false / true / []；fresh → true / true / loaded；stale → true / false / []）| ✅ |
| `proc_bookmarks_list` try_load 区分 fresh vs stale | 走 `BookmarkFile::try_load(path)` 返 `Option<BookmarkFile>`，None 时根据 `sidecar_present` 决定 `source_healthy` | ✅ |
| `proc_bookmarks_add` 双路径 frame_idx 校验 | stage 3 决策 2（v3 用 `Player::total_frames + frame_at(frame_idx).timestamp` / VT100 用 `VtPlayer::total_frames + time_range_ms 内插`，total=1 兜底避免除零）| ✅ |
| `proc_bookmarks_add` label 默认值 | `Some(s) if !s.is_empty() → s` / 否则 → `format!("书签 #{}", file.bookmarks.len() + 1)`（与 stage 2 id 算法对齐）| ✅ |
| `proc_bookmarks_add` dry_run 路径 | `dry_run=true` 仍调 add（计算真实 id）但不写 sidecar；`dry_run=false` 调 add + write_sidecar，失败时返 `sidecar_written=false + warning`（决策 3 + brainstorm §决策 7）| ✅ |
| `proc_bookmarks_edit` 先查后改（保留 old_label） | stage 3 决策 4（让 agent 看 diff，避免重复遍历；调 `BookmarkFile::edit_label` 仍返 bool 但 step 1 已确认 id 存在，debug_assert 兜底）| ✅ |
| `proc_bookmarks_delete` 先查后删（保留 frame_idx + label） | stage 3 决策 5（让 agent 知道删了什么，调 `BookmarkFile::remove` 仍返 bool 但 step 1 已确认 id 存在，debug_assert 兜底）| ✅ |
| `write_sidecar` 替代 `BookmarkFile::write` 静默失败 | stage 3 决策 3（返 `(bool, Option<String>)` 让 handler 在 JSON 顶层加 `warning` 字段透出错误）| ✅ |
| 4 个 bookmarks tool description 去 "Stage 1 stub" | `grep "Stage 1 stub" src/mcp/handler/mod.rs` 全清零 | ✅ |

**判定**：stage 3 bookmarks 4 helper 业务路径稳定，决策 1-6 全部落地。✅

#### 1.1.4 测试覆盖度：cycle 累计 +36 新测试

| Stage | 新测试数 | 累计基线 | 覆盖维度 |
|---|---|---|---|
| stage 1 | +0 | 1281 → 1281 | 仅加 stub 不动业务代码，无新测试（与 stage 1 §决策 1 surgical 原则一致）|
| stage 2 | +18 | 1281 → 1299 | replay_info 5 case（v3 + VT100 + missing + invalid + sidecar existence）+ replay_search 7 case（substring + FilterExpr cpu/mem + 默认 limit 100 + 自定义 limit + VT100 拒绝 + 无效表达式）+ eject_status 6 case（empty drive + non-alpha + invalid letter + normalize + JSON shape + suggestion 4 档枚举）|
| stage 3 | +18 | 1299 → 1317 | bookmarks_list 5 case（无 sidecar + fresh + stale + missing + VT100）+ bookmarks_add 6 case（默认 label + 空 label + 显式 label + dry_run + 真实写盘 + frame_idx 越界）+ bookmarks_edit 4 case（existing + non-existing + dry_run + missing）+ bookmarks_delete 3 case（existing + non-existing + dry_run）|
| **cycle 累计** | **+36** | **1281 → 1317** | **全维度覆盖** |

**判定**：cycle 测试覆盖度优秀，每 stage 测试数与新增功能复杂度匹配。stage 1 仅加 stub 不加测试是 surgical 原则的体现（与 v0.15 stage 1 同款规则）。✅

---

### 1.2 架构审查

#### 1.2.1 MCP 模块改动范围最小

| 检查 | 实测 | 结果 |
|---|---|---|
| stage 1 改动文件 | `src/mcp/handler/{mod.rs(顶部 +pub mod record; + use record::*; + impl 块末尾追加 7 个 #[tool] stub 方法 ~85 行), record.rs(新 ~220 行 = 7 Args struct + 7 stub helper + 模块 doc comment)}` + `docs/adr/{0025a, 0025b}.md` + `CONTEXT.md`（本地）+ `docs/stages/v0.16-stage-1.md` + `docs/stages/v0.16-brainstorm.md` | ✅（仅 src/mcp/handler/ + docs/adr/ + docs/stages/，不污染业务模块）|
| stage 2 改动文件 | `src/mcp/handler/record.rs(stub helper 替换为真实业务：3 helper 替换 + 2 私有辅助函数新增)` + `tests/test_mcp_v0_16.rs(新 18 case)` + `docs/stages/v0.16-stage-2.md` + `CHANGELOG.md`（[Unreleased] 段）| ✅（仅 src/mcp/handler/record.rs + tests/）|
| stage 3 改动文件 | `src/mcp/handler/record.rs(stub helper 替换为真实业务：4 helper 替换 + 2 私有辅助函数新增 + 同步清理 stage 1 残留 stub doc comment 引用)` + `tests/test_mcp_v0_16.rs(扩 18 case + write_sidecar_with_bookmarks fixture helper)` + `docs/stages/v0.16-stage-3.md` + `CHANGELOG.md`（[Unreleased] 段）| ✅（仅 src/mcp/handler/record.rs + tests/）|

**判定**：3 个 stage 改动范围都最小，仅 src/mcp/handler/record.rs + tests/，不污染业务模块。✅

#### 1.2.2 ProcMcpHandler 字段 / Clone / Default / new 是否破坏 v0.12 TD-36 持久 dns_collector 契约

| 检查 | 实测 | 结果 |
|---|---|---|
| stage 1-3 是否动 mod.rs struct 定义 | 仅 impl 块末尾追加 7 个 `#[tool]` 方法（stage 1）/ 不动 struct 字段 / Clone / Default / new | ✅ |
| `dns_collector: Arc<Mutex<Option<Box<dyn DnsLogCollector>>>>` 字段保留 | v0.16 cycle 未动 mod.rs struct 定义，字段仍在 mod.rs（v0.12 TD-36 fix 不动）| ✅ |
| `Clone` derive 共享 collector | rmcp 内部 clone handler 时共享同一 collector 实例 | ✅ |
| `new()` 调 `detect_collector()` spawn 一次 | 生产入口（serve 调）spawn ETW / PowerShell（v0.16 cycle 未动）| ✅ |
| `Default` 保持 `None` 不强制 spawn | 测试路径不 spawn ETW / PowerShell | ✅（与 v0.12 TD-36 同款规则）|

**判定**：v0.12 TD-36 持久 dns_collector 契约零回归。✅

#### 1.2.3 record.rs 子 module 命名与 crate::record 业务模块同名

| 检查 | 实测 | 结果 |
|---|---|---|
| 子 module 路径 | `crate::mcp::handler::record`（MCP 子 module 容器）| ✅ |
| 业务模块路径 | `crate::record`（录屏 v2 业务模块）| ✅ |
| Rust 模块系统天然区分 | 路径前缀不同（`mcp::handler::record` vs `record`），编译器无歧义 | ✅ |
| record.rs 内调业务模块用全限定 | `crate::record::Player::open(...)` / `crate::record::BookmarkFile::load_or_empty(...)` / `crate::record::is_vt100_file(...)` / `crate::record::UiFrame` 等，避免 `use crate::record::*;` 与本子 module 名字冲突 | ✅（stage 1 doc 已知风险 3 文档化）|

**判定**：record.rs 子 module 命名冲突通过全限定路径规避，Rust 模块系统天然区分。✅

---

### 1.3 性能审查

#### 1.3.1 proc_replay_search 全帧遍历 ~9s/30min session

| 检查 | 实测 | 结果 |
|---|---|---|
| 全帧遍历 cost | 30 min × 30 FPS × 165 µs/frame = ~9 秒（与 ADR-0025a + brainstorm §proc_replay_search 性能段同款估算）| ⚠ **已知限制**（详见 ADR-0025a §性能段）|
| agent 实际场景 | agent 一次性调用，agent 低频调用 search（典型 task 调 1-2 次）| ✅ |
| MCP 协议 timeout | 主流 client（Claude Desktop / Cursor / mcp-inspector）默认 timeout 通常 30s+，9 秒可接受 | ✅（brainstorm §Q3）|
| stage 4 Review 决策 | 评估是否加 `offset: Option<usize>` 分页参数（暂留 v0.17+ cycle 候选，与 brainstorm §决策 5 同款推迟理由）| 见 §3 |

**判定**：proc_replay_search 全帧遍历 ~9s 是已知限制（ADR-0025a 文档化），agent 低频调用可接受。✅

#### 1.3.2 proc_replay_info 双路径 Player::open 延迟

| 检查 | 实测 | 结果 |
|---|---|---|
| v3 Player::open 按需加载 | v0.14 stage 1 落地（LRU 单帧缓存 + IdxSidecar v1/v2 兼容层），30 min × 1000 进程 < 100ms | ✅（PERF-BASELINE TD-45 闭环）|
| VT100 VtPlayer::open | 直接 mmap 整文件读 header，< 50ms | ✅ |
| `is_vt100_file` 检查 | 读 magic 4 字节，< 1ms | ✅ |

**判定**：proc_replay_info 双路径 Player::open 延迟受 v0.14 stage 1 按需加载优化保护，无性能瓶颈。✅

#### 1.3.3 proc_eject_status PowerShell 子进程开销

| 检查 | 实测 | 结果 |
|---|---|---|
| `scan_all_devices` 调 PowerShell | ~500ms-1s（与 v0.7 既有 `proc_eject` 同款开销）| ⚠ **已知限制**（与 v0.7 既有 tool 同款，brainstorm §决策 9 不调 `flush_write_cache`）|
| `scan_device_locks` 调 filelocksmith COM | ~200-500ms（与 v0.7 既有 `proc_eject` 同款）| ⚠ **已知限制** |
| `proc_eject_status` 总延迟 | ~1-1.5s（agent 调用可接受）| ✅ |

**判定**：proc_eject_status PowerShell 子进程开销与 v0.7 既有 `proc_eject` 同款（不调 `flush_write_cache` 阻塞 3s+），agent 调用可接受。✅

#### 1.3.4 39 tool 启动开销

| 检查 | 实测 | 结果 |
|---|---|---|
| rmcp `#[tool_router]` 编译期宏 | runtime 不扫描 tool 列表（与 v0.7 17 tool + v0.15 32 tool 同款开销）| ✅ |
| 新增 7 tool 不影响 MCP server 启动延迟 | stage 1 落地后 `cargo run --release -- mcp serve` 启动延迟与 v0.15 同 | ✅ |
| 每个 tool lazy 调用 | agent 不调不耗资源 | ✅（brainstorm §Q6）|

**判定**：39 tool 启动开销与 v0.7 17 tool / v0.15 32 tool 同（编译期宏），无 runtime 扫描开销。✅

---

### 1.4 完整性检查

#### 1.4.1 brainstorm.md cycle 总览表

| 检查 | 实测 | 状态 |
|---|---|---|
| 阶段总览表反映 4 stage（1 Spike + 2 Slice + 1 Review+收尾） | `docs/stages/v0.16-brainstorm.md:106-109` 4 stage 都列；stage 1/2/3 全 ✅，stage 4 行缺 ✅ | ⬜ stage 4 完工时改（任务 7，与 P1-1 stage 1 doc ✅ 同款修复时机）|
| cycle 决策（拍板记录）段 | brainstorm §决策 1-9 完整（record 不暴露 / 7 tool 范围 / 4 stage 节奏 / dry_run=false / search limit + truncated / file_path 安全 / sidecar 写失败兜底 / handler 子 module 扩 record.rs / USB status tool 设计）| ✅ |
| 7 tool 详细范围段 | brainstorm §6 个 tool 详细范围（实际 7 tool——含 USB status 用户追加）| ✅（标题「6 个 tool」是 brainstorm 起草阶段非正式 miscount，实际 7 tool——已通过 §决策 2 + §v0.16 cycle 实际范围表 + USB status 段独立列出文档化）|

**判定**：1 个完整性问题（stage 4 行缺 ✅），stage 4 收尾段任务 7 修复。无 miscount 需更正（brainstorm 起草阶段已含 USB status 追加段）。✅

#### 1.4.2 stage docs 头部 ✅ 标记

| 检查 | 实测 | 状态 |
|---|---|---|
| `docs/stages/v0.16-stage-1.md` 头部 ✅ | 第 1 行标题末尾有 ✅（`### 阶段 1：Spike — ... ✅`），但**缺独立 `> ✅ **已完成**` 行**（stage 2/3/4 都有独立行）| ❌ **P1-1** |
| `docs/stages/v0.16-stage-2.md` 头部 ✅ | 第 3 行 `> ✅ **已完成**（v0.16.0 阶段 2 会话产出，2026-07-07）` | ✅ |
| `docs/stages/v0.16-stage-3.md` 头部 ✅ | 第 3 行 `> ✅ **已完成**（v0.16.0 阶段 3 会话产出，2026-07-07）` | ✅ |
| `docs/stages/v0.16-stage-4.md` 头部 ✅ | 第 3 行 `> ✅ **已完成**（v0.16.0 阶段 4 会话产出，2026-07-07）`（本会话产出）| ✅ |

**P1-1**：stage 1 doc 头部缺独立 `> ✅ **已完成**` 行（标题末尾有 ✅，但与 stage 2/3/4 格式不一致）。与 v0.14 stage 5 P1-1 / v0.15 stage 4 P1-1 同款问题（cycle 末段 Review 时发现 stage 1 doc 头部 ✅ 标记漏加）。stage 4 收尾段任务 8 修复。

**判定**：1 个 P1（stage 1 doc 头部 ✅ 缺），stage 4 收尾段任务 8 修复。✅

#### 1.4.3 CHANGELOG `[Unreleased]` 段

| 检查 | 实测 | 状态 |
|---|---|---|
| `[Unreleased]` 段 | `CHANGELOG.md:8-29` 含 v0.16.0 cycle 主题段 + stage 1/2/3 落地条目 + v0.17+ 候选方向占位 | ⬜ 待 stage 4 收尾段任务 4 改 `[0.16.0] - 2026-07-07` + 加阶段汇总 + 关键数字表 + `[Unreleased]` 改 v0.17 候选方向 |
| `[0.15.0]` 段保留（v0.15 cycle 历史） | `CHANGELOG.md:31` 起 `[0.15.0] - 2026-07-06` | ✅ |

**判定**：CHANGELOG `[Unreleased]` 段 stage 1-3 各阶段单独加条目（v0.15 cycle 各 stage 未单独加是 cycle 末段统一收尾模式，v0.16 cycle 改为各 stage 落地时即加 + stage 4 收尾段统一改为 `[0.16.0]`）。stage 4 收尾段任务 4 一次性改 `[Unreleased]` → `[0.16.0]` + 加阶段汇总 + 关键数字表。✅

#### 1.4.4 README MCP 章节

| 检查 | 实测 | 状态 |
|---|---|---|
| MCP 章节提 v0.15 落地的 32 tool | `README.md:5` 当前 banner 是 v0.15.0 + MCP 段落（v0.15 落地时已加） | ✅ |
| README banner v0.16.0 段 | 缺 | ⬜ 待 stage 4 收尾段任务 6 加 v0.16.0 banner |
| MCP 章节扩 39 tool 列表 | 当前 32 tool 列表 | ⬜ 待 stage 4 收尾段任务 6 加 7 新 tool 列表（按 Replay/Bookmarks/UsbStatus 3 类分组）|

**判定**：README banner / MCP 章节 v0.16 内容缺失，stage 4 收尾段任务 6 加 v0.16.0 banner + 39 tool 列表。✅

#### 1.4.5 CONTEXT.md 演进历史段

| 检查 | 实测 | 状态 |
|---|---|---|
| v0.16.0 段存在 | `CONTEXT.md:212` `### v0.16.0 新增术语（开发中，2026-07-07 启动）` + `CONTEXT.md:225` `### v0.16.0 落地变更（开发中，2026-07-07 启动）` | ✅（stage 1 落地时已加）|
| stage 1 行 | `CONTEXT.md:229` 完整描述 stage 1（4 术语 McP RecordCategory / ReplaySearchQuery / BookmarkAction / EjectSuggestion + record.rs 子 module 骨架 + 7 tool stub + ADR-0025a/0025b）| ✅ |
| stage 2 行 | 缺 | ⬜ 待 stage 4 收尾段任务 9 加（本地不入 commit）|
| stage 3 行 | 缺 | ⬜ 待 stage 4 收尾段任务 9 加（本地不入 commit）|
| stage 4 行 | 缺 | ⬜ 待 stage 4 收尾段任务 9 加（本地不入 commit）|
| 术语段状态升级（开发中 → 已落地） | 当前仍是「开发中，2026-07-07 启动」 | ⬜ 待 stage 4 收尾段任务 9 改「已落地，2026-07-07 发布」（本地不入 commit）|

**判定**：CONTEXT.md 演进历史段 stage 1 行齐全（stage 1 落地时已加），stage 2/3 行未单独加（与 v0.15 cycle stage 2/3 同款模式，cycle 末段统一收尾时补）。stage 4 收尾段任务 9 加 stage 2/3/4 行 + 状态升级。✅

#### 1.4.6 tech-debt.md

| 检查 | 实测 | 状态 |
|---|---|---|
| v0.17.0+ 候选段 | 当前 tech-debt.md 含 v0.16.0+ 候选补遗段 TD-50~54（v0.15 cycle 归档），无 v0.16 cycle 新 TD 候选 | ⬜ 待 stage 4 Review §3 决策（如有 TD-55+ 候选）|
| TD-44~54 终态 | 全部归档正确（v0.13 归档 5 项 TD-44~48 + v0.14 归档 1 项 TD-49 + v0.15 归档 5 项 TD-50~54）| ✅ |

**判定**：tech-debt TD-44~54 终态正确，stage 4 §3 决策无新 TD-55+ 候选（v0.16 cycle 量级偏轻 + brainstorm 决策段已穷尽未来考虑项）。✅

---

### 1.5 安全 / 跨平台审查

#### 1.5.1 file_path 参数路径遍历防护

| 检查 | 实测 | 结果 |
|---|---|---|
| 不做白名单 / 黑名单 | brainstorm §决策 6（agent 视角已通过 v0.7 ADR-0009 字段裁剪 + 用户授权天然约束）| ✅ |
| 文件不存在 / 损坏返友好 error | `record.rs` 6 个 file_path 入参 helper 顶部统一 `if !path.exists() → super::err(format!("录屏文件不存在: {file_path}"))`（list/add/edit/delete）/ `match std::fs::metadata(path) { Err(e) → err(...) }`（info）/ `match Player::open(path) { Err(e) → err(...) }`（search）| ✅ |
| 录屏本体不存在时 sidecar 操作返 ok=false | brainstorm §决策 6（不返"空书签列表"，避免 agent 误以为新建空录屏）| ✅ |
| MCP server 跑在用户本地权限下 | 与 `proc replay` CLI 同款，agent 不能突破用户权限 | ✅ |

**判定**：file_path 路径遍历防护通过 OS / 用户授权层（不在 MCP 层重复实现，surgical 原则）。✅

#### 1.5.2 dry_run=false 默认

| 检查 | 实测 | 结果 |
|---|---|---|
| bookmarks add/edit/delete dry_run=false 默认 | `record.rs` 3 helper 都 `let dry_run = dry_run.unwrap_or(false);`（stage 3 决策 + brainstorm §决策 4）| ✅ |
| `dry_run=true` opt-in 预演 | stage 3 测试 `test_bookmarks_add_dry_run_does_not_write_sidecar` / `test_bookmarks_edit_dry_run_does_not_modify_sidecar` / `test_bookmarks_delete_dry_run_keeps_bookmark` 验证 | ✅ |
| 与 v0.7 proc_kill / proc_pkill + v0.15 proc_monitor_add/remove 契约一致 | stage 1-3 未动既有 tool，dry_run=false 默认模式延续 | ✅ |

**判定**：写操作 dry_run=false 默认落地正确，与既有写操作契约一致。✅

#### 1.5.3 sidecar 写失败兜底

| 检查 | 实测 | 结果 |
|---|---|---|
| `write_sidecar` helper 替代 `BookmarkFile::write` 静默失败 | stage 3 决策 3 + brainstorm §决策 7（返 `(bool, Option<String>)`，序列化 / IO 失败时返 false + warning）| ✅ |
| 失败时返 `ok: true + sidecar_written: false + warning` | bookmarks add/edit/delete 3 helper 在 write_sidecar 失败时插入 `warning` 字段（业务逻辑成功 in-memory BookmarkFile 已更新 + 写盘失败警告）| ✅ |
| agent 决定是否重试或上报 | brainstorm §决策 7（agent 视角看到 warning 字段即可决定下一步）| ✅ |

**判定**：sidecar 写失败兜底落地正确，agent 不再被静默失败误导。✅

#### 1.5.4 proc_eject_status 平台差异兜底

| 检查 | 实测 | 结果 |
|---|---|---|
| 非 Windows / `scan_all_devices` 失败返 suggestion="unavailable" | `record.rs::make_eject_status_json` 走 `match crate::eject::scan_all_devices() { Err(e) → suggestion="unavailable" + warning }` | ✅（brainstorm §决策 9 平台兜底）|
| drive 字符无效 / 非 removable 返 suggestion="unknown_drive" | `record.rs` 走 `cleaned.chars().next()` 检查 + `devices.iter().find()` 检查 | ✅ |
| `ejectable = lock_count == 0` 简化决策 | cache_status 字段砍掉（`flush_write_cache` 阻塞 3s+ 不适合 MCP）| ✅ |
| `is_removable: true` 字段必返 | 让 agent 区分「USB 设备」vs「固定硬盘」（drive 字符可能误传）| ✅ |

**判定**：proc_eject_status 平台差异兜底全部落地（unavailable / unknown_drive / kill_locks / eject_now 4 档 suggestion 决策树），与 brainstorm §决策 9 一致。✅

#### 1.5.5 mod.rs 顶部 mod 声明 pub mod record（让测试能 import）

| 检查 | 实测 | 结果 |
|---|---|---|
| stage 1 落地时 mod 声明是 `pub mod record;` | stage 1 doc §决策 1（一开始就 pub mod 让测试能 import，吸取 v0.15 stage 1 私有 mod → stage 2 改 pub mod 的教训）| ✅ |
| production 路径影响 | 仅暴露 module 给 tests，production 路径调用方仍走 `crate::mcp::handler::*` re-export | ✅（surgical，吸取 v0.15 教训）|

**判定**：mod 声明从一开始就 pub mod 是 stage 1 吸取 v0.15 教训的最小调整，不影响 production 路径。✅

---

### 1.6 P0 / P1 / P2 列表

#### P0（阻断 v0.16.0 发布）：0 项

无。cycle 业务代码 +36 测试全过，fmt / clippy / build / bench 全过，无编译 / 测试 / 关键文档阻断问题。

#### P1（cycle 内闭环）：1 项

| 编号 | 问题 | 修复 |
|---|---|---|
| **P1-1** | `docs/stages/v0.16-stage-1.md` 头部缺独立 `> ✅ **已完成**` 行（标题末尾有 ✅，但与 stage 2/3/4 格式不一致），与 v0.14 stage 5 P1-1 / v0.15 stage 4 P1-1 同款问题 | stage 4 收尾段任务 8 加 ✅ 标记（在标题下方插入独立行）|

#### P2（归档 v0.17+ cycle）：0 项

无。v0.16 cycle 量级偏轻（~810 行 vs v0.15 ~1700 行）+ brainstorm 决策段已穷尽未来考虑项（record 不暴露 ADR-0025b / search offset 分页 / cache_status 字段 / VT100 replay search 支持 / USB release 一次完成 / docker-rm 写操作 等），无新 TD-55+ 候选归档。

**与 v0.15 cycle P2-1~5 对比**：v0.15 cycle 归档 5 个 TD（proc_metrics_smart vs proc_smart 入口重叠 / MonitorManager 无持久化 / metrics sparkline 历史不暴露 / per-process disk_io 不暴露 / 多次调用 SystemSnapshot 累积开销），v0.16 cycle 量级偏轻且 brainstorm 决策段已穷尽未来考虑项，无新 TD 归档。

---

## 2. P1 修复方案

### P1-1：stage 1 doc 头部加独立 ✅ 标记

在 `docs/stages/v0.16-stage-1.md` 第 1 行（`### 阶段 1：Spike — ...` 行）下面插入：

```markdown

> ✅ **已完成**（v0.16.0 阶段 1 会话产出，2026-07-07）
```

**修复位置**：`docs/stages/v0.16-stage-1.md:1` 后插入独立 ✅ 行（与 stage 2/3/4 同款格式）。

**注意**：stage 1 doc 头部 ✅ 标记修复不动业务代码（仅 docs/* 改动），与 v0.14 stage 5 P1-1 / v0.15 stage 4 P1-1 同款规则。

---

## 3. P2 归档

**v0.16 cycle 无新 TD-55+ 候选**。理由：

1. **量级偏轻**：v0.16 cycle ~810 行业务代码（vs v0.15 ~1700 行），新增功能范围紧凑（7 tool 集中在录屏 v2 + USB status），无新性能瓶颈 / 架构债 / 完整性缺位
2. **brainstorm 决策段已穷尽未来考虑项**：
   - record 不暴露 → ADR-0025b 文档化（spawn 子进程 / worker 持续采样 / MCP-level confirm 评估留 v0.17+ cycle）
   - search offset 分页 → brainstorm §决策 5（v0.17+ cycle 评估，与 brainstorm FAQ Q4 同款推迟理由）
   - cache_status 字段 → brainstorm §决策 9 砍掉（`flush_write_cache` 阻塞 3s+ 不适合 MCP，未来 `proc_usb_flush_cache` 候选）
   - VT100 replay 不支持 search → ADR-0025a + brainstorm §Q4（与 TD-49 VT100 字节流转码 UiFrame 同款方向，留主题 F cycle 评估）
   - USB release tool → brainstorm §决策 9（`proc_usb_release(drive, kill_pids, dry_run=false)` 用户设计偏好留 v0.17+ cycle）
   - docker-rm 写操作 → brainstorm §主题 D2 + §Q1（agent 低频推迟，留 v0.17+ cycle 评估）
3. **既有 TD-50~54 终态不变**：v0.15 cycle 归档的 5 个 TD 仍待 v0.17+ cycle 评估（v0.16 cycle 未触及），无需新归档

**v0.17+ 候选方向**（cycle 末段总结时给方向建议，用户最终拍板）：
- **主题 B**：可观测性 cycle（rmcp Resource subscribe / SSE transport / 实时流，与 TD-52 sparkline 历史同款方向）
- **主题 A**：性能优化 cycle（TD-54 MCP handler 内 SystemSnapshot/App 复用 + TD-44~47 残留 PERF-BASELINE）
- **主题 F**：VT100 replay 增强 cycle（TD-49 字节流转码 UiFrame / 反向解释器，让 VT100 录屏也支持 search / 倒放）
- **record 暴露评估**（spawn `proc record` 子进程 / worker 持续采样 / MCP-level confirm 机制，与本 cycle 决策 1 + ADR-0025b 同款推迟理由）
- **USB release / docker-rm 写操作 cycle**（`proc_usb_release(drive, kill_pids, dry_run=false)` 一次完成 kill + flush + eject / `proc_docker_rm` 系列）

---

## 4. 验收

### 4.1 全量回归

`cargo test --release -q` = **1317 passed / 0 failed / 3 ignored**（v0.15.0 → v0.16 stage 1 → stage 2 → stage 3 全程基线递增 1281 → 1281 → 1299 → 1317，cycle 累计 +36 新测试）。

理由：v0.16 cycle 3 个 stage 全部交付业务代码 + 测试，每个 stage 测试数与新增功能复杂度匹配（stage 1 仅加 stub 不加测试是 surgical 原则）。

### 4.2 静态检查

| 检查 | 命令 | 结果 |
|---|---|---|
| 格式化 | `cargo fmt --all -- --check` | ✅ 通过 |
| Clippy | `cargo clippy --release --all-targets -- -D warnings` | ✅ 通过 |
| 无默认 feature 构建 | `cargo build --release --no-default-features` | ✅ 通过 |
| Bench 编译 | `cargo bench --no-run` | ✅ 通过 |

### 4.3 P0 / P1 / P2 闭环

- **P0 = 0** ✓
- **P1 = 1**（P1-1 stage 1 doc 头部 ✅ 标记修复——见 §2 修复方案，stage 4 收尾段任务 8 修复）
- **P2 = 0**（v0.16 cycle 无新 TD 候选——见 §3）

---

## 5. 后续（stage 4 收尾段 + cycle 闭环）

stage 4 Review 段（本文）完工后，stage 4 收尾段任务（按 stage 4 doc §任务清单）：

1. **CHANGELOG.md**：`[Unreleased]` → `[0.16.0] - 2026-07-07` + 4 stage 阶段汇总 + 关键数字表（32 → 39 tool / 36 新测试 / 1317 全量回归 / ~810 行业务代码）
2. **Cargo.toml**：`0.15.0` → `0.16.0` + Cargo.lock 自动同步
3. **README.md**：banner 加 v0.16.0 段（5 大能力：record.rs 子 module + replay 2 tool + bookmarks 4 tool + USB status 1 tool + record 不暴露决策）+ MCP 章节扩 39 tool 列表
4. **brainstorm.md**：cycle 阶段总览表 stage 4 行 ⬜ → ✅（任务 7）+ 末尾加 cycle 总结段
5. **stage 1 doc 头部 ✅**（P1-1 修复，含 stage 4 doc 本身已有）
6. **CONTEXT.md**：演进历史加 stage 2/3/4 行 + 状态升级（本地，不入 commit）
7. **commit**：`release(v0.16.0): MCP 全功能暴露录屏 v2 + 操作类 cycle（4 stage 全交付 + REVIEW-v0.16 + tag v0.16.0）`
8. **git tag v0.16.0**：等用户确认 push（与 v0.14.0 / v0.15.0 同款规则）

---

## 6. 总结

v0.16 cycle 是「MCP 全功能暴露 cycle 第二弹（录屏 v2 + 操作类）」（主题 D 子方向 D2，4 stage 中轻 cycle）：
- **stage 1** Spike（commit `bf71d5d`）：MCP handler 子 module 扩第 5 个文件 record.rs — 7 tool Args struct + 7 stub helper + ADR-0025a（replay_search agent schema）+ ADR-0025b（record 不暴露）
- **stage 2** Slice（commit `2e5a1a6`）：replay + USB status 业务逻辑填充（3 tool）+ 18 集成测试
- **stage 3** Slice（commit `e2a36d4`）：bookmarks 业务逻辑填充（4 tool）+ 18 集成测试
- **stage 4** Review + 收尾（commit 待）：本 Review + 收尾 + tag v0.16.0

**核心结论**：MCP 模块从 v0.15 的「32 tool 查询类全功能透出」升级到「39 tool + 录屏 v2 + USB status」。agent 视角录屏 v2 4 大能力（v3 footer / 书签 / 时间轴搜索 / 倒放）+ v0.7 USB 弹盘业务模块全部透出——agent 可调 `proc_replay_info` 查录屏元数据 / `proc_replay_search` FilterExpr 5 维度搜命中帧 / `proc_bookmarks_*` CRUD 书签 / `proc_eject_status` 查 USB 弹盘状态 + 4 档 suggestion 决策。**用户痛点「U 盘异常 kill 进程后不知道是否成功」通过 `proc_eject_status → proc_kill → proc_eject_status` 三步反馈循环解决**。record 启动 / docker-rm 写操作 / USB release 一次完成 留 v0.17+ cycle 评估（详见 ADR-0025b + brainstorm §决策 9）。

**cycle 数据**：
- 全量回归：1281 passed（v0.15.0 基线）→ 1317 passed（v0.16.0 落地），+36 新测试
- 业务代码：~810 行（与主题 D2 预期 ~810-910 行对齐）
- MCP tool 总数：32 → 39（32 v0.15 既有 + 7 v0.16 新增）
- handler 子 module：4 文件（mod / cli / inspect / metrics）→ 5 文件（+ record.rs 801 行）

**REVIEW-v0.16 完工交付**：
- 本报告（~330 行）
- P1 修复（1 项：stage 1 doc 头部 ✅，stage 4 收尾段任务 8 修复）
- 0 P2 归档（v0.16 cycle 无新 TD 候选——量级偏轻 + brainstorm 决策段已穷尽未来考虑项）
- stage 4 收尾段（CHANGELOG + Cargo + README + brainstorm + CONTEXT + git tag v0.16.0）
- stage 1-4 docs 头部 ✅（含 stage 4 doc 本身）
- v0.17.0 cycle 启动指引（基于 v0.16 落地情况 + TD-50~54 残留 + brainstorm §决策 1/9 同款推迟理由 + 主题 B/A/F 候选方向）

**v0.17.0 候选方向**（stage 4 收尾段总结时给方向建议，用户最终拍板）：
- 主题 B：可观测性 cycle — rmcp Resource subscribe / SSE transport / 实时流（与 TD-52 sparkline 历史同款方向）
- 主题 A：性能优化 cycle — TD-54（MCP handler 内 SystemSnapshot / App 复用）+ TD-44~47 残留（PERF-BASELINE）
- 主题 F：VT100 replay 增强 cycle — TD-49（VT100 字节流转码 UiFrame / 反向解释器）
- record 暴露评估 — spawn `proc record` 子进程 / worker 持续采样 / MCP-level confirm 机制（与本 cycle ADR-0025b 同款推迟理由）
- USB release / docker-rm 写操作 cycle — `proc_usb_release(drive, kill_pids, dry_run=false)` 一次完成 kill + flush + eject / `proc_docker_rm` 系列
