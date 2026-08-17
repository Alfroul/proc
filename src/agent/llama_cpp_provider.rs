//! LlamaCppProvider — llama.cpp 本地 impl（feature `llama-cpp`，默认启用）。
//!
//! spawn llama-server 子进程（[`LlamaServerHandle`] 惰性触发，跨调用复用）+
//! OpenAI 协议 client（`http://127.0.0.1:PORT/v1/chat/completions`）：
//! - [`LlmProvider::complete`]：非流式请求（`stream:false`），响应一次性解析
//! - [`LlmProvider::stream`]：SSE 流式（`stream:true`），`data: {...}` 帧分割 +
//!   tool_calls 分片聚合
//! - GBNF 接线（ADR-0030 D7）：[`CompleteOptions::grammar`] → 请求体 `grammar`
//!   字段透传 llama-server（llama.cpp 扩展字段）；启用策略由 stage 3b
//!   AgentRunner 决定
//!
//! 按需 spawn 核心约束（brainstorm 决策 6）：仅在首次 complete/stream 调用时
//! spawn，provider drop（server 句柄归零）→ kill 子进程，日常使用零影响。

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::{StreamExt, TryStreamExt};
use tokio::sync::Mutex;

use super::llama_server_handle::{LlamaServerHandle, SpawnOptions};
use super::provider::{
    CompleteOptions, CompleteResponse, Delta, LlmError, LlmProvider, ProviderStream, StopReason,
    Usage,
};
use super::types::{Message, Role, ToolCall, ToolSchema};

pub struct LlamaCppProvider {
    inner: Arc<ProviderInner>,
}

struct ProviderInner {
    server_path: PathBuf,
    model_path: PathBuf,
    spawn_opts: SpawnOptions,
    client: reqwest::Client,
    /// 惰性 spawn 的 llama-server 句柄（首次 complete/stream 触发，跨调用复用）。
    server: Mutex<Option<Arc<LlamaServerHandle>>>,
}

impl LlamaCppProvider {
    pub fn new(server_path: PathBuf, model_path: PathBuf) -> Self {
        Self::with_options(server_path, model_path, SpawnOptions::default())
    }

    /// 带 agent.toml `[llama-cpp]` 启动参数构造（stage 3b CLI 入口用）。
    pub fn with_options(
        server_path: PathBuf,
        model_path: PathBuf,
        spawn_opts: SpawnOptions,
    ) -> Self {
        Self {
            inner: Arc::new(ProviderInner {
                server_path,
                model_path,
                spawn_opts,
                client: reqwest::Client::new(),
                server: Mutex::new(None),
            }),
        }
    }

    pub fn server_path(&self) -> &PathBuf {
        &self.inner.server_path
    }

    pub fn model_path(&self) -> &PathBuf {
        &self.inner.model_path
    }
}

impl ProviderInner {
    /// 惰性 spawn：已启动直接复用；未启动才 spawn（按需 spawn 核心约束）。
    async fn ensure_server(&self) -> Result<Arc<LlamaServerHandle>, LlmError> {
        let mut guard = self.server.lock().await;
        if let Some(handle) = guard.as_ref() {
            return Ok(handle.clone());
        }
        let handle = Arc::new(
            LlamaServerHandle::spawn(&self.server_path, &self.model_path, self.spawn_opts.clone())
                .await?,
        );
        *guard = Some(handle.clone());
        Ok(handle)
    }
}

