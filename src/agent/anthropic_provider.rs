//! AnthropicProvider — 云端对照路径 impl（opt-in feature `anthropic`，
//! `cargo build --features anthropic` + `ANTHROPIC_API_KEY` env）。
//!
//! Anthropic Messages API（`POST https://api.anthropic.com/v1/messages`）：
//! - [`LlmProvider::complete`]：非流式请求，content blocks 一次性解析
//! - [`LlmProvider::stream`]：SSE 流式（`event: content_block_delta` /
//!   `message_delta`，data JSON 的 `type` 字段判别事件），input_json_delta
//!   分片累积（stage 4 决策 A：分帧复用 [`super::sse::SseFrameBuffer`]）
//! - `tool_choice=Required` 映射 `{"type":"any"}`（与 stage 3b 决策 I 的
//!   proc_finish 循环语义对齐——控制 tool 由 runner 注入 tools 数组，
//!   provider 只透传 schema）
//! - 采样参数至多设一个（Anthropic API 约束），优先级 temperature > top_p >
//!   top_k（stage 4 决策 C）；`grammar`（GBNF）是 llama.cpp 专属字段，忽略

use async_trait::async_trait;
use futures_util::{StreamExt, TryStreamExt};

use super::provider::{
    CompleteOptions, CompleteResponse, Delta, LlmError, LlmProvider, ProviderStream, StopReason,
    ToolChoice, Usage,
};
use super::sse::SseFrameBuffer;
use super::types::{Message, Role, ToolCall, ToolSchema};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
/// agent.toml `[anthropic]` / 代码默认模型（决策 9）。
pub const DEFAULT_MODEL: &str = "claude-sonnet-4-6";

pub struct AnthropicProvider {
    pub model: String,
    pub max_tokens: u32,
    api_key: String,
    client: reqwest::Client,
}

impl AnthropicProvider {
    /// 从 `ANTHROPIC_API_KEY` env 读 key；缺失时 friendly error（不写配置
    /// 文件避免泄漏，ADR-0030 D4）。
    pub fn from_env(model: String, max_tokens: u32) -> Result<Self, LlmError> {
        match std::env::var("ANTHROPIC_API_KEY") {
            Ok(key) if !key.trim().is_empty() => Ok(Self {
                model,
                max_tokens,
                api_key: key,
                client: reqwest::Client::new(),
            }),
            _ => Err(LlmError::Config(
                "ANTHROPIC_API_KEY 环境变量未设置（云端对照路径 opt-in，不写配置文件避免泄漏）"
                    .to_string(),
            )),
        }
    }

    async fn post(&self, body: serde_json::Value) -> Result<reqwest::Response, LlmError> {
        let resp = self
            .client
            .post(API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(LlmError::Api {
                status: status.as_u16(),
                body: text,
            });
        }
        Ok(resp)
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &'static str {
        "anthropic"
    }

    async fn complete(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolSchema>,
        options: CompleteOptions,
    ) -> Result<CompleteResponse, LlmError> {
        // max_tokens 是 Anthropic 必填字段（决策 D 回退链）。
        let max_tokens = options.max_tokens.unwrap_or(self.max_tokens);
        let body = build_request_body(&self.model, max_tokens, &messages, &tools, &options, false);
        let resp = self.post(body).await?;
        let json: serde_json::Value = resp.json().await?;
        parse_messages_response(&json)
    }

    fn stream(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolSchema>,
        options: CompleteOptions,
    ) -> ProviderStream<'static> {
        // stream 非 async fn（trait 决策 2），请求发起包在 stream::once 里，
        // try_flatten 展平成 Delta 流（与 stage 3a 决策 C 同款；Anthropic 无
        // 子进程 spawn，setup 只发 HTTP 请求）。
        let client = self.client.clone();
        let api_key = self.api_key.clone();
        let max_tokens = options.max_tokens.unwrap_or(self.max_tokens);
        let body = build_request_body(&self.model, max_tokens, &messages, &tools, &options, true);
        let setup = async move {
            let resp = client
                .post(API_URL)
                .header("x-api-key", api_key)
                .header("anthropic-version", API_VERSION)
                .json(&body)
                .send()
                .await?;
            let status = resp.status();
            if !status.is_success() {
                let text = resp.text().await.unwrap_or_default();
                return Err(LlmError::Api {
                    status: status.as_u16(),
                    body: text,
                });
            }
            Ok::<_, LlmError>(response_body_to_delta_stream(resp))
        };
        Box::pin(futures_util::stream::once(setup).try_flatten())
    }
}

