//! v0.11 阶段 5：父子链构建 + R17 可疑链检测（ADR-0011 之外，新增 R17）。
//!
//! 与 v0.7 `src/security/parent_chain.rs::analyze_parent_chain` **不冲突**：
//! 后者是「即时分析函数」（接受 `&[ProcessInfo]` 计算 RiskFactor，不持久化），
//! 本模块是「数据载体」（基于 `ProcessInfo.parent_chain` 字段，由 HeavyWorker
//! 在 collect 时填实）。两者评分会叠加（v0.7 office_spawning_shell 30 + R17
//! OfficeToShell 35 = 65 分），surgical 原则下保留 v0.7 不动——安全评分偏向严格。
//!
//! R17 命中条件（基于 `ProcessInfo.parent_chain` + `ProcessInfo.name`）：
//! - **OfficeToShell**（扣 35）：当前进程是 shell（cmd/powershell/wscript/...）
//!   且直接父进程是 Office 应用（Word/Excel/PowerPoint/...）。典型 macro attack。
//! - **BrowserToShell**（扣 25）：当前进程是 shell 且直接父是浏览器。
//! - **ScriptInterpreter**（扣 15）：当前进程是 wscript/cscript/mshta（无论祖先）。
//! - **Custom**（按用户配置 weight）：来自 `~/.config/proc/lineage_rules.toml`，
//!   child_pattern 匹配当前进程名 + parent_pattern 匹配 chain 中任一祖先。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use super::score::{RiskCategory, RiskFactor};
use crate::collect::ProcessInfo;

/// parent_chain 最大深度（防恶意构造超长链 / sysinfo 异常死循环）。
/// stage-5.md 任务 1 指定值：32。
pub const MAX_PARENT_CHAIN_DEPTH: usize = 32;

/// R17 命中后的可疑模式分类。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuspiciousPattern {
    /// Office 应用（Word/Excel/...）启动 shell —— 典型 macro attack。
    OfficeToShell,
    /// 浏览器启动 shell（可能合法，如扩展调 shell，但有恶意可能）。
    BrowserToShell,
    /// wscript/cscript/mshta 直接运行（可能合法，如系统脚本）。
    ScriptInterpreter,
    /// 用户在 lineage_rules.toml 配置的自定义规则命中。
    Custom { name: String, weight: u32 },
}

impl SuspiciousPattern {
    /// 该模式对应的扣分（Custom 用配置 weight，其他用内置常量）。
    #[must_use]
    pub fn default_weight(&self) -> u32 {
        match self {
            Self::OfficeToShell => 35,
            Self::BrowserToShell => 25,
            Self::ScriptInterpreter => 15,
            Self::Custom { weight, .. } => *weight,
        }
    }

    /// RiskFactor.name 字段值（用于去重 / 测试断言）。
    #[must_use]
    pub fn risk_name(&self) -> String {
        match self {
            Self::OfficeToShell => "office_to_shell".to_string(),
            Self::BrowserToShell => "browser_to_shell".to_string(),
            Self::ScriptInterpreter => "script_interpreter".to_string(),
            Self::Custom { name, .. } => format!("lineage_custom:{name}"),
        }
    }

    /// RiskFactor.description 字段值（含 chain 摘要便于用户排查）。
    fn description(&self, current_name: &str, chain: &[(u32, String)]) -> String {
        let chain_text = chain_summary(chain);
        match self {
            Self::OfficeToShell => {
                format!("Office 启动 shell（典型 macro attack）：{current_name} ← {chain_text}")
            }
            Self::BrowserToShell => {
                format!("浏览器启动 shell（可能恶意）：{current_name} ← {chain_text}")
            }
            Self::ScriptInterpreter => {
                format!("脚本解释器直接运行：{current_name} ← {chain_text}")
            }
            Self::Custom { name, .. } => {
                format!("自定义规则「{name}」命中：{current_name} ← {chain_text}")
            }
        }
    }
}

fn chain_summary(chain: &[(u32, String)]) -> String {
    chain
        .iter()
        .map(|(_, n)| n.as_str())
        .collect::<Vec<_>>()
        .join(" → ")
}

