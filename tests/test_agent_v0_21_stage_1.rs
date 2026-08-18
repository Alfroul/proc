//! v0.21 stage 1 — TUI AgentPanel 骨架集成测试（Spike）。
//!
//! 覆盖：
//! 1. `src/agent/session.rs` 类型骨架（SessionEvent 8 变体 / ConfirmRequest /
//!    ConfirmDecision / MAX_HISTORY_TURNS / oneshot 协议 round-trip）
//! 2. `AppMode::Agent` 第 10 变体 + palette 入口（`A` 键与告警弹窗冲突，stage 1
//!    按 stage doc 风险 2 预授权 fallback 走命令面板唯一入口）+ Ctrl+D/Esc 退出
//! 3. 占位渲染不 panic（TestBackend）
//! 4. CLI ask / MCP tool 数回归锚（v0.20 路径不动）

use clap::Parser;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use proc::agent::runner::StopCause;
use proc::agent::session::{ConfirmDecision, ConfirmRequest, MAX_HISTORY_TURNS, SessionEvent};
use proc::app::App;
use proc::app_panel::AppMode;
use proc::tui::command_palette::{CommandAction, CommandPalette};
use proc::view_models::{AgentPanelController, AgentPanelMode};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

// ── session.rs 骨架 ─────────────────────────────────────────────────────────

#[test]
fn test_session_module_compiles() {
    // 类型可 import 且可作类型标注（AgentSession / SessionHandle 是空 struct，
    // 字段私有不可构造，stage 2 实装 spawn 后才有构造路径）。
    fn assert_types(
        _: &SessionEvent,
        _: &proc::agent::session::AgentSession,
        _: &proc::agent::session::SessionHandle,
    ) {
    }
    let _ = assert_types
        as fn(
            &SessionEvent,
            &proc::agent::session::AgentSession,
            &proc::agent::session::SessionHandle,
        );
}

#[test]
fn test_session_event_variants_constructible() {
    // 不含 oneshot 的 7 个变体可构造（ConfirmRequested 见 round-trip 测试）。
    let events = [
        SessionEvent::QueryStarted("列出 top 10 进程".to_string()),
        SessionEvent::TextDelta("根据".to_string()),
        SessionEvent::ToolStart {
            name: "proc_ls".to_string(),
            arguments: serde_json::json!({"limit": 10}),
        },
        SessionEvent::ToolFinished {
            name: "proc_ls".to_string(),
            is_error: false,
            result_chars: 1234,
        },
        SessionEvent::TurnFinished,
        SessionEvent::SessionFinished {
            final_text: "回答".to_string(),
            stop: StopCause::EndTurn,
        },
        SessionEvent::Error("ctx 溢出".to_string()),
    ];
    assert_eq!(events.len(), 7);
    // Debug derive 可用（SessionEvent 派生链含 oneshot::Sender 也实现 Debug）。
    assert!(format!("{:?}", events[0]).contains("QueryStarted"));
}

#[test]
fn test_confirm_decision_variants() {
    // 决策 D3：Approved（confirm: true 真执行）/ Denied（blocked JSON）。
    assert_ne!(ConfirmDecision::Approved, ConfirmDecision::Denied);
    // Copy 语义（面板 y/n 回传走 oneshot 单次 send）。
    let d = ConfirmDecision::Approved;
    let copy = d;
    assert_eq!(d, copy);
}

#[test]
fn test_confirm_request_oneshot_roundtrip() {
    // ConfirmRequest 协议形状：reply oneshot 通道 send → blocking_recv 收到。
    // （blocking_recv 不需要 tokio runtime，测试零 async 基础设施。）
    let (tx, rx) = tokio::sync::oneshot::channel();
    let req = ConfirmRequest {
        tool_name: "proc_kill".to_string(),
        arguments: serde_json::json!({"pids": [1234], "confirm": false}),
        summary: "终止进程 1234 (chrome)".to_string(),
        reply: tx,
    };
    assert_eq!(req.tool_name, "proc_kill");
    assert!(req.summary.contains("1234"));
    // 面板按 y → Approved 回传。
    req.reply.send(ConfirmDecision::Approved).unwrap();
    assert_eq!(rx.blocking_recv(), Ok(ConfirmDecision::Approved));
}

#[test]
fn test_max_history_turns_constant() {
    // 决策 D4 锚定：滑动窗口 = system prompt + 最近 12 轮（代码常量，不预加配置）。
    assert_eq!(MAX_HISTORY_TURNS, 12);
    // mod.rs re-export 同一常量。
    assert_eq!(proc::agent::MAX_HISTORY_TURNS, 12);
}

// ── AppMode::Agent + 入口 / 退出 ────────────────────────────────────────────

#[test]
fn test_app_mode_has_agent_variant() {
    assert_eq!(AppMode::Agent, AppMode::Agent);
    assert_ne!(AppMode::Agent, AppMode::ProcessList);
    assert_ne!(AppMode::Agent, AppMode::Help);
}

