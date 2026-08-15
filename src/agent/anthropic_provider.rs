//! AnthropicProvider — 云端对照路径 impl（opt-in feature `anthropic`，
//! `cargo build --features anthropic` + `ANTHROPIC_API_KEY` env）。
//!
//! stage 1 骨架；stage 4 实装：reqwest SSE 流式 Messages API client
//! （`POST /v1/messages` with `stream: true`）+ tool_use block 提取。

use async_trait::async_trait;

use super::provider::{CompleteOptions, CompleteResponse, LlmError, LlmProvider, ProviderStream};
use super::types::{Message, ToolSchema};

pub struct AnthropicProvider {
    pub model: String,
    pub max_tokens: u32,
}

impl AnthropicProvider {
    /// 从 `ANTHROPIC_API_KEY` env 读 key；缺失时 friendly error。
    pub fn from_env(model: String, max_tokens: u32) -> Result<Self, LlmError> {
        match std::env::var("ANTHROPIC_API_KEY") {
            Ok(_) => Ok(Self { model, max_tokens }),
            Err(_) => Err(LlmError::Config(
                "ANTHROPIC_API_KEY 环境变量未设置（云端对照路径 opt-in，不写配置文件避免泄漏）"
                    .to_string(),
            )),
        }
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &'static str {
        "anthropic"
    }

    async fn complete(
        &self,
        _messages: Vec<Message>,
        _tools: Vec<ToolSchema>,
        _options: CompleteOptions,
    ) -> Result<CompleteResponse, LlmError> {
        // stage 4 落地：reqwest POST /v1/messages + tool_use block 提取
        todo!("v0.20 stage 4 落地 Messages API client")
    }

    fn stream(
        &self,
        _messages: Vec<Message>,
        _tools: Vec<ToolSchema>,
        _options: CompleteOptions,
    ) -> ProviderStream<'static> {
        // stage 4 落地：SSE 解析（event: content_block_delta / data: {...}）
        todo!("v0.20 stage 4 落地 SSE 流式解析")
    }
}
