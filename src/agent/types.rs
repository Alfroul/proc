//! agent 核心数据结构：Message / ToolSchema / ToolCall / ToolResult / ToolCategory。

use serde::{Deserialize, Serialize};

/// 对话消息（OpenAI / Anthropic tool-use 语义的超集）。
///
/// `content` 是 assistant / user 文本；`tool_calls` 是 assistant 决定调的 tool；
/// `tool_results` 是系统执行后注入回 LLM 的结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_results: Vec<ToolResult>,
}

impl Message {
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// tool 的 JSON Schema 描述（LLM 看这个决定怎么调）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    /// JSON Schema（`{"type": "object", "properties": {...}}`）
    pub parameters: serde_json::Value,
    pub category: ToolCategory,
    /// schema 注入 context 的估算 token 占用（ToolRegistry token 预算监控用）
    pub estimated_tokens: usize,
}

/// tool 分类（proc_help 元 tool 按 category 发现 Layer 1 tool 的索引键）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ToolCategory {
    Process,
    Performance,
    Docker,
    Usb,
    Security,
    Recording,
    Flow,
    Monitor,
    Dns,
    /// proc_help 自己（meta-tool）
    Meta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub content: String,
    pub is_error: bool,
}
