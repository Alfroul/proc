//! v0.20 stage 3a 集成测试：LlamaServerHandle + OpenAI 协议 client + GBNF 接线。
//!
//! 分两段：
//! - 纯逻辑测试（无 llama-server，CI 必跑）：spawn 命令 flag 断言 / OpenAI
//!   消息转换 / 请求体构造（grammar 接线）/ 非流式响应解析 / SSE 分帧 /
//!   tool_calls 聚合
//! - 真实 llama-server 测试（本机 Gemma 4 E2B 实测；server / 模型文件缺失时
//!   skip——CI 无 llama-server 自动跳过，零开销）

#![cfg(feature = "llama-cpp")]

use std::path::PathBuf;
use std::time::Duration;

use futures_util::StreamExt;
use proc::agent::llama_cpp_provider::{
    SseFrameBuffer, ToolCallAccum, build_request_body, messages_to_openai, parse_chat_response,
};
use proc::agent::llama_server_handle::{
    DEFAULT_CTX_SIZE, SpawnOptions, allocate_port, build_spawn_command,
};
use proc::agent::provider::{CompleteOptions, Delta, LlmError, LlmProvider, StopReason};
use proc::agent::types::{Message, Role, ToolCall, ToolResult};

// ---------------------------------------------------------------------------
// spawn 命令断言
// ---------------------------------------------------------------------------

fn spawn_args(cmd: &std::process::Command) -> Vec<String> {
    cmd.get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect()
}

#[test]
fn test_llama_server_handle_disables_thinking_mode() {
    // brainstorm 风险 2 mitigate 指名测试：启动命令必须含 thinking 禁用 flag。
    // b8685 实测 --no-thinks 不存在，等效 flag 是 --reasoning off（决策 A）。
    let cmd = build_spawn_command(
        PathBuf::from("llama-server.exe").as_path(),
        PathBuf::from("model.gguf").as_path(),
        12345,
        &SpawnOptions::default(),
    );
    let args = spawn_args(&cmd);
    let pos = args
        .iter()
        .position(|a| a == "--reasoning")
        .expect("默认 no_thinks=true 应传 --reasoning off");
    assert_eq!(args[pos + 1], "off");
}

#[test]
fn test_build_spawn_command_full_flag_set() {
    let opts = SpawnOptions {
        ctx_size: Some(4096),
        no_thinks: true,
        // 显式覆盖位：用户可传（默认 None 不传 flag，决策 F）
        chat_template: Some("chatml".to_string()),
        startup_timeout: Duration::from_secs(60),
    };
    let cmd = build_spawn_command(
        PathBuf::from("D:\\llama-server.exe").as_path(),
        PathBuf::from("D:\\model.gguf").as_path(),
        45678,
        &opts,
    );
    assert_eq!(cmd.get_program().to_string_lossy(), "D:\\llama-server.exe");
    let args = spawn_args(&cmd);
    let expect = [
        "--model",
        "D:\\model.gguf",
        "--host",
        "127.0.0.1",
        "--port",
        "45678",
        "--jinja",
        "--ctx-size",
        "4096",
        "--chat-template",
        "chatml",
        "--reasoning",
        "off",
    ];
    assert_eq!(args, expect);
}

#[test]
fn test_build_spawn_command_no_thinks_false_omits_reasoning() {
    let opts = SpawnOptions {
        no_thinks: false,
        ..SpawnOptions::default()
    };
    let args = spawn_args(&build_spawn_command(
        PathBuf::from("s.exe").as_path(),
        PathBuf::from("m.gguf").as_path(),
        1,
        &opts,
    ));
    assert!(!args.contains(&"--reasoning".to_string()));
}

#[test]
fn test_build_spawn_command_default_ctx_size_and_no_template() {
    let opts = SpawnOptions {
        chat_template: None,
        ..SpawnOptions::default()
    };
    let args = spawn_args(&build_spawn_command(
        PathBuf::from("s.exe").as_path(),
        PathBuf::from("m.gguf").as_path(),
        1,
        &opts,
    ));
    let pos = args.iter().position(|a| a == "--ctx-size").unwrap();
    assert_eq!(args[pos + 1], DEFAULT_CTX_SIZE.to_string());
    assert!(!args.contains(&"--chat-template".to_string()));
}

