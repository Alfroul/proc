# ADR-0028：VT100 字节流转码 UiFrame 路径（临时转码）

**Status**：Accepted
**Date**：2026-07-07（v0.17.0 阶段 1 落地决策，brainstorm 决策 6 拍板方案 a 临时转码）
**Related**：v0.14 TD-49 归档（VT100 replay 路径无倒放 / 搜索）、v0.6 落地的 VtPlayer 正向 replay 路径、ADR-0011（FilterExpr 5 维度 FrameField）

## 背景（Context）

v0.6 落地的 VT100 录屏（`.prec` 文件 v2 格式）走字节流录制——`VtRecorder` 喂 VT100 字节流（CSI / SGR / cursor move / clear 序列）到 `.prec` 文件，`VtPlayer` 正向 replay 时按帧索引切片字节流 + 用 `vt100` crate 解析为屏幕 buffer 渲染。

v0.14 cycle 落地录屏 v2 4 大能力（v3 footer + 书签 + 时间轴搜索 + 倒放），但仅适用于 v3 UiFrame 格式（结构化帧索引）——VT100 字节流无结构化帧索引，倒放需反向解释器（~1000+ 行，clear / cursor move / SGR 反向应用），search 无法 apply FilterExpr（5 维度 timestamp / cpu / mem / name / anomaly.severity 都依赖 UiFrame 结构）。

v0.14 TD-49 归档「VT100 replay 路径无倒放 / 搜索」，留 v0.17 cycle 主题 F 评估。两条路径：

- **方案 (a) 临时转码**：VT100 → UiFrame 转换器 + 临时文件 + 走 v3 Player 路径
- **方案 (b) 永久转码**：转码并写新 `.v3` 文件，后续回放走 v3 路径
- **方案 (c) 仅 VT100 反向解释器**：~1000+ 行独立实装，不转码

brainstorm 决策 6 用户拍板方案 (a) 临时转码（本 ADR 落地）。

## 决策（Decision）

**`Vt100ToUiFrameConverter` 增量解析 + 累积屏幕 buffer + 30 FPS 切片为 UiFrame + 临时文件管理**：

### 1. 转换器结构（`src/record/vt100_to_uiframe.rs`）

```rust
pub struct Vt100ToUiFrameConverter {
    // stage 5 实装：
    // - 屏幕 buffer（vt100 crate::Screen 累积）
    // - 当前 SGR 状态（前景 / 背景 / 加粗 / 斜体等）
    // - 30 FPS 切片定时器（与 v0.6 VtRecorder 30 FPS 同款节奏）
    // - 帧计数器 + 起始时间戳
}

impl Vt100ToUiFrameConverter {
    pub fn new() -> Self;
    pub fn feed_bytes(&mut self, bytes: &[u8]) -> Result<(), String>;
    pub fn snapshot_frame(&self) -> Result<UiFrame, String>;
}
```

stage 1 Spike 仅声明 struct + 三方法 stub（返 "v0.17-stage-5 未实装" 错误）。stage 5 Slice 实装增量解析 + 累积屏幕 buffer + 30 FPS 切片为 UiFrame。

### 2. 临时转码路径（`proc replay <file>` 自动检测）

```
proc replay recording.prec
  ├─ is_vt100_file(file) == true ?
  │   ├─ YES → Vt100ToUiFrameConverter 转码到 <file>.tmp.v3
  │   │        ├─ Player::open(<file>.tmp.v3) 走 v3 路径
  │   │        ├─ replay 完成后删 <file>.tmp.v3
  │   │        └─ 转码失败 → fallback 走 VtPlayer 正向 replay
  │   └─ NO  → Player::open(file) 走 v3 路径（既有行为）
```

MCP `proc_replay_info` / `proc_replay_search` 双路径也自动透明转码——agent 调用时无需关心文件格式。

### 3. VT500 序列解析器扩（`src/record/vt100.rs`）

v0.6 落地的 `VtPlayer` 仅做正向 replay（按帧索引切片字节流 + `vt100` crate 解析）。stage 5 实装时需扩 VT500 序列反序列化能力——CSI / SGR / cursor move / clear 全套反序列化，让 `Vt100ToUiFrameConverter::feed_bytes` 能增量解析 VT100 字节流并累积屏幕 buffer。

### 4. UiFrame 字段填充策略

VT100 路径无 anomaly 标记（VT100 字节流不含 anomaly 信息），转码后 UiFrame 的 `anomalies` 字段恒空 `Vec::new()`。`processes` 字段填充当前屏幕 buffer 的进程名集合（从 `vt100::Screen` 提取文本 + 解析进程名 pattern）。`cpu_usage` / `memory_used` 字段填 0（VT100 路径无系统指标数据）。

## 关键设计点

### 1. 不破坏原 VT100 文件

临时转码路径写 `<file>.tmp.v3` 文件，原 VT100 文件保留。用户可选 VT100 replay（走 VtPlayer 正向 replay）或转码后 v3 replay（享受 search / 倒放 / 书签全部能力）。

### 2. 转码失败可回退

