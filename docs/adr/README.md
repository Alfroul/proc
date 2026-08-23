# 架构决策记录（ADR）

> 本目录记录 proc 项目所有重要架构决策及其原因。新决策追加编号，旧决策被推翻时改 Status 为 `Superseded by ADR-NNNN`，不允许两个 Accepted 决策并存。

## 编号规则

- 文件名：`NNNN-简短标题.md`（4 位编号 + kebab-case 标题）
- 编号严格递增，不复用

## 索引

| ADR | 标题 | Status | 阶段 |
|---|---|---|---|
| [0001](./0001-phased-project-adoption.md) | phased-project skill adoption | Accepted | v0.5.0 阶段 1 |
| [0002](./0002-inspector-tab-extension-mechanism.md) | Inspector Tab 扩展机制（enum + match） | Accepted | v0.5.0 阶段 4 |
| [0003](./0003-smart-subprocess-vs-library.md) | SMART 采集走 smartctl 子进程而非 libatasmart | Accepted | v0.5.0 阶段 5 |
| [0004](./0004-gpu-via-nvtop-subprocess.md) | Linux GPU 采集走 nvtop 子进程 | Accepted | v0.5.0 阶段 6 |
| [0005](./0005-netflow-windows-iphelper-not-etw.md) | Windows per-process 网络流量走 IP Helper 而非 ETW | Accepted | v0.5.0 阶段 7 |
| [0006](./0006-dns-subprocess-not-etw-dbus.md) | DNS 查询日志走 PowerShell 子进程而非 ETW / DBus | Accepted | v0.5.0 阶段 8 |
| [0007](./0007-container-exec-pty-bridge.md) | 容器 exec 走 spawn docker exec -it + portable-pty | Accepted | v0.5.0 阶段 9 |
| [0008](./0008-self-mitigation-policy.md) | 进程自我加固策略（DEP+ASLR+DynamicCode+ExtPoint，不开 Signature） | Accepted | v0.6.0 阶段 2 |
| [0026](./0026-mcp-handler-persistent-fields.md) | MCP handler 持久字段策略（TD-54 + TD-52 + record 暴露前置） | Accepted | v0.17.0 阶段 1 |
| [0027](./0027-rmcp-resource-subscribe-sse-transport.md) | rmcp 0.11 Resource subscribe + SSE transport 设计 | Accepted | v0.17.0 阶段 1 |
| [0028](./0028-vt100-to-uiframe-converter.md) | VT100 字节流转码 UiFrame 路径（临时转码） | Accepted | v0.17.0 阶段 1 |
| [0029](./0029-record-exposure-and-confirm-mechanism.md) | record 暴露 + 写操作 confirm 机制 | Accepted | v0.17.0 阶段 1 |
| [0030](./0030-builtin-ai-agent.md) | 内置 AI agent + Tool registry 两层架构 | Accepted | v0.20.0 阶段 1 |
| [0031](./0031-tui-agent-panel.md) | TUI AgentPanel + AgentSession 流式会话架构 | Accepted | v0.21.0 阶段 1 |
| [0032](./0032-eval-harness.md) | `proc agent eval` 评测 harness + session observability | Accepted | v0.22.0 阶段 1 |
| [0033](./0033-eval-experiments-and-record-tools.md) | eval 变量实验（GBNF × prompt v2）+ proc_record_start/stop agent 侧支持 | Accepted | v0.23.0 阶段 1 |

## ADR 模板

```markdown
# ADR-NNNN: 标题

- **Status**: Proposed | Accepted | Deprecated | Superseded by ADR-NNNN
- **Date**: YYYY-MM-DD
- **Phase**: 落地阶段（如 v0.5.0 阶段 5）

## 背景

为什么需要做这个决策？遇到了什么问题？

## 选项

列出 2-4 个候选方案。

| 方案 | 优点 | 缺点 |
|---|---|---|
| A | ... | ... |
| B | ... | ... |

## 决策

选了哪个，为什么。

## 后果

- 正面后果
- 负面后果 / 已知限制
- 后续工作（如适用）

## 参考

- 相关 ADR
- 相关 PR / commit
- 外部资料链接
```