#[test]
fn test_spawn_options_defaults_match_decisions() {
    let opts = SpawnOptions::default();
    assert!(opts.no_thinks, "ADR-0030 D6：thinking 禁用默认开");
    // 决策 F：默认不传 --chat-template（GGUF 自带模板；显式 gemma + --jinja
    // 实测会丢 user content）
    assert_eq!(opts.chat_template, None);
    assert_eq!(opts.startup_timeout, Duration::from_secs(120));
}

#[test]
fn test_allocate_port_in_valid_range() {
    let port = allocate_port().expect("动态端口分配不应失败");
    assert!(port > 0);
    // 连续分配不应撞同一个端口（OS 顺序分配）
    let port2 = allocate_port().expect("第二次分配不应失败");
    assert_ne!(port, port2);
}

// ---------------------------------------------------------------------------
// OpenAI 协议：消息转换 + 请求体
// ---------------------------------------------------------------------------

#[test]
fn test_messages_to_openai_plain_roles() {
    let messages = vec![
        Message::new(Role::System, "你是系统运维 agent"),
        Message::new(Role::User, "列出 CPU 最高的进程"),
    ];
    let out = messages_to_openai(&messages);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0]["role"], "system");
    assert_eq!(out[0]["content"], "你是系统运维 agent");
    assert_eq!(out[1]["role"], "user");
}

#[test]
fn test_messages_to_openai_assistant_tool_calls() {
    let mut msg = Message::new(Role::Assistant, "");
    msg.content = None;
    msg.tool_calls.push(ToolCall {
        id: "call_1".to_string(),
        name: "proc_ls".to_string(),
        arguments: serde_json::json!({"sort": "cpu", "limit": 5}),
    });
    let out = messages_to_openai(std::slice::from_ref(&msg));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["role"], "assistant");
    let tc = &out[0]["tool_calls"][0];
    assert_eq!(tc["id"], "call_1");
    assert_eq!(tc["type"], "function");
    assert_eq!(tc["function"]["name"], "proc_ls");
    // arguments 序列化回 JSON 字符串（OpenAI 协议要求；serde_json 对象 key
    // 字母序，解析回 Value 比较避免顺序问题）
    let args_value: serde_json::Value =
        serde_json::from_str(tc["function"]["arguments"].as_str().unwrap()).unwrap();
    assert_eq!(args_value["sort"], "cpu");
    assert_eq!(args_value["limit"], 5);
}

#[test]
fn test_messages_to_openai_tool_results_expand_per_message() {
    // 一条 Role::Tool message 持 2 个 tool_results → 展开 2 条 OpenAI tool 消息
    let mut msg = Message::new(Role::Tool, "");
    msg.content = None;
    msg.tool_results = vec![
        ToolResult {
            tool_call_id: "call_1".to_string(),
            content: r#"{"ok":true}"#.to_string(),
            is_error: false,
        },
        ToolResult {
            tool_call_id: "call_2".to_string(),
            content: "error".to_string(),
            is_error: true,
        },
    ];
    let out = messages_to_openai(std::slice::from_ref(&msg));
    assert_eq!(out.len(), 2);
    assert_eq!(out[0]["role"], "tool");
    assert_eq!(out[0]["tool_call_id"], "call_1");
    assert_eq!(out[1]["tool_call_id"], "call_2");
}

#[test]
fn test_build_request_body_grammar_wiring() {
    // GBNF 接线（ADR-0030 D7）：options.grammar → 请求体 grammar 字段。
    let messages = vec![Message::new(Role::User, "hi")];
    let options = CompleteOptions {
        grammar: Some("root ::= ...".to_string()),
        temperature: Some(0.6),
        max_tokens: Some(1024),
        ..Default::default()
    };
    let body = build_request_body(&messages, &[], &options, false);
    assert_eq!(body["grammar"], "root ::= ...");
    // f32 0.6 → JSON Number 有精度尾差，容差比较
    assert!((body["temperature"].as_f64().unwrap() - 0.6).abs() < 1e-6);
    assert_eq!(body["max_tokens"], 1024);
    assert_eq!(body["stream"], false);
}

#[test]
fn test_build_request_body_omits_optionals() {
    let messages = vec![Message::new(Role::User, "hi")];
    let body = build_request_body(&messages, &[], &CompleteOptions::default(), true);
    assert!(body.get("grammar").is_none());
    assert!(body.get("temperature").is_none());
    assert!(body.get("tools").is_none());
    assert_eq!(body["stream"], true);
}