// ---------------------------------------------------------------------------
// 请求构造（纯函数）
// ---------------------------------------------------------------------------

/// proc Message 超集 → Anthropic messages（stage 4 决策 B 转换表）。
///
/// 返回 `(system, messages)`：Anthropic 不允许 messages 里出现 system role，
/// 提取为顶层字段；`Role::Tool` 的 tool_results 转 user 消息的 tool_result
/// blocks（Anthropic 要求）；空 assistant 消息（content 空 + 无 tool_calls）
/// 整条跳过（Anthropic 要求每条消息 content 非空）。
pub fn messages_to_anthropic(messages: &[Message]) -> (Option<String>, Vec<serde_json::Value>) {
    let mut system_parts: Vec<String> = Vec::new();
    let mut out = Vec::with_capacity(messages.len());
    for m in messages {
        match m.role {
            Role::System => {
                if let Some(text) = m.content.as_deref().filter(|t| !t.is_empty()) {
                    system_parts.push(text.to_string());
                }
            }
            Role::User => {
                out.push(serde_json::json!({
                    "role": "user",
                    "content": m.content.clone().unwrap_or_default(),
                }));
            }
            Role::Assistant => {
                let mut blocks: Vec<serde_json::Value> = Vec::new();
                if let Some(text) = m.content.as_deref().filter(|t| !t.is_empty()) {
                    blocks.push(serde_json::json!({"type": "text", "text": text}));
                }
                for call in &m.tool_calls {
                    blocks.push(serde_json::json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": call.arguments.as_object().cloned().unwrap_or_default(),
                    }));
                }
                if !blocks.is_empty() {
                    out.push(serde_json::json!({"role": "assistant", "content": blocks}));
                }
            }
            Role::Tool => {
                if m.tool_results.is_empty() {
                    continue;
                }
                let blocks: Vec<serde_json::Value> = m
                    .tool_results
                    .iter()
                    .map(|r| {
                        serde_json::json!({
                            "type": "tool_result",
                            "tool_use_id": r.tool_call_id,
                            "content": r.content,
                        })
                    })
                    .collect();
                out.push(serde_json::json!({"role": "user", "content": blocks}));
            }
        }
    }
    let system = (!system_parts.is_empty()).then(|| system_parts.join("\n\n"));
    (system, out)
}

/// ToolSchema 列表 → Anthropic tools 字段（schema 键名是 `input_schema`）。
fn tools_to_anthropic(tools: &[ToolSchema]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.parameters,
            })
        })
        .collect()
}

/// 构造 Messages API 请求体（complete / stream 共用，`stream` 位由调用方给）。
pub fn build_request_body(
    model: &str,
    max_tokens: u32,
    messages: &[Message],
    tools: &[ToolSchema],
    options: &CompleteOptions,
    stream: bool,
) -> serde_json::Value {
    let (system, msgs) = messages_to_anthropic(messages);
    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": msgs,
        "stream": stream,
    });
    if let Some(system) = system {
        body["system"] = system.into();
    }
    if !tools.is_empty() {
        body["tools"] = tools_to_anthropic(tools).into();
        if let Some(choice) = options.tool_choice {
            // 决策 D：Required → {"type":"any"}（强制本轮必调 tool，OpenAI
            // `required` 等价语义）。
            body["tool_choice"] = match choice {
                ToolChoice::Auto => serde_json::json!({"type": "auto"}),
                ToolChoice::Required => serde_json::json!({"type": "any"}),
            };
        }
    }
    // 决策 C：采样参数至多设一个（同设 API 400），优先级 temperature >
    // top_p > top_k；其余忽略。grammar（GBNF）是 llama.cpp 专属，忽略。
    if let Some(temperature) = options.temperature {
        body["temperature"] = temperature.into();
    } else if let Some(top_p) = options.top_p {
        body["top_p"] = top_p.into();
    } else if let Some(top_k) = options.top_k {
        body["top_k"] = top_k.into();
    }
    if !options.stop_sequences.is_empty() {
        body["stop_sequences"] = options.stop_sequences.clone().into();
    }
    body
}

