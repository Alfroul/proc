//! agent system prompt（stage 1 落地 3 层结构骨架 + 3 个 few-shot 示例，
//! stage 3b 实装 AgentRunner 时注入 `{{SYSTEM_SNAPSHOT}}` 当前系统快照）。

/// 3 层结构：L1 角色定位 / L2 工具策略 / L3 当前系统快照注入点
/// （`{{SYSTEM_SNAPSHOT}}` 占位符由 runner 运行时替换）。
pub const SYSTEM_PROMPT: &str = include_str!("system.md");
