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
    parent_chain: Vec<(u32, std::sync::Arc<str>)>,
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
        vec![(100, std::sync::Arc::<str>::from("WINWORD.EXE"))],
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
        vec![(101, std::sync::Arc::<str>::from("excel.exe"))],
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
        vec![(200, std::sync::Arc::<str>::from("chrome.exe"))],
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
        vec![(100, std::sync::Arc::<str>::from("explorer.exe"))],
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
        vec![(100, std::sync::Arc::<str>::from("explorer.exe"))],
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
            (100, std::sync::Arc::<str>::from("explorer.exe")),
            (50, std::sync::Arc::<str>::from("winword.exe")),
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
            (100, std::sync::Arc::<str>::from("WINWORD.EXE")),
            (50, std::sync::Arc::<str>::from("explorer.exe")),
            (4, std::sync::Arc::<str>::from("System")),
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
        vec![(100, std::sync::Arc::<str>::from("WINWORD.EXE"))],
    );
    let all = vec![cmd.clone()];
    let mut scorer = SecurityScorer::new();
    let first = scorer.score(&cmd, &all, &[], &[]);
    let second = scorer.score(&cmd, &all, &[], &[]);
    assert_eq!(first.factors.len(), second.factors.len());
    assert!(first.factors.iter().any(|f| f.name == "office_to_shell"));
    assert!(second.factors.iter().any(|f| f.name == "office_to_shell"));
}

// --- TD-32（v0.12 阶段 5）：ScriptInterpreter 系统启动白名单 ---

/// ScriptInterpreter 直接父是 services.exe → 不扣 R17 ScriptInterpreter 15 分
/// （系统登录脚本 / SCM 服务初始化路径，合法场景）。
#[test]
fn td32_r17_script_interpreter_whitelisted_when_parent_is_services() {
    let wscript = make_proc(
        1100,
        "wscript.exe",
        Some("C:\\Windows\\System32\\wscript.exe"),
        vec![(500, std::sync::Arc::<str>::from("services.exe"))],
    );
    let all = vec![wscript.clone()];
    let mut scorer = SecurityScorer::new();
    let score = scorer.score(&wscript, &all, &[], &[]);
    assert!(
        !score.factors.iter().any(|f| f.name == "script_interpreter"),
        "services.exe → wscript.exe 应被白名单豁免，factors: {:?}",
        score.factors
    );
}

/// ScriptInterpreter 直接父是 svchost.exe → 不扣分（scheduled task / SCM trigger 路径）。
#[test]
fn td32_r17_script_interpreter_whitelisted_when_parent_is_svchost() {
    let cscript = make_proc(
        1101,
        "cscript.exe",
        Some("C:\\Windows\\System32\\cscript.exe"),
        vec![(501, std::sync::Arc::<str>::from("svchost.exe"))],
    );
    let all = vec![cscript.clone()];
    let mut scorer = SecurityScorer::new();
    let score = scorer.score(&cscript, &all, &[], &[]);
    assert!(
        !score.factors.iter().any(|f| f.name == "script_interpreter"),
        "svchost.exe → cscript.exe 应被白名单豁免"
    );
}

/// ScriptInterpreter 直接父是 wininit.exe → 不扣分（Session 0 初始化路径）。
#[test]
fn td32_r17_script_interpreter_whitelisted_when_parent_is_wininit() {
    let mshta = make_proc(
        1102,
        "mshta.exe",
        Some("C:\\Windows\\System32\\mshta.exe"),
        vec![(502, std::sync::Arc::<str>::from("wininit.exe"))],
    );
    let all = vec![mshta.clone()];
    let mut scorer = SecurityScorer::new();
    let score = scorer.score(&mshta, &all, &[], &[]);
    assert!(
        !score.factors.iter().any(|f| f.name == "script_interpreter"),
        "wininit.exe → mshta.exe 应被白名单豁免"
    );
}