// ---------------------------------------------------------------------------
// 响应解析（纯函数）
// ---------------------------------------------------------------------------

/// Anthropic `stop_reason` → [`StopReason`]。
fn map_stop_reason(reason: &str) -> StopReason {
    match reason {
        "tool_use" => StopReason::ToolUse,
        "max_tokens" => StopReason::MaxTokens,
        "stop_sequence" => StopReason::StopSequence,
        // "end_turn" 及未来新增值
        _ => StopReason::EndTurn,
    }
}

/// content blocks 数组元素（tool_use）→ [`ToolCall`]。
fn parse_tool_use_block(v: &serde_json::Value) -> Option<ToolCall> {
    if v.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
        return None;
    }
    Some(ToolCall {
        id: v
            .get("id")
            .and_then(|i| i.as_str())
            .unwrap_or_default()
            .to_string(),
        name: v.get("name")?.as_str()?.to_string(),
        arguments: v.get("input").cloned().unwrap_or(serde_json::Value::Null),
    })
}

/// 非流式 Messages API 响应 → [`CompleteResponse`]。
pub fn parse_messages_response(body: &serde_json::Value) -> Result<CompleteResponse, LlmError> {
    if body.get("type").and_then(|t| t.as_str()) == Some("error") {
        return Err(LlmError::Api {
            status: 200,
            body: body.to_string(),
        });
    }
    let blocks = body
        .get("content")
        .and_then(|c| c.as_array())
        .ok_or_else(|| LlmError::Api {
            status: 200,
            body: format!("响应缺 content: {body}"),
        })?;
    let mut texts: Vec<&str> = Vec::new();
    let mut tool_calls = Vec::new();
    for block in blocks {
        match block.get("type").and_then(|t| t.as_str()) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                    texts.push(text);
                }
            }
            Some("tool_use") => {
                if let Some(call) = parse_tool_use_block(block) {
                    tool_calls.push(call);
                }
            }
            _ => {}
        }
    }
    let content = (!texts.is_empty()).then(|| texts.join(""));
    let stop_reason = body
        .get("stop_reason")
        .and_then(|s| s.as_str())
        .map(map_stop_reason)
        .unwrap_or(StopReason::EndTurn);
    let usage = body.get("usage");
    let get_tokens = |key: &str| {
        usage
            .and_then(|u| u.get(key))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32
    };
    Ok(CompleteResponse {
        message: Message {
            role: Role::Assistant,
            content,
            tool_calls,
            tool_results: Vec::new(),
        },
        stop_reason,
        usage: Usage {
            input_tokens: get_tokens("input_tokens"),
            output_tokens: get_tokens("output_tokens"),
        },
    })
}

// ---------------------------------------------------------------------------
// stream SSE 聚合
// ---------------------------------------------------------------------------

/// 流式 tool_use 聚合器：`content_block_start` 记 id/name，
/// `input_json_delta.partial_json` 按 index 累积，`content_block_stop` 时
/// JSON 字符串 → Value（失败 / 空串降级 Null）转 [`ToolCall`]。
#[derive(Default)]
pub struct ToolUseAccum {
    blocks: Vec<PartialToolUse>,
}

#[derive(Default)]
struct PartialToolUse {
    id: String,
    name: String,
    arguments: String,
}

impl ToolUseAccum {
    fn slot(&mut self, index: usize) -> &mut PartialToolUse {
        while self.blocks.len() <= index {
            self.blocks.push(PartialToolUse::default());
        }
        &mut self.blocks[index]
    }

