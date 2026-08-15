# ADR-0030：内置 AI agent + Tool registry 两层架构

**Status**: Accepted（v0.20.0 stage 1 Spike 落地骨架）

**Date**: 2026-08-15（v0.20 cycle stage 1 Spike）

**Related**: ADR-0009（MCP server，互补方向）、ADR-0008（self-mitigation policy，写操作 confirm gate）、ADR-0029（record 暴露 + confirm 机制，写操作 confirm bool 契约复用）、ADR-0026（MCP handler 持久字段策略，record_handle pattern 参考）

## Context

v0.19 cycle 完结后，proc 已是完整 MCP server（46 tool 暴露给外部 LLM，stdio + SSE transport + multi-client subscribe-push），但调用方向只有一个：**外部 LLM → proc**。proc 自身没有 LLM 调用能力——用户在终端只能敲结构化命令（`proc ls` / `proc metrics`），不能用自然语言问「我电脑为什么这么卡？」。

用户作为 AI 应用开发工程师岗位候选人，需要把 proc 从「Rust 系统工具」重塑为「AI-native 系统运维 agent 平台」。深度调研（2026-08-14）给出 6 个方向，用户拍板 v0.20 cycle 落地方向 A（内置 AI agent 端到端 demo）+ 方向 E（本地 LLM 集成）合并。

用户机器现有 Gemma 4 E2B 本地模型（`D:\llama.cpp\models\gemma4-e2b\gemma-4-E2B-it-Q4_K_M.gguf`，~1.6GB 量化 2B 模型），作为本地优先路径——隐私架构默认数据零外发，Anthropic 云端是 opt-in 对照路径。

小模型驱动 agent 的核心挑战是 tool schema token 占用：46 tool 全注入约 ~15K token，2B 模型 context 预算下成功率崩塌（E2B 在 τ²-bench Retail agent tool use 评测仅 29.4%）。

## Decision

- **D1：内置 agent**——proc 自身有 LLM 调用能力（`src/agent/` 新 module），不再仅是 MCP server 暴露 tool 给外部 LLM。入口 CLI `proc agent ask "<query>"`（v0.20）/ TUI AgentPanel（v0.21）
- **D2：Tool registry 两层架构**——Layer 0 默认 4 个 entry tool（`proc_ls` / `proc_metrics_system` / `proc_inspect` / `proc_help`，~600 token）+ Layer 1 通过 `proc_help(category)` 元 tool 动态发现剩余 42 个 tool 的 schema（峰值 ~1.5K token）。单轮 tool-context 从 ~15K 降至 ~1.5K（96% 减少），让 2B 模型也能稳定驱动
- **D3：multi-provider 抽象**——`LlmProvider` trait（`complete` async + `stream` 返 `BoxStream`）+ 三 impl：LlamaCppProvider（默认，spawn llama-server 子进程 + OpenAI 协议）/ AnthropicProvider（opt-in feature）/ MockProvider（fixture 回放，CI 零 LLM 调用）
- **D4：Gemma 4 E2B 本地优先**——默认走 LlamaCppProvider（隐私架构：数据零外发），AnthropicProvider 是 opt-in feature（`--features anthropic` + `ANTHROPIC_API_KEY`）。按需 spawn：仅用户显式调 `proc agent ask` 时才 spawn llama-server（不占日常使用的 RAM / 端口）
- **D5：Mock provider fixture 回放**——70 query fixture 按 JSONL 格式录制（9 场景 × 3 level 拆 27 文件），CI 跑 agent loop 零 LLM 调用、确定性强、零 API 成本
- **D6：thinking mode 强制禁用**——llama-server 启动强制加 `--no-thinks` CLI flag（Gemma 4 系列专用），避免 E2B thinking mode TTFT 5.8s（禁用后 < 0.3s，20× 加速）
- **D7：GBNF grammar 嵌入 binary**——`include_str!` 编译时嵌入 `tool_call.gbnf`，约束 Gemma 4 输出合法 JSON tool call（E2B 偶尔输出乱码 JSON 的保命手段）

## Consequences

