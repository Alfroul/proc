//! v0.20 stage 3b 集成测试 — dispatch 层 + AgentRunner ReAct loop + CLI 侧。
//!
//! CI 测试零 LLM 调用（决策 G：loop 逻辑测试用本文件内 ScriptedProvider 逐轮
//! 弹出脚本化响应；MockProvider 保持单轮回放语义——多轮 tool loop 中「最后
//! 一条 user message」hash 不变会重复命中同一 fixture 死循环）。
//!
//! 两个 `#[ignore]` 真实测试（本机显式跑，brainstorm 风险 1 验收口径）：
//! - `test_agent_stage3b_acceptance`：50 query（L0 23 + L1 27）完整 agent loop，
//!   断言 expected tool 被调用 + final_text 非空；L0 23/23 硬性、L1 ≥ 22/27
//! - `test_agent_stage3b_record_real_fixtures`：FixtureRecorder + LlamaCppProvider
//!   真实录制覆盖 seed fixture（决策 C / 决策 5）
//!
//! 运行（两个真实测试串行，同机 6GB VRAM 只跑 1 个 llama-server 实例）：
//! ```text
//! cargo test --release --test test_agent_v0_20_stage_3b -- --ignored --test-threads=1
//! ```

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{Value, json};

use proc::agent::provider::{
    CompleteOptions, CompleteResponse, Delta, LlmError, LlmProvider, ProviderStream, StopReason,
    Usage,
};
use proc::agent::runner::{AgentOptions, AgentRunner, StepEvent, StepTrace, StopCause};
use proc::agent::tool_registry::ToolRegistry;
use proc::agent::tools::{catalog, dispatch};
use proc::agent::types::{Message, Role, ToolCall};

// ===========================================================================
// ScriptedProvider — 逐 complete 弹出脚本化响应（决策 G）
// ===========================================================================

struct ScriptedProvider {
    responses: Mutex<VecDeque<CompleteResponse>>,
    seen_messages: Mutex<Vec<Vec<Message>>>,
    seen_tool_counts: Mutex<Vec<usize>>,
    seen_options: Mutex<Vec<Option<proc::agent::provider::ToolChoice>>>,
}

impl ScriptedProvider {
    fn new(responses: Vec<CompleteResponse>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            seen_messages: Mutex::new(Vec::new()),
            seen_tool_counts: Mutex::new(Vec::new()),
            seen_options: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl LlmProvider for ScriptedProvider {
    fn name(&self) -> &'static str {
        "scripted"
    }

    async fn complete(
        &self,
        messages: Vec<Message>,
        tools: Vec<proc::agent::types::ToolSchema>,
        options: CompleteOptions,
    ) -> Result<CompleteResponse, LlmError> {
        self.seen_messages.lock().unwrap().push(messages);
        self.seen_tool_counts.lock().unwrap().push(tools.len());
        self.seen_options.lock().unwrap().push(options.tool_choice);
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or(LlmError::StreamEnded)
    }

    fn stream(
        &self,
        _messages: Vec<Message>,
        _tools: Vec<proc::agent::types::ToolSchema>,
        _options: CompleteOptions,
    ) -> ProviderStream<'static> {
        // runner 只走 complete（决策 D）；stream 不参与测试。
        futures_util::stream::empty::<Result<Delta, LlmError>>().boxed()
    }
}

fn text_resp(text: &str) -> CompleteResponse {
    CompleteResponse {
        message: Message::new(Role::Assistant, text),
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
    }
}

fn tool_resp(name: &str, args: Value) -> CompleteResponse {
    CompleteResponse {
        message: Message {
            role: Role::Assistant,
            content: None,
            tool_calls: vec![ToolCall {
                id: format!("call-{name}"),
                name: name.to_string(),
                arguments: args,
            }],
            tool_results: Vec::new(),
        },
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
    }
}

fn empty_resp() -> CompleteResponse {
    CompleteResponse {
        message: Message {
            role: Role::Assistant,
            content: None,
            tool_calls: Vec::new(),
            tool_results: Vec::new(),
        },
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
    }
}