#[test]
fn test_build_request_body_tools_openai_format() {
    let schema = proc::agent::types::ToolSchema {
        name: "proc_ls".to_string(),
        description: "列出进程".to_string(),
        parameters: serde_json::json!({"type":"object","properties":{}}),
        category: proc::agent::types::ToolCategory::Process,
        estimated_tokens: 60,
    };
    let body = build_request_body(
        &[Message::new(Role::User, "hi")],
        std::slice::from_ref(&schema),
        &CompleteOptions::default(),
        false,
    );
    let tool = &body["tools"][0];
    assert_eq!(tool["type"], "function");
    assert_eq!(tool["function"]["name"], "proc_ls");
    assert_eq!(tool["function"]["description"], "列出进程");
}

// ---------------------------------------------------------------------------
// 非流式响应解析
// ---------------------------------------------------------------------------

#[test]
fn test_parse_chat_response_text_only() {
    let body = serde_json::json!({
        "choices": [{
            "finish_reason": "stop",
            "message": {"role": "assistant", "content": "CPU 23%"}
        }],
        "usage": {"prompt_tokens": 100, "completion_tokens": 8}
    });
    let resp = parse_chat_response(&body).unwrap();
    assert_eq!(resp.message.content.as_deref(), Some("CPU 23%"));
    assert_eq!(resp.stop_reason, StopReason::EndTurn);
    assert!(resp.message.tool_calls.is_empty());
    assert_eq!(resp.usage.input_tokens, 100);
    assert_eq!(resp.usage.output_tokens, 8);
}

#[test]
fn test_parse_chat_response_tool_calls() {
    let body = serde_json::json!({
        "choices": [{
            "finish_reason": "tool_calls",
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_abc",
                    "type": "function",
                    "function": {"name": "proc_ls", "arguments": "{\"sort\":\"cpu\"}"}
                }]
            }
        }],
        "usage": {"prompt_tokens": 50, "completion_tokens": 12}
    });
    let resp = parse_chat_response(&body).unwrap();
    assert_eq!(resp.stop_reason, StopReason::ToolUse);
    assert_eq!(resp.message.tool_calls.len(), 1);
    let call = &resp.message.tool_calls[0];
    assert_eq!(call.id, "call_abc");
    assert_eq!(call.name, "proc_ls");
    // arguments JSON 字符串 → Value
    assert_eq!(call.arguments["sort"], "cpu");
}

#[test]
fn test_parse_chat_response_finish_reason_mapping() {
    for (reason, expect) in [
        ("stop", StopReason::EndTurn),
        ("tool_calls", StopReason::ToolUse),
        ("function_call", StopReason::ToolUse),
        ("length", StopReason::MaxTokens),
        ("stop_sequence", StopReason::StopSequence),
    ] {
        let body = serde_json::json!({
            "choices": [{"finish_reason": reason, "message": {"content": "x"}}]
        });
        assert_eq!(parse_chat_response(&body).unwrap().stop_reason, expect);
    }
}

#[test]
fn test_parse_chat_response_bad_arguments_degrades_to_null() {
    let body = serde_json::json!({
        "choices": [{
            "finish_reason": "tool_calls",
            "message": {"tool_calls": [{
                "id": "call_1",
                "function": {"name": "proc_ls", "arguments": "不是 json"}
            }]}
        }]
    });
    let resp = parse_chat_response(&body).unwrap();
    assert_eq!(resp.message.tool_calls[0].name, "proc_ls");
    assert!(resp.message.tool_calls[0].arguments.is_null());
}

#[test]
fn test_parse_chat_response_missing_choices_is_error() {
    let err = parse_chat_response(&serde_json::json!({"error": "boom"})).unwrap_err();
    assert!(matches!(err, LlmError::Api { .. }));
}

// ---------------------------------------------------------------------------
// SSE 分帧 + tool_calls 聚合
// ---------------------------------------------------------------------------