stage 1 Spike 落地骨架；以下数字 stage 2/3 实装后填充实测：

- proc 二进制体积 +5-8MB（reqwest + gguf + tokio-stream deps，release build）
- Cargo.toml 加 `[features]`：`llama-cpp` / `mock-provider`（默认启用）+ `anthropic`（opt-in）
- 70 query fixture 让 46 tool agent loop 测试零 LLM 调用 + 确定性回归
- ToolRegistry 两层架构让单轮 tool schema token 从 ~15K 降至 ~1.5K（峰值），96% 减少
- E2B agent 成功率风险（29.4% 单步评测）：GBNF + few-shot 3 示例 + entry tool 4 个子集 mitigate；v0.20 验收标准 L0 23 query 硬性 / L1 27 query 80% 通过率 / L2 20 query 录 fixture 留 v0.21
- llama-server 子进程生命周期：LlmServerHandle 显式 spawn + Drop kill（参考 ADR-0026 record_handle pattern 但独立实现），日常使用零影响（不跑 `proc agent ask` 就不 spawn）

## 与既有 ADR 关系

- **互补 ADR-0009（MCP server）**：ADR-0009 是「proc 暴露给外部 LLM」（协议层），ADR-0030 是「proc 自身调 LLM」（参考实现）。双轨并存，复用同一套 tool 的 Rust API
- **复用 ADR-0008（self-mitigation policy）**：agent 调用写操作（proc_kill / proc_docker_rm / proc_usb_release）必须走 confirm gate
- **复用 ADR-0029（record 暴露 + confirm 机制）**：写操作 `confirm: bool` 必传契约，agent 写操作同款复用
- **复用 v0.6 secret mask 12 关键字**：agent PII 过滤层（tool_result 送 LLM 前过滤 KEY / TOKEN / PASSWORD 等）

## Alternatives Considered

### A. 不内置 agent，仅维护 MCP server（外部 LLM 调 proc）

**否决**：proc 用户视角仍是纯系统工具；简历叙事缺失「端到端 AI agent 应用」；本地 LLM 隐私架构无法表达。

### B. Tool schema 全 46 tool 一次注入

**否决**：~15K token 单轮 tool-context，2B 模型 context 预算下成功率崩塌；需 70B+ 模型才能稳定驱动，违背「本地小模型优先」决策。

### C. MCP 协议自环（proc 起内置 MCP client 调自己的 MCP server）

**否决**：JSON-RPC 序列化开销 + transport 复杂度（stdio/SSE）；内置 agent 直接调内部 Rust API 更简单高效。

## Migration path

- **v0.20 stage 1 Spike**（本 ADR 落地）：`src/agent/` 11 文件骨架（trait / struct 定义 + `todo!()` 占位）+ Cargo deps + CLI subcommand stub + 本 ADR 骨架
- **v0.20 stage 2 Slice A**：LlmProvider trait 实装 + MockProvider fixture 回放 + GGUF scanner + ToolRegistry 两层架构（46 tool 注册）
- **v0.20 stage 3a Slice B1**：LlamaCppProvider + LlamaServerHandle + GBNF grammar 实装
- **v0.20 stage 3b Slice B2**：AgentRunner ReAct loop + system prompt + CLI `proc agent ask` 实装 + Gemma 4 E2B L0/L1 验收
- **v0.20 stage 4 Review+收尾**：AnthropicProvider（opt-in feature）+ REVIEW-v0.20 + tag v0.20.0
- **v0.21+ cycle**：TUI AgentPanel（streaming chat）+ Eval / Observability + L2 多步 ReAct fixture 启用

## References

- [`docs/stages/v0.20-brainstorm.md`](../stages/v0.20-brainstorm.md)：cycle 总览 + 9 决策 + 70 query 清单 + few-shot 示例
- [`docs/stages/v0.20-fixtures.md`](../stages/v0.20-fixtures.md)：70 query fixture 录制计划
- llama.cpp `--no-thinks` flag（Gemma 4 系列，b8828+）：thinking mode 禁用实证
- Gemma 4 E2B τ²-bench Retail agent tool use 评测（29.4% 单步成功率）
