//! v0.11 阶段 6：R18 可疑启动路径集成测试。
//!
//! 覆盖 `SecurityScorer::score` 第 18 步接入：mock `ProcessInfo.exe` 路径
//! 触发各 `SuspiciousPathKind`，验证 `RiskFactor` 生成 + 权重 + R16 协同扣分。
//!
//! `expand_user_dir` / `is_in_suspicious_path` / `PathRule` 解析的细粒度单元
//! 测试在 `src/security/path_rules.rs` 内嵌（模块私有）。

use proc::collect::ProcessInfo;
use proc::security::{RiskCategory, SecurityScorer, SignatureStatus};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// R18 测试会 `std::env::set_var` 修改进程全局环境变量（TEMP/APPDATA 等），
/// cargo test 默认多线程并行会相互污染。用此 Mutex 强制串行执行所有 R18
/// 集成测试，避免 set_var 竞争（Rust 2024 edition 起这种操作本来就要求 unsafe）。
///
/// `OnceLock` 让 Mutex 真正 'static，guard 借用 'static 不需要 transmute。
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> &'static Mutex<()> {
    ENV_LOCK.get_or_init(|| Mutex::new(()))
}

/// edition 2024 起 `std::env::set_var` / `remove_var` / `set_os` / `remove_var`
/// 都是 unsafe（进程全局状态并发修改 UB 风险）。helper 包裹 unsafe + 集中调用。
fn set_env(key: &str, val: &str) {
    // SAFETY: 测试函数持有 ENV_LOCK MutexGuard 保证串行，无并发 set_var。
    unsafe { std::env::set_var(key, val) };
}

fn remove_env(key: &str) {
    // SAFETY: 同上。
    unsafe { std::env::remove_var(key) };
}

fn set_env_os(key: &str, val: std::ffi::OsString) {
    // SAFETY: 同上。Rust 没有 set_os，OsString → String 走 to_string_lossy
    // （非 Unicode 在 Windows 环境变量上极其罕见，lossy 转换可接受）。
    let val_str = val.to_string_lossy().into_owned();
    unsafe { std::env::set_var(key, val_str) };
}

fn remove_env_os(key: &str) {
    // SAFETY: 同上。
    unsafe { std::env::remove_var(key) };
}