fn call(name: &str, args: Value) -> ToolCall {
    ToolCall {
        id: format!("call-{name}"),
        name: name.to_string(),
        arguments: args,
    }
}

fn registry() -> ToolRegistry {
    catalog::default_registry()
}

// ===========================================================================
// dispatch 层测试（CI）
// ===========================================================================

#[test]
fn test_dispatch_proc_ls_returns_processes_with_default_limit() {
    let result = dispatch::execute_tool(&registry(), &call("proc_ls", json!({})));
    assert!(!result.is_error);
    let v: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["ok"], json!(true));
    assert!(v["count"].as_u64().unwrap() > 0, "真实机器应有进程");
    // 默认 limit 20 + note 提示（决策 E）。
    assert!(v["count"].as_u64().unwrap() <= 20);
    assert!(v["note"].is_string());
    assert!(v["processes"][0]["name"].is_string());
}

#[test]
fn test_dispatch_proc_ls_filter_path() {
    let result = dispatch::execute_tool(
        &registry(),
        &call("proc_ls", json!({"filter": "cpu > -1", "limit": 5})),
    );
    let v: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["ok"], json!(true));
    assert!(v["count"].as_u64().unwrap() <= 5);

    // 非法 filter → ok:false 业务错误（不 panic）。
    let bad = dispatch::execute_tool(&registry(), &call("proc_ls", json!({"filter": "cpu >>>"})));
    let bv: Value = serde_json::from_str(&bad.content).unwrap();
    assert_eq!(bv["ok"], json!(false));
}

#[test]
fn test_dispatch_proc_metrics_system_ok() {
    let result = dispatch::execute_tool(&registry(), &call("proc_metrics_system", json!({})));
    assert!(!result.is_error);
    let v: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["ok"], json!(true));
    assert!(v["cpu_usage_pct"].is_number());
}

#[test]
fn test_dispatch_proc_help_docker_category() {
    let result = dispatch::execute_tool(
        &registry(),
        &call("proc_help", json!({"category": "docker"})),
    );
    let v: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["ok"], json!(true));
    assert_eq!(v["count"], json!(10));
    let names: Vec<&str> = v["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"proc_docker_ps"));
}

#[test]
fn test_dispatch_unknown_tool_is_error() {
    let result = dispatch::execute_tool(&registry(), &call("proc_nope", json!({})));
    assert!(result.is_error);
    let v: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["ok"], json!(false));
    assert!(v["error"].as_str().unwrap().contains("proc_help"));
}

#[test]
fn test_dispatch_write_tools_blocked() {
    for name in dispatch::WRITE_TOOL_NAMES {
        let result = dispatch::execute_tool(&registry(), &call(name, json!({"pid": 9999})));
        assert!(result.is_error, "{name} 应被拦截");
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(v["ok"], json!(false), "{name}");
        assert_eq!(v["blocked"], json!(true), "{name}");
    }
}

#[test]
fn test_dispatch_missing_required_args_is_business_error() {
    // proc_eject_status 缺 drive → ok:false 业务 JSON（is_error=false——业务层
    // 失败让 LLM 读语义自行调整）。
    let result = dispatch::execute_tool(&registry(), &call("proc_eject_status", json!({})));
    let v: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["ok"], json!(false));
    assert!(v["error"].as_str().unwrap().contains("drive"));
}

#[test]
fn test_truncate_result() {
    let short = "hello";
    assert_eq!(dispatch::truncate_result(short), "hello");

    let long: String = "x".repeat(dispatch::MAX_TOOL_RESULT_CHARS + 100);
    let out = dispatch::truncate_result(&long);
    assert!(out.contains("[truncated, original"));
    assert!(out.chars().count() < long.chars().count() + 40);
}