#[async_trait]
impl LlmProvider for LlamaCppProvider {
    fn name(&self) -> &'static str {
        "llama-cpp"
    }

    async fn complete(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolSchema>,
        options: CompleteOptions,
    ) -> Result<CompleteResponse, LlmError> {
        let server = self.inner.ensure_server().await?;
        let body = build_request_body(&messages, &tools, &options, false);
        let resp = self
            .inner
            .client
            .post(format!("{}/v1/chat/completions", server.base_url()))
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
        let json: serde_json::Value = resp.json().await?;
        parse_chat_response(&json)
    }

    fn stream(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolSchema>,
        options: CompleteOptions,
    ) -> ProviderStream<'static> {
        // stream 非 async fn（trait 决策 2），惰性 spawn 的 async setup 包在
        // stream::once 里，try_flatten 展平成 Delta 流（stage-3a 决策 C）。
        let inner = self.inner.clone();
        let setup = async move {
            let server = inner.ensure_server().await?;
            let body = build_request_body(&messages, &tools, &options, true);
            let resp = inner
                .client
                .post(format!("{}/v1/chat/completions", server.base_url()))
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

fn role_as_str(role: Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

/// proc Message → OpenAI messages。
///
/// `Role::Tool` 的 `tool_results` 按协议**逐条展开**成独立 `{"role":"tool"}`
/// 消息（OpenAI 工具结果是消息级，不是字段级）；assistant 的 `tool_calls`
/// 的 `arguments`（[`serde_json::Value`]）序列化回 JSON 字符串。
pub fn messages_to_openai(messages: &[Message]) -> Vec<serde_json::Value> {
    let mut out = Vec::with_capacity(messages.len());
    for m in messages {
        match m.role {
            Role::Tool => {
                for result in &m.tool_results {
                    out.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": result.tool_call_id,
                        "content": result.content,
                    }));
                }
            }
            role => {
                let mut v = serde_json::json!({
                    "role": role_as_str(role),
                    "content": m.content.clone().unwrap_or_default(),
                });
                if !m.tool_calls.is_empty() {
                    v["tool_calls"] = m
                        .tool_calls
                        .iter()
                        .map(|c| {
                            serde_json::json!({
                                "id": c.id,
                                "type": "function",
                                "function": {
                                    "name": c.name,
                                    "arguments": c.arguments.to_string(),
                                },
                            })
                        })
                        .collect::<Vec<_>>()
                        .into();
                }
                out.push(v);
            }
        }
    }
    out
}

/// ToolSchema 列表 → OpenAI tools 字段。
pub fn build_request_body(
    messages: &[Message],
    tools: &[ToolSchema],
    options: &CompleteOptions,
    stream: bool,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "messages": messages_to_openai(messages),
        "stream": stream,
    });
    if !tools.is_empty() {
        body["tools"] = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    },
                })
            })
            .collect::<Vec<_>>()
            .into();
    }
    if let Some(max_tokens) = options.max_tokens {
        body["max_tokens"] = max_tokens.into();
    }
    if let Some(temperature) = options.temperature {
        body["temperature"] = temperature.into();
    }
    if let Some(top_p) = options.top_p {
        body["top_p"] = top_p.into();
    }
    // llama.cpp 扩展字段（OpenAI 标准无 top_k）。
    if let Some(top_k) = options.top_k {
        body["top_k"] = top_k.into();
    }
    if !options.stop_sequences.is_empty() {
        body["stop"] = options.stop_sequences.clone().into();
    }
    // GBNF 接线（ADR-0030 D7）：llama.cpp 扩展字段，强制输出合法 JSON tool call。
    if let Some(grammar) = &options.grammar {
        body["grammar"] = grammar.clone().into();
    }
    // stage 3b：required 强制本轮必调 tool（E2B 实测 auto 模式倾向凭空文字回答）。
    if let Some(choice) = options.tool_choice {
        body["tool_choice"] = match choice {
            super::provider::ToolChoice::Auto => "auto",
            super::provider::ToolChoice::Required => "required",
        }
        .into();
    }
    body
}

/// OpenAI `finish_reason` → [`StopReason`]。
fn map_finish_reason(reason: &str) -> StopReason {
    match reason {
        "tool_calls" | "function_call" => StopReason::ToolUse,
        "length" => StopReason::MaxTokens,
        "stop_sequence" => StopReason::StopSequence,
        // "stop" 及未来新增值
        _ => StopReason::EndTurn,
    }
}