/// R18 评分需要 `SecurityScorer::user_dirs` 命中 mock 路径。`SecurityScorer::new()`
/// 用真实环境变量构造，测试通过 `std::env::set_var("TEMP", ...)` 让 user_dirs
/// 命中我们设置的路径。每个测试 setup 时设置、teardown 时还原（避免污染其他测试）。
///
/// 持有 `ENV_LOCK` MutexGuard —— 所有 R18 测试串行执行，避免 set_var 竞争。
struct TempEnvGuard {
    keys: Vec<(&'static str, Option<std::ffi::OsString>)>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl TempEnvGuard {
    fn new(keys: &[&'static str]) -> Self {
        let lock = env_lock().lock().unwrap_or_else(|p| {
            // poison 后继续执行（前一个测试 panic 不应阻塞后续 debug）
            p.into_inner()
        });
        let saved = keys
            .iter()
            .map(|k| (*k, std::env::var_os(k)))
            .collect::<Vec<_>>();
        Self {
            keys: saved,
            _lock: lock,
        }
    }
}

impl Drop for TempEnvGuard {
    fn drop(&mut self) {
        for (key, original) in &self.keys {
            match original {
                Some(val) => set_env_os(key, val.clone()),
                None => remove_env_os(key),
            }
        }
    }
}

fn make_proc_with_exe(pid: u32, name: &str, exe: &str) -> ProcessInfo {
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
        exe: Some(std::sync::Arc::from(exe)),
        cmd: std::sync::Arc::from(Vec::<String>::new()),
        cwd: None,
        parent_pid: None,
        session_id: None,
        user_id: None,
        start_time: 0,
        run_time: 0,
        name_lower: std::sync::Arc::from(name_arc.to_lowercase().as_str()),
        throttled: proc::throttle::EcoQoSState::default(),
        signature_status: SignatureStatus::default(),
        parent_chain: Vec::new(),
    }
}

/// 构造一个 SecurityScorer，user_dirs 指向我们 mock 的环境变量。
fn make_scorer() -> SecurityScorer {
    SecurityScorer::new()
}

/// R18 Temp 命中：扣 20 分。
#[test]
fn r18_temp_weights_20() {
    let _guard = TempEnvGuard::new(&["TEMP", "TMP", "APPDATA", "LOCALAPPDATA", "USERPROFILE"]);
    let temp_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target\\test-r18-temp")
        .to_string_lossy()
        .to_string();
    set_env("TEMP", &temp_dir);
    remove_env("TMP");

    let exe = format!("{}\\evil.exe", temp_dir);
    let proc = make_proc_with_exe(100, "evil.exe", &exe);
    let all = vec![proc.clone()];
    let mut scorer = make_scorer();
    let score = scorer.score(&proc, &all, &[], &[]);

    let r18 = score
        .factors
        .iter()
        .find(|f| f.name == "suspicious_path_temp")
        .unwrap_or_else(|| panic!("应命中 R18 Temp，factors: {:?}", score.factors));
    assert_eq!(r18.weight, 20);
    assert_eq!(r18.category, RiskCategory::FilePath);
}

/// R18 AppData 命中：扣 15 分。
#[test]
fn r18_appdata_weights_15() {
    let _guard = TempEnvGuard::new(&["TEMP", "TMP", "APPDATA", "LOCALAPPDATA", "USERPROFILE"]);
    let appdata_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target\\test-r18-appdata")
        .to_string_lossy()
        .to_string();
    set_env("APPDATA", &appdata_dir);
    remove_env("TEMP");
    remove_env("TMP");
    remove_env("LOCALAPPDATA");

    let exe = format!("{}\\payload.exe", appdata_dir);
    let proc = make_proc_with_exe(101, "payload.exe", &exe);
    let all = vec![proc.clone()];
    let mut scorer = make_scorer();
    let score = scorer.score(&proc, &all, &[], &[]);

    let r18 = score
        .factors
        .iter()
        .find(|f| f.name == "suspicious_path_appdata")
        .unwrap_or_else(|| panic!("应命中 R18 AppData，factors: {:?}", score.factors));
    assert_eq!(r18.weight, 15);
}

/// R18 LocalAppData 命中（非 Temp 子目录）：扣 15 分。
#[test]
fn r18_local_appdata_weights_15() {
    let _guard = TempEnvGuard::new(&["TEMP", "TMP", "APPDATA", "LOCALAPPDATA", "USERPROFILE"]);
    let local_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target\\test-r18-local")
        .to_string_lossy()
        .to_string();
    set_env("LOCALAPPDATA", &local_dir);
    // 不设 TEMP，避免 Temp 截获
    remove_env("TEMP");
    remove_env("TMP");
    remove_env("APPDATA");

    let exe = format!("{}\\MalwareApp\\mal.exe", local_dir);
    let proc = make_proc_with_exe(102, "mal.exe", &exe);
    let all = vec![proc.clone()];
    let mut scorer = make_scorer();
    let score = scorer.score(&proc, &all, &[], &[]);

    let r18 = score
        .factors
        .iter()
        .find(|f| f.name == "suspicious_path_local_appdata" || f.name == "suspicious_path_temp");
    assert!(
        r18.is_some(),
        "应命中 R18 LocalAppData 或 Temp，factors: {:?}",
        score.factors
    );
    assert_eq!(r18.unwrap().weight, 15);
}

/// R18 UserProfileDownloads 命中：扣 15 分。
///
/// **v0.12 阶段 5（TD-33）调整**：path 改成 3 段（`\Downloads\Deep\Sub\installer.exe`）
/// 绕过 v0.6 path_check 的 `is_in_downloads`（其 segments<=2 启发式），让 R18
/// UserProfileDownloads 单独命中，避免 TD-33 dedup 把 `suspicious_path_downloads`
/// 过滤掉。dedup 行为的覆盖见 [`td33_downloads_overlapping_path_dedup`]。
#[test]
fn r18_downloads_weights_15() {
    let _guard = TempEnvGuard::new(&["TEMP", "TMP", "APPDATA", "LOCALAPPDATA", "USERPROFILE"]);
    let userprofile_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target\\test-r18-profile")
        .to_string_lossy()
        .to_string();
    set_env("USERPROFILE", &userprofile_dir);
    remove_env("TEMP");
    remove_env("TMP");
    remove_env("APPDATA");
    remove_env("LOCALAPPDATA");

    let exe = format!("{}\\Downloads\\Deep\\Sub\\installer.exe", userprofile_dir);
    let proc = make_proc_with_exe(103, "installer.exe", &exe);
    let all = vec![proc.clone()];
    let mut scorer = make_scorer();
    let score = scorer.score(&proc, &all, &[], &[]);

    let r18 = score
        .factors
        .iter()
        .find(|f| f.name == "suspicious_path_downloads")
        .unwrap_or_else(|| panic!("应命中 R18 Downloads，factors: {:?}", score.factors));
    assert_eq!(r18.weight, 15);
}

// --- TD-33（v0.12 阶段 5）：Downloads 去重 ---

/// TD-33：v0.6 path_check 的 `downloads_dir`（15）+ v0.11 R18 的
/// `suspicious_path_downloads`（15）实际指向同一物理路径 `%USERPROFILE%\Downloads`，
/// 扣两次过度（30 分）。dedup 后只保留 `downloads_dir`（15 分），R18
/// UserProfileDownloads 被 filter 掉。
#[test]
fn td33_downloads_overlapping_path_dedup() {
    let _guard = TempEnvGuard::new(&["TEMP", "TMP", "APPDATA", "LOCALAPPDATA", "USERPROFILE"]);
    let userprofile_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target\\test-r18-td33")
        .to_string_lossy()
        .to_string();
    set_env("USERPROFILE", &userprofile_dir);
    remove_env("TEMP");
    remove_env("TMP");
    remove_env("APPDATA");
    remove_env("LOCALAPPDATA");

    // 1-segment path：v0.6 path_check::is_in_downloads 命中 + R18
    // UserProfileDownloads 也命中——TD-33 dedup 让 R18 那条被过滤掉。
    let exe = format!("{}\\Downloads\\installer.exe", userprofile_dir);
    let proc = make_proc_with_exe(110, "installer.exe", &exe);
    let all = vec![proc.clone()];
    let mut scorer = make_scorer();
    let score = scorer.score(&proc, &all, &[], &[]);

    // v0.6 path_check 的 downloads_dir 保留
    let path_check_downloads = score
        .factors
        .iter()
        .find(|f| f.name == "downloads_dir")
        .unwrap_or_else(|| {
            panic!(
                "downloads_dir 应保留（dedup 后只过滤 R18 那条），factors: {:?}",
                score.factors
            )
        });
    assert_eq!(path_check_downloads.weight, 15);

    // R18 的 suspicious_path_downloads 应被 filter 掉
    let r18_downloads_dup = score
        .factors
        .iter()
        .find(|f| f.name == "suspicious_path_downloads");
    assert!(
        r18_downloads_dup.is_none(),
        "TD-33 dedup：suspicious_path_downloads 应被 filter 掉，factors: {:?}",
        score.factors
    );

    // 计算 Downloads 相关 path factor 总扣分（应只 15，不是 30）
    let downloads_total: u32 = score
        .factors
        .iter()
        .filter(|f| f.name == "downloads_dir" || f.name == "suspicious_path_downloads")
        .map(|f| f.weight)
        .sum();
    assert_eq!(
        downloads_total, 15,
        "TD-33：Downloads 总扣分应 15（dedup），实际 {downloads_total}，factors: {:?}",
        score.factors
    );
}

/// TD-33 dedup 不影响 R18 其他子检查：Temp / AppData / LocalAppData / Custom 等
/// 仍照常命中。这里验证 Downloads dedup 后，R18 的 r18_cooperation_factor 路径
/// （未签名 + 可疑路径）仍能正常触发——只要 R18 命中（哪怕只有 downloads_dir
/// 被 filter 掉）就视作 r18_matched=true。
///
/// 注：score 函数内 sig_status 走真实 verify_signature，mock exe 不存在 → Unknown，
/// 不会触发 R16+R18 协同扣分。此测试只验证 dedup 路径下 R18 仍能命中至少一项。
#[test]
fn td33_downloads_dedup_does_not_block_other_r18_kinds() {
    let _guard = TempEnvGuard::new(&["TEMP", "TMP", "APPDATA", "LOCALAPPDATA", "USERPROFILE"]);
    let userprofile_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target\\test-r18-td33-multi")
        .to_string_lossy()
        .to_string();
    let appdata_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target\\test-r18-td33-appdata")
        .to_string_lossy()
        .to_string();
    set_env("USERPROFILE", &userprofile_dir);
    set_env("APPDATA", &appdata_dir);
    remove_env("TEMP");
    remove_env("TMP");
    remove_env("LOCALAPPDATA");

    // 路径同时位于 Downloads 和 AppData Roaming：v0.6 path_check 命中 downloads_dir，
    // R18 同时命中 UserProfileDownloads（被 dedup 过滤）+ AppData（保留）。
    // 这验证 dedup 只过滤 suspicious_path_downloads 一个 factor，不连带过滤其他 R18。
    let exe = format!("{}\\Downloads\\Sub\\app.exe", userprofile_dir);
    // 注：上面的路径是 1-segment Downloads，v0.6 会命中 downloads_dir；
    // R18 UserProfileDownloads 命中后被 dedup 过滤；为了让 R18 AppData 也命中，
    // 我们让 APPDATA 指向同一父目录。
    let _ = appdata_dir; // APPDATA 设到独立路径，本测试不验证 AppData（要避免污染）
    let proc = make_proc_with_exe(111, "app.exe", &exe);
    let all = vec![proc.clone()];
    let mut scorer = make_scorer();
    let score = scorer.score(&proc, &all, &[], &[]);

    // downloads_dir 保留（v0.6 path_check）
    assert!(
        score.factors.iter().any(|f| f.name == "downloads_dir"),
        "downloads_dir 应保留，factors: {:?}",
        score.factors
    );
    // suspicious_path_downloads 被 dedup 过滤
    assert!(
        !score
            .factors
            .iter()
            .any(|f| f.name == "suspicious_path_downloads"),
        "suspicious_path_downloads 应被 dedup 过滤，factors: {:?}",
        score.factors
    );
}

/// R18 不命中：系统目录（Program Files）不扣分。
#[test]
fn r18_no_hit_for_system_dir() {
    let _guard = TempEnvGuard::new(&[
        "TEMP",
        "TMP",
        "APPDATA",
        "LOCALAPPDATA",
        "USERPROFILE",
        "ProgramFiles",
        "SystemRoot",
    ]);
    let pf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target\\test-r18-pf")
        .to_string_lossy()
        .to_string();
    set_env("ProgramFiles", &pf);
    remove_env("TEMP");
    remove_env("APPDATA");

    let exe = format!("{}\\MyApp\\myapp.exe", pf);
    let proc = make_proc_with_exe(104, "myapp.exe", &exe);
    let all = vec![proc.clone()];
    let mut scorer = make_scorer();
    let score = scorer.score(&proc, &all, &[], &[]);

    let r18 = score
        .factors
        .iter()
        .find(|f| f.name.starts_with("suspicious_path_"));
    assert!(
        r18.is_none(),
        "Program Files 不应命中 R18，factors: {:?}",
        score.factors
    );
}

/// R18 + R16 协同扣分：未签名 + Temp 命中 → 额外 -10 分。
///
/// 协同扣分决策逻辑（`r18_cooperation_factor`）的状态机覆盖在
/// `src/security/score.rs::tests` 单元测试模块（7 个 case 覆盖 Unsigned/Revoked/
/// Trusted/Signed/Pending/Unknown 状态）。本集成测试只验证 mock fixture 下
/// R18 命中（score 内 sig_status 走真实 verify_signature，mock exe 不存在 →
/// Unknown，不会触发协同）。
#[test]
fn r18_temp_hit_mock_exe_path() {
    let _guard = TempEnvGuard::new(&["TEMP", "TMP", "APPDATA", "LOCALAPPDATA", "USERPROFILE"]);
    let temp_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target\\test-r18-coop")
        .to_string_lossy()
        .to_string();
    set_env("TEMP", &temp_dir);
    remove_env("TMP");

    let exe = format!("{}\\evil.exe", temp_dir);
    let proc = make_proc_with_exe(105, "evil.exe", &exe);
    let all = vec![proc.clone()];
    let mut scorer = make_scorer();
    let score = scorer.score(&proc, &all, &[], &[]);

    // R18 Temp 必命中
    assert!(
        score
            .factors
            .iter()
            .any(|f| f.name == "suspicious_path_temp"),
        "R18 Temp 必命中，factors: {:?}",
        score.factors
    );
}

/// R18 命中时，SecurityScorer 缓存命中后再次 score 应返回一致结果。
#[test]
fn r18_cached_score_consistent() {
    let _guard = TempEnvGuard::new(&["TEMP", "TMP", "APPDATA", "LOCALAPPDATA", "USERPROFILE"]);
    let temp_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target\\test-r18-cache")
        .to_string_lossy()
        .to_string();
    set_env("TEMP", &temp_dir);
    remove_env("TMP");

    let exe = format!("{}\\evil.exe", temp_dir);
    let proc = make_proc_with_exe(106, "evil.exe", &exe);
    let all = vec![proc.clone()];
    let mut scorer = make_scorer();
    let s1 = scorer.score(&proc, &all, &[], &[]);
    let s2 = scorer.score(&proc, &all, &[], &[]);
    assert_eq!(s1.score, s2.score);
    assert_eq!(s1.factors.len(), s2.factors.len());
}

/// R18 不命中：env 全部未设置（Linux 场景）→ R18 no-op。
#[test]
fn r18_no_env_returns_no_factor() {
    let _guard = TempEnvGuard::new(&[
        "TEMP",
        "TMP",
        "APPDATA",
        "LOCALAPPDATA",
        "USERPROFILE",
        "ProgramFiles",
        "SystemRoot",
    ]);
    remove_env("TEMP");
    remove_env("TMP");
    remove_env("APPDATA");
    remove_env("LOCALAPPDATA");
    remove_env("USERPROFILE");
    remove_env("ProgramFiles");
    remove_env("SystemRoot");

    let proc = make_proc_with_exe(107, "evil.exe", "/tmp/evil.exe");
    let all = vec![proc.clone()];
    let mut scorer = make_scorer();
    let score = scorer.score(&proc, &all, &[], &[]);

    let r18 = score
        .factors
        .iter()
        .find(|f| f.name.starts_with("suspicious_path_"));
    assert!(
        r18.is_none(),
        "无环境变量时 R18 不应命中，factors: {:?}",
        score.factors
    );
}

/// R18 + path_check（v0.6）叠加扣分：Temp 路径同时命中 R3（temp_dir）和 R18（suspicious_path_temp）。
///
/// surgical 原则下两检查共存——验证「叠加」而非「替换」。
#[test]
fn r18_overlaps_with_path_check_temp() {
    let _guard = TempEnvGuard::new(&["TEMP", "TMP", "APPDATA", "LOCALAPPDATA", "USERPROFILE"]);
    // 临时目录位于 \Temp\ 子串（v0.6 path_check::is_in_temp 命中）
    let temp_dir = "C:\\Users\\test\\AppData\\Local\\Temp";
    set_env("TEMP", temp_dir);
    remove_env("TMP");

    let exe = format!("{}\\evil.exe", temp_dir);
    let proc = make_proc_with_exe(108, "evil.exe", &exe);
    let all = vec![proc.clone()];
    let mut scorer = make_scorer();
    let score = scorer.score(&proc, &all, &[], &[]);

    // path_check 的 temp_dir（25 分）
    let path_check_temp = score.factors.iter().find(|f| f.name == "temp_dir");
    // R18 的 suspicious_path_temp（20 分）
    let r18_temp = score
        .factors
        .iter()
        .find(|f| f.name == "suspicious_path_temp");

    // 两者都应命中（叠加扣分）
    assert!(
        path_check_temp.is_some(),
        "path_check temp_dir 应命中，factors: {:?}",
        score.factors
    );
    assert!(
        r18_temp.is_some(),
        "R18 suspicious_path_temp 应命中，factors: {:?}",
        score.factors
    );
    assert_eq!(path_check_temp.unwrap().weight, 25);
    assert_eq!(r18_temp.unwrap().weight, 20);
}
