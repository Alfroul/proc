//! AgentRunner — ReAct tool-use 主循环（stage 3b，决策 D）。
//!
//! 用户 query → LLM → tool_call → dispatch 执行 → tool_result 回填 → LLM →
//! ... → 自然语言最终回答。走 [`LlmProvider::complete`] 非流式（CLI ask 是单
//! query 批处理场景，流式渲染留 v0.21 TUI AgentPanel）。
//!
//! - **system prompt 注入**（决策 F）：`SYSTEM_PROMPT` 的 `{{SYSTEM_SNAPSHOT}}`
//!   占位符替换为运行时轻量快照（OS / CPU / 内存 / 进程数，详细数据让模型调
//!   proc_metrics_system）
//! - **max_steps 兜底**：LLM 轮数上限，达到时返回已收集信息 + trace 摘要
//! - **空响应重试**：E2B 偶发空 content + 无 tool_calls → nudge 一次
//! - **GBNF grammar 不进主循环**（决策 C）：OpenAI tools 协议解析可靠
//!   （stage 3a 实测）；grammar 逃生舱经 [`AgentOptions::grammar`] 显式启用

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures_util::StreamExt;
use serde_json::Value;
use tokio::sync::oneshot;

use super::prompts::SYSTEM_PROMPT;
use super::provider::{
    CompleteOptions, CompleteResponse, Delta, LlmError, LlmProvider, ToolChoice,
};
use super::session::{ConfirmDecision, ConfirmRequest};
use super::tool_registry::ToolRegistry;
use super::tools::dispatch::{self, execute_tool};
use super::types::{Message, Role, ToolResult, ToolSchema};

/// loop 内部控制 tool 名（决策 I）：模型调它提交最终答案并结束循环。
pub const FINISH_TOOL_NAME: &str = "proc_finish";

fn finish_tool_schema() -> ToolSchema {
    ToolSchema {
        name: FINISH_TOOL_NAME.to_string(),
        description: "Submit your final natural-language answer to the user. Call this when \
                      you have enough information; the answer field holds the complete \
                      Chinese answer with actionable advice."
            .to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "answer": {"type": "string", "description": "完整的中文最终回答"}
            },
            "required": ["answer"]
        }),
        category: super::types::ToolCategory::Meta,
        estimated_tokens: 90,
    }
}

