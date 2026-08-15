# agent fixture 目录（v0.20 stage 2）

MockProvider 的回放数据源。格式 / 录制策略详见
[`docs/stages/v0.20-fixtures.md`](../../../docs/stages/v0.20-fixtures.md)。

## ⚠ 当前内容是 seed fixture（合成数据）

stage 2 落地的 70 query 是**合成 seed**（结构合法、确定性回放，供 CI 回放
测试与格式验证），不是真实 LLM 录制。真实录制（Gemma 4 E2B +
Anthropic Sonnet）在 stage 3b 末段 provider 可用后由
`src/agent/record_fixture.rs::FixtureRecorder` 执行，逐文件覆盖本目录
（stage-2.md 决策 C）。

## 文件组织（9 场景 × 3 level = 27 文件）

`<scenario>-l<level>.jsonl`，每行一个 JSON 对象：

```jsonc
{
  "query": "我电脑为什么这么卡？",       // 回放匹配键原文（必填）
  "query_hash": "a1b2c3d4e5f60718",     // 可选：SHA-256(query) 前 16 hex，加载时校验
  "request": {"provider": "seed"},      // 录制时的请求快照（diff / debug 用）
  "response_deltas": [                  // Delta 序列，回放时逐条 yield
    {"Text": "让我先看一下系统整体情况。"},
    {"ToolCall": {"id": "c1", "name": "proc_metrics_system", "arguments": {}}},
    {"EndTurn": {"stop_reason": "tool_use"}}
  ]
}
```

匹配规则：`MockProvider` 取**最后一条 user message** 计算
`SHA-256(query)` 前 16 hex 查索引（hash 不含 provider 名——录制 provider
与回放 provider 名不同）。

## Level 分布（70 query）

| Level | 数量 | 说明 |
|---|---|---|
| L0（单步 tool call）| 23 | v0.20 验收硬性 |
| L1（单步 + 总结）| 27 | 80% 通过率 |
| L2（多步 ReAct）| 20 | 录制留档，v0.21 启用 |

`monitor-l2.jsonl` 为空文件（场景 8 无 L2 query），保持 27 文件结构完整。