/// OpenAI tool_calls 数组元素 → [`ToolCall`]。
///
/// `function.arguments` 是 JSON 字符串；解析失败降级 `Null`（不丢整个 call，
/// agent 侧还能看到 name 报错重试）。
fn parse_openai_tool_call(v: &serde_json::Value) -> Option<ToolCall> {
    let function = v.get("function")?;
    let name = function.get("name")?.as_str()?.to_string();
    let raw_args = function
        .get("arguments")
        .and_then(|a| a.as_str())
        .unwrap_or("{}");
    let arguments = serde_json::from_str(raw_args).unwrap_or_else(|_| {
        tracing::warn!(tool = %name, raw = raw_args, "tool_call arguments 非合法 JSON，降级 Null");
        serde_json::Value::Null
    });
    let id = v
        .get("id")
        .and_then(|i| i.as_str())
        .unwrap_or_default()
        .to_string();
    Some(ToolCall {
        id,
        name,
        arguments,
    })
}

/// 非流式 `/v1/chat/completions` 响应 → [`CompleteResponse`]。
pub fn parse_chat_response(body: &serde_json::Value) -> Result<CompleteResponse, LlmError> {
    let choice = body
        .get("choices")
        .and_then(|c| c.get(0))
        .ok_or_else(|| LlmError::Api {
            status: 200,
            body: format!("响应缺 choices: {body}"),
        })?;
    let message = choice
        .get("message")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let content = message
        .get("content")
        .and_then(|c| c.as_str())
        .map(String::from);
    let tool_calls = message
        .get("tool_calls")
        .and_then(|t| t.as_array())
        .map(|arr| arr.iter().filter_map(parse_openai_tool_call).collect())
        .unwrap_or_default();
    let finish = choice
        .get("finish_reason")
        .and_then(|f| f.as_str())
        .unwrap_or("stop");
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
        stop_reason: map_finish_reason(finish),
        usage: Usage {
            input_tokens: get_tokens("prompt_tokens"),
            output_tokens: get_tokens("completion_tokens"),
        },
    })
}

/// SSE `data:` 帧提取器：feed 字节块，按空行分帧返回 data payload。
///
/// 跨 chunk 半行安全（未完整的帧留在缓冲）；`event:` / 注释行忽略；
/// 多行 `data:` 按规范以 `\n` 连接。`data: [DONE]` 哨兵原样返回，调用方判定。
#[derive(Default)]
pub struct SseFrameBuffer {
    buf: Vec<u8>,
}

impl SseFrameBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(chunk);
        let mut frames = Vec::new();
        while let Some(end) = find_frame_end(&self.buf) {
            let frame: Vec<u8> = self.buf.drain(..end).collect();
            if let Some(payload) = extract_data_payload(&frame) {
                frames.push(payload);
            }
        }
        frames
    }
}

/// 返回下一帧结束位置（含 `\n\n` / `\r\n\r\n` 分隔符）。
fn find_frame_end(buf: &[u8]) -> Option<usize> {
    if buf.len() < 2 {
        return None;
    }
    for i in 0..buf.len() - 1 {
        if buf[i] == b'\n' && buf[i + 1] == b'\n' {
            return Some(i + 2);
        }
        if buf[i] == b'\r'
            && buf[i + 1] == b'\n'
            && buf.len() >= i + 4
            && buf[i + 2] == b'\r'
            && buf[i + 3] == b'\n'
        {
            return Some(i + 4);
        }
    }
    None
}

fn extract_data_payload(frame: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(frame);
    let mut data_lines: Vec<&str> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    if data_lines.is_empty() {
        None
    } else {
        Some(data_lines.join("\n"))
    }
}

/// 流式 tool_calls 聚合器（OpenAI 协议 arguments 分片按 index 拼接）。
#[derive(Default)]
pub struct ToolCallAccum {
    calls: Vec<PartialToolCall>,
}

#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

