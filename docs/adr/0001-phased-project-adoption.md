# ADR-0001: phased-project skill adoption

- **Status**: Accepted
- **Date**: 2026-06-12
- **Phase**: v0.5.0 阶段 1

## 背景

proc 0.5.0+ 周期计划交付 14 项功能（Inspector 6 Tab、GPU 多厂商、SMART、per-process 流量、DNS 日志、Docker 深化、容器 exec），预估代码量 > 5000 行。单会话开发必然上下文溢出，跨会话开发缺统一的进度追踪 / 中断交接 / 术语治理机制 → 极易术语漂移、架构退化。

## 选项

| 方案 | 优点 | 缺点 |
|---|---|---|
| A. 自由开发，让 LLM 自行决定何时跨会话 | 灵活 | 上下文溢出频繁，跨会话上下文丢失 |
| B. 纯技术层拆分（数据层→逻辑层→API 层） | 拆分清晰 | 上层需求倒逼下层频繁回溯，前几个阶段跑不起来 |
| C. **phased-project skill**（Spike + Slice + Review + Batch） | 用户可感知功能切片、强制 Review、术语治理、Checkpoint 接力 | 学习成本 |

## 决策

采用方案 C（phased-project skill）。理由：

1. **Spike 阶段锁定接口**：上层 Slice 不会因为下层未定义而空转
2. **每个 Slice 一个用户可感知功能**：可独立验证、可发布 patch
3. **强制 Review + Batch 两段式收尾**：技术债显式归档，不积累
4. **CONTEXT.md + ADR** 承载跨会话项目记忆
5. **Checkpoint 机制** 允许上下文不足时精确中断点交接

## 后果

- 正面：v0.5.0 11 阶段全部按计划交付，611 测试 / clippy 0 warnings / fmt clean
- 正面：术语漂移通过 CONTEXT.md 「术语演进历史」段治理
- 负面：每阶段必须硬停止，用户开新对话成本上升
- 已知限制：plan.md / CONTEXT.md 保持私有（.gitignore 排除），仅 docs/adr/ + docs/stages/ 入仓

## 参考

- v0.5.0 阶段 1 落地：CHANGELOG.md
- 后续 ADR 均按本 skill 的 NNNN 编号规则