#[test]
fn test_pii_filter_json_form() {
    // 直接形态：secret 名字段 → 值被 mask。
    let input = r#"{"api_key":"sk-1234567890abcdef","api_version":"v1"}"#;
    let out = dispatch::apply_pii_filter(input);
    assert!(out.contains("sk***("), "secret 值应被 mask: {out}");
    assert!(!out.contains("sk-1234567890abcdef"), "原值不应残留: {out}");
    // 短值（< 8 chars）不误伤（风险 5）。
    assert!(
        out.contains(r#""api_version":"v1""#),
        "短值不应被 mask: {out}"
    );
}

#[test]
fn test_pii_filter_env_vars_shape_documents_double_layer() {
    // env tab 真实形状：{"key":"VAR_NAME","value":"..."}——值由 MCP 层
    // reveal=false 先 mask 一层；agent 层对「值本身长得像 secret 名」的 key
    // 字段（≥8 chars）再 mask 一层（宁可误检，与 env_mask 同哲学）。
    // 短 key 名（PATH）不受影响。
    let input = r#"{"env_vars":[{"key":"PATH","value":"C:\Windows"},{"key":"AWS_SECRET_ACCESS_KEY","value":"wJ***(40 B)"}]}"#;
    let out = dispatch::apply_pii_filter(input);
    assert!(
        out.contains(r#""key":"PATH""#),
        "短 key 名不应被 mask: {out}"
    );
    assert!(
        !out.contains("AWS_SECRET_ACCESS_KEY"),
        "长得像 secret 名的长 key 应被 mask: {out}"
    );
}

#[test]
fn test_pii_filter_kv_form() {
    let input = "cmd: chrome.exe --api-key=AKIA1234567890ABCD --headless";
    let out = dispatch::apply_pii_filter(input);
    assert!(out.contains("AK***("), "kv 形态 secret 应被 mask: {out}");
    assert!(!out.contains("AKIA1234567890ABCD"), "原值不应残留: {out}");
}

// ===========================================================================
// AgentRunner loop 测试（CI，ScriptedProvider）
// ===========================================================================

fn runner_with(script: Vec<CompleteResponse>) -> (AgentRunner, Arc<ScriptedProvider>) {
    let provider = Arc::new(ScriptedProvider::new(script));
    let runner = AgentRunner::new(
        provider.clone() as Arc<dyn LlmProvider>,
        registry(),
        AgentOptions::default(),
    );
    (runner, provider)
}

#[tokio::test]
async fn test_runner_single_turn_text_answer() {
    let (runner, _p) = runner_with(vec![text_resp("CPU 23%，一切正常")]);
    let outcome = runner.run("当前 CPU 使用率是多少").await.unwrap();
    assert_eq!(outcome.final_text, "CPU 23%，一切正常");
    assert!(outcome.steps.is_empty());
    assert_eq!(outcome.stop, StopCause::EndTurn);
}

#[tokio::test]
async fn test_runner_tool_loop_executes_and_feeds_back() {
    let (runner, provider) = runner_with(vec![
        tool_resp("proc_metrics_system", json!({})),
        text_resp("CPU 23%"),
    ]);
    let outcome = runner.run("系统卡吗").await.unwrap();
    assert_eq!(outcome.stop, StopCause::EndTurn);
    assert_eq!(outcome.final_text, "CPU 23%");
    assert_eq!(outcome.steps.len(), 1);
    assert_eq!(outcome.steps[0].tool_name, "proc_metrics_system");

    // 第二轮 complete 收到的 messages：末尾是 Role::Tool 的 tool_result
    // 回填消息（ReAct 循环核心契约）。
    let seen = provider.seen_messages.lock().unwrap();
    assert_eq!(seen.len(), 2);
    let second = &seen[1];
    let last = second.last().unwrap();
    assert!(matches!(last.role, Role::Tool));
    assert_eq!(last.tool_results.len(), 1);
    let tr: Value = serde_json::from_str(&last.tool_results[0].content).unwrap();
    assert_eq!(tr["ok"], json!(true), "真实 dispatch 执行（非 mock 结果）");
}

#[tokio::test]
async fn test_runner_proc_help_expands_tools_dynamically() {
    // 决策 J：OpenAI 协议下模型只能调用请求 tools 数组里声明的 tool——
    // proc_help(category) 执行后，该类别 schema 动态加入后续轮 tools。
    let (runner, provider) = runner_with(vec![
        tool_resp("proc_help", json!({"category": "docker"})),
        tool_resp("proc_docker_ps", json!({})),
        text_resp("docker 列表如上"),
    ]);
    let outcome = runner.run("列出所有 Docker 容器").await.unwrap();
    assert_eq!(outcome.steps.len(), 2);

    let counts = provider.seen_tool_counts.lock().unwrap();
    assert_eq!(counts[0], 5, "第 1 轮 = entry 4 + proc_finish");
    assert_eq!(
        counts[1], 15,
        "第 2 轮 = 5 + docker 类 10 个（动态扩，决策 J）: {counts:?}"
    );
    // 第 3 轮（proc_docker_ps 结果回填后）保持 15（去重不重复加）。
    assert_eq!(counts[2], 15);
}

#[tokio::test]
async fn test_runner_proc_finish_submits_answer_and_ends() {
    // 决策 I：模型调 proc_finish(answer=...) → 循环终止，answer 成为 final_text，
    // proc_finish 不进 steps trace、不进 dispatch。
    let (runner, _p) = runner_with(vec![
        tool_resp("proc_metrics_system", json!({})),
        tool_resp(
            "proc_finish",
            json!({"answer": "CPU 23%，内存 57%，系统正常。"}),
        ),
    ]);
    let outcome = runner.run("系统卡吗").await.unwrap();
    assert_eq!(outcome.stop, StopCause::EndTurn);
    assert_eq!(outcome.final_text, "CPU 23%，内存 57%，系统正常。");
    assert_eq!(outcome.steps.len(), 1, "proc_finish 不应计入 steps");
    assert_eq!(outcome.steps[0].tool_name, "proc_metrics_system");
}

#[tokio::test]
async fn test_runner_sends_tool_choice_required() {
    // 决策 I：每轮 complete 携带 tool_choice=Required（E2B auto 模式会凭空回答）。
    let (runner, provider) = runner_with(vec![text_resp("ok")]);
    runner.run("q").await.unwrap();
    let opts = provider.seen_options.lock().unwrap();
    assert!(
        opts.iter()
            .all(|o| o == &Some(proc::agent::provider::ToolChoice::Required))
    );
}

#[tokio::test]
async fn test_runner_proc_finish_missing_answer_falls_back() {
    // proc_finish 缺 answer 字段 → 不拦截，继续走正常终止路径。
    let (runner, _p) = runner_with(vec![tool_resp("proc_finish", json!({}))]);
    let outcome = runner.run("q").await.unwrap();
    assert_ne!(outcome.final_text, "");
}

#[tokio::test]
async fn test_runner_max_steps_fallback() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        tool_resp("proc_metrics_system", json!({})),
        tool_resp("proc_metrics_system", json!({})),
        tool_resp("proc_metrics_system", json!({})),
        tool_resp("proc_metrics_system", json!({})), // 多备一个防 off-by-one
    ]));
    let runner = AgentRunner::new(
        provider,
        registry(),
        AgentOptions {
            max_steps: 3,
            ..Default::default()
        },
    );
    let outcome = runner.run("无限调工具").await.unwrap();
    assert_eq!(outcome.stop, StopCause::MaxSteps);
    assert_eq!(outcome.steps.len(), 3, "步数应被 max_steps 截断");
    assert!(outcome.final_text.contains("最大步数"));
}