/// 白名单只看直接父：间接祖先是 services.exe（chain[1]）不豁免，仍扣分。
#[test]
fn td32_r17_script_interpreter_whitelist_only_direct_parent() {
    // 模拟攻击者伪造祖先链：wscript ← evil ← services
    // 直接父 evil.exe 不在白名单 → 正常扣分。
    let wscript = make_proc(
        1103,
        "wscript.exe",
        Some("C:\\Windows\\System32\\wscript.exe"),
        vec![
            (200, std::sync::Arc::<str>::from("evil.exe")),
            (100, std::sync::Arc::<str>::from("services.exe")),
        ],
    );
    let all = vec![wscript.clone()];
    let mut scorer = SecurityScorer::new();
    let score = scorer.score(&wscript, &all, &[], &[]);
    assert!(
        score.factors.iter().any(|f| f.name == "script_interpreter"),
        "间接祖先是 services.exe 不应豁免（只看直接父），factors: {:?}",
        score.factors
    );
}

// --- v0.17 stage 2 TD-47：parent_chain Arc<str> 重构行为测试 ---
//
// 这些测试验证 v0.17 stage 2 把 `parent_chain: Vec<(u32, String)>` 改为
// `Vec<(u32, Arc<str>)>` 后的关键不变量：
//
// 1. **Arc 共享**：build_parent_chain 返回的 chain[i].1 与源 ProcessInfo.name 是同一
//    个 Arc 实例（指针相同），证明走 refcount 共享而非字符串拷贝。
// 2. **旧 JSON 兼容**：v0.16 旧格式 JSON（chain 元素是 String）反序列化到新结构
//    等价（serde 透明转发让 Arc<str> 直接从 String 反序列化）。
// 3. **clone 共享**：chain.clone() 后的 Arc 仍与原 Arc 共享（Vec clone 时元素走
//    Arc::clone 原子计数 inc，不拷贝字符串）。
//
// 不验 heap alloc 数字本身（count allocs 需 jemalloc/dhat 介入，跨平台不稳），
// 数字下降验证留给 `cargo bench --bench bench_refresh_heavy -- --baseline v0.16` 实跑。

/// TD-47 行为测试 1：build_parent_chain 返回的 chain 元素与源 ProcessInfo.name
/// 共享同一 Arc 实例（指针相同）—— 证明走 refcount 共享，零 heap alloc。
#[test]
fn parent_chain_arc_sharing_after_build() {
    use proc::security::lineage::build_parent_chain;
    use std::collections::HashMap;
    use std::sync::Arc;

    // 构造 chain：root(4) ← parent(100) ← child(200)，每个进程的 name 都是独立 Arc。
    let make = |pid: u32, name: &str, parent_pid: Option<u32>| {
        let name_arc: Arc<str> = Arc::from(name);
        ProcessInfo {
            pid,
            name: Arc::clone(&name_arc),
            parent_pid,
            ..ProcessInfo::default()
        }
    };
    let procs = [
        make(4, "System", None),
        make(100, "explorer.exe", Some(4)),
        make(200, "cmd.exe", Some(100)),
    ];
    let map: HashMap<u32, ProcessInfo> = procs.iter().map(|p| (p.pid, p.clone())).collect();

    let chain = build_parent_chain(200, &map);
    assert_eq!(chain.len(), 2);

    // chain[0] = (100, "explorer.exe")，与源 ProcessInfo(100).name 共享 Arc 指针。
    let explorer_in_map = map.get(&100).unwrap();
    let explorer_arc_ptr = Arc::as_ptr(&explorer_in_map.name);
    let chain0_arc_ptr = Arc::as_ptr(&chain[0].1);
    assert_eq!(
        explorer_arc_ptr, chain0_arc_ptr,
        "chain[0].1 应与源 ProcessInfo(100).name 共享 Arc 指针（refcount 共享，零拷贝）"
    );

    // chain[1] = (4, "System")，与源 ProcessInfo(4).name 共享。
    let system_in_map = map.get(&4).unwrap();
    let system_arc_ptr = Arc::as_ptr(&system_in_map.name);
    let chain1_arc_ptr = Arc::as_ptr(&chain[1].1);
    assert_eq!(
        system_arc_ptr, chain1_arc_ptr,
        "chain[1].1 应与源 ProcessInfo(4).name 共享 Arc 指针"
    );
}