VT100 字节流损坏时（如文件截断 / magic 错），转码失败 → fallback 走 `VtPlayer::open` 正向 replay（v0.6 落地的既有路径）。agent / 用户视角无感（replay 仍能进行，仅失去 search / 倒放能力）。

### 3. 转码开销 ~3s/30min session 可接受

30 min × 30 FPS × 1000 进程 VT100 文件转码到 UiFrame 预估 ~3s 开销（与 `proc_replay_search` ~9s/30min session 同款可接受范围，agent 一次性调用）。MCP `proc_replay_info` / `proc_replay_search` 双路径每次都转码——如 agent 多次调用累积 ~3s × N，但 brainstorm §风险 5 已 mitigate：v0.18+ cycle 评估 (b) 永久转码 CLI 子命令（`proc replay --convert <file>`）。

### 4. VT500 序列解析器扩工作量

v0.6 VtPlayer 仅做正向 replay，需扩反向解释能力支持转码。stage 5 实装时如 VT500 序列解析器扩工作量超预期（> 200 行）→ 触发 brainstorm §决策 8 自适应拆分规则（stage 5a 转码器骨架 + 正向转码 / stage 5b 反向解释器 + 倒放集成）。

## 备选方案（Alternatives）

### (a) 临时转码（**本 ADR 选此**，brainstorm 决策 6 拍板）

**接受**：`Vt100ToUiFrameConverter` 增量解析 + 累积屏幕 buffer + 30 FPS 切片 + 临时文件管理。不破坏原 VT100 文件 + 转码失败可回退 + 转码开销 ~3s 可接受。

### (b) 永久转码

**否决**：用户需手动管理 `.v3` 文件（或 proc 自动删除原 VT100？破坏性）。转码后文件 size 可能 2-3x（UiFrame 含 ProcessInfo 结构化数据，比 VT100 字节流大）。如用户有需求可加 `proc replay --convert <file>` CLI 子命令（v0.18+ cycle 候选）。

### (c) 仅 VT100 反向解释器（不转码）

**否决**：~1000+ 行独立实装（clear / cursor move / SGR 反向应用全套反向解释器），不享受 v3 Player 全部能力（书签 sidecar / footer 元数据 / FilterExpr search）。stage 5 工作量 ~900 行 vs 方案 (a) ~900 行（含 VT500 序列解析器扩），但方案 (a) 复用 v3 Player 全部能力，ROI 更高。

## 结果（Consequences）

- **stage 1 Spike 落地**：`src/record/vt100_to_uiframe.rs` 骨架（struct + 三方法 stub）+ `src/record/mod.rs` 加 `pub mod vt100_to_uiframe;` + re-export `Vt100ToUiFrameConverter`
- **stage 5 Slice 实装**：转换器业务逻辑（增量解析 + 累积屏幕 buffer + 30 FPS 切片）+ VT500 序列解析器扩 + replay 路径集成（`proc replay <file>` 自动检测 + 临时转码）
- **VT100 录屏享受 v3 全部能力**：search（FilterExpr 5 维度）/ 倒放（ReplayDirection::Reverse）/ 书签 CRUD 全部适用

### 负面（Trade-offs）

- **转码开销 ~3s/30min session**：agent 多次调用 search 累积 ~3s × N。brainstorm §风险 5 已 mitigate（v0.18+ cycle 评估永久转码 CLI 子命令）
- **VT500 序列解析器扩工作量风险**：如 stage 5 实装时 VT500 反序列化复杂度超预期，触发拆分（stage 5a + stage 5b，brainstorm §决策 8 自适应规则）
- **UiFrame 字段填充不完整**：VT100 路径无 anomaly / cpu_usage / memory_used 数据，转码后 UiFrame 这些字段填默认值（0 / 空 Vec）。agent 视角需理解「VT100 转码帧无系统指标」

## Migration path

- **v0.17 stage 1 Spike**（本 ADR 落地）：`vt100_to_uiframe.rs` 骨架 + `record/mod.rs` re-export
- **v0.17 stage 5 Slice**：转换器业务逻辑 + VT500 序列解析器扩 + replay 路径集成 + MCP `proc_replay_info` / `proc_replay_search` 双路径透明转码
- **v0.18+ cycle**：评估 (b) 永久转码 CLI 子命令 `proc replay --convert <file>`（如 agent 反馈多次转码开销可感）

## 相关 ADR / 文档

- v0.14 TD-49 归档：VT100 replay 路径无倒放 / 搜索
- v0.6 落地的 VtPlayer 正向 replay 路径：`src/record/vt100.rs::VtPlayer`
- [ADR-0011](0011-filter-expression.md)：FilterExpr 5 维度 FrameField（VT100 转码后可 apply）
- [ADR-0025a](0025a-mcp-replay-search-agent-schema.md)：v0.16 stage 1 落地的 `proc_replay_search` agent 视角 schema（VT100 转码后透明享受）
- [`docs/stages/v0.17-stage-1.md`](../stages/v0.17-stage-1.md) §决策 8（vt100_to_uiframe.rs 转码器骨架）