#[tokio::test]
async fn test_runner_empty_response_nudge_retry() {
    let (runner, provider) = runner_with(vec![empty_resp(), text_resp("这是答案")]);
    let outcome = runner.run("随便问").await.unwrap();
    assert_eq!(outcome.final_text, "这是答案");
    assert_eq!(outcome.stop, StopCause::EndTurn);

    // 第二次 complete 前追加了 nudge user message（决策 D）。
    let seen = provider.seen_messages.lock().unwrap();
    assert_eq!(seen.len(), 2);
    let last = seen[1].last().unwrap();
    assert!(matches!(last.role, Role::User));
    assert!(last.content.as_deref().unwrap().contains("空的"));
}

#[tokio::test]
async fn test_runner_empty_response_gives_up_after_retry() {
    let (runner, _p) = runner_with(vec![empty_resp(), empty_resp()]);
    let outcome = runner.run("再问一次").await.unwrap();
    assert_eq!(outcome.stop, StopCause::EmptyAfterRetry);
    assert!(outcome.final_text.contains("未产出有效回答"));
}

#[tokio::test]
async fn test_runner_system_prompt_injected_with_snapshot() {
    let (runner, provider) = runner_with(vec![text_resp("好的")]);
    runner.run("hi").await.unwrap();
    let seen = provider.seen_messages.lock().unwrap();
    let first = &seen[0][0];
    assert!(matches!(first.role, Role::System));
    let content = first.content.as_deref().unwrap();
    assert!(content.contains("系统运维 agent"), "L1 角色段应注入");
    assert!(
        !content.contains("{{SYSTEM_SNAPSHOT}}"),
        "占位符应被真实快照替换（决策 F）"
    );
    assert!(content.contains("CPU:"), "快照摘要行应注入");
}