/// TD-47 行为测试 2：v0.16 旧格式 JSON（parent_chain 元素是 String）能被新
/// ProcessInfo（Vec<(u32, Arc<str>)>）反序列化，且 chain 内容等价。
/// serde 透明转发：Arc<str> 直接从 String 反序列化，无需迁移层。
#[test]
fn parent_chain_serde_legacy_json_round_trip() {
    // 模拟 v0.16 录制的 JSON：parent_chain 字段值是 [[pid, "name"]] 形式（serde
    // 序列化元组 (u32, String) 为 JSON array）。新代码反序列化时 Arc<str>
    // 从中提取字符串。
    let legacy_json = r#"{
        "pid": 1234,
        "name": "cmd.exe",
        "cpu_usage": 5.0,
        "memory": 8388608,
        "virtual_memory": 8388608,
        "disk_usage": [1024, 512],
        "disk_read_speed": 100,
        "disk_write_speed": 50,
        "net_sent_rate": 10,
        "net_recv_rate": 20,
        "status": "Run",
        "cmd": [],
        "start_time": 1700000000,
        "run_time": 600,
        "parent_chain": [
            [100, "explorer.exe"],
            [4, "System"]
        ]
    }"#;
    let decoded: ProcessInfo =
        serde_json::from_str(legacy_json).expect("deserialize legacy v0.16 JSON");
    assert_eq!(decoded.pid, 1234);
    assert_eq!(decoded.parent_chain.len(), 2);
    // 内容等价（Arc<str> AsRef<str> → &str，再与 &str 比较）。
    assert_eq!(decoded.parent_chain[0].1.as_ref(), "explorer.exe");
    assert_eq!(decoded.parent_chain[0].0, 100);
    assert_eq!(decoded.parent_chain[1].1.as_ref(), "System");
    assert_eq!(decoded.parent_chain[1].0, 4);

    // 反过来：新结构序列化后再反序列化，chain 内容仍等价。
    let reencoded = serde_json::to_string(&decoded).expect("serialize new format");
    let redecoded: ProcessInfo =
        serde_json::from_str(&reencoded).expect("deserialize new format re-encoded");
    assert_eq!(redecoded.parent_chain, decoded.parent_chain);
}

/// TD-47 行为测试 3：chain.clone() 后的 Arc 仍与原 Arc 共享（Vec clone 时元素
/// 走 Arc::clone 原子计数 inc，不拷贝字符串）。collect.rs:969 `proc.parent_chain
/// = chain.clone()` 路径走的就是这个。
#[test]
fn parent_chain_clone_preserves_arc_sharing() {
    use std::sync::Arc;

    let original: Vec<(u32, Arc<str>)> =
        vec![(100, Arc::from("explorer.exe")), (4, Arc::from("System"))];

    // 记录原始 Arc 指针。
    let orig_ptr0 = Arc::as_ptr(&original[0].1);
    let orig_ptr1 = Arc::as_ptr(&original[1].1);

    // clone 整个 Vec（与 collect.rs:969 `chain.clone()` 同款路径）。
    let cloned = original.clone();

    // 新 Vec 的 Arc 指针应与原始相同（refcount 共享）。
    assert_eq!(Arc::as_ptr(&cloned[0].1), orig_ptr0);
    assert_eq!(Arc::as_ptr(&cloned[1].1), orig_ptr1);

    // 强引用计数应为 2（original + cloned 各持一份）。
    assert_eq!(Arc::strong_count(&original[0].1), 2);
    assert_eq!(Arc::strong_count(&original[1].1), 2);
}
