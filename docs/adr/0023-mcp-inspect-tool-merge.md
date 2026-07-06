# ADR-0023：proc_inspect tool 合并 6 个详情页 Tab

**Status**：Accepted
**Date**：2026-07-06（v0.15.0 阶段 1 落地）
**Related**：ADR-0009（MCP server 字段裁剪原则）

## 背景（Context）

proc 自 v0.5.0 起详情页（Inspector）有 6 个 Tab：`Summary` / `Env` / `Network` / `Dlls` / `MemoryMap` / `Handles`（src/tui/detail_view.rs::InspectionTab）。v0.6.0 加 `env_reveal` toggle + secret mask 12 关键字。v0.7.0 阶段 2 落地 MCP server 时只暴露 `proc_handles`（仅 1/6 Tab），其他 5 个 Tab 数据对 LLM agent 不可见。

v0.15 cycle 主题 D（MCP 全功能暴露）需要把剩余 5 Tab 也暴露。设计候选：

- **方案 A**：6 个独立 tool — `proc_inspect_summary` / `proc_inspect_env` / `proc_inspect_network` / `proc_inspect_dlls` / `proc_inspect_memory_map` / `proc_inspect_handles`
- **方案 B**：合并 1 个 tool 含 tab 参数 — `proc_inspect(pid, tab=summary|env|network|dlls|memory_map|handles, reveal=false)`

## 决策（Decision）

**选方案 B**：合并 1 个 `proc_inspect(pid, tab=..., reveal=...)` tool。

具体设计（v0.15 stage 1 落地骨架，stage 2 填业务逻辑）：

```rust
#[derive(Deserialize, schemars::JsonSchema)]
pub struct ProcInspectArgs {
    pub pid: u32,
    #[serde(default)]
    pub tab: InspectTab,
    #[serde(default)]
    pub reveal: bool,
}

#[derive(Deserialize, Default, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InspectTab {
    #[default]
    Summary,
    Env,
    Network,
    Dlls,
    MemoryMap,
    Handles,
}
```

**关键设计点**：

1. **tab 默认 Summary**：agent 不传 tab 也能拿到最有用的「概要 + R1-R18 风险因子 + signature 9 状态机 + parent_chain」一档数据。
2. **reveal 默认 false = mask secret**：与 v0.6 env_reveal 同款契约（按 `v` toggle 显式 opt-in）。secret pattern 12 关键字（KEY/TOKEN/SECRET/PASSWORD/PASSWD/PWD/CREDENTIAL/PRIVATE/AUTH/API/DSN/CONNECTION_STRING）+ `*_AUTHORIZATION` 后缀 + `DATABASE_URL` 特例。仅 `tab=env` 时 reveal 生效，其他 tab 忽略。
3. **字段裁剪与 proc_ls 不同**：`proc_inspect(tab=summary)` 返完整 cmd / exe / cwd 真值（详情页视角 = agent 已主动查单个进程 = 已同意看真值）；v0.7 `proc_ls` 列表视角裁剪 exe/cwd/user_id 避免大量进程的敏感路径污染 LLM context。两视角互补不冲突（详见 brainstorm FAQ Q1）。
4. **InspectionTab vs InspectTab 独立类型**：v0.5 TUI `InspectionTab` 与 v0.15 MCP `InspectTab` 同义但**不共享类型**——TUI 用 Display / Clone / PartialEq，MCP 用 serde / JsonSchema；derive 污染分开避免相互影响（surgical 原则）。
5. **handles tab 与 proc_handles 共存**：v0.7 既有的 `proc_handles` tool 不动。`proc_inspect(tab=handles)` 走同款 helper 但返 schema 略不同（带 PID context）。stage 4 Review 决策是否标 `proc_handles` 为 deprecated。

## 备选方案（Alternatives）

### (a) 6 个独立 `proc_inspect_*` tool

**否决理由**：
- tool 列表膨胀：17 → 23（多 6 个 vs 合并方案的 17 → 18）
- agent 调用复杂：要先知道哪个 tab 用哪个 tool 名
- schema 重复：每个 tool 都要单独定义 PID 参数 + 描述
- brainstorm FAQ Q3 决策：合并让 tool list 精简，调用语义清晰

### (b) 合并 + tab 字段用 String