#[test]
fn test_palette_contains_agent_entry() {
    let palette = CommandPalette::new();
    let item = palette
        .items()
        .iter()
        .find(|i| i.id == "switch_to_agent_panel")
        .expect("palette 应含 switch_to_agent_panel 条目");
    assert_eq!(item.action, CommandAction::SwitchPanel(AppMode::Agent));
    assert!(item.label.contains("AI Agent"));
}

#[test]
fn test_palette_agent_entry_switches_mode() {
    // E2E：Ctrl+P 打开 → fuzzy 搜 agentpanel → 选中 AI Agent 条目 → Enter 切模式。
    // （`A` 键已被「打开告警弹窗」占用，palette 是唯一键盘入口——决策 8 注记。）
    let mut app = App::new().expect("App::new");
    assert_eq!(app.mode, AppMode::ProcessList);

    app.handle_key(ctrl(KeyCode::Char('p')));
    assert!(app.is_palette_open(), "Ctrl+P 应打开命令面板");

    for c in "agentpanel".chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
    let selected = app
        .command_palette
        .selected_item()
        .expect("fuzzy 查询应命中 AI Agent 条目");
    assert_eq!(selected.id, "switch_to_agent_panel");

    app.handle_key(key(KeyCode::Enter));
    assert!(!app.is_palette_open(), "Enter 应关闭面板并执行");
    assert_eq!(app.mode, AppMode::Agent, "palette Enter 应切到 Agent 模式");
}

#[test]
fn test_a_key_still_toggles_alert_popup() {
    // 回归锚（stage doc 风险 2 fallback 的决策记录）：`A` 键保持 v0.7 既有语义
    // 「打开告警弹窗」，不被 Agent 面板占用。未来重拍键位时此测试随决策更新。
    let mut app = App::new().expect("App::new");
    assert_eq!(app.mode, AppMode::ProcessList);
    assert!(!app.alert_popup_open);
    app.handle_key(key(KeyCode::Char('A')));
    assert!(
        app.alert_popup_open,
        "A 键应打开告警弹窗（不被 Agent 占用）"
    );
    // 弹窗打开时 A/Esc 关闭（既有行为）。
    app.handle_key(key(KeyCode::Char('A')));
    assert!(!app.alert_popup_open);
}

#[test]
fn test_ctrl_d_exits_agent_mode() {
    let mut app = App::new().expect("App::new");
    app.mode = AppMode::Agent;
    app.handle_key(ctrl(KeyCode::Char('d')));
    assert_eq!(app.mode, AppMode::ProcessList);
}

#[test]
fn test_esc_exits_agent_mode() {
    let mut app = App::new().expect("App::new");
    app.mode = AppMode::Agent;
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.mode, AppMode::ProcessList);
}

// ── AgentPanelController / 渲染骨架 ─────────────────────────────────────────

#[test]
fn test_agent_panel_controller_skeleton() {
    let controller = AgentPanelController::new();
    assert_eq!(controller.panel().mode, AgentPanelMode::Idle);
    assert!(controller.panel().input.is_empty());
    // Default 同 new（App::new 初始化路径）。
    assert_eq!(
        AgentPanelController::default().panel().mode,
        AgentPanelMode::Idle
    );
    // 状态机三变体可枚举（AwaitingConfirm 在 stage 3 confirm UI 使用）。
    let modes = [
        AgentPanelMode::Idle,
        AgentPanelMode::Streaming,
        AgentPanelMode::AwaitingConfirm,
    ];
    assert_eq!(modes.len(), 3);
    assert_ne!(AgentPanelMode::Idle, AgentPanelMode::Streaming);
}

#[test]
fn test_agent_panel_draw_does_not_panic() {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut app = App::new().expect("App::new");
    app.mode = AppMode::Agent;
    let backend = TestBackend::new(60, 20);
    let mut terminal = Terminal::new(backend).expect("Terminal::new");
    // 占位渲染不 panic、不读 session（stage 1 无 session 可读）。
    terminal
        .draw(|f| proc::tui::layout::draw(f, &app))
        .expect("draw");
}

// ── v0.20 路径回归锚 ────────────────────────────────────────────────────────

#[test]
fn test_cli_agent_ask_unchanged() {
    // CLI ask 仍走 v0.20 路径（非流式 + 写操作拦截），stage 1 零改动。
    let cli = proc::cli::Cli::try_parse_from(["proc", "agent", "ask", "列出 top 10 进程"]).unwrap();
    match cli.command {
        Some(proc::cli::Command::Agent {
            sub:
                proc::cli::def::AgentSub::Ask {
                    ref query,
                    max_steps,
                    ..
                },
        }) => {
            assert_eq!(query, "列出 top 10 进程");
            assert_eq!(max_steps, 10);
        }
        other => panic!("expected Agent(Ask), got {other:?}"),
    }
}

#[test]
fn test_mcp_tool_count_unchanged() {
    // v0.21 cycle 不动 MCP 层：46 tool 不变（brainstorm 验证矩阵锚点）。
    let names = proc::mcp::handler::list_tool_names();
    assert_eq!(names.len(), 46, "MCP tool 总数应为 46（v0.21 不变）");
    assert!(names.iter().all(|n| n.starts_with("proc_")));
}