    /// `content_block_start`：tool_use 块记 id / name。
    pub fn on_block_start(&mut self, index: usize, block: &serde_json::Value) {
        if block.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
            return;
        }
        let slot = self.slot(index);
        if let Some(id) = block.get("id").and_then(|v| v.as_str()) {
            slot.id = id.to_string();
        }
        if let Some(name) = block.get("name").and_then(|v| v.as_str()) {
            slot.name = name.to_string();
        }
    }

    /// `content_block_delta`：`input_json_delta.partial_json` 追加（无
    /// content_block_start 的防御路径也建槽）。
    pub fn on_block_delta(&mut self, index: usize, delta: &serde_json::Value) {
        if delta.get("type").and_then(|t| t.as_str()) != Some("input_json_delta") {
            return;
        }
        if let Some(partial) = delta.get("partial_json").and_then(|v| v.as_str()) {
            self.slot(index).arguments.push_str(partial);
        }
    }

    /// `content_block_stop`：该 index 聚合完成转 ToolCall（非 tool_use 块 /
    /// 无 name 返 None；槽清空防重复 yield）。
    pub fn take_finished(&mut self, index: usize) -> Option<ToolCall> {
        if index >= self.blocks.len() {
            return None;
        }
        let slot = std::mem::take(&mut self.blocks[index]);
        if slot.name.is_empty() {
            return None;
        }
        let arguments = serde_json::from_str(&slot.arguments).unwrap_or_else(|_| {
            tracing::warn!(tool = %slot.name, raw = %slot.arguments,
                "流式 tool_use input 非合法 JSON，降级 Null");
            serde_json::Value::Null
        });
        Some(ToolCall {
            id: slot.id,
            name: slot.name,
            arguments,
        })
    }
}

/// 流式会话状态（SSE data payload → Delta 序列；纯逻辑可测试）。
pub struct StreamState {
    tool_uses: ToolUseAccum,
    end_turn_sent: bool,
}

impl Default for StreamState {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamState {
    pub fn new() -> Self {
        Self {
            tool_uses: ToolUseAccum::default(),
            end_turn_sent: false,
        }
    }

    /// 处理一帧 data payload（Anthropic 事件 JSON），产出 0..n 个 Delta：
    /// - `content_block_delta.text_delta` → [`Delta::Text`]（逐帧）
    /// - `content_block_stop`（tool_use）→ [`Delta::ToolCall`]（聚合后一次）
    /// - `message_delta.stop_reason` → [`Delta::EndTurn`]（恰好一次；
    ///   `message_stop` 兜底补发）
    /// - `message_start` / `ping` 等忽略
    pub fn feed_payload(&mut self, payload: &str) -> Vec<Result<Delta, LlmError>> {
        let mut out = Vec::new();
        let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) else {
            return out;
        };
        let index = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
        match v.get("type").and_then(|t| t.as_str()) {
            Some("content_block_start") => {
                if let Some(block) = v.get("content_block") {
                    self.tool_uses.on_block_start(index, block);
                }
            }
            Some("content_block_delta") => {
                if let Some(delta) = v.get("delta") {
                    self.tool_uses.on_block_delta(index, delta);
                    if delta.get("type").and_then(|t| t.as_str()) == Some("text_delta") {
                        if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                            if !text.is_empty() {
                                out.push(Ok(Delta::Text(text.to_string())));
                            }
                        }
                    }
                }
            }
            Some("content_block_stop") => {
                if let Some(call) = self.tool_uses.take_finished(index) {
                    out.push(Ok(Delta::ToolCall(call)));
                }
            }
            Some("message_delta") => {
                let reason = v
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(|r| r.as_str())
                    .map(map_stop_reason)
                    .unwrap_or(StopReason::EndTurn);
                if !self.end_turn_sent {
                    self.end_turn_sent = true;
                    out.push(Ok(Delta::EndTurn {
                        stop_reason: reason,
                    }));
                }
            }
            Some("message_stop") if !self.end_turn_sent => {
                self.end_turn_sent = true;
                out.push(Ok(Delta::EndTurn {
                    stop_reason: StopReason::EndTurn,
                }));
            }
            _ => {}
        }
        out
    }
}

/// SSE 响应字节流 → [`Delta`] 流（SseFrameBuffer 分帧 + StreamState 聚合）。
fn response_body_to_delta_stream(
    resp: reqwest::Response,
) -> impl futures_util::Stream<Item = Result<Delta, LlmError>> {
    let mut sse = SseFrameBuffer::new();
    let mut state = StreamState::new();
    resp.bytes_stream().flat_map(move |chunk| {
        let mut out: Vec<Result<Delta, LlmError>> = Vec::new();
        match chunk {
            Ok(bytes) => {
                for payload in sse.feed(&bytes) {
                    out.extend(state.feed_payload(&payload));
                }
            }
            Err(e) => out.push(Err(LlmError::Network(e))),
        }
        futures_util::stream::iter(out)
    })
}
