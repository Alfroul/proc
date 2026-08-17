//! v0.20 stage 4 集成测试：AnthropicProvider（Messages API client）。
//!
//! 分两段：
//! - 纯逻辑测试（CI 必跑，零网络）：消息转换（system 顶层提取 / tool_result
//!   进 user 消息 / 空 assistant 跳过）/ 请求体（input_schema 字段名 /
//!   tool_choice 映射 / 采样参数最多一 / grammar 忽略）/ 非流式响应解析 /
//!   stream SSE 聚合（text_delta / input_json_delta 分片 / EndTurn 恰好一次）
//! - 真实 Sonnet 测试（`#[ignore]`，本机显式跑，需 ANTHROPIC_API_KEY）：
//!   50 query 对照验收（≥ 48/50，brainstorm 风险 1 mitigate 5 口径）
//!
//! 注意：本文件整体 `cfg(feature = "anthropic")`——默认 build 不编译（opt-in
//! feature），`cargo test --release --features anthropic` 才跑。

#![cfg(feature = "anthropic")]

use std::sync::Arc;

use proc::agent::anthropic_provider::{
    StreamState, ToolUseAccum, build_request_body, messages_to_anthropic, parse_messages_response,
};
use proc::agent::provider::{CompleteOptions, Delta, LlmError, LlmProvider, ToolChoice};
use proc::agent::runner::{AgentOptions, AgentRunner};
use proc::agent::tools::catalog;
use proc::agent::types::{Message, Role, ToolCall, ToolResult, ToolSchema};

// ---------------------------------------------------------------------------
// 消息转换（决策 B 转换表）
// ---------------------------------------------------------------------------

#[test]
fn messages_system_extracted_to_top_level() {
    let messages = vec![
        Message::new(Role::System, "你是系统运维 agent"),
        Message::new(Role::User, "列出 CPU 最高的进程"),
    ];
    let (system, msgs) = messages_to_anthropic(&messages);
    assert_eq!(system.as_deref(), Some("你是系统运维 agent"));
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["role"], "user");
    assert_eq!(msgs[0]["content"], "列出 CPU 最高的进程");
    // system 不能出现在 messages 里（Anthropic 不允许）。
    assert!(msgs.iter().all(|m| m["role"] != "system"));
}

#[test]
fn messages_multiple_system_joined() {
    let messages = vec![
        Message::new(Role::System, "第一段"),
        Message::new(Role::System, "第二段"),
        Message::new(Role::User, "q"),
    ];
    let (system, msgs) = messages_to_anthropic(&messages);
    assert_eq!(system.as_deref(), Some("第一段\n\n第二段"));
    assert_eq!(msgs.len(), 1);
}

#[test]
fn messages_assistant_tool_calls_become_tool_use_blocks() {
    let messages = vec![Message {
        role: Role::Assistant,
        content: Some("我先看看".to_string()),
        tool_calls: vec![ToolCall {
            id: "toolu_01".to_string(),
            name: "proc_ls".to_string(),
            arguments: serde_json::json!({"limit": 5}),
        }],
        tool_results: Vec::new(),
    }];
    let (_, msgs) = messages_to_anthropic(&messages);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["role"], "assistant");
    let blocks = msgs[0]["content"].as_array().unwrap();
    assert_eq!(blocks[0]["type"], "text");
    assert_eq!(blocks[0]["text"], "我先看看");
    assert_eq!(blocks[1]["type"], "tool_use");
    assert_eq!(blocks[1]["id"], "toolu_01");
    assert_eq!(blocks[1]["name"], "proc_ls");
    assert_eq!(blocks[1]["input"]["limit"], 5);
}

#[test]
fn messages_tool_results_become_user_tool_result_blocks() {
    let messages = vec![Message {
        role: Role::Tool,
        content: None,
        tool_calls: Vec::new(),
        tool_results: vec![
            ToolResult {
                tool_call_id: "toolu_01".to_string(),
                content: "{\"ok\":true}".to_string(),
                is_error: false,
            },
            ToolResult {
                tool_call_id: "toolu_02".to_string(),
                content: "{\"ok\":false}".to_string(),
                is_error: true,
            },
        ],
    }];
    let (system, msgs) = messages_to_anthropic(&messages);
    assert!(system.is_none());
    assert_eq!(msgs.len(), 1);
    // Anthropic 要求 tool_result 在 user 消息里。
    assert_eq!(msgs[0]["role"], "user");
    let blocks = msgs[0]["content"].as_array().unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0]["type"], "tool_result");
    assert_eq!(blocks[0]["tool_use_id"], "toolu_01");
    assert_eq!(blocks[1]["tool_use_id"], "toolu_02");
}