/// 从 tool_calls 中提取 proc_finish 的 answer（无 proc_finish 或 answer 缺失
/// /非字符串返 None）。
fn extract_finish_answer(calls: &[super::types::ToolCall]) -> Option<String> {
    calls
        .iter()
        .find(|c| c.name == FINISH_TOOL_NAME)
        .and_then(|c| c.arguments.get("answer"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|a| !a.trim().is_empty())
}

/// 动态扩 tools 的 token 预算上限（决策 J：防止 proc_help(None) 把 43 个
/// schema 全塞进 tools 数组撑爆 8192 ctx）。
const ACTIVE_TOOLS_TOKEN_BUDGET: usize = 6_000;

/// 决策 J：本轮 tool_calls 里若有 `proc_help(category=X)`，把 registry 中
/// X 类别的 schema 去重后加入 active tools（后续轮可调用）。
///
/// `category` 缺省（None = 全部非 entry）不扩——43 个 schema 会爆 token 预算，
/// 模型仍可从 proc_help 的 result 文本里看到全部名录再按类精查。
fn expand_tools_for_help_calls(
    registry: &ToolRegistry,
    calls: &[super::types::ToolCall],
    active: &mut Vec<ToolSchema>,
) {
    let token_sum: usize = active.iter().map(|t| t.estimated_tokens).sum();
    let mut budget = ACTIVE_TOOLS_TOKEN_BUDGET.saturating_sub(token_sum);
    for call in calls.iter().filter(|c| c.name == "proc_help") {
        let Some(cat) = call
            .arguments
            .get("category")
            .and_then(|v| v.as_str())
            .and_then(super::tools::dispatch::parse_category)
        else {
            continue;
        };
        for schema in registry.tools_by_category(Some(cat)) {
            if budget < schema.estimated_tokens {
                break;
            }
            if active.iter().any(|t| t.name == schema.name) {
                continue;
            }
            budget -= schema.estimated_tokens;
            active.push(schema.clone());
        }
    }
}

/// loop 运行选项（CLI flag > agent.toml > 代码默认，构造见 agent_cmd）。
#[derive(Debug, Clone)]
pub struct AgentOptions {
    /// LLM 轮数上限（每轮可执行多个 tool call）。brainstorm 决策 1：L0/L1 用 10。
    pub max_steps: u32,
    /// GBNF grammar（决策 C 默认 None——不启用；agent.toml grammar_file 显式
    /// 配置时启用逃生舱）。约束下模型只能输出 `{"tool_calls":[...]}` JSON，
    /// 与自然语言最终回答互斥，故不进主循环。
    pub grammar: Option<String>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
}

impl Default for AgentOptions {
    fn default() -> Self {
        Self {
            max_steps: 10,
            grammar: None,
            temperature: None,
            top_p: None,
            top_k: None,
        }
    }
}

/// loop 终止原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopCause {
    /// LLM 产出最终文本回答（正常路径）。
    EndTurn,
    /// 达到 max_steps 上限（final_text 是最后一条 assistant 文本或 trace 摘要）。
    MaxSteps,
    /// 空响应 nudge 重试后仍空（决策 D）。
    EmptyAfterRetry,
    /// 用户中断（v0.21 run_streaming：cancel flag 命中；当前 run 丢弃、
    /// session 层已完成 history 保留）。
    Interrupted,
}

/// 单个 tool call 的执行 trace（验收「expected tool 被调用」断言 + CLI 渲染）。
#[derive(Debug, Clone)]
pub struct StepTrace {
    pub tool_name: String,
    pub arguments: Value,
    pub is_error: bool,
    pub result_chars: usize,
}

#[derive(Debug, Clone)]
pub struct RunnerOutcome {
    pub final_text: String,
    pub steps: Vec<StepTrace>,
    pub stop: StopCause,
}

/// loop 进度事件（CLI stderr 实时渲染用）。
#[derive(Debug)]
pub enum StepEvent<'a> {
    /// 第 N 轮 LLM 调用开始（0-based）。
    LlmTurn(usize),
    /// 开始执行一个 tool call。
    ToolStart(&'a str, &'a Value),
}

/// 流式事件（v0.21 `run_streaming` 的 sink 增量；session 层转 `SessionEvent`）。
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// assistant 文本增量（透传 provider 的 `Delta::Text`）。
    TextDelta(String),
    /// 开始执行一个 tool call。
    ToolStart { name: String, arguments: Value },
    /// 一个 tool call 执行完成。
    ToolFinished {
        name: String,
        is_error: bool,
        result_chars: usize,
    },
    /// 一轮 LLM 流式调用结束（多轮 ReAct 的轮边界）。
    TurnFinished,
}

/// confirm hook：runner 自建 oneshot（reply 放进 request），hook 只负责把
/// `ConfirmRequest` 发出去（session 层 send `SessionEvent::ConfirmRequested`），
/// rx 由 runner 持有 await 用户 y/n 决策。
pub type ConfirmHook<'a> = &'a (dyn Fn(ConfirmRequest) + Send + Sync);

pub struct AgentRunner {
    provider: Arc<dyn LlmProvider>,
    registry: Arc<ToolRegistry>,
    options: AgentOptions,
}