**否决理由**：
- 失去类型检查（agent 可以传 `tab="summry"` typo 不报错，run-time 才发现）
- JsonSchema enum 自动生成 oneOf schema 让 client IDE 自动补全
- stage 2 实装时还要写 string → enum 转换 boilerplate

### (c) 单 tool 返所有 tab 数据

**否决理由**：
- payload 过大（典型 Summary ~2KB + Handles 列表 ~50KB + Dlls 列表 ~10KB = ~60KB+）
- agent 大多数场景只需要 1 个 tab，全返浪费 LLM context
- 不能按 tab 字段裁剪（不同 tab 有不同敏感度：env 需 mask，handles 不需要）

### (d) tab 字段为必需参数（无默认）

**否决理由**：
- 与 v0.7 `proc_ls` 等其他 tool 默认行为不一致（多数有 sensible default）
- agent 不传 tab 时直接报错 UX 差；用 Summary 默认返最常用数据更友好

## 结果（Consequences）

### 正面

- **tool 列表精简**：17 → 18（而非 17 → 23）；agent schema 浏览快
- **agent 调用语义清晰**：`proc_inspect(pid=1234, tab="env")` 自描述
- **schema 自描述**：`InspectTab` enum 变体 doc 即 tab 描述（schemars 自动转 description）
- **secret 默认 mask 与 v0.6 同款契约**：用户已熟悉 `v` 键 toggle；MCP 用 `reveal=true` opt-in 一致体验
- **未来 tab 扩展友好**：v0.16+ cycle 加新 tab（如「性能历史」/「线程列表」）只需扩 enum 变体，不增 tool 数

### 负面

- **schema 不能精确表达「不同 tab 返不同字段集」**：rmcp 0.11 + schemars 对 oneOf return schema 支持有限，stage 2 实装时返 `serde_json::Value` + doc 注释描述（与 brainstorm FAQ Q3 fallback 设计一致）
- **enum + nested struct 在 schemars 偶发问题**：v0.15 stage 1 stage 数量自适应规则提此风险；如 stage 2 实装失败 fallback 到方案 (b) String 字段
- **`proc_handles` 与 `proc_inspect(tab=handles)` 共存**：v0.7 已暴露的 `proc_handles` 不动，stage 4 Review 决策 deprecated 与否

### 中性

- **InspectionTab vs InspectTab 双类型**：v0.5 TUI 类型保留不动；v0.15 MCP 类型独立。两者变体集相同，仅 derive 不同。CONTEXT.md 术语段同时维护两个词条。

## Migration path

| 用户类型 | 影响 |
|---|---|
| LLM agent（Claude / Cursor） | 新 tool 可用：`proc_inspect(pid, tab, reveal)`。stage 2 实装后即可调用 |
| MCP client（Claude Desktop / mcp-inspector） | stage 2 实装后 tool list 从 17 涨到 18（含 `proc_inspect`） |
| 既有 `proc_handles` 用户 | 不动，与 `proc_inspect(tab=handles)` 共存；stage 4 Review 决策是否 deprecated |
| proc 内部代码 | v0.5 `InspectionTab` 不动；v0.15 `InspectTab` 在 `src/mcp/handler/inspect.rs` 独立定义 |

## 实装路径

- **stage 1（v0.15 Spike）**：`src/mcp/handler/inspect.rs` 创建 + `ProcInspectArgs` + `InspectTab` enum + `make_inspect_json` stub 占位返回
- **stage 2（v0.15 Slice）**：填 6 tab 字段裁剪业务逻辑——Summary 走 ProcessInfo + SecurityScorer / Env 走 inspect::env + env_mask / Network 走 port_map + dns_log / Dlls 走 inspect::dlls / MemoryMap 走 inspect::memory / Handles 复用既有 `make_handles_json`

## 相关 ADR

- **ADR-0009（MCP server）**：v0.7 落地的字段裁剪原则（列表视角）继续生效；本 ADR 是详情页视角的补充
- **v0.6 env mask ADR**（如有）：secret 12 关键字 pattern + env_reveal toggle，本 ADR MCP 路径同款契约
- **ADR-0024（handler 子 module 拆分）**：本 ADR 的 `inspect.rs` 文件落地依赖 0024 的子 module 重构