#[tokio::test]
async fn test_runner_progress_events_emitted() {
    let (runner, _p) = runner_with(vec![
        tool_resp("proc_metrics_system", json!({})),
        text_resp("done"),
    ]);
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let ev_clone = events.clone();
    runner
        .run_with_progress("q", &move |ev: StepEvent<'_>| match ev {
            StepEvent::LlmTurn(n) => ev_clone.lock().unwrap().push(format!("llm:{n}")),
            StepEvent::ToolStart(name, _) => ev_clone.lock().unwrap().push(format!("tool:{name}")),
        })
        .await
        .unwrap();
    let events = events.lock().unwrap();
    assert!(events.contains(&"llm:0".to_string()));
    assert!(events.contains(&"tool:proc_metrics_system".to_string()));
    assert!(events.contains(&"llm:1".to_string()));
}

#[test]
fn test_step_trace_shape() {
    // StepTrace 字段布局（验收测试按 tool_name 断言 expected tool）。
    let t = StepTrace {
        tool_name: "proc_ls".into(),
        arguments: json!({"limit": 5}),
        is_error: false,
        result_chars: 1234,
    };
    assert_eq!(t.tool_name, "proc_ls");
    assert_eq!(t.result_chars, 1234);
}

// ===========================================================================
// 真实 E2B 测试（#[ignore]，本机显式跑）
// ===========================================================================

/// 用户机器 llama-server / 模型路径（brainstorm 前置依赖段）。
const LLAMA_SERVER: &str = r"D:\llama.cpp\bin\llama-b8685-bin-win-cuda-12.4-x64\llama-server.exe";
const GEMMA_E2B: &str = r"D:\llama.cpp\models\gemma4-e2b\gemma-4-E2B-it-Q4_K_M.gguf";

/// 50 query 验收表（L0 23 + L1 27，query 原文逐字取自 brainstorm 附录 A；
/// expected_tool = 期望被调用的主 tool，trace 命中即算该步通过）。
const QUERY_TABLE: &[(&str, u8, &str, &str)] = &[
    // 场景 1 performance-diagnose
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
    // 场景 2 process-diagnose
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
    // 场景 3 docker
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
    // 场景 4 usb
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
    // 场景 5 security
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
    // 场景 6 recording
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
    // 场景 7 flow
    ("flow", 0, "当前所有 TLS 连接的 SNI 列表", "proc_flows"),
    ("flow", 0, "哪些进程在连接非标准端口？", "proc_flows"),
    ("flow", 1, "TCP 重传率为什么这么高？", "proc_port"),
    ("flow", 1, "chrome 在和哪些远程服务器通信？", "proc_flows"),
    // 场景 8 monitor
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
    // 场景 9 dns
    ("dns", 0, "最近 10 分钟访问过哪些域名？", "proc_dns"),
    ("dns", 0, "chrome 最近查询了哪些 DNS？", "proc_dns"),
    ("dns", 1, "哪些进程在访问广告 / 追踪域名？", "proc_dns"),
    ("dns", 1, "DNS 查询频率有异常吗？", "proc_dns"),
];