#[test]
fn messages_empty_assistant_skipped() {
    // runner 空响应重试路径 push 的空 assistant 占位——Anthropic 要求每条
    // 消息 content 非空，整条跳过。
    let messages = vec![
        Message::new(Role::User, "q"),
        Message {
            role: Role::Assistant,
            content: None,
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
        },
        Message::new(Role::User, "nudge"),
    ];
    let (_, msgs) = messages_to_anthropic(&messages);
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["content"], "q");
    assert_eq!(msgs[1]["content"], "nudge");
}

#[test]
fn messages_tool_use_null_arguments_become_empty_object() {
    let messages = vec![Message {
        role: Role::Assistant,
        content: None,
        tool_calls: vec![ToolCall {
            id: "toolu_01".to_string(),
            name: "proc_finish".to_string(),
            arguments: serde_json::Value::Null,
        }],
        tool_results: Vec::new(),
    }];
    let (_, msgs) = messages_to_anthropic(&messages);
    let blocks = msgs[0]["content"].as_array().unwrap();
    assert_eq!(blocks[0]["input"], serde_json::json!({}));
}

// ---------------------------------------------------------------------------
// 请求体（决策 C / D）
// ---------------------------------------------------------------------------

fn sample_tools() -> Vec<ToolSchema> {
    vec![ToolSchema {
        name: "proc_ls".to_string(),
        description: "列出进程".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {"limit": {"type": "integer"}}
        }),
        category: proc::agent::types::ToolCategory::Process,
        estimated_tokens: 60,
    }]
}

#[test]
fn request_body_schema_field_is_input_schema() {
    let messages = vec![Message::new(Role::User, "q")];
    let body = build_request_body(
        "claude-sonnet-4-6",
        1024,
        &messages,
        &sample_tools(),
        &CompleteOptions::default(),
        false,
    );
    assert_eq!(body["model"], "claude-sonnet-4-6");
    assert_eq!(body["max_tokens"], 1024);
    let tool = &body["tools"][0];
    assert_eq!(tool["name"], "proc_ls");
    // Anthropic 键名是 input_schema（不是 parameters）。
    assert!(tool.get("input_schema").is_some());
    assert!(tool.get("parameters").is_none());
    assert_eq!(tool["input_schema"]["type"], "object");
}

#[test]
fn request_body_tool_choice_required_maps_to_any() {
    let messages = vec![Message::new(Role::User, "q")];
    let options = CompleteOptions {
        tool_choice: Some(ToolChoice::Required),
        ..Default::default()
    };
    let body = build_request_body("m", 1024, &messages, &sample_tools(), &options, false);
    assert_eq!(body["tool_choice"]["type"], "any");

    let options = CompleteOptions {
        tool_choice: Some(ToolChoice::Auto),
        ..Default::default()
    };
    let body = build_request_body("m", 1024, &messages, &sample_tools(), &options, false);
    assert_eq!(body["tool_choice"]["type"], "auto");
}

#[test]
fn request_body_no_tool_choice_without_tools() {
    // tools 为空时不带 tool_choice（Anthropic 对空 tools + tool_choice 报 400）。
    let messages = vec![Message::new(Role::User, "q")];
    let options = CompleteOptions {
        tool_choice: Some(ToolChoice::Required),
        ..Default::default()
    };
    let body = build_request_body("m", 1024, &messages, &[], &options, false);
    assert!(body.get("tool_choice").is_none());
    assert!(body.get("tools").is_none());
}

