//! FixtureRecorder — fixture 录制工具（开发期，调真实 LLM 录制响应到 JSONL）。
//!
//! stage 2 落地工具本体（任意 `Box<dyn LlmProvider>` 可注入）；真实录制
//! （Gemma 4 E2B / Anthropic Sonnet）在 stage 3b 末段 provider 可用后执行，
//! 覆盖替换 `tests/fixtures/agent/` 的 seed fixture（stage-2.md 决策 C）。

use std::io::Write;
use std::path::PathBuf;

use serde::Serialize;

use super::mock_provider::query_hash;
use super::provider::{Delta, LlmError, LlmProvider};
use super::types::{Message, Role};

pub struct FixtureRecorder {
    provider: Box<dyn LlmProvider>,
    output_dir: PathBuf,
    /// 可选 system message（stage 3b）：录制请求形状与 agent loop 第 1 轮一致
    /// （含 system prompt + 快照），录到的响应才有代表性；回放匹配键仍是
    /// user query hash 不受影响。
    system_message: Option<String>,
}

pub struct FixtureQuery {
    pub scenario: &'static str,
    pub level: u8,
    pub query: String,
}

#[derive(Debug, Default)]
pub struct RecordReport {
    pub recorded: usize,
    pub failed: Vec<String>,
}

#[derive(Serialize)]
struct RecordedEntry {
    query: String,
    query_hash: String,
    request: serde_json::Value,
    response_deltas: Vec<Delta>,
}

impl FixtureRecorder {
    pub fn new(provider: Box<dyn LlmProvider>, output_dir: PathBuf) -> Self {
        Self {
            provider,
            output_dir,
            system_message: None,
        }
    }

    /// 注入与 agent loop 同款的 system prompt（stage 3b 真实录制用）。
    pub fn with_system_message(mut self, system: String) -> Self {
        self.system_message = Some(system);
        self
    }

    /// 批量录制：逐 query 调 provider.complete → append 到
    /// `<scenario>-l<level>.jsonl`。单 query 失败计 `failed` 不阻塞其余。
    pub async fn record_all(&mut self, queries: &[FixtureQuery]) -> Result<RecordReport, LlmError> {
        std::fs::create_dir_all(&self.output_dir)?;
        let mut report = RecordReport::default();
        for fq in queries {
            match self.record_one(fq).await {
                Ok(()) => report.recorded += 1,
                Err(e) => report
                    .failed
                    .push(format!("{}-l{}: {e}", fq.scenario, fq.level)),
            }
        }
        Ok(report)
    }

    async fn record_one(&mut self, fq: &FixtureQuery) -> Result<(), LlmError> {
        let mut messages = Vec::new();
        if let Some(system) = &self.system_message {
            messages.push(Message::new(Role::System, system.clone()));
        }
        messages.push(Message::new(Role::User, fq.query.clone()));
        let tools = super::tools::catalog::default_registry()
            .entry_tools()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        // 与 agent loop 第 1 轮同款约束（决策 I）：required 让录到的首响应必是
        // 真实 tool call（不带 proc_finish——录制的查询工具集只有 entry 4，
        // 模型只能从中选）。MockProvider 回放时忽略 options，不受影响。
        let options = super::provider::CompleteOptions {
            tool_choice: Some(super::provider::ToolChoice::Required),
            ..Default::default()
        };
        let response = self
            .provider
            .complete(messages.clone(), tools.clone(), options)
            .await?;

        let mut deltas = Vec::new();
        if let Some(text) = &response.message.content {
            deltas.push(Delta::Text(text.clone()));
        }
        for call in &response.message.tool_calls {
            deltas.push(Delta::ToolCall(call.clone()));
        }
        deltas.push(Delta::EndTurn {
            stop_reason: response.stop_reason.clone(),
        });

        let entry = RecordedEntry {
            query: fq.query.clone(),
            query_hash: query_hash(&fq.query),
            request: serde_json::json!({
                "provider": self.provider.name(),
                "messages": messages,
                "tools": tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            }),
            response_deltas: deltas,
        };
        let path = self
            .output_dir
            .join(format!("{}-l{}.jsonl", fq.scenario, fq.level));
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        let mut line = serde_json::to_string(&entry)
            .map_err(|e| LlmError::Config(format!("fixture 序列化失败: {e}")))?;
        line.push('\n');
        file.write_all(line.as_bytes())?;
        Ok(())
    }
}