fn real_env_available() -> bool {
    let ok =
        std::path::Path::new(LLAMA_SERVER).is_file() && std::path::Path::new(GEMMA_E2B).is_file();
    if !ok {
        eprintln!("[skip] llama-server / 模型不存在，跳过真实 E2B 测试");
    }
    ok
}

fn real_runner(max_steps: u32) -> AgentRunner {
    let provider = proc::agent::llama_cpp_provider::LlamaCppProvider::with_options(
        PathBuf::from(LLAMA_SERVER),
        PathBuf::from(GEMMA_E2B),
        // 决策 J 配套：首轮实测 8192 ctx 多轮 + 动态扩 tools 后 prompt 溢出
        // （12K tokens 400 错），真实测试显式用 16384（6GB VRAM 无压力）。
        proc::agent::llama_server_handle::SpawnOptions {
            ctx_size: Some(16384),
            ..Default::default()
        },
    );
    AgentRunner::new(
        Arc::new(provider),
        catalog::default_registry(),
        AgentOptions {
            max_steps,
            ..Default::default()
        },
    )
}

/// 快速模式（`PROC_AGENT_QUICK=1`）：每 (scenario, level) 抽第 1 条 ≈ 18 query
/// + max_steps=4 + 不重试 ≈ 10-20 分钟。
///
/// 全量 50 query × 重试 × max_steps=10 实测 2.7-4h，隔夜挂跑用。CI / 迭代
/// 开发用快速模式，正式验收用全量。
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

/// L0 全过硬性 + L1 ≥ 80%（brainstorm 风险 1 验收口径；QUICK 模式按抽样数
/// 等比例折算）。
#[cfg(feature = "llama-cpp")]
#[tokio::test]
#[ignore]
async fn test_agent_stage3b_acceptance() {
    if !real_env_available() {
        return;
    }
    let quick = quick_mode();
    let runner = real_runner(if quick { 5 } else { 10 });
    let queries = selected_queries();
    let attempts = if quick { 1 } else { 2 };
    let l0_total = queries.iter().filter(|(_, l, _, _)| *l == 0).count();
    let l1_total = queries.iter().filter(|(_, l, _, _)| *l == 1).count();
    let l1_min = ((l1_total as f64) * 0.8).ceil() as usize;
    eprintln!(
        "== acceptance: {} 模式，{} query（L0 {l0_total} / L1 {l1_total}），max_steps={} ==",
        if quick { "QUICK" } else { "FULL" },
        queries.len(),
        if quick { 5 } else { 10 },
    );

    let mut l0_pass = 0usize;
    let mut l0_fail = Vec::new();
    let mut l1_pass = 0usize;
    let mut l1_fail = Vec::new();

    for (scenario, level, query, expected) in queries {
        let mut passed = false;
        for attempt in 0..attempts {
            let outcome = match runner.run(query).await {
                Ok(o) => o,
                Err(e) => {
                    eprintln!("  [attempt {attempt}] LLM error: {e}");
                    continue;
                }
            };
            let tool_hit = outcome.steps.iter().any(|s| s.tool_name == *expected);
            let text_ok = !outcome.final_text.trim().is_empty()
                && outcome.final_text != "模型未产出有效回答（空响应重试后仍为空）。";
            passed = tool_hit && text_ok;
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
            if passed {
                break;
            }
        }
        if passed {
            if *level == 0 {
                l0_pass += 1;
            } else {
                l1_pass += 1;
            }
        } else if *level == 0 {
            l0_fail.push(format!("L0 {scenario}: {query} (期望 {expected})"));
        } else {
            l1_fail.push(format!("L1 {scenario}: {query} (期望 {expected})"));
        }
    }

    eprintln!("\n===== L0: {l0_pass}/{l0_total} =====");
    eprintln!("===== L1: {l1_pass}/{l1_total} =====");
    assert!(
        l0_pass == l0_total,
        "L0 硬性验收不达标（{l0_pass}/{l0_total}）：\n{}",
        l0_fail.join("\n")
    );
    assert!(
        l1_pass >= l1_min,
        "L1 验收不达标（{l1_pass}/{l1_total} < {l1_min}）：\n{}",
        l1_fail.join("\n")
    );
}

