//! v0.11 阶段 5：父子链 + R17 集成测试。
//!
//! 覆盖 `SecurityScorer::score` 第 17 步接入：mock `ProcessInfo.parent_chain`
//! + `name` 触发各 `SuspiciousPattern`，验证 `RiskFactor` 生成 + 权重。
//!
//! `build_parent_chain` / `detect_suspicious_chain` / `LineageRule` 解析的
//! 细粒度单元测试在 `src/security/lineage.rs` 内嵌（模块私有）。

use proc::collect::ProcessInfo;
use proc::security::{RiskCategory, SecurityScorer};

fn make_proc(
    pid: u32,
    name: &str,
    exe: Option<&str>,
    parent_chain: Vec<(u32, String)>,
) -> ProcessInfo {
    let name_arc: std::sync::Arc<str> = std::sync::Arc::from(name);
    ProcessInfo {
        pid,
        name: std::sync::Arc::clone(&name_arc),
        cpu_usage: 0.0,
        memory: 0,
        virtual_memory: 0,
        disk_usage: (0, 0),
        disk_read_speed: 0,
        disk_write_speed: 0,
        net_sent_rate: 0,
        net_recv_rate: 0,
        status: proc::collect::ProcessStatus::Run,
        exe: exe.map(std::sync::Arc::from),
        cmd: std::sync::Arc::from(Vec::<String>::new()),
        cwd: None,
        // parent_pid 与 parent_chain[0] 对齐，模拟 HeavyWorker 真实填充结果。
        parent_pid: parent_chain.first().map(|(p, _)| *p),
        session_id: None,
        user_id: None,
        start_time: 0,
        run_time: 0,
        name_lower: std::sync::Arc::from(name_arc.to_lowercase().as_str()),
        throttled: proc::throttle::EcoQoSState::default(),
        signature_status: proc::security::SignatureStatus::default(),
        parent_chain,
    }
}

/// R17 OfficeToShell：cmd ← WINWORD 应扣 35 分。
#[test]
fn r17_office_to_shell_powershell_weights_35() {
    let cmd_proc = make_proc(
        200,
        "powershell.exe",
        Some("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"),
        vec![(100, "WINWORD.EXE".to_string())],
    );
    let all = vec![cmd_proc.clone()];
    let mut scorer = SecurityScorer::new();
    let score = scorer.score(&cmd_proc, &all, &[], &[]);
    let r17 = score
        .factors
        .iter()
        .find(|f| f.name == "office_to_shell")
        .unwrap_or_else(|| panic!("应命中 R17 OfficeToShell，factors: {:?}", score.factors));
    assert_eq!(r17.weight, 35);
    assert_eq!(r17.category, RiskCategory::ParentChain);
}

#[test]
fn r17_office_to_shell_cmd_weights_35() {
    let cmd_proc = make_proc(
        201,
        "cmd.exe",
        Some("C:\\Windows\\System32\\cmd.exe"),
        vec![(101, "excel.exe".to_string())],
    );
    let all = vec![cmd_proc.clone()];
    let mut scorer = SecurityScorer::new();
    let score = scorer.score(&cmd_proc, &all, &[], &[]);
    assert!(
        score
            .factors
            .iter()
            .any(|f| f.name == "office_to_shell" && f.weight == 35),
        "Excel → cmd 应命中 OfficeToShell 35: {:?}",
        score.factors
    );
}

/// R17 BrowserToShell：cmd ← chrome 应扣 25 分。
#[test]
fn r17_browser_to_shell_weights_25() {
    let cmd_proc = make_proc(
        300,
        "cmd.exe",
        Some("C:\\Windows\\System32\\cmd.exe"),
        vec![(200, "chrome.exe".to_string())],
    );
    let all = vec![cmd_proc.clone()];
    let mut scorer = SecurityScorer::new();
    let score = scorer.score(&cmd_proc, &all, &[], &[]);
    let r17 = score
        .factors
        .iter()
        .find(|f| f.name == "browser_to_shell")
        .unwrap_or_else(|| panic!("应命中 BrowserToShell，factors: {:?}", score.factors));
    assert_eq!(r17.weight, 25);
}

/// R17 ScriptInterpreter：wscript.exe 直接运行扣 15（空 parent_chain 也命中）。
#[test]
fn r17_script_interpreter_weights_15() {
    let wscript_proc = make_proc(
        400,
        "wscript.exe",
        Some("C:\\Windows\\System32\\wscript.exe"),
        vec![],
    );
    let all = vec![wscript_proc.clone()];
    let mut scorer = SecurityScorer::new();
    let score = scorer.score(&wscript_proc, &all, &[], &[]);
    let r17 = score
        .factors
        .iter()
        .find(|f| f.name == "script_interpreter")
        .unwrap_or_else(|| panic!("应命中 ScriptInterpreter，factors: {:?}", score.factors));
    assert_eq!(r17.weight, 15);
}

