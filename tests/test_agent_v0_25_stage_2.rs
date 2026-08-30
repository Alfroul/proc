//! v0.25 stage 2 测试：空会话治理三路径回归（ADR-0035 D1 延迟创建）。
//!
//! - TUI 路径（AgentPanel 语义）：进面板不发问 → 无文件；Error-only → 2 行
//!   文件；query 进行中退出 → 已落盘文件保留（brainstorm 风险 3 边界）
//! - ask / eval 路径：不落盘是**现状**而非治理引入（stage-1 Spike 归因——
//!   build_runner 无 recorder）——锚定「跑完 query 后 sessions 目录不新增
//!   文件」，防未来把 recorder 接进 build_runner 时口径漂移
//!
//! 发问首两行（session_start → query_started）锚由既有
//! `tests/test_agent_v0_22_stage_3.rs::test_session_records_full_log_sequence`
//! 覆盖（完整生命周期开头序断言），本文件不重复。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;

use proc::agent::session_log::{SessionLogEntry, SessionRecorder};
use proc::agent::tools::catalog;
use proc::agent::types::Message;
use proc::agent::{
    AgentOptions, AgentSession, CompleteOptions, CompleteResponse, Delta, LlmError, LlmProvider,
    ProviderStream, SessionEvent,
};

// ===========================================================================
// helpers
// ===========================================================================

fn jsonl_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_entries(path: &Path) -> Vec<SessionLogEntry> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn kind_of(e: &SessionLogEntry) -> String {
    serde_json::to_value(&e.event).unwrap()["kind"]
        .as_str()
        .unwrap()
        .to_string()
}

/// stream 永不 yield 的 provider——模拟「query 进行中」（session 线程阻塞在
/// block_on，测试侧 drop handle 即 TUI 中途退出；detached 线程随进程退出收割）。
struct HangingProvider;

#[async_trait]
impl LlmProvider for HangingProvider {
    fn name(&self) -> &'static str {
        "hanging"
    }

    async fn complete(
        &self,
        _messages: Vec<Message>,
        _tools: Vec<proc::agent::types::ToolSchema>,
        _options: CompleteOptions,
    ) -> Result<CompleteResponse, LlmError> {
        Err(LlmError::Config("streaming only".to_string()))
    }

    fn stream(
        &self,
        _messages: Vec<Message>,
        _tools: Vec<proc::agent::types::ToolSchema>,
        _options: CompleteOptions,
    ) -> ProviderStream<'static> {
        futures_util::stream::pending::<Result<Delta, LlmError>>().boxed()
    }
}

fn spawn_hanging_session(recorder: SessionRecorder) -> proc::agent::SessionHandle {
    AgentSession::spawn(
        Arc::new(HangingProvider),
        catalog::default_registry(),
        AgentOptions {
            max_steps: 3,
            ..Default::default()
        },
        recorder,
        None,
    )
}

// ===========================================================================
// TUI 路径（AgentPanel 语义）
// ===========================================================================

#[test]
fn d1_recorder_no_events_no_file() {
    // 治理主断言：构造成功（is_enabled 语义不变——目录可写性检查留在构造时）
    // 但无任何事件 → 不落盘。原行为是构造即写 session_start 单行空文件。
    let dir = tempfile::tempdir().unwrap();
    let rec = SessionRecorder::start_in_dir(dir.path(), "mock");
    assert!(rec.is_enabled());
    assert!(
        jsonl_files(dir.path()).is_empty(),
        "无事件 → 无文件（D1 延迟创建）"
    );
}

#[test]
fn d1_error_only_session_materializes_two_line_file() {
    // ADR-0035 D1：Error 事件也触发落盘（诊断价值：留 2 行文件）。
    let dir = tempfile::tempdir().unwrap();
    let rec = SessionRecorder::start_in_dir(dir.path(), "mock");
    rec.log(&SessionEvent::Error("tokio runtime 创建失败".to_string()));
    let files = jsonl_files(dir.path());
    assert_eq!(files.len(), 1, "Error 触发物化");
    let kinds: Vec<String> = parse_entries(&files[0]).iter().map(kind_of).collect();
    assert_eq!(kinds, vec!["session_start", "error"]);
}