#[test]
fn request_body_sampling_params_at_most_one() {
    let messages = vec![Message::new(Role::User, "q")];
    // temperature + top_p + top_k 同设 → 只留 temperature（Anthropic API 约束）。
    let options = CompleteOptions {
        temperature: Some(0.6),
        top_p: Some(0.9),
        top_k: Some(40),
        ..Default::default()
    };
    let body = build_request_body("m", 1024, &messages, &[], &options, false);
    assert!((body["temperature"].as_f64().unwrap() - 0.6).abs() < 1e-6);
    assert!(body.get("top_p").is_none());
    assert!(body.get("top_k").is_none());

    let options = CompleteOptions {
        top_p: Some(0.9),
        top_k: Some(40),
        ..Default::default()
    };
    let body = build_request_body("m", 1024, &messages, &[], &options, false);
    assert!(body.get("temperature").is_none());
    assert!((body["top_p"].as_f64().unwrap() - 0.9).abs() < 1e-6);
    assert!(body.get("top_k").is_none());
}

#[test]
fn request_body_grammar_and_stop_sequences() {
    let messages = vec![Message::new(Role::User, "q")];
    let options = CompleteOptions {
        grammar: Some("root ::= \"yes\"".to_string()),
        stop_sequences: vec!["END".to_string()],
        ..Default::default()
    };
    let body = build_request_body("m", 1024, &messages, &[], &options, true);
    // GBNF 是 llama.cpp 专属字段，不进 Anthropic 请求体。
    assert!(body.get("grammar").is_none());
    assert_eq!(body["stop_sequences"][0], "END");
    assert_eq!(body["stream"], true);
}

// ---------------------------------------------------------------------------
// 非流式响应解析
// ---------------------------------------------------------------------------

#[test]
fn parse_response_text_only() {
    let body = serde_json::json!({
        "content": [{"type": "text", "text": "当前 CPU 23%"}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 100, "output_tokens": 20}
    });
    let resp = parse_messages_response(&body).unwrap();
    assert_eq!(resp.message.content.as_deref(), Some("当前 CPU 23%"));
    assert!(resp.message.tool_calls.is_empty());
    assert_eq!(resp.stop_reason, proc::agent::provider::StopReason::EndTurn);
    assert_eq!(resp.usage.input_tokens, 100);
    assert_eq!(resp.usage.output_tokens, 20);
}

#[test]
fn parse_response_text_and_tool_use() {
    let body = serde_json::json!({
        "content": [
            {"type": "text", "text": "我先看看"},
            {"type": "tool_use", "id": "toolu_01", "name": "proc_ls",
             "input": {"limit": 10, "sort": "cpu"}}
        ],
        "stop_reason": "tool_use",
        "usage": {"input_tokens": 100, "output_tokens": 30}
    });
    let resp = parse_messages_response(&body).unwrap();
    assert_eq!(resp.message.content.as_deref(), Some("我先看看"));
    assert_eq!(resp.message.tool_calls.len(), 1);
    assert_eq!(resp.message.tool_calls[0].name, "proc_ls");
    assert_eq!(resp.message.tool_calls[0].arguments["sort"], "cpu");
    assert_eq!(resp.stop_reason, proc::agent::provider::StopReason::ToolUse);
}

#[test]
fn parse_response_stop_reason_mapping() {
    for (raw, expected) in [
        ("end_turn", proc::agent::provider::StopReason::EndTurn),
        ("tool_use", proc::agent::provider::StopReason::ToolUse),
        ("max_tokens", proc::agent::provider::StopReason::MaxTokens),
        (
            "stop_sequence",
            proc::agent::provider::StopReason::StopSequence,
        ),
    ] {
        let body = serde_json::json!({
            "content": [{"type": "text", "text": "x"}],
            "stop_reason": raw,
        });
        let resp = parse_messages_response(&body).unwrap();
        assert_eq!(resp.stop_reason, expected, "stop_reason={raw}");
    }
}

#[test]
fn parse_response_error_shape_returns_api_error() {
    let body = serde_json::json!({
        "type": "error",
        "error": {"type": "not_found_error", "message": "model not found"}
    });
    let result = parse_messages_response(&body);
    assert!(matches!(result, Err(LlmError::Api { .. })));
}

#[test]
fn parse_response_missing_content_is_error() {
    let body = serde_json::json!({"stop_reason": "end_turn"});
    assert!(parse_messages_response(&body).is_err());
}

