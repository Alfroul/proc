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
        }
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
        let messages = vec![Message::new(Role::User, fq.query.clone())];
        let tools = super::tools::catalog::default_registry()
            .entry_tools()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        let response = self
            .provider
            .complete(messages.clone(), tools.clone(), Default::default())
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