impl AgentRunner {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        registry: ToolRegistry,
        options: AgentOptions,
    ) -> Self {
        Self {
            provider,
            registry: Arc::new(registry),
            options,
        }
    }

    pub async fn run(&self, query: &str) -> Result<RunnerOutcome, LlmError> {
        self.run_with_progress(query, &|_| {}).await
    }

    pub async fn run_with_progress(
        &self,
        query: &str,
        progress: &(dyn Fn(StepEvent<'_>) + Send + Sync),
    ) -> Result<RunnerOutcome, LlmError> {
        if query.trim().is_empty() {
            return Err(LlmError::Config("query 不能为空".to_string()));
        }

        let system = build_system_prompt();
        let mut messages = vec![
            Message::new(Role::System, system),
            Message::new(Role::User, query.to_string()),
        ];
        // 每轮 tools = entry 4 + proc_finish 控制 tool（决策 I）+ proc_help 发现
        // 后动态加入的类别 tool（决策 J——OpenAI 协议下模型只能调用请求 tools
        // 数组里声明的工具，schema 仅靠 tool result 文本回传是调不出去的）。
        let mut tools: Vec<_> = self.registry.entry_tools().into_iter().cloned().collect();
        tools.push(finish_tool_schema());
        let options = CompleteOptions {
            grammar: self.options.grammar.clone(),
            temperature: self.options.temperature,
            top_p: self.options.top_p,
            top_k: self.options.top_k,
            // 不限时 E2B 单轮能生成超长答案（实测 proc_finish 写千字分析），
            // 每轮 60-90s 拖垮验收时长；1024 token 足够一次完整中文总结。
            max_tokens: Some(1024),
            // 决策 I：required 强制每轮必调 tool——E2B 在 auto 模式下对「听起来
            // 能直接回答」的 query 倾向凭空文字回答；要结束必须显式调 proc_finish。
            tool_choice: Some(ToolChoice::Required),
            ..Default::default()
        };

        let mut steps: Vec<StepTrace> = Vec::new();
        let mut nudged = false;
        let mut last_assistant_text: Option<String> = None;

        for step in 0..self.options.max_steps as usize {
            progress(StepEvent::LlmTurn(step));
            let resp = self
                .provider
                .complete(messages.clone(), tools.clone(), options.clone())
                .await?;

            // 空响应（content 空 + 无 tool_calls）nudge 重试一次（决策 D）。
            let resp = if is_empty_response(&resp) && !nudged {
                messages.push(resp.message.clone());
                messages.push(Message::new(
                    Role::User,
                    "你上一条回复是空的。请基于已获得的信息直接回答用户的问题；\
                     如果还需要数据，请调用 tool。",
                ));
                nudged = true;
                progress(StepEvent::LlmTurn(step));
                self.provider
                    .complete(messages.clone(), tools.clone(), options.clone())
                    .await?
            } else {
                resp
            };

            if let Some(text) = resp.message.content.as_deref().filter(|t| !t.is_empty()) {
                last_assistant_text = Some(text.to_string());
            }
            messages.push(resp.message.clone());

            // 决策 I：proc_finish 控制 tool——模型显式提交最终答案结束循环。
            if let Some(answer) = extract_finish_answer(&resp.message.tool_calls) {
                return Ok(RunnerOutcome {
                    final_text: answer,
                    steps,
                    stop: StopCause::EndTurn,
                });
            }

            let real_calls: Vec<_> = resp
                .message
                .tool_calls
                .iter()
                .filter(|c| c.name != FINISH_TOOL_NAME)
                .cloned()
                .collect();
            if !real_calls.is_empty() {
                // 决策 J：proc_help(category) 执行后把该类别 schema 动态加入
                // 后续轮的 tools 数组（OpenAI 协议只能调用已声明的 tool）。
                expand_tools_for_help_calls(&self.registry, &real_calls, &mut tools);
                let results = self
                    .execute_tool_calls(&real_calls, &mut steps, progress)
                    .await;
                messages.push(Message {
                    role: Role::Tool,
                    content: None,
                    tool_calls: Vec::new(),
                    tool_results: results,
                });
                continue;
            }

            return Ok(match last_assistant_text {
                Some(text) => RunnerOutcome {
                    final_text: text,
                    steps,
                    stop: StopCause::EndTurn,
                },
                None => RunnerOutcome {
                    final_text: "模型未产出有效回答（空响应重试后仍为空）。".to_string(),
                    steps,
                    stop: StopCause::EmptyAfterRetry,
                },
            });
        }

        // max_steps 兜底：最后一条 assistant 文本优先，否则 trace 摘要。
        let final_text = last_assistant_text.unwrap_or_else(|| {
            let tools = if steps.is_empty() {
                "（无）".to_string()
            } else {
                steps
                    .iter()
                    .map(|s| s.tool_name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            format!(
                "已达到最大步数（{}），未能生成最终总结。已执行 tool：{tools}",
                self.options.max_steps
            )
        });
        Ok(RunnerOutcome {
            final_text,
            steps,
            stop: StopCause::MaxSteps,
        })
    }

    /// 流式变体（v0.21 ADR-0031 D2）：消费 `provider.stream()` 逐 delta 产出。
    ///
    /// proc_finish / max_steps / 动态扩 tools / 空响应 nudge 与 complete 路径
    /// 语义对齐；`history` 由 session 层维护（runner 无状态）。cancel 检查点
    /// 3 处：每 turn 开头 / 流消费每个 delta 后 / confirm await 期间——命中即
    /// 返 `StopCause::Interrupted`（已完成 steps 保留在 trace）。
    pub async fn run_streaming(
        &self,
        query: &str,
        history: &[Message],
        sink: &(dyn Fn(StreamEvent) + Send + Sync),
        confirm: Option<ConfirmHook<'_>>,
        cancel: &AtomicBool,
    ) -> Result<RunnerOutcome, LlmError> {
        if query.trim().is_empty() {
            return Err(LlmError::Config("query 不能为空".to_string()));
        }

        let system = build_system_prompt();
        let mut messages = vec![Message::new(Role::System, system)];
        messages.extend(history.iter().cloned());
        messages.push(Message::new(Role::User, query.to_string()));

        let mut tools: Vec<_> = self.registry.entry_tools().into_iter().cloned().collect();
        tools.push(finish_tool_schema());
        let options = CompleteOptions {
            grammar: self.options.grammar.clone(),
            temperature: self.options.temperature,
            top_p: self.options.top_p,
            top_k: self.options.top_k,
            max_tokens: Some(1024),
            tool_choice: Some(ToolChoice::Required),
            ..Default::default()
        };

        let mut steps: Vec<StepTrace> = Vec::new();
        let mut nudged = false;
        let mut last_assistant_text: Option<String> = None;

        for _step in 0..self.options.max_steps as usize {
            if cancel.load(Ordering::Relaxed) {
                return Ok(interrupted_outcome(steps));
            }

            let Some((text, calls)) = self
                .stream_turn(&mut messages, &tools, &options, sink, cancel, &mut nudged)
                .await?
            else {
                return Ok(interrupted_outcome(steps));
            };

            if !text.trim().is_empty() {
                last_assistant_text = Some(text.clone());
            }
            messages.push(Message {
                role: Role::Assistant,
                content: (!text.is_empty()).then(|| text.clone()),
                tool_calls: calls.clone(),
                tool_results: Vec::new(),
            });

            // 决策 I：proc_finish 控制 tool——模型显式提交最终答案结束循环。
            if let Some(answer) = extract_finish_answer(&calls) {
                return Ok(RunnerOutcome {
                    final_text: answer,
                    steps,
                    stop: StopCause::EndTurn,
                });
            }

            let real_calls: Vec<_> = calls
                .iter()
                .filter(|c| c.name != FINISH_TOOL_NAME)
                .cloned()
                .collect();
            if !real_calls.is_empty() {
                expand_tools_for_help_calls(&self.registry, &real_calls, &mut tools);
                let results = self
                    .execute_tool_calls_streaming(&real_calls, &mut steps, sink, confirm, cancel)
                    .await;
                messages.push(Message {
                    role: Role::Tool,
                    content: None,
                    tool_calls: Vec::new(),
                    tool_results: results,
                });
                continue;
            }

            return Ok(match last_assistant_text {
                Some(text) => RunnerOutcome {
                    final_text: text,
                    steps,
                    stop: StopCause::EndTurn,
                },
                None => RunnerOutcome {
                    final_text: "模型未产出有效回答（空响应重试后仍为空）。".to_string(),
                    steps,
                    stop: StopCause::EmptyAfterRetry,
                },
            });
        }

        // max_steps 兜底：最后一条 assistant 文本优先，否则 trace 摘要。
        let final_text = last_assistant_text.unwrap_or_else(|| {
            let tools = if steps.is_empty() {
                "（无）".to_string()
            } else {
                steps
                    .iter()
                    .map(|s| s.tool_name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            format!(
                "已达到最大步数（{}），未能生成最终总结。已执行 tool：{tools}",
                self.options.max_steps
            )
        });
        Ok(RunnerOutcome {
            final_text,
            steps,
            stop: StopCause::MaxSteps,
        })
    }

    /// 消费一轮 `provider.stream()`。返回 `None` = cancel 命中（调用方走
    /// Interrupted 收尾）。空响应（text 空 + 无 calls）nudge 重试一次（决策 D）。
    #[allow(clippy::type_complexity)]
    async fn stream_turn(
        &self,
        messages: &mut Vec<Message>,
        tools: &[ToolSchema],
        options: &CompleteOptions,
        sink: &(dyn Fn(StreamEvent) + Send + Sync),
        cancel: &AtomicBool,
        nudged: &mut bool,
    ) -> Result<Option<(String, Vec<super::types::ToolCall>)>, LlmError> {
        loop {
            let mut text = String::new();
            let mut calls = Vec::new();
            let mut stream =
                self.provider
                    .stream(messages.clone(), tools.to_vec(), options.clone());
            let mut cancelled = false;
            while let Some(delta) = stream.next().await {
                match delta? {
                    Delta::Text(t) => {
                        text.push_str(&t);
                        sink(StreamEvent::TextDelta(t));
                    }
                    Delta::ToolCall(c) => calls.push(c),
                    // MockProvider fixture 回放含 ToolResult delta——runner 自己
                    // 执行 tool，忽略。
                    Delta::ToolResult(_) | Delta::EndTurn { .. } => {}
                }
                if cancel.load(Ordering::Relaxed) {
                    cancelled = true;
                    break;
                }
            }
            drop(stream);
            if cancelled {
                return Ok(None);
            }
            sink(StreamEvent::TurnFinished);

            if text.trim().is_empty() && calls.is_empty() && !*nudged {
                messages.push(Message {
                    role: Role::Assistant,
                    content: None,
                    tool_calls: Vec::new(),
                    tool_results: Vec::new(),
                });
                messages.push(Message::new(
                    Role::User,
                    "你上一条回复是空的。请基于已获得的信息直接回答用户的问题；\
                     如果还需要数据，请调用 tool。",
                ));
                *nudged = true;
                continue;
            }
            return Ok(Some((text, calls)));
        }
    }

    /// 流式路径的 tool 执行（confirm 感知）：写 tool + confirm hook → 确认
    /// 流程（Approved → confirm:true 真执行 / Denied → blocked JSON）；其余
    /// 与 complete 路径同款 spawn_blocking。
    async fn execute_tool_calls_streaming(
        &self,
        calls: &[super::types::ToolCall],
        steps: &mut Vec<StepTrace>,
        sink: &(dyn Fn(StreamEvent) + Send + Sync),
        confirm: Option<ConfirmHook<'_>>,
        cancel: &AtomicBool,
    ) -> Vec<ToolResult> {
        let mut results = Vec::with_capacity(calls.len());
        for call in calls {
            sink(StreamEvent::ToolStart {
                name: call.name.clone(),
                arguments: call.arguments.clone(),
            });
            let result = self.execute_one_tool(call, confirm, cancel).await;
            let trace = StepTrace {
                tool_name: call.name.clone(),
                arguments: call.arguments.clone(),
                is_error: result.is_error,
                result_chars: result.content.len(),
            };
            sink(StreamEvent::ToolFinished {
                name: call.name.clone(),
                is_error: trace.is_error,
                result_chars: trace.result_chars,
            });
            steps.push(trace);
            results.push(result);
        }
        results
    }

    async fn execute_one_tool(
        &self,
        call: &super::types::ToolCall,
        confirm: Option<ConfirmHook<'_>>,
        cancel: &AtomicBool,
    ) -> ToolResult {
        let Some(hook) = confirm.filter(|_| dispatch::is_write_tool(&call.name)) else {
            return self.spawn_execute(call, &call.arguments, false).await;
        };

        let (reply_tx, reply_rx) = oneshot::channel();
        hook(ConfirmRequest {
            tool_name: call.name.clone(),
            arguments: call.arguments.clone(),
            summary: dispatch::confirm_summary(&call.name, &call.arguments),
            reply: reply_tx,
        });

        match await_confirm_decision(reply_rx, cancel).await {
            Some(ConfirmDecision::Approved) => {
                // Approved 即用户代传 confirm: true（ADR-0008/0029 契约）。
                let mut args = if call.arguments.is_object() {
                    call.arguments.clone()
                } else {
                    serde_json::json!({})
                };
                if let Some(obj) = args.as_object_mut() {
                    obj.insert("confirm".to_string(), Value::Bool(true));
                }
                self.spawn_execute(call, &args, true).await
            }
            // Denied / reply Sender 被 drop（面板退出未答复）→ blocked JSON。
            Some(ConfirmDecision::Denied) => dispatch::blocked_tool_result(call),
            // cancel 命中：run 随即 Interrupted 收尾，该占位不会被 LLM 消费。
            None => crate::agent::types::ToolResult {
                tool_call_id: call.id.clone(),
                content: "{\"ok\":false,\"error\":\"已中断\"}".to_string(),
                is_error: true,
            },
        }
    }

    /// spawn_blocking 执行（决策 J 配套：dispatch 同步层内嵌 block_on 的
    /// helper 在 async 上下文会 panic）。`confirmed_write=true` 仅在 confirm
    /// Approved 后由调用方置位——写 tool 真实执行（`execute_confirmed_tool`）；
    /// 其余一律走既有 `execute_tool`（写 tool 在其中被 blocked 拦截）。
    async fn spawn_execute(
        &self,
        call: &super::types::ToolCall,
        args: &Value,
        confirmed_write: bool,
    ) -> ToolResult {
        let registry = Arc::clone(&self.registry);
        let moved = super::types::ToolCall {
            id: call.id.clone(),
            name: call.name.clone(),
            arguments: args.clone(),
        };
        match tokio::task::spawn_blocking(move || {
            if confirmed_write {
                dispatch::execute_confirmed_tool(&moved)
            } else {
                execute_tool(&registry, &moved)
            }
        })
        .await
        {
            Ok(result) => result,
            Err(join_err) => crate::agent::types::ToolResult {
                tool_call_id: call.id.clone(),
                content: format!("{{\"ok\":false,\"error\":\"tool 执行线程 panic: {join_err}\"}}"),
                is_error: true,
            },
        }
    }

    async fn execute_tool_calls(
        &self,
        calls: &[super::types::ToolCall],
        steps: &mut Vec<StepTrace>,
        progress: &(dyn Fn(StepEvent<'_>) + Send + Sync),
    ) -> Vec<ToolResult> {
        let mut results = Vec::with_capacity(calls.len());
        for call in calls {
            progress(StepEvent::ToolStart(&call.name, &call.arguments));
            let tool_name = call.name.clone();
            let arguments = call.arguments.clone();
            let registry = Arc::clone(&self.registry);
            let moved = call.clone();
            // 决策 J 配套：dispatch 是同步层，部分内部 helper（DockerMonitor /
            // flows 的 App::new / DNS PowerShell collector）会自建 runtime
            // block_on——在 async 上下文直接调会 panic（MCP 路径靠 rmcp 的
            // spawn_blocking 保护）。丢到 blocking 线程池执行，干净且不阻塞
            // LLM 网络等待。
            let result =
                match tokio::task::spawn_blocking(move || execute_tool(&registry, &moved)).await {
                    Ok(result) => result,
                    Err(join_err) => crate::agent::types::ToolResult {
                        tool_call_id: call.id.clone(),
                        content: format!(
                            "{{\"ok\":false,\"error\":\"tool 执行线程 panic: {join_err}\"}}"
                        ),
                        is_error: true,
                    },
                };
            steps.push(StepTrace {
                tool_name,
                arguments,
                is_error: result.is_error,
                result_chars: result.content.len(),
            });
            results.push(result);
        }
        results
    }
}

fn is_empty_response(resp: &CompleteResponse) -> bool {
    resp.message.tool_calls.is_empty() && resp.message.content.as_deref().is_none_or(str::is_empty)
}

fn interrupted_outcome(steps: Vec<StepTrace>) -> RunnerOutcome {
    RunnerOutcome {
        final_text: "已中断".to_string(),
        steps,
        stop: StopCause::Interrupted,
    }
}

/// await 用户 y/n 决策。返回 `None` = cancel 命中（整个 run 走 Interrupted
/// 收尾）；reply Sender 被 drop（面板退出未答复）视同 Denied——两个方向都
/// 不允许 runner 协程悬挂（风险 2）。
async fn await_confirm_decision(
    mut rx: oneshot::Receiver<ConfirmDecision>,
    cancel: &AtomicBool,
) -> Option<ConfirmDecision> {
    loop {
        tokio::select! {
            res = &mut rx => return Some(res.unwrap_or(ConfirmDecision::Denied)),
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                if cancel.load(Ordering::Relaxed) {
                    return None;
                }
            }
        }
    }
}

/// system prompt 组装（决策 F）：`{{SYSTEM_SNAPSHOT}}` → 运行时轻量快照。
pub fn build_system_prompt() -> String {
    SYSTEM_PROMPT.replace("{{SYSTEM_SNAPSHOT}}", &snapshot_summary())
}

/// 快照失败不阻塞 agent（模型没有背景也能跑，只是少了锚点）。
fn snapshot_summary() -> String {
    let mut snapshot = match crate::collect::SystemSnapshot::new() {
        Ok(s) => s,
        Err(_) => return "（系统快照采集失败）".to_string(),
    };
    if snapshot.refresh().is_err() {
        return "（系统快照采集失败）".to_string();
    }
    let cpu = snapshot.cpu_usage();
    let (mem_used, mem_total) = snapshot.memory_usage();
    let process_count = snapshot.process_count();
    let gib = 1024.0 * 1024.0 * 1024.0;
    format!(
        "OS: {} | CPU: {:.1}% | 内存: {:.1}/{:.1} GB ({:.0}%) | 进程数: {}",
        std::env::consts::OS,
        cpu,
        mem_used as f64 / gib,
        mem_total as f64 / gib,
        if mem_total > 0 {
            mem_used as f64 / mem_total as f64 * 100.0
        } else {
            0.0
        },
        process_count,
    )
}

/// 停止原因的短标签（CLI / 测试输出用）。
impl StopCause {
    pub fn label(&self) -> &'static str {
        match self {
            Self::EndTurn => "end_turn",
            Self::MaxSteps => "max_steps",
            Self::EmptyAfterRetry => "empty_after_retry",
            Self::Interrupted => "interrupted",
        }
    }
}