#[test]
fn test_sse_frame_buffer_single_chunk_multi_frames() {
    let mut sse = SseFrameBuffer::new();
    let frames = sse.feed(b"data: {\"a\":1}\n\ndata: [DONE]\n\n");
    assert_eq!(frames, vec![r#"{"a":1}"#.to_string(), "[DONE]".to_string()]);
}

#[test]
fn test_sse_frame_buffer_split_across_chunks() {
    // 半行跨 chunk：完整帧只在收齐后产出
    let mut sse = SseFrameBuffer::new();
    assert!(sse.feed(b"data: {\"par").is_empty());
    assert!(sse.feed(b"tial\":true}").is_empty());
    let frames = sse.feed(b"\n\ndata: [DONE]\n\n");
    assert_eq!(
        frames,
        vec![r#"{"partial":true}"#.to_string(), "[DONE]".to_string()]
    );
}

#[test]
fn test_sse_frame_buffer_ignores_non_data_lines() {
    let mut sse = SseFrameBuffer::new();
    let frames = sse.feed(b"event: chunk\ndata: {\"x\":1}\n: comment\n\n");
    assert_eq!(frames, vec![r#"{"x":1}"#.to_string()]);
}

#[test]
fn test_sse_frame_buffer_crlf_separator() {
    let mut sse = SseFrameBuffer::new();
    let frames = sse.feed(b"data: {\"x\":1}\r\n\r\ndata: [DONE]\r\n\r\n");
    assert_eq!(frames, vec![r#"{"x":1}"#.to_string(), "[DONE]".to_string()]);
}

#[test]
fn test_tool_call_accum_assembles_fragmented_arguments() {
    let mut accum = ToolCallAccum::default();
    accum.apply(&[serde_json::json!({
        "index": 0, "id": "call_1",
        "function": {"name": "proc_ls", "arguments": "{\"so"}
    })]);
    accum.apply(&[serde_json::json!({
        "index": 0,
        "function": {"arguments": "rt\":\"cpu\"}"}
    })]);
    assert!(!accum.is_empty());
    let calls = accum.finish();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_1");
    assert_eq!(calls[0].name, "proc_ls");
    assert_eq!(calls[0].arguments["sort"], "cpu");
}

#[test]
fn test_tool_call_accum_multiple_indices() {
    let mut accum = ToolCallAccum::default();
    accum.apply(&[serde_json::json!({
        "index": 0, "id": "c0", "function": {"name": "proc_ls", "arguments": "{}"}
    })]);
    accum.apply(&[serde_json::json!({
        "index": 1, "id": "c1", "function": {"name": "proc_dns", "arguments": "{\"limit\":10}"}
    })]);
    let calls = accum.finish();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "proc_ls");
    assert_eq!(calls[1].name, "proc_dns");
    assert_eq!(calls[1].arguments["limit"], 10);
}

#[test]
fn test_tool_call_accum_empty_name_filtered() {
    let mut accum = ToolCallAccum::default();
    accum.apply(&[serde_json::json!({"index": 0, "function": {"arguments": "x"}})]);
    assert!(accum.is_empty());
    assert!(accum.finish().is_empty());
}

#[test]
fn test_tool_call_grammar_rule_names_have_no_underscore() {
    // 决策 G：GBNF 规则名不支持下划线（llama.cpp 语法只允许 [a-zA-Z0-9-]），
    // 含下划线的规则名会让整个 grammar 被 llama-server 静默忽略。
    // JSON key 字面量（"tool_calls"）不受限——那是字符串内容不是规则名。
    let grammar = proc::agent::grammars::TOOL_CALL_GRAMMAR;
    let mut rule_count = 0;
    for line in grammar.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, rest)) = line.split_once("::=") else {
            continue;
        };
        assert!(
            !name.trim().contains('_'),
            "GBNF 规则名含下划线会被 llama-server 静默忽略: {name:?}"
        );
        assert!(!rest.trim().is_empty(), "规则体不能为空: {name:?}");
        rule_count += 1;
    }
    assert!(rule_count >= 6, "应有 6+ 条规则，实际 {rule_count}");
    // 修复后的规则名（stage 1 初版的 tool_call 是 bug）
    assert!(grammar.contains("tool-call ::="));
}

// ---------------------------------------------------------------------------
// 真实 llama-server 实测（本机 Gemma 4 E2B；缺环境自动 skip）
// ---------------------------------------------------------------------------

const REAL_SERVER: &str = r"D:\llama.cpp\bin\llama-b8685-bin-win-cuda-12.4-x64\llama-server.exe";
const REAL_MODEL: &str = r"D:\llama.cpp\models\gemma4-e2b\gemma-4-E2B-it-Q4_K_M.gguf";

fn real_env_available() -> bool {
    let ok =
        std::path::Path::new(REAL_SERVER).exists() && std::path::Path::new(REAL_MODEL).exists();
    if !ok {
        eprintln!("SKIP: 本机 llama-server / 模型不存在，真实推理测试跳过");
    }
    ok
}

/// 端到端：spawn → /health → complete 真实推理 → stream → Drop 清理。
/// 合并为单个 #[tokio::test] 控制全量回归耗时（模型加载 ~5-10s 只发生一次）。
#[tokio::test]
async fn test_real_llama_server_end_to_end() {
    if !real_env_available() {
        return;
    }
    let provider = proc::agent::llama_cpp_provider::LlamaCppProvider::new(
        PathBuf::from(REAL_SERVER),
        PathBuf::from(REAL_MODEL),
    );

    // complete：Gemma E2B 真实推理
    let messages = vec![
        Message::new(Role::System, "你是系统运维助手，用中文简短回答。"),
        Message::new(Role::User, "1 加 1 等于几？只回答阿拉伯数字。"),
    ];
    let started = std::time::Instant::now();
    let resp = provider
        .complete(messages, Vec::new(), CompleteOptions::default())
        .await
        .expect("真实 complete 应成功");
    let elapsed = started.elapsed();
    eprintln!(
        "real complete: {:?} content={:?} stop={:?} usage={:?}",
        elapsed, resp.message.content, resp.stop_reason, resp.usage
    );
    let content = resp
        .message
        .content
        .as_deref()
        .expect("complete 应返回 content");
    assert!(content.trim().contains('2'), "回答应含数字 2: {content:?}");

    // stream：SSE 流式增量 + 正常终止
    let mut deltas = Vec::new();
    let mut stream = provider.stream(
        vec![Message::new(
            Role::User,
            "用一句话介绍 proc 是什么（可虚构）",
        )],
        Vec::new(),
        CompleteOptions::default(),
    );
    while let Some(item) = stream.next().await {
        deltas.push(item.expect("stream item 应 Ok"));
    }
    let has_text = deltas
        .iter()
        .any(|d| matches!(d, Delta::Text(t) if !t.is_empty()));
    let has_end = deltas.iter().any(|d| matches!(d, Delta::EndTurn { .. }));
    eprintln!(
        "real stream: {} deltas, has_text={has_text} has_end={has_end}",
        deltas.len()
    );
    assert!(has_text, "流式应产出文本增量");
    assert!(has_end, "流式应以 EndTurn 终止");

    // Drop 清理：句柄释放后 llama-server 子进程退出
    drop(stream);
    drop(provider);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let still_alive = std::process::Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq llama-server.exe", "/FO", "CSV", "/NH"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("llama-server.exe"))
        .unwrap_or(false);
    assert!(!still_alive, "provider drop 后 llama-server 应已退出");
}

/// GBNF grammar 端到端验证：TOOL_CALL_GRAMMAR 经请求体 grammar 字段透传后
/// llama-server 真实施加约束（输出必须是 root 规则形状的 JSON）。
/// 实测依据（b8685）：`root ::= "yes"` 强制输出 "yes"；下划线规则名会被
/// 静默忽略（决策 G）。
#[tokio::test]
async fn test_real_llama_server_grammar_constrains_output() {
    if !real_env_available() {
        return;
    }
    let provider = proc::agent::llama_cpp_provider::LlamaCppProvider::new(
        PathBuf::from(REAL_SERVER),
        PathBuf::from(REAL_MODEL),
    );
    let options = CompleteOptions {
        grammar: Some(proc::agent::grammars::TOOL_CALL_GRAMMAR.to_string()),
        max_tokens: Some(128),
        ..Default::default()
    };
    let resp = provider
        .complete(
            vec![Message::new(
                Role::User,
                "Tell me a long story about the ocean",
            )],
            Vec::new(),
            options,
        )
        .await
        .expect("grammar 请求应成功");
    let content = resp.message.content.as_deref().unwrap_or_default();
    eprintln!(
        "grammar constrained: {:?}",
        &content[..content.len().min(120)]
    );
    // 无 grammar 时此 query 会输出长篇故事；grammar 生效时必须是 JSON 形状。
    // 小模型在 grammar 框架内会编内容（把 story 名塞进 name），可能被
    // max_tokens 截断——只断言形状（root 规则强制），不断言完整可 parse。
    assert!(
        content.trim_start().starts_with('{'),
        "grammar 应强制输出 JSON，实际: {content:?}"
    );
    assert!(
        content.contains("\"tool_calls\""),
        "root 规则要求 tool_calls 键，实际: {content:?}"
    );
}