#[test]
fn d1_session_no_query_no_file() {
    // AgentPanel 语义：进面板（spawn session）→ 不发问 → 退出（drop handle）。
    let dir = tempfile::tempdir().unwrap();
    let rec = SessionRecorder::start_in_dir(dir.path(), "hanging");
    let handle = spawn_hanging_session(rec);
    drop(handle);
    assert!(jsonl_files(dir.path()).is_empty(), "不发问的会话不产文件");
}

#[test]
fn d1_query_in_progress_exit_preserves_file() {
    // brainstorm 风险 3 边界：query 进行中 TUI 退出 → 已落盘文件保留
    //（有 QueryStarted——「慢启动但有内容」的会话不在治理误伤范围）。
    let dir = tempfile::tempdir().unwrap();
    let rec = SessionRecorder::start_in_dir(dir.path(), "hanging");
    let handle = spawn_hanging_session(rec);
    assert!(handle.send_query("进行中的问题"));
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        match handle.drain_event() {
            Some(SessionEvent::QueryStarted(_)) => break,
            Some(_) => {}
            None if std::time::Instant::now() > deadline => {
                panic!("10s 内未收到 QueryStarted")
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    drop(handle); // TUI 中途退出（query 仍在 HangingProvider 上阻塞）
    let files = jsonl_files(dir.path());
    assert_eq!(files.len(), 1, "进行中会话的文件保留");
    let kinds: Vec<String> = parse_entries(&files[0]).iter().map(kind_of).collect();
    assert_eq!(kinds, vec!["session_start", "query_started"], "首两行口径");
}

// ===========================================================================
// ask / eval 路径锚（现状显式化）
// ===========================================================================

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// USERPROFILE 重定向 guard（test_path_rules.rs 先例模式；drop 恢复原值）。
/// 本测试二进制内仅此测试写 USERPROFILE。
struct TempEnvGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    prev: Option<String>,
}

impl TempEnvGuard {
    fn redirect_userprofile(dir: &str) -> Self {
        let guard = Self {
            _lock: ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap(),
            prev: std::env::var("USERPROFILE").ok(),
        };
        // SAFETY: 持有 ENV_LOCK，本二进制内无并发 set_var。
        unsafe { std::env::set_var("USERPROFILE", dir) };
        guard
    }
}

impl Drop for TempEnvGuard {
    fn drop(&mut self) {
        // SAFETY: 同上。
        match self.prev.take() {
            Some(v) => unsafe { std::env::set_var("USERPROFILE", v) },
            None => unsafe { std::env::remove_var("USERPROFILE") },
        }
    }
}

#[test]
fn ask_eval_path_creates_no_session_file() {
    // stage-1 Spike 归因：ask / eval 走 build_runner（无 recorder）——不落盘
    // 是现状而非 D1 治理引入。eval 与 ask 共享 build_runner 构造链
    //（AgentRunner 无 recorder 字段），ask 锚即覆盖两路径的接线口径。
    // 实跑一个 mock fixture query（与 run_agent_ask 同款 run_with_progress 调用），
    // 若未来 recorder 接进 build_runner，QueryStarted 会让本锚变红。
    let tmp = tempfile::tempdir().unwrap();
    let _guard = TempEnvGuard::redirect_userprofile(&tmp.path().to_string_lossy());
    let sessions = tmp.path().join(".config").join("proc").join("sessions");

    let (runner, _spec) =
        proc::agent::builder::build_runner(Some("mock"), None, 6).expect("build_runner(mock)");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(runner.run_with_progress("当前系统 CPU 和内存使用率是多少", &|_| {}))
        .expect("mock fixture query 应跑通（tests/fixtures/agent 既有命中）");

    assert!(
        jsonl_files(&sessions).is_empty(),
        "ask/eval 路径不应落 session 文件（现状锚）"
    );
}
