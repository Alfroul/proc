//! MockProvider — fixture JSONL 回放 impl（feature `mock-provider`，默认启用）。
//!
//! stage 1 骨架；stage 2 实装：按 query hash 匹配 `tests/fixtures/agent/*.jsonl`
//! 中的 `MockResponse`，确定性回放（CI 零 LLM 调用）。

use async_trait::async_trait;

use super::provider::{CompleteOptions, CompleteResponse, LlmError, LlmProvider, ProviderStream};
use super::types::{Message, ToolSchema};

pub struct MockProvider {
    pub fixtures_dir: std::path::PathBuf,
}

impl MockProvider {
    pub fn new(fixtures_dir: std::path::PathBuf) -> Self {
        Self { fixtures_dir }
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    fn name(&self) -> &'static str {
        "mock"
    }

    async fn complete(
        &self,
        _messages: Vec<Message>,
        _tools: Vec<ToolSchema>,
        _options: CompleteOptions,
    ) -> Result<CompleteResponse, LlmError> {
        // stage 2 落地：按 query hash 匹配 fixture，确定性回放
        todo!("v0.20 stage 2 落地 fixture 回放")
    }

    fn stream(
        &self,
        _messages: Vec<Message>,
        _tools: Vec<ToolSchema>,
        _options: CompleteOptions,
    ) -> ProviderStream<'static> {
        // stage 2 落地：fixture response_deltas 逐条 yield
        todo!("v0.20 stage 2 落地 fixture 流式回放")
    }
}
