//! `LlmProvider` trait 抽象层（ADR-0030 D3，brainstorm 决策 2 拍板：
//! `BoxStream` + `async_trait` 仅 complete）。
//!
//! 三 impl：
//! - [`crate::agent::llama_cpp_provider::LlamaCppProvider`]（默认，本地 llama-server）
//! - [`crate::agent::anthropic_provider::AnthropicProvider`]（opt-in feature）
//! - [`crate::agent::mock_provider::MockProvider`]（fixture 回放，CI 用）

use async_trait::async_trait;
use futures_util::stream::BoxStream;
use serde::{Deserialize, Serialize};

use super::types::{Message, ToolCall, ToolResult, ToolSchema};

/// LLM 调用抽象。`complete` 一次性返完整响应（`async_trait` 简单）；
/// `stream` 返 [`ProviderStream`]（流式增量，不能用 `async_trait`——会包一层 future）。
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// provider 标识（日志 / fixture 路由 / agent.toml 配置匹配用）。
    fn name(&self) -> &'static str;

    /// 一次性 complete（非流式）。
    async fn complete(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolSchema>,
        options: CompleteOptions,
    ) -> Result<CompleteResponse, LlmError>;

    /// 流式 complete（stage 2/3 各 provider 按需实装增量回放）。
    fn stream(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolSchema>,
        options: CompleteOptions,
    ) -> ProviderStream<'static>;
}

pub type ProviderStream<'a> = BoxStream<'a, Result<Delta, LlmError>>;

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("provider returned error: status={status} body={body}")]
    Api { status: u16, body: String },
    #[error("stream ended unexpectedly")]
    StreamEnded,
    #[error("model not loaded: {0}")]
    ModelNotLoaded(String),
    #[error("config error: {0}")]
    Config(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// complete 调用选项（`None` 字段走 provider / agent.toml 默认值）。
#[derive(Debug, Clone, Default)]
pub struct CompleteOptions {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub stop_sequences: Vec<String>,
    /// GBNF grammar 文件内容（仅 LlamaCppProvider 用，强制 JSON tool call 输出）。
    pub grammar: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CompleteResponse {
    pub message: Message,
    pub stop_reason: StopReason,
    pub usage: Usage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    StopSequence,
}

#[derive(Debug, Clone, Default)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// stream 的增量单元。serde externally tagged（fixture JSONL 的
/// `response_deltas` 行格式：`{"Text":"..."}` / `{"ToolCall":{...}}`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Delta {
    /// 文本增量（assistant message 文本）
    Text(String),
    /// tool call 增量（assistant 决定调 tool）
    ToolCall(ToolCall),
    /// tool result（系统注入，告诉 LLM tool 执行结果）
    ToolResult(ToolResult),
    /// 一轮结束（含 stop_reason）
    EndTurn { stop_reason: StopReason },
}