const OFFICE_APPS: &[&str] = &[
    "winword.exe",
    "excel.exe",
    "powerpnt.exe",
    "outlook.exe",
    "msaccess.exe",
    "mspub.exe",
    "visio.exe",
    "project.exe",
    "onenote.exe",
    "onenotem.exe",
];

const SHELL_PROCESSES: &[&str] = &[
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "wscript.exe",
    "cscript.exe",
    "mshta.exe",
];

const BROWSER_PROCESSES: &[&str] = &[
    "chrome.exe",
    "msedge.exe",
    "firefox.exe",
    "brave.exe",
    "opera.exe",
    "iexplore.exe",
];

const SCRIPT_INTERPRETERS: &[&str] = &["wscript.exe", "cscript.exe", "mshta.exe"];

/// TD-32（v0.12 阶段 5）：系统启动入口白名单。当 ScriptInterpreter（wscript/
/// cscript/mshta）的直接父进程是这里列出的系统 service host 时，视为合法系统
/// 登录脚本 / 服务初始化脚本，不扣 R17 ScriptInterpreter 15 分。
///
/// 来源：Windows 服务管理器启动 service → service 启动 Session 0 → Session 0 里
/// 跑的登录脚本 / scheduled task / SCM trigger 直接 spawn wscript 是合法路径
/// （如组策略登录脚本、管理性 scheduled task）。侦测这种场景扣分会导致
/// 企业域控 + SCCM 部署环境的用户体验不可接受（每个登录都报警）。
///
/// 大小写不敏感（`name_in_list` 自带 lowercase 归一）。
const SYSTEM_BOOT_ENTRIES: &[&str] = &["services.exe", "wininit.exe", "svchost.exe"];

fn name_in_list(list: &[&str], name: &str) -> bool {
    let name_lower = name.to_lowercase();
    list.iter().any(|s| *s == name_lower)
}

/// 从 `pid` 出发构建 parent_chain（不含 pid 本身）。
///
/// 遍历顺序：`chain[0] = 直接父进程`，`chain[1] = 祖父`，... 直到根
/// （`parent_pid == 0` / 找不到 / 达到 `MAX_PARENT_CHAIN_DEPTH`）。
///
/// 防循环：`visited` HashSet 检测重复 PID 立即终止（应对 PID 复用导致的环
/// A → B → A）。stage-5.md 任务 1 要求。
#[must_use]
pub fn build_parent_chain(pid: u32, processes: &HashMap<u32, ProcessInfo>) -> Vec<(u32, String)> {
    let mut chain = Vec::new();
    let mut visited: HashSet<u32> = HashSet::new();
    // 防自身引用：parent_pid == pid 时 break；先把自己加入 visited。
    visited.insert(pid);

    let mut current = pid;
    while chain.len() < MAX_PARENT_CHAIN_DEPTH {
        let Some(proc) = processes.get(&current) else {
            break;
        };
        let Some(parent_pid) = proc.parent_pid else {
            break;
        };
        if parent_pid == 0 || parent_pid == current {
            break;
        }
        if !visited.insert(parent_pid) {
            break; // 环检测命中
        }
        let Some(parent_proc) = processes.get(&parent_pid) else {
            break;
        };
        chain.push((parent_pid, parent_proc.name.to_string()));
        current = parent_pid;
    }
    chain
}