// ---------------------------------------------------------------------------
// stream 聚合
// ---------------------------------------------------------------------------

#[test]
fn stream_accum_input_json_delta_fragments() {
    let mut accum = ToolUseAccum::default();
    accum.on_block_start(
        0,
        &serde_json::json!({"type": "tool_use", "id": "toolu_01", "name": "proc_ls"}),
    );
    accum.on_block_delta(
        0,
        &serde_json::json!({
            "type": "input_json_delta", "partial_json": "{\"li"
        }),
    );
    accum.on_block_delta(
        0,
        &serde_json::json!({
            "type": "input_json_delta", "partial_json": "mit\":10}"
        }),
    );
    let call = accum.take_finished(0).unwrap();
    assert_eq!(call.name, "proc_ls");
    assert_eq!(call.id, "toolu_01");
    assert_eq!(call.arguments["limit"], 10);
    // 槽已清空，重复 stop 不再 yield。
    assert!(accum.take_finished(0).is_none());
}

#[test]
fn stream_accum_non_tool_use_block_yields_none() {
    let mut accum = ToolUseAccum::default();
    accum.on_block_start(0, &serde_json::json!({"type": "text", "text": ""}));
    accum.on_block_delta(
        0,
        &serde_json::json!({
            "type": "text_delta", "text": "hi"
        }),
    );
    assert!(accum.take_finished(0).is_none());
}

#[test]
fn stream_accum_invalid_json_degrades_to_null() {
    let mut accum = ToolUseAccum::default();
    accum.on_block_start(
        0,
        &serde_json::json!({"type": "tool_use", "id": "t", "name": "proc_finish"}),
    );
    accum.on_block_delta(
        0,
        &serde_json::json!({
            "type": "input_json_delta", "partial_json": "not-json"
        }),
    );
    let call = accum.take_finished(0).unwrap();
    assert_eq!(call.arguments, serde_json::Value::Null);
}

#[test]
fn stream_state_text_delta_yields_per_frame() {
    let mut state = StreamState::new();
    let out = state.feed_payload(
        &serde_json::json!({
            "type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": "当前 "}
        })
        .to_string(),
    );
    assert_eq!(out.len(), 1);
    assert!(matches!(&out[0], Ok(Delta::Text(t)) if t == "当前 "));

    let out = state.feed_payload(
        &serde_json::json!({
            "type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": "CPU 23%"}
        })
        .to_string(),
    );
    assert!(matches!(&out[0], Ok(Delta::Text(t)) if t == "CPU 23%"));
}

#[test]
fn stream_state_tool_use_lifecycle_and_end_turn_exactly_once() {
    let mut state = StreamState::new();
    // message_start / ping 忽略。
    assert!(
        state
            .feed_payload(&serde_json::json!({"type": "message_start", "message": {}}).to_string())
            .is_empty()
    );
    assert!(
        state
            .feed_payload(&serde_json::json!({"type": "ping"}).to_string())
            .is_empty()
    );

    // tool_use 块：start → delta → stop 聚合出一个 ToolCall。
    state.feed_payload(
        &serde_json::json!({
            "type": "content_block_start", "index": 1,
            "content_block": {"type": "tool_use", "id": "toolu_09",
                              "name": "proc_finish", "input": {}}
        })
        .to_string(),
    );
    state.feed_payload(
        &serde_json::json!({
            "type": "content_block_delta", "index": 1,
            "delta": {"type": "input_json_delta", "partial_json": "{\"answer\":"}
        })
        .to_string(),
    );
    state.feed_payload(
        &serde_json::json!({
            "type": "content_block_delta", "index": 1,
            "delta": {"type": "input_json_delta", "partial_json": "\"完成\"}"}
        })
        .to_string(),
    );
    let out = state
        .feed_payload(&serde_json::json!({"type": "content_block_stop", "index": 1}).to_string());
    assert_eq!(out.len(), 1);
    match &out[0] {
        Ok(Delta::ToolCall(call)) => {
            assert_eq!(call.name, "proc_finish");
            assert_eq!(call.arguments["answer"], "完成");
        }
        other => panic!("期望 ToolCall，实际 {other:?}"),
    }

    // message_delta stop_reason → EndTurn 恰好一次；message_stop 兜底不重复。
    let out = state.feed_payload(
        &serde_json::json!({
            "type": "message_delta",
            "delta": {"stop_reason": "tool_use"},
            "usage": {"output_tokens": 50}
        })
        .to_string(),
    );
    assert_eq!(out.len(), 1);
    assert!(matches!(
        &out[0],
        Ok(Delta::EndTurn {
            stop_reason: proc::agent::provider::StopReason::ToolUse
        })
    ));
    let out = state.feed_payload(&serde_json::json!({"type": "message_stop"}).to_string());
    assert!(out.is_empty());
}

