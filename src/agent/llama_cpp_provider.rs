//! LlamaCppProvider — llama.cpp 本地 impl（feature `llama-cpp`，默认启用）。
//!
//! spawn llama-server 子进程（LlamaServerHandle，stage 3a 实装：动态端口 +
//! `--no-thinks` 强制禁用 Gemma 4 thinking mode，ADR-0030 D6）+ OpenAI 协议
//! client（`http://127.0.0.1:PORT/v1/chat/completions` + SSE 流式解析）。
//!
//! 按需 spawn 核心约束（brainstorm 决策 6）：仅在用户显式跑 `proc agent ask`
//! 时才 spawn，Drop 时 kill 子进程 + 释放 RAM / 端口，日常使用零影响。

use async_trait::async_trait;

use super::provider::{CompleteOptions, CompleteResponse, LlmError, LlmProvider, ProviderStream};
use super::types::{Message, ToolSchema};

pub struct LlamaCppProvider {
    /// llama-server.exe 路径（agent.toml `[llama-cpp].server_path` 或默认探测）。
    pub server_path: std::path::PathBuf,
    /// GGUF 模型文件路径（ModelRegistry 解析）。
    pub model_path: std::path::PathBuf,
}

impl LlamaCppProvider {
    pub fn new(server_path: std::path::PathBuf, model_path: std::path::PathBuf) -> Self {
        Self {
            server_path,
            model_path,
        }
    }
}

#[async_trait]
impl LlmProvider for LlamaCppProvider {
    fn name(&self) -> &'static str {
        "llama-cpp"
    }

    async fn complete(
        &self,
        _messages: Vec<Message>,
        _tools: Vec<ToolSchema>,
        _options: CompleteOptions,
    ) -> Result<CompleteResponse, LlmError> {
        // stage 3a 落地：spawn llama-server（如未启动）+ POST /v1/chat/completions
        todo!("v0.20 stage 3a 落地 OpenAI 协议 client")
    }

    fn stream(
        &self,
        _messages: Vec<Message>,
        _tools: Vec<ToolSchema>,
        _options: CompleteOptions,
    ) -> ProviderStream<'static> {
        // stage 3a 落地：SSE 流式响应解析（data: {...} 帧分割 + tool_calls 提取）
        todo!("v0.20 stage 3a 落地 SSE 流式解析")
    }
}