/// 真实录制：删 L0/L1 的 18 个 seed jsonl → FixtureRecorder 逐 query 录制
/// → MockProvider 回放 50 query 确定性验证。
///
/// QUICK 模式（`PROC_AGENT_QUICK=1`）：**不删 seed 文件**，只 append 抽样的
/// ~18 query（MockProvider 加载时后行覆盖前行，录到的真实行覆盖同 query 的
/// seed 行，未录到的保持 seed 兜底）——回放验证只断言录到的 query。
/// 覆盖替换的完整目标（50/50 + 全量回放）用全量模式跑。
#[cfg(all(feature = "llama-cpp", feature = "mock-provider"))]
#[tokio::test]
#[ignore]
async fn test_agent_stage3b_record_real_fixtures() {
    if !real_env_available() {
        return;
    }
    let quick = quick_mode();
    let fixtures_dir = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "\\tests\\fixtures\\agent"
    ));

    // 覆盖替换（全量模式）：先删 L0/L1 的 18 个 seed 文件（L2 seed 留档）。
    // QUICK 模式不删（见 doc comment）。
    if !quick {
        let scenarios = [
            "performance-diagnose",
            "process-diagnose",
            "docker",
            "usb",
            "security",
            "recording",
            "flow",
            "monitor",
            "dns",
        ];
        for s in scenarios {
            for level in [0u8, 1] {
                let p = fixtures_dir.join(format!("{s}-l{level}.jsonl"));
                if p.exists() {
                    std::fs::remove_file(&p).unwrap();
                }
            }
        }
    }

    let provider = proc::agent::llama_cpp_provider::LlamaCppProvider::with_options(
        PathBuf::from(LLAMA_SERVER),
        PathBuf::from(GEMMA_E2B),
        proc::agent::llama_server_handle::SpawnOptions {
            ctx_size: Some(16384),
            ..Default::default()
        },
    );
    let mut recorder =
        proc::agent::record_fixture::FixtureRecorder::new(Box::new(provider), fixtures_dir.clone())
            .with_system_message(proc::agent::runner::build_system_prompt());

    let table = selected_queries();
    let queries: Vec<_> = table
        .iter()
        .map(|(s, l, q, _)| proc::agent::record_fixture::FixtureQuery {
            scenario: s,
            level: *l,
            query: q.to_string(),
        })
        .collect();
    let expected = queries.len();
    let report = recorder.record_all(&queries).await.unwrap();
    eprintln!(
        "录制完成: {} recorded / {} failed（{} 模式，{expected} query）",
        report.recorded,
        report.failed.len(),
        if quick { "QUICK" } else { "FULL" },
    );
    for f in &report.failed {
        eprintln!("  failed: {f}");
    }
    assert_eq!(
        report.recorded, expected,
        "应录制 {expected} query（失败清单见上）：{:?}",
        report.failed
    );

    // 回放验证：MockProvider 加载真实 fixture，录到的 query 确定性 complete。
    let mock = proc::agent::mock_provider::MockProvider::new(fixtures_dir);
    for (_, _, q, _) in &table {
        let resp = mock
            .complete(
                vec![Message::new(Role::User, q.to_string())],
                Vec::new(),
                CompleteOptions::default(),
            )
            .await
            .unwrap_or_else(|e| panic!("回放失败 {q:?}: {e}"));
        assert!(
            !resp.message.tool_calls.is_empty()
                || resp
                    .message
                    .content
                    .as_deref()
                    .is_some_and(|c| !c.is_empty()),
            "回放响应应非空: {q:?}"
        );
    }
    // Delta serde 通道 sanity（fixture 行是 Delta 序列化格式）。
    let _ = Delta::Text("x".into());
}