#[test]
fn stream_state_message_stop_fallback_sends_end_turn() {
    // 异常流缺 message_delta 时 message_stop 兜底补发 EndTurn。
    let mut state = StreamState::new();
    let out = state.feed_payload(&serde_json::json!({"type": "message_stop"}).to_string());
    assert_eq!(out.len(), 1);
    assert!(matches!(&out[0], Ok(Delta::EndTurn { .. })));
}

// ---------------------------------------------------------------------------
// from_env / provider 构造
// ---------------------------------------------------------------------------

// 两个 env 形态断言合一个测试：同 binary 内测试默认并行，set_var / remove_var
// 操纵同一环境变量会互踩（edition 2024 下需 unsafe——全局状态可变）。
#[test]
fn from_env_reads_api_key_and_blank_key_is_error() {
    unsafe { std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-test") };
    let provider = proc::agent::anthropic_provider::AnthropicProvider::from_env(
        "claude-sonnet-4-6".to_string(),
        4096,
    )
    .unwrap();
    assert_eq!(provider.name(), "anthropic");
    assert_eq!(provider.model, "claude-sonnet-4-6");
    assert_eq!(provider.max_tokens, 4096);

    unsafe { std::env::set_var("ANTHROPIC_API_KEY", "   ") };
    let result = proc::agent::anthropic_provider::AnthropicProvider::from_env(
        "claude-sonnet-4-6".to_string(),
        4096,
    );
    assert!(matches!(result, Err(LlmError::Config(_))));
    unsafe { std::env::remove_var("ANTHROPIC_API_KEY") };
}

// ---------------------------------------------------------------------------
// 真实 Sonnet 对照验收（#[ignore]，本机显式跑）
// ---------------------------------------------------------------------------

/// 50 query 验收表（L0 23 + L1 27，与 stage 3b QUERY_TABLE 逐字一致——query
/// 原文取自 brainstorm 附录 A；expected_tool = 期望被调用的主 tool）。
const QUERY_TABLE: &[(&str, u8, &str, &str)] = &[
    (
        "performance-diagnose",
        0,
        "列出 CPU 占用最高的 10 个进程",
        "proc_ls",
    ),
    (
        "performance-diagnose",
        0,
        "当前系统 CPU 和内存使用率是多少",
        "proc_metrics_system",
    ),
    (
        "performance-diagnose",
        0,
        "内存使用最多的 5 个进程",
        "proc_ls",
    ),
    (
        "performance-diagnose",
        1,
        "我电脑为什么这么卡？",
        "proc_metrics_system",
    ),
    (
        "performance-diagnose",
        1,
        "CPU 使用率为什么这么高？",
        "proc_metrics_system",
    ),
    (
        "performance-diagnose",
        1,
        "内存快满了，哪些进程占用最多？",
        "proc_metrics_system",
    ),
    (
        "performance-diagnose",
        1,
        "磁盘 I/O 异常活跃，是什么进程造成的？",
        "proc_metrics_disk_io",
    ),
    (
        "performance-diagnose",
        1,
        "GPU 温度为什么这么高？",
        "proc_metrics_gpu",
    ),
    (
        "process-diagnose",
        0,
        "PID 1234 是什么进程？",
        "proc_inspect",
    ),
    (
        "process-diagnose",
        0,
        "chrome.exe 的完整进程信息",
        "proc_ls",
    ),
    (
        "process-diagnose",
        0,
        "显示 explorer.exe 的环境变量",
        "proc_inspect",
    ),
    (
        "process-diagnose",
        1,
        "chrome.exe 为什么占用这么多内存？",
        "proc_inspect",
    ),
    (
        "process-diagnose",
        1,
        "PID 5678 加载了哪些 DLL？",
        "proc_inspect",
    ),
    (
        "process-diagnose",
        1,
        "哪些进程在监听 8080 端口？",
        "proc_port",
    ),
    (
        "process-diagnose",
        1,
        "PID 1234 打开了哪些文件句柄？",
        "proc_inspect",
    ),
    ("docker", 0, "列出所有 Docker 容器", "proc_docker_ps"),
    ("docker", 0, "nginx 容器的健康状态", "proc_docker_inspect"),
    (
        "docker",
        0,
        "本地有哪些 Docker 镜像？",
        "proc_docker_images",
    ),
    (
        "docker",
        1,
        "postgres 容器为什么 unhealthy？",
        "proc_docker_inspect",
    ),
    (
        "docker",
        1,
        "redis 容器最近的日志有什么异常？",
        "proc_docker_logs",
    ),
    (
        "docker",
        1,
        "哪些 Docker 镜像没在用可以删？",
        "proc_docker_images",
    ),
    ("usb", 0, "E 盘当前的占用情况", "proc_eject_status"),
    ("usb", 0, "E 盘能不能安全弹出？", "proc_eject_status"),
    ("usb", 0, "谁在占用 F 盘？", "proc_eject_status"),
    (
        "usb",
        1,
        "E 盘为什么不能弹出？列出占用进程",
        "proc_eject_status",
    ),
    (
        "usb",
        1,
        "杀掉占用 E 盘的进程后能弹了吗？",
        "proc_eject_status",
    ),
    ("usb", 1, "帮我安全释放 E 盘", "proc_usb_release"),
    ("security", 0, "列出安全分低于 50 的进程", "proc_ls"),
    ("security", 0, "chrome.exe 的签名状态", "proc_inspect"),
    ("security", 0, "哪些进程未签名？", "proc_ls"),
    (
        "security",
        1,
        "PID 1234 为什么安全分这么低？",
        "proc_inspect",
    ),
    ("security", 1, "哪些进程的父子链可疑？", "proc_ls"),
    ("security", 1, "临时目录下运行的进程有哪些？", "proc_ls"),
    (
        "recording",
        0,
        "录屏文件 recording.prec 的元数据",
        "proc_replay_info",
    ),
    (
        "recording",
        0,
        "录屏里有多少个异常事件？",
        "proc_replay_info",
    ),
    (
        "recording",
        1,
        "在录屏里搜 CPU > 80% 的时刻",
        "proc_replay_search",
    ),
    (
        "recording",
        1,
        "书签 #3 那个时刻系统发生了什么？",
        "proc_bookmarks_list",
    ),
    ("flow", 0, "当前所有 TLS 连接的 SNI 列表", "proc_flows"),
    ("flow", 0, "哪些进程在连接非标准端口？", "proc_flows"),
    ("flow", 1, "TCP 重传率为什么这么高？", "proc_port"),
    ("flow", 1, "chrome 在和哪些远程服务器通信？", "proc_flows"),
    (
        "monitor",
        0,
        "当前有哪些 active 告警？",
        "proc_monitor_list",
    ),
    ("monitor", 0, "列出当前所有监控配置", "proc_monitor_list"),
    (
        "monitor",
        1,
        "添加监控：CPU > 90% 持续 3 分钟告警",
        "proc_monitor_add",
    ),
    (
        "monitor",
        1,
        "监控 cargo dev 进程，挂了自动重启",
        "proc_monitor_add",
    ),
    (
        "monitor",
        1,
        "删除 ID 为 2 的监控配置",
        "proc_monitor_remove",
    ),
    ("dns", 0, "最近 10 分钟访问过哪些域名？", "proc_dns"),
    ("dns", 0, "chrome 最近查询了哪些 DNS？", "proc_dns"),
    ("dns", 1, "哪些进程在访问广告 / 追踪域名？", "proc_dns"),
    ("dns", 1, "DNS 查询频率有异常吗？", "proc_dns"),
];

fn anthropic_env_available() -> bool {
    let ok = std::env::var("ANTHROPIC_API_KEY")
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false);
    if !ok {
        eprintln!("[skip] ANTHROPIC_API_KEY 未设置，跳过真实 Sonnet 验收");
    }
    ok
}

