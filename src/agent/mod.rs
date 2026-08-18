//! v0.20 内置 AI agent 模块（ADR-0030）
//!
//! 让 proc 自身有 LLM 调用能力（vs v0.7~v0.19 仅 MCP server 暴露 tool 给外部 LLM）。
//! 调用方向：proc → LLM（proc 是 client），入口 CLI `proc agent ask "<query>"`。
//!
//! 三 provider（[`LlmProvider`] trait impl）：
//! - [`LlamaCppProvider`]（默认启用，feature `llama-cpp`）：spawn llama-server 子进程 +
//!   OpenAI 协议，Gemma 4 E2B 本地优先（隐私架构，数据零外发）——stage 3a 实装
//! - [`AnthropicProvider`]（opt-in feature `anthropic`）：云端对照路径——stage 4 实装
//! - [`MockProvider`]（feature `mock-provider`，默认启用）：fixture JSONL 回放，
//!   CI 零 LLM 调用——stage 2 实装
//!
//! Tool registry 两层架构（brainstorm 决策 3）：Layer 0 默认 4 个 entry tool
//! （proc_ls / proc_metrics_system / proc_inspect / proc_help，~600 token）+ Layer 1
//! 通过 `proc_help(category)` 元 tool 动态发现剩余 tool schema（峰值 ~1.5K token）。
//! 单轮 tool-context 从全 46 tool 注入的 ~15K 降至 ~1.5K（96% 减少）。
//!
//! 详见 `docs/adr/0030-builtin-ai-agent.md`。

pub mod config;
pub mod gguf_scan;
pub mod grammars;
#[cfg(feature = "llama-cpp")]
pub mod llama_cpp_provider;
#[cfg(feature = "llama-cpp")]
pub mod llama_server_handle;
#[cfg(feature = "mock-provider")]
pub mod mock_provider;
pub mod model_registry;
pub mod prompts;
pub mod provider;
#[cfg(feature = "mock-provider")]
pub mod record_fixture;
pub mod runner;
pub mod session;
pub mod sse;
pub mod tool_registry;
pub mod tools;
pub mod types;

#[cfg(feature = "anthropic")]
pub mod anthropic_provider;

pub use config::AgentConfig;
pub use provider::{
    CompleteOptions, CompleteResponse, Delta, LlmError, LlmProvider, ProviderStream, StopReason,
    Usage,
};
pub use runner::{AgentOptions, AgentRunner, RunnerOutcome, StepTrace, StopCause};
pub use session::{
    AgentSession, ConfirmDecision, ConfirmRequest, MAX_HISTORY_TURNS, SessionEvent, SessionHandle,
};
pub use tool_registry::ToolRegistry;
pub use types::{Message, Role, ToolCall, ToolCategory, ToolResult, ToolSchema};