/// 检测 chain + 当前进程名是否构成可疑模式。
///
/// - `chain`：`build_parent_chain` 返回的祖先链（不含当前进程）。
/// - `current_name`：被检测进程的 name（用于判定 shell / script interpreter 角色）。
/// - `custom_rules`：来自 `lineage_rules.toml` 的用户配置。
///
/// 检测顺序（先命中先返回）：ScriptInterpreter → OfficeToShell → BrowserToShell → Custom。
///
/// **TD-32（v0.12 阶段 5）**：ScriptInterpreter 命中前先检查直接父（`chain[0]`）
/// 是否在 [`SYSTEM_BOOT_ENTRIES`] 白名单里（services.exe / wininit.exe / svchost.exe）；
/// 命中白名单 → 视为系统登录脚本 / SCM 服务初始化脚本，不扣 ScriptInterpreter 15 分。
/// Custom 规则仍照常评估（不在 ScriptInterpreter 早返回路径里）。
#[must_use]
pub fn detect_suspicious_chain(
    chain: &[(u32, String)],
    current_name: &str,
    custom_rules: &[LineageRule],
) -> Option<SuspiciousPattern> {
    // ScriptInterpreter 优先（无论祖先）：wscript/cscript/mshta 直接运行。
    // TD-32：但若直接父是系统启动入口（services/wininit/svchost），视为合法系统
    // 登录脚本，跳过 ScriptInterpreter 扣分（仍可被 Custom 规则评估）。
    if name_in_list(SCRIPT_INTERPRETERS, current_name) {
        let direct_parent_name = chain.first().map(|(_, n)| n.as_str()).unwrap_or("");
        if !name_in_list(SYSTEM_BOOT_ENTRIES, direct_parent_name) {
            return Some(SuspiciousPattern::ScriptInterpreter);
        }
    }

    // Office/Browser → Shell：当前进程必须是 shell 才能命中。
    if name_in_list(SHELL_PROCESSES, current_name) {
        // 直接父进程（chain[0]）。
        let direct_parent_name = chain.first().map(|(_, n)| n.as_str()).unwrap_or("");
        if name_in_list(OFFICE_APPS, direct_parent_name) {
            return Some(SuspiciousPattern::OfficeToShell);
        }
        if name_in_list(BROWSER_PROCESSES, direct_parent_name) {
            return Some(SuspiciousPattern::BrowserToShell);
        }
    }

    // 自定义规则（无论 shell 与否都检查）。
    match_custom_rule(chain, current_name, custom_rules)
}

fn match_custom_rule(
    chain: &[(u32, String)],
    current_name: &str,
    custom_rules: &[LineageRule],
) -> Option<SuspiciousPattern> {
    for rule in custom_rules {
        if !rule.child_regex.is_match(current_name) {
            continue;
        }
        let parent_match = chain.iter().any(|(_, n)| rule.parent_regex.is_match(n));
        if parent_match {
            return Some(SuspiciousPattern::Custom {
                name: rule.name.clone(),
                weight: rule.weight,
            });
        }
    }
    None
}

/// 用户自定义规则。从 `~/.config/proc/lineage_rules.toml` 加载。
#[derive(Debug, Clone)]
pub struct LineageRule {
    pub name: String,
    pub parent_regex: regex::Regex,
    pub child_regex: regex::Regex,
    pub weight: u32,
}

#[derive(serde::Deserialize)]
struct LineageRuleRaw {
    name: String,
    parent_pattern: String,
    child_pattern: String,
    #[serde(default = "default_custom_weight")]
    weight: u32,
}

const fn default_custom_weight() -> u32 {
    20
}

#[derive(serde::Deserialize, Default)]
struct LineageRulesFile {
    #[serde(default)]
    rule: Vec<LineageRuleRaw>,
}

/// 默认 lineage_rules.toml 路径：`~/.config/proc/lineage_rules.toml`。
#[must_use]
pub fn default_rules_path() -> PathBuf {
    crate::dirs_config_dir().join("lineage_rules.toml")
}

/// 从默认路径加载自定义规则。
///
/// 文件不存在 / 解析失败 → 空 Vec（只用内置 3 种 pattern）。stage-5.md 任务 5
/// 明确「默认不存在 → 只用内置 pattern」。
#[must_use]
pub fn load_lineage_rules() -> Vec<LineageRule> {
    load_lineage_rules_from(&default_rules_path())
}

/// 测试 / 自定义路径入口。文件不存在 / 解析失败 → 空 Vec。
#[must_use]
pub fn load_lineage_rules_from(path: &std::path::Path) -> Vec<LineageRule> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(parsed) = toml::from_str::<LineageRulesFile>(&content) else {
        tracing::warn!("lineage_rules.toml 解析失败：{}", path.display());
        return Vec::new();
    };
    parsed
        .rule
        .into_iter()
        .filter_map(|raw| {
            let Ok(parent_regex) = regex::Regex::new(&raw.parent_pattern) else {
                tracing::warn!(
                    "lineage rule「{}」parent_pattern 正则编译失败：{}",
                    raw.name,
                    raw.parent_pattern
                );
                return None;
            };
            let Ok(child_regex) = regex::Regex::new(&raw.child_pattern) else {
                tracing::warn!(
                    "lineage rule「{}」child_pattern 正则编译失败：{}",
                    raw.name,
                    raw.child_pattern
                );
                return None;
            };
            Some(LineageRule {
                name: raw.name,
                parent_regex,
                child_regex,
                weight: raw.weight,
            })
        })
        .collect()
}

