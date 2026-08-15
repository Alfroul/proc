//! MockProvider — fixture JSONL 回放 impl（feature `mock-provider`，默认启用）。
//!
//! 构造时只需 fixtures_dir；首次 complete/stream 时惰性扫描全部 `*.jsonl`
//! 建 `query_hash → response_deltas` 索引（解析失败行 `tracing::warn` 跳过）。
//! 回放匹配键：最后一条 user message 的 SHA-256 前 16 hex（决策 D：hash 只含
//! query 文本——录制 provider 与回放 provider 名不同，含 provider 名会永不
//! 命中）。CI 零 LLM 调用、同输入同输出。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::provider::{
    CompleteOptions, CompleteResponse, LlmError, LlmProvider, ProviderStream, StopReason,
};
use super::types::{Message, Role, ToolSchema};

/// fixture JSONL 单行结构（决策 D：`query` 原文必填，hash 加载时现算校验）。
#[derive(Debug, Deserialize)]
pub struct FixtureEntry {
    pub query: String,
    /// 校验用；缺省时以现算 hash 为准。
    #[serde(default)]
    pub query_hash: Option<String>,
    /// 录制时的请求快照（provider / messages / tools），diff / debug 用。
    #[serde(default)]
    pub request: serde_json::Value,
    pub response_deltas: Vec<super::provider::Delta>,
}

/// 回放匹配键：SHA-256 前 16 hex。
pub fn query_hash(query: &str) -> String {
    let digest = Sha256::digest(query.as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

pub struct MockProvider {
    pub fixtures_dir: PathBuf,
    index: OnceLock<HashMap<String, Arc<Vec<super::provider::Delta>>>>,
}

impl MockProvider {
    pub fn new(fixtures_dir: PathBuf) -> Self {
        Self {
            fixtures_dir,
            index: OnceLock::new(),
        }
    }

    fn lookup(&self, query: &str) -> Option<Arc<Vec<super::provider::Delta>>> {
        let index = self.index.get_or_init(|| self.load_index());
        index.get(&query_hash(query)).cloned()
    }

    fn load_index(&self) -> HashMap<String, Arc<Vec<super::provider::Delta>>> {
        let mut index = HashMap::new();
        let Ok(files) = collect_jsonl_files(&self.fixtures_dir) else {
            return index;
        };
        for file in files {
            let Ok(content) = std::fs::read_to_string(&file) else {
                tracing::warn!(file = ?file, "fixture 不可读，跳过");
                continue;
            };
            for (lineno, line) in content.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                match serde_json::from_str::<FixtureEntry>(line) {
                    Ok(entry) => {
                        let computed = query_hash(&entry.query);
                        if let Some(recorded) = &entry.query_hash {
                            if !recorded.eq_ignore_ascii_case(&computed) {
                                tracing::warn!(
                                    file = ?file,
                                    lineno = lineno + 1,
                                    "fixture query_hash 与 query 不匹配，以现算 hash 为准"
                                );
                            }
                        }
                        index.insert(computed, Arc::new(entry.response_deltas));
                    }
                    Err(e) => {
                        tracing::warn!(file = ?file, lineno = lineno + 1, error = %e,
                            "fixture 行解析失败，跳过");
                    }
                }
            }
        }
        index
    }
}

fn collect_jsonl_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"))
        {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

#[async_trait]
impl LlmProvider for MockProvider {
    fn name(&self) -> &'static str {
        "mock"
    }

    async fn complete(
        &self,
        messages: Vec<Message>,
        _tools: Vec<ToolSchema>,
        _options: CompleteOptions,
    ) -> Result<CompleteResponse, LlmError> {
        let query = last_user_query(&messages)
            .ok_or_else(|| LlmError::Config("mock 回放需要至少一条 user message".to_string()))?;
        let deltas = self.lookup(query).ok_or_else(|| {
            LlmError::Config(format!(
                "fixture 缺失: query_hash={} query={query:?} (fixtures_dir={})",
                query_hash(query),
                self.fixtures_dir.display()
            ))
        })?;
        let mut content = String::new();
        let mut tool_calls = Vec::new();
        let mut stop_reason = StopReason::EndTurn;
        for delta in deltas.iter() {
            match delta {
                super::provider::Delta::Text(t) => content.push_str(t),
                super::provider::Delta::ToolCall(call) => tool_calls.push(call.clone()),
                super::provider::Delta::EndTurn { stop_reason: sr } => stop_reason = sr.clone(),
                super::provider::Delta::ToolResult(_) => {}
            }
        }
        Ok(CompleteResponse {
            message: Message {
                role: Role::Assistant,
                content: if content.is_empty() {
                    None
                } else {
                    Some(content)
                },
                tool_calls,
                tool_results: Vec::new(),
            },
            stop_reason,
            usage: Default::default(),
        })
    }

    fn stream(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolSchema>,
        _options: CompleteOptions,
    ) -> ProviderStream<'static> {
        let result = match last_user_query(&messages) {
            Some(query) => match self.lookup(query) {
                Some(deltas) => Ok(deltas),
                None => Err(LlmError::Config(format!(
                    "fixture 缺失: query_hash={} query={query:?} (fixtures_dir={})",
                    query_hash(query),
                    self.fixtures_dir.display()
                ))),
            },
            None => Err(LlmError::Config(
                "mock 回放需要至少一条 user message".to_string(),
            )),
        };
        let _ = tools;
        futures_util::stream::iter(match result {
            Ok(deltas) => deltas
                .iter()
                .cloned()
                .map(Ok)
                .collect::<Vec<Result<_, LlmError>>>(),
            Err(e) => vec![Err(e)],
        })
        .boxed()
    }
}

fn last_user_query(messages: &[Message]) -> Option<&str> {
    messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .and_then(|m| m.content.as_deref())
        .filter(|c| !c.is_empty())
}
