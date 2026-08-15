//! proc_help 元 tool（Layer 0 entry tool 之一，brainstorm 决策 3 / 已拍板 Q2：
//! 仅 agent 内部可见，不进 MCP 46 tool handler）。
//!
//! agent 调 `proc_help(category=Some("docker"))` → 返回 docker 相关 tool 的
//! schema 列表；`category=None` → 返回所有非 entry tool 的列表。

use crate::agent::tool_registry::ToolRegistry;
use crate::agent::types::{ToolCategory, ToolSchema};

pub fn schema() -> ToolSchema {
    ToolSchema {
        name: "proc_help".to_string(),
        description: "Discover available tools by category. Call with category=Some(\"docker\") to get docker tool schemas, or category=None for all non-entry tools.".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "category": {
                    "type": "string",
                    "enum": ["process", "performance", "docker", "usb", "security",
                             "recording", "flow", "monitor", "dns"],
                    "description": "Tool category to discover. Omit for all non-entry tools."
                }
            }
        }),
        category: ToolCategory::Meta,
        estimated_tokens: 120,
    }
}

/// 实际执行：从 registry 取 tools_by_category。
pub fn execute(registry: &ToolRegistry, category: Option<ToolCategory>) -> Vec<ToolSchema> {
    registry
        .tools_by_category(category)
        .into_iter()
        .cloned()
        .collect()
}