/// R17 不命中场景：notepad ← explorer，正常链路。
#[test]
fn r17_normal_chain_does_not_match() {
    let notepad = make_proc(
        500,
        "notepad.exe",
        Some("C:\\Windows\\System32\\notepad.exe"),
        vec![(100, "explorer.exe".to_string())],
    );
    let all = vec![notepad.clone()];
    let mut scorer = SecurityScorer::new();
    let score = scorer.score(&notepad, &all, &[], &[]);
    let lineage_hit = score.factors.iter().find(|f| {
        f.name == "office_to_shell"
            || f.name == "browser_to_shell"
            || f.name == "script_interpreter"
            || f.name.starts_with("lineage_custom:")
    });
    assert!(
        lineage_hit.is_none(),
        "notepad ← explorer 不应命中 R17: {:?}",
        score.factors
    );
}

/// R17 不命中场景：shell ← explorer（合法 shell 启动）。
#[test]
fn r17_shell_from_explorer_no_match() {
    let cmd = make_proc(
        600,
        "cmd.exe",
        Some("C:\\Windows\\System32\\cmd.exe"),
        vec![(100, "explorer.exe".to_string())],
    );
    let all = vec![cmd.clone()];
    let mut scorer = SecurityScorer::new();
    let score = scorer.score(&cmd, &all, &[], &[]);
    assert!(
        !score
            .factors
            .iter()
            .any(|f| f.name == "office_to_shell" || f.name == "browser_to_shell"),
        "explorer 启动 cmd 不应命中 Office/Browser → Shell: {:?}",
        score.factors
    );
}

/// R17 只看直接父：cmd ← explorer ← winword 不应命中 OfficeToShell
/// （间接祖先 office 不算典型 macro attack 链）。
#[test]
fn r17_indirect_office_ancestor_no_match() {
    let cmd = make_proc(
        700,
        "cmd.exe",
        Some("C:\\Windows\\System32\\cmd.exe"),
        vec![
            (100, "explorer.exe".to_string()),
            (50, "winword.exe".to_string()),
        ],
    );
    let all = vec![cmd.clone()];
    let mut scorer = SecurityScorer::new();
    let score = scorer.score(&cmd, &all, &[], &[]);
    assert!(
        !score.factors.iter().any(|f| f.name == "office_to_shell"),
        "间接祖先 office 不应命中 OfficeToShell: {:?}",
        score.factors
    );
}

/// 父进程链 serde round-trip：完整 chain 序列化 / 反序列化保持等价。
#[test]
fn parent_chain_serde_round_trip() {
    let proc = make_proc(
        800,
        "powershell.exe",
        Some("C:\\ps.exe"),
        vec![
            (100, "WINWORD.EXE".to_string()),
            (50, "explorer.exe".to_string()),
            (4, "System".to_string()),
        ],
    );
    let json = serde_json::to_string(&proc).expect("serialize");
    let decoded: ProcessInfo = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded.parent_chain, proc.parent_chain);
}

/// 旧录屏兼容：缺 `parent_chain` 字段的 JSON 反序列化为默认空 Vec。
#[test]
fn parent_chain_missing_field_defaults_to_empty() {
    // 故意省略 parent_chain 字段（模拟 v0.10 录屏文件）
    let json = r#"{
        "pid": 900,
        "name": "test.exe",
        "cpu_usage": 0.0,
        "memory": 0,
        "virtual_memory": 0,
        "disk_usage": [0, 0],
        "disk_read_speed": 0,
        "disk_write_speed": 0,
        "net_sent_rate": 0,
        "net_recv_rate": 0,
        "status": "Run",
        "cmd": [],
        "start_time": 0,
        "run_time": 0
    }"#;
    let decoded: ProcessInfo = serde_json::from_str(json).expect("deserialize legacy");
    assert!(decoded.parent_chain.is_empty());
    assert_eq!(decoded.parent_pid, None);
}

/// SecurityScorer 缓存命中不应丢失 R17 因子。
#[test]
fn r17_factor_preserved_on_cache_hit() {
    let cmd = make_proc(
        1000,
        "powershell.exe",
        Some("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"),
        vec![(100, "WINWORD.EXE".to_string())],
    );
    let all = vec![cmd.clone()];
    let mut scorer = SecurityScorer::new();
    let first = scorer.score(&cmd, &all, &[], &[]);
    let second = scorer.score(&cmd, &all, &[], &[]);
    assert_eq!(first.factors.len(), second.factors.len());
    assert!(first.factors.iter().any(|f| f.name == "office_to_shell"));
    assert!(second.factors.iter().any(|f| f.name == "office_to_shell"));
}