fn quick_mode() -> bool {
    std::env::var("PROC_AGENT_QUICK").ok().as_deref() == Some("1")
}

fn selected_queries() -> Vec<&'static (&'static str, u8, &'static str, &'static str)> {
    if !quick_mode() {
        return QUERY_TABLE.iter().collect();
    }
    let mut seen = std::collections::HashSet::new();
    QUERY_TABLE
        .iter()
        .filter(|(s, l, _, _)| seen.insert((*s, *l)))
        .collect()
}

/// Sonnet 对照验收（决策 F）：50 query 合计 ≥ 48/50（brainstorm 风险 1
/// mitigate 5 口径；L0/L1 分项打印但不分别硬性——Sonnet 对两档无能力差异）。
/// QUICK 模式（`PROC_AGENT_QUICK=1`）抽样 ~18 query 仅链路验证，不 assert。
#[tokio::test]
#[ignore]
async fn test_agent_stage4_anthropic_acceptance() {
    if !anthropic_env_available() {
        return;
    }
    let quick = quick_mode();
    let provider = proc::agent::anthropic_provider::AnthropicProvider::from_env(
        proc::agent::anthropic_provider::DEFAULT_MODEL.to_string(),
        4096,
    )
    .unwrap();
    let runner = AgentRunner::new(
        Arc::new(provider),
        catalog::default_registry(),
        AgentOptions {
            max_steps: if quick { 5 } else { 10 },
            ..Default::default()
        },
    );
    let queries = selected_queries();
    let total = queries.len();
    let l0_total = queries.iter().filter(|(_, l, _, _)| *l == 0).count();
    let l1_total = queries.iter().filter(|(_, l, _, _)| *l == 1).count();
    eprintln!(
        "== anthropic acceptance: {} 模式，{} query（L0 {l0_total} / L1 {l1_total}）==",
        if quick { "QUICK" } else { "FULL" },
        queries.len(),
    );

    let mut pass = 0usize;
    let mut fail = Vec::new();
    let mut l0_pass = 0usize;
    let mut l1_pass = 0usize;

    for (scenario, level, query, expected) in queries {
        let outcome = match runner.run(query).await {
            Ok(o) => o,
            Err(e) => {
                eprintln!("  [L{level}] {scenario}: LLM error: {e}");
                fail.push(format!("L{level} {scenario}: {query}（LLM error: {e}）"));
                continue;
            }
        };
        let tool_hit = outcome.steps.iter().any(|s| s.tool_name == *expected);
        let text_ok = !outcome.final_text.trim().is_empty()
            && outcome.final_text != "模型未产出有效回答（空响应重试后仍为空）。";
        let passed = tool_hit && text_ok;
        if passed {
            pass += 1;
            if *level == 0 {
                l0_pass += 1;
            } else {
                l1_pass += 1;
            }
        } else {
            fail.push(format!(
                "L{level} {scenario}: {query}（期望 {expected}，tools: [{}]）",
                outcome
                    .steps
                    .iter()
                    .map(|s| s.tool_name.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        eprintln!(
            "[L{level}] {scenario}: {} (tools: [{}], {} 步, stop: {})",
            if passed { "PASS" } else { "FAIL" },
            outcome
                .steps
                .iter()
                .map(|s| s.tool_name.as_str())
                .collect::<Vec<_>>()
                .join(","),
            outcome.steps.len(),
            outcome.stop.label(),
        );
    }

    eprintln!(
        "\n===== Sonnet 对照: {pass}/{total}（L0 {l0_pass}/{l0_total} / L1 {l1_pass}/{l1_total}）====="
    );
    if !fail.is_empty() {
        eprintln!("失败清单：");
        for f in &fail {
            eprintln!("  {f}");
        }
    }
    if !quick {
        assert!(
            pass >= 48,
            "Sonnet 对照验收不达标（{pass}/{total} < 48）：\n{}",
            fail.join("\n")
        );
    }
}