/// R17 评分入口：对每个进程检测 `parent_chain`，命中则生成 `RiskFactor`。
///
/// stage-5.md 任务 3 签名是 `check_lineage_risk(procs: &[ProcessInfo])`，
/// 这里加 `custom_rules` 参数承载用户配置（SecurityScorer 持有，每次调用传入）。
pub fn check_lineage_risk(procs: &[ProcessInfo], custom_rules: &[LineageRule]) -> Vec<RiskFactor> {
    procs
        .iter()
        .filter_map(|proc| {
            let pattern = detect_suspicious_chain(&proc.parent_chain, &proc.name, custom_rules)?;
            Some(RiskFactor {
                category: RiskCategory::ParentChain,
                name: pattern.risk_name(),
                weight: pattern.default_weight(),
                description: pattern.description(&proc.name, &proc.parent_chain),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! build_parent_chain / detect_suspicious_chain / LineageRule 解析的单元测试。
    //! R17 评分集成测试在 `tests/test_lineage.rs`。
    use super::*;

    fn make_proc(pid: u32, name: &str, parent_pid: Option<u32>) -> ProcessInfo {
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
            status: crate::collect::ProcessStatus::Run,
            exe: None,
            cmd: std::sync::Arc::from(Vec::<String>::new()),
            cwd: None,
            parent_pid,
            session_id: None,
            user_id: None,
            start_time: 0,
            run_time: 0,
            name_lower: std::sync::Arc::from(name_arc.to_lowercase().as_str()),
            throttled: crate::throttle::EcoQoSState::default(),
            signature_status: crate::security::SignatureStatus::default(),
            parent_chain: Vec::new(),
        }
    }

    fn build_map(procs: &[ProcessInfo]) -> HashMap<u32, ProcessInfo> {
        procs.iter().map(|p| (p.pid, p.clone())).collect()
    }

    #[test]
    fn build_chain_walks_to_root() {
        // System(4) ← explorer(100) ← cmd(200)
        let procs = vec![
            make_proc(4, "System", None),
            make_proc(100, "explorer.exe", Some(4)),
            make_proc(200, "cmd.exe", Some(100)),
        ];
        let map = build_map(&procs);
        let chain = build_parent_chain(200, &map);
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0], (100, "explorer.exe".to_string()));
        assert_eq!(chain[1], (4, "System".to_string()));
    }

    #[test]
    fn build_chain_cycle_breaks() {
        // A(1) → B(2) → A(1)（PID 复用环）
        let procs = vec![
            make_proc(1, "a.exe", Some(2)),
            make_proc(2, "b.exe", Some(1)),
        ];
        let map = build_map(&procs);
        let chain = build_parent_chain(1, &map);
        // chain[0] = (2, b.exe)，下一层 parent=1 已 visited → 终止
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].0, 2);
    }

    #[test]
    fn build_chain_max_depth_32() {
        // 33 层链（pid 0..32），从 pid 32 追溯到根 pid 0
        let procs: Vec<ProcessInfo> = (0..=33u32)
            .map(|i| {
                make_proc(
                    i,
                    &format!("p{i}.exe"),
                    if i == 0 { None } else { Some(i - 1) },
                )
            })
            .collect();
        let map = build_map(&procs);
        let chain = build_parent_chain(33, &map);
        assert_eq!(chain.len(), MAX_PARENT_CHAIN_DEPTH);
        assert_eq!(chain[0].0, 32);
    }

    #[test]
    fn build_chain_missing_parent_stops() {
        // pid 100 的 parent 999 不在 map 里
        let procs = vec![make_proc(100, "orphan.exe", Some(999))];
        let map = build_map(&procs);
        let chain = build_parent_chain(100, &map);
        assert!(chain.is_empty());
    }

    #[test]
    fn build_chain_self_loop_breaks() {
        let procs = vec![make_proc(1, "loop.exe", Some(1))];
        let map = build_map(&procs);
        let chain = build_parent_chain(1, &map);
        assert!(chain.is_empty());
    }

    #[test]
    fn detect_office_to_shell() {
        let chain = vec![(100, "WINWORD.EXE".to_string())];
        let pat = detect_suspicious_chain(&chain, "powershell.exe", &[]);
        assert_eq!(pat, Some(SuspiciousPattern::OfficeToShell));
        assert_eq!(pat.unwrap().default_weight(), 35);
    }

    #[test]
    fn detect_browser_to_shell() {
        let chain = vec![(100, "chrome.exe".to_string())];
        let pat = detect_suspicious_chain(&chain, "cmd.exe", &[]);
        assert_eq!(pat, Some(SuspiciousPattern::BrowserToShell));
        assert_eq!(pat.unwrap().default_weight(), 25);
    }

    #[test]
    fn detect_script_interpreter() {
        let pat = detect_suspicious_chain(&[], "wscript.exe", &[]);
        assert_eq!(pat, Some(SuspiciousPattern::ScriptInterpreter));
        assert_eq!(pat.unwrap().default_weight(), 15);
    }

    // --- TD-32（v0.12 阶段 5）：ScriptInterpreter 系统启动白名单 ---

    #[test]
    fn td32_script_interpreter_whitelisted_when_parent_is_services() {
        // services.exe → wscript.exe（典型 SCM 登录脚本路径）→ 不扣分。
        let chain = vec![(100, "services.exe".to_string())];
        let pat = detect_suspicious_chain(&chain, "wscript.exe", &[]);
        assert!(
            pat.is_none(),
            "services.exe → wscript.exe 应被白名单豁免，got {pat:?}"
        );
    }

    #[test]
    fn td32_script_interpreter_whitelisted_when_parent_is_wininit() {
        let chain = vec![(100, "wininit.exe".to_string())];
        let pat = detect_suspicious_chain(&chain, "cscript.exe", &[]);
        assert!(pat.is_none(), "wininit.exe → cscript.exe 应被白名单豁免");
    }

    #[test]
    fn td32_script_interpreter_whitelisted_when_parent_is_svchost() {
        let chain = vec![(100, "svchost.exe".to_string())];
        let pat = detect_suspicious_chain(&chain, "mshta.exe", &[]);
        assert!(pat.is_none(), "svchost.exe → mshta.exe 应被白名单豁免");
    }

    #[test]
    fn td32_script_interpreter_whitelist_is_case_insensitive() {
        // Windows 进程名大小写不固定（services.exe 也可能写作 SERVICES.EXE）。
        let chain = vec![(100, "SERVICES.EXE".to_string())];
        let pat = detect_suspicious_chain(&chain, "WScript.exe", &[]);
        assert!(
            pat.is_none(),
            "大小写不敏感：SERVICES.EXE → WScript.exe 应豁免"
        );
    }

    #[test]
    fn td32_script_interpreter_whitelist_only_direct_parent() {
        // 间接祖先是 services.exe（chain[1]）不算白名单——只看直接父（chain[0]）。
        // 例如 evil.exe ← services.exe ← wscript.exe：典型恶意 macro attack
        // 把 services.exe 伪造为祖先（PID 复用 / 用户态混淆）。
        let chain = vec![
            (100, "evil.exe".to_string()),
            (50, "services.exe".to_string()),
        ];
        let pat = detect_suspicious_chain(&chain, "wscript.exe", &[]);
        assert_eq!(
            pat,
            Some(SuspiciousPattern::ScriptInterpreter),
            "间接祖先是 services.exe 不豁免，应正常扣分"
        );
    }

    #[test]
    fn td32_whitelist_does_not_affect_custom_rule_evaluation() {
        // 即使 ScriptInterpreter 被白名单豁免，Custom 规则仍照常评估。
        let chain = vec![(100, "services.exe".to_string())];
        let rule = LineageRule {
            name: "test".to_string(),
            parent_regex: regex::Regex::new("(?i)services").unwrap(),
            child_regex: regex::Regex::new("(?i)wscript").unwrap(),
            weight: 30,
        };
        let pat = detect_suspicious_chain(&chain, "wscript.exe", &[rule]);
        match pat {
            Some(SuspiciousPattern::Custom { name, weight }) => {
                assert_eq!(name, "test");
                assert_eq!(weight, 30);
            }
            other => panic!("expected Custom rule hit even with whitelist, got {other:?}"),
        }
    }

    #[test]
    fn detect_no_match_for_normal_proc() {
        let chain = vec![(100, "explorer.exe".to_string())];
        let pat = detect_suspicious_chain(&chain, "notepad.exe", &[]);
        assert!(pat.is_none());
    }

    #[test]
    fn detect_no_match_when_shell_parent_is_explorer() {
        // cmd ← explorer 是正常情况，不应命中 OfficeToShell/BrowserToShell
        let chain = vec![(100, "explorer.exe".to_string())];
        let pat = detect_suspicious_chain(&chain, "cmd.exe", &[]);
        assert!(pat.is_none());
    }

    #[test]
    fn detect_indirect_office_parent_does_not_match() {
        // cmd ← explorer ← winword（间接父是 Office）—— 只看直接父，不应命中
        let chain = vec![
            (100, "explorer.exe".to_string()),
            (50, "winword.exe".to_string()),
        ];
        let pat = detect_suspicious_chain(&chain, "cmd.exe", &[]);
        assert!(pat.is_none());
    }

    #[test]
    fn check_lineage_risk_returns_factor_for_office_shell() {
        let mut cmd = make_proc(200, "powershell.exe", Some(100));
        cmd.parent_chain = vec![(100, "WINWORD.EXE".to_string())];
        let procs = vec![cmd];
        let factors = check_lineage_risk(&procs, &[]);
        assert_eq!(factors.len(), 1);
        assert_eq!(factors[0].name, "office_to_shell");
        assert_eq!(factors[0].weight, 35);
        assert_eq!(factors[0].category, RiskCategory::ParentChain);
    }

    #[test]
    fn load_rules_nonexistent_returns_empty() {
        let rules =
            load_lineage_rules_from(std::path::Path::new("/nonexistent/path/lineage_rules.toml"));
        assert!(rules.is_empty());
    }

    #[test]
    fn load_rules_parses_valid_toml() {
        let tmp =
            std::env::temp_dir().join(format!("proc-lineage-test-{}.toml", std::process::id()));
        std::fs::write(
            &tmp,
            r#"
[[rule]]
name = "my_editor_to_shell"
parent_pattern = "(?i)my_editor"
child_pattern = "(?i)(cmd|powershell)"
weight = 30
"#,
        )
        .unwrap();
        let rules = load_lineage_rules_from(&tmp);
        let _ = std::fs::remove_file(&tmp);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "my_editor_to_shell");
        assert_eq!(rules[0].weight, 30);
        assert!(rules[0].parent_regex.is_match("My_Editor.exe"));
        assert!(rules[0].child_regex.is_match("cmd.exe"));
    }

    #[test]
    fn load_rules_skips_invalid_regex() {
        let tmp =
            std::env::temp_dir().join(format!("proc-lineage-invalid-{}.toml", std::process::id()));
        std::fs::write(
            &tmp,
            r#"
[[rule]]
name = "bad"
parent_pattern = "(unclosed"
child_pattern = "ok"
weight = 10
"#,
        )
        .unwrap();
        let rules = load_lineage_rules_from(&tmp);
        let _ = std::fs::remove_file(&tmp);
        assert!(rules.is_empty());
    }

    #[test]
    fn custom_rule_matches() {
        let rule = LineageRule {
            name: "test".to_string(),
            parent_regex: regex::Regex::new("(?i)my_editor").unwrap(),
            child_regex: regex::Regex::new("(?i)cmd").unwrap(),
            weight: 30,
        };
        let chain = vec![(100, "my_editor.exe".to_string())];
        let pat = detect_suspicious_chain(&chain, "cmd.exe", &[rule]);
        match pat {
            Some(SuspiciousPattern::Custom { name, weight }) => {
                assert_eq!(name, "test");
                assert_eq!(weight, 30);
            }
            other => panic!("expected Custom, got {other:?}"),
        }
    }
}