impl ToolCallAccum {
    /// 应用一帧 delta.tool_calls（index 必有；id / name 首帧给，arguments 追加）。
    pub fn apply(&mut self, delta_tool_calls: &[serde_json::Value]) {
        for tc in delta_tool_calls {
            let idx = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
            while self.calls.len() <= idx {
                self.calls.push(PartialToolCall::default());
            }
            let slot = &mut self.calls[idx];
            if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                slot.id = id.to_string();
            }
            if let Some(function) = tc.get("function") {
                if let Some(name) = function.get("name").and_then(|v| v.as_str()) {
                    slot.name = name.to_string();
                }
                if let Some(args) = function.get("arguments").and_then(|v| v.as_str()) {
                    slot.arguments.push_str(args);
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.calls.iter().all(|c| c.name.is_empty())
    }

    /// 聚合完成，转 [`ToolCall`]（arguments JSON 字符串 → `Value`，失败降级 Null）。
    pub fn finish(self) -> Vec<ToolCall> {
        self.calls
            .into_iter()
            .filter(|c| !c.name.is_empty())
            .map(|c| {
                let arguments = serde_json::from_str(&c.arguments).unwrap_or_else(|_| {
                    tracing::warn!(tool = %c.name, raw = %c.arguments,
                        "流式 tool_call arguments 非合法 JSON，降级 Null");
                    serde_json::Value::Null
                });
                ToolCall {
                    id: c.id,
                    name: c.name,
                    arguments,
                }
            })
            .collect()
    }
}

/// SSE 响应字节流 → [`Delta`] 流。
///
/// - `delta.content` → [`Delta::Text`]（逐帧）
/// - `delta.tool_calls` → 聚合（不逐帧），finish_reason / `[DONE]` 时一次性
///   yield [`Delta::ToolCall`]
/// - `finish_reason` → [`Delta::EndTurn`]（恰好一次；`[DONE]` 兜底补发）
fn response_body_to_delta_stream(
    resp: reqwest::Response,
) -> impl futures_util::Stream<Item = Result<Delta, LlmError>> {
    let mut sse = SseFrameBuffer::new();
    let mut tool_accum = ToolCallAccum::default();
    let mut end_turn_sent = false;
    resp.bytes_stream().flat_map(move |chunk| {
        let mut out: Vec<Result<Delta, LlmError>> = Vec::new();
        match chunk {
            Ok(bytes) => {
                for payload in sse.feed(&bytes) {
                    if payload == "[DONE]" {
                        if !end_turn_sent {
                            end_turn_sent = true;
                            for call in std::mem::take(&mut tool_accum).finish() {
                                out.push(Ok(Delta::ToolCall(call)));
                            }
                            out.push(Ok(Delta::EndTurn {
                                stop_reason: StopReason::EndTurn,
                            }));
                        }
                        continue;
                    }
                    let Ok(v) = serde_json::from_str::<serde_json::Value>(&payload) else {
                        continue;
                    };
                    let Some(choice) = v.get("choices").and_then(|c| c.get(0)) else {
                        continue;
                    };
                    if let Some(delta) = choice.get("delta") {
                        if let Some(text) = delta.get("content").and_then(|c| c.as_str()) {
                            if !text.is_empty() {
                                out.push(Ok(Delta::Text(text.to_string())));
                            }
                        }
                        if let Some(tcs) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                            tool_accum.apply(tcs);
                        }
                    }
                    if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                        if !end_turn_sent {
                            end_turn_sent = true;
                            let stop_reason = map_finish_reason(reason);
                            if !tool_accum.is_empty() {
                                for call in std::mem::take(&mut tool_accum).finish() {
                                    out.push(Ok(Delta::ToolCall(call)));
                                }
                            }
                            out.push(Ok(Delta::EndTurn { stop_reason }));
                        }
                    }
                }
            }
            Err(e) => out.push(Err(LlmError::Network(e))),
        }
        futures_util::stream::iter(out)
    })
}
