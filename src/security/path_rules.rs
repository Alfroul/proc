//! v0.11 阶段 6：可疑启动路径 R18（malware 启动位置）。
//!
//! 与 v0.6 `src/security/path_check.rs` **不冲突**：后者是 R3-R5 等检查
//! （temp_dir / downloads_dir / network_path / desktop / system32_impersonation），
//! 基于纯字符串子串匹配；本模块是 R18 的独立入口，基于环境变量展开 +
//! 大小写不敏感前缀匹配，专注 malware 常见的「用户可写目录」。
//!
//! **叠加扣分**：R18 命中时，与 path_check 的 temp_dir（25）/ downloads_dir（15）
//! 扣分会叠加——与 R17 + v0.7 office_spawning_shell（30+35=65）同款 surgical
//! 原则，安全评分偏向严格。用户在 stage-6.md 明确「协同扣分：R16 + R18 → 额外
//! 扣 10」，本模块在 score.rs 第 18 步接入时一并实装协同段。
//!
//! **R18 命中条件**（stage-6.md 任务 1）：
//! - **Temp**（扣 20）：`%TEMP%` / `%TMP%`（Windows = `C:\Users\{user}\AppData\Local\Temp`）
//! - **AppData**（扣 15）：`%APPDATA%`（= `C:\Users\{user}\AppData\Roaming`）
//! - **LocalAppData**（扣 15）：`%LOCALAPPDATA%`（= `C:\Users\{user}\AppData\Local`）
//! - **UserProfileDownloads**（扣 15）：`%USERPROFILE%\Downloads`
//! - **Custom**（按用户配置 weight）：来自 `~/.config/proc/path_rules.toml`
//!
//! 不标记：`Program Files` / `Windows` / `System32`（系统目录，签名进程正常）。

use std::path::{Path, PathBuf};

use super::score::{RiskCategory, RiskFactor};
use crate::collect::ProcessInfo;

/// R18 命中后的可疑路径分类。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuspiciousPathKind {
    /// `%TEMP%` / `%TMP%`（malware 最爱）。
    Temp,
    /// `%APPDATA%`（= `AppData\Roaming`）。
    AppData,
    /// `%LOCALAPPDATA%`（= `AppData\Local`，包含 Temp 子目录但本变体命中整个 Local）。
    LocalAppData,
    /// `%USERPROFILE%\Downloads`（浏览器默认下载目录）。
    UserProfileDownloads,
    /// 用户在 `path_rules.toml` 配置的自定义目录命中。
    Custom { name: String, weight: u32 },
}

impl SuspiciousPathKind {
    /// 该 kind 对应的扣分（Custom 用配置 weight，其他用内置常量）。
    /// stage-6.md 任务 2：Temp=20 / 其他=15。
    #[must_use]
    pub fn default_weight(&self) -> u32 {
        match self {
            Self::Temp => 20,
            Self::AppData | Self::LocalAppData | Self::UserProfileDownloads => 15,
            Self::Custom { weight, .. } => *weight,
        }
    }

    /// RiskFactor.name 字段值（用于去重 / 测试断言）。
    #[must_use]
    pub fn risk_name(&self) -> String {
        match self {
            Self::Temp => "suspicious_path_temp".to_string(),
            Self::AppData => "suspicious_path_appdata".to_string(),
            Self::LocalAppData => "suspicious_path_local_appdata".to_string(),
            Self::UserProfileDownloads => "suspicious_path_downloads".to_string(),
            Self::Custom { name, .. } => format!("suspicious_path_custom:{name}"),
        }
    }

    /// RiskFactor.description 字段值（含完整 exe_path 便于用户排查）。
    fn description(&self, exe_path: &str) -> String {
        match self {
            Self::Temp => format!("从临时目录运行（malware 常用）：{exe_path}"),
            Self::AppData => format!("从 AppData\\Roaming 运行（可疑位置）：{exe_path}"),
            Self::LocalAppData => format!("从 AppData\\Local 运行（可疑位置）：{exe_path}"),
            Self::UserProfileDownloads => {
                format!("从下载目录运行（可疑位置）：{exe_path}")
            }
            Self::Custom { name, .. } => {
                format!("自定义可疑目录「{name}」命中：{exe_path}")
            }
        }
    }
}

/// 系统目录白名单（不标记为可疑）。stage-6.md 任务 1：「不标记 Program Files
/// / Windows / System32（系统目录，签名进程正常）」。
///
/// 命中其中任一前缀 → `is_in_suspicious_path` 直接返回 None（即便同时位于
/// `%TEMP%` 也不计——这是 stage-6.md 的优先级规则：系统目录优先于可疑标记）。
const SYSTEM_DIR_ENVS: &[&str] = &["ProgramFiles", "ProgramFiles(x86)", "SystemRoot", "windir"];

/// `UserDirs` 缓存的环境变量展开结果。`None` = 环境变量未设置（如 Linux 上
/// `%TEMP%` 不存在）。
#[derive(Debug, Clone, Default)]
pub struct UserDirs {
    pub temp: Option<PathBuf>,
    pub appdata: Option<PathBuf>,
    pub local_appdata: Option<PathBuf>,
    pub userprofile: Option<PathBuf>,
    /// 系统目录白名单（展开后的实际路径）。
    pub system_dirs: Vec<PathBuf>,
}

/// 展开 `%ENV_VAR%`。环境变量未设置 / 非 Unicode → None。
///
/// stage-6.md 任务 1：用 `std::env::var(env_var)` + 路径前缀匹配。
#[must_use]
pub fn expand_user_dir(env_var: &str) -> Option<PathBuf> {
    std::env::var_os(env_var).map(PathBuf::from)
}

impl UserDirs {
    /// 从当前进程环境变量构造。Linux / 非 Windows 上 TEMP / APPDATA 等通常 None，
    /// R18 自动降级为 no-op。
    #[must_use]
    pub fn from_env() -> Self {
        // TEMP / TMP 在 Windows 上等价，两个都展开（取 TEMP 优先）。
        let temp = expand_user_dir("TEMP").or_else(|| expand_user_dir("TMP"));
        let appdata = expand_user_dir("APPDATA");
        let local_appdata = expand_user_dir("LOCALAPPDATA");
        let userprofile = expand_user_dir("USERPROFILE");
        let system_dirs = SYSTEM_DIR_ENVS
            .iter()
            .filter_map(|env| std::env::var_os(env).map(PathBuf::from))
            .collect();
        Self {
            temp,
            appdata,
            local_appdata,
            userprofile,
            system_dirs,
        }
    }

    /// 测试 / 自定义入口：用 caller 提供的 PathBuf 构造（不读环境变量）。
    /// stage-6.md 任务 5 单元测试入口。
    #[must_use]
    pub fn new(
        temp: Option<PathBuf>,
        appdata: Option<PathBuf>,
        local_appdata: Option<PathBuf>,
        userprofile: Option<PathBuf>,
    ) -> Self {
        Self {
            temp,
            appdata,
            local_appdata,
            userprofile,
            system_dirs: Vec::new(),
        }
    }
}

/// 大小写不敏感的路径前缀匹配（Windows 路径不区分大小写）。
///
/// `dir` = 展开后的可疑目录，`path` = 待检查的 exe 路径。命中条件：
/// `path` == `dir` 或 `path` 以 `{dir}\` / `{dir}/` 开头。
///
/// 注意：必须先验证 `path_str.starts_with(dir_str)`，否则不同前缀的 dir
/// 切片会误判（如 dir="C:\Program Files" 长度 18，path="D:\..." 第 18 字符
/// 恰好是 `\` 时会被错误判定为命中）。
fn path_starts_with_ci(dir: &Path, path: &Path) -> bool {
    let dir_str = dir.to_string_lossy().to_lowercase();
    let path_str = path.to_string_lossy().to_lowercase();
    if path_str.len() < dir_str.len() {
        return false;
    }
    if path_str == dir_str {
        return true;
    }
    // 必须先验证前缀匹配（前缀 + 路径分隔符），避免不同前缀切片误判。
    let prefix_with_sep = format!("{dir_str}\\");
    let prefix_with_fwd = format!("{dir_str}/");
    path_str.starts_with(&prefix_with_sep) || path_str.starts_with(&prefix_with_fwd)
}

/// 系统目录白名单命中检查（Program Files / Windows / System32）。
fn is_in_system_dir(user_dirs: &UserDirs, path: &Path) -> bool {
    user_dirs
        .system_dirs
        .iter()
        .any(|dir| path_starts_with_ci(dir, path))
}

/// 检测 exe_path 是否位于可疑目录。
///
/// 返回 `Some(SuspiciousPathKind)` 命中信息；返回 `None` 表示不可疑
/// （系统目录白名单优先级最高 —— 即便 `%TEMP%` 子目录位于 `C:\Windows\Temp`
/// 也不命中 R18，因为 SystemRoot 白名单优先）。
///
/// stage-6.md 任务 1 + 任务 5：
/// - 大小写不敏感（`c:\users` vs `C:\Users`）
/// - 环境变量未设置 → 该 kind 直接 skip（如 Linux 上 `%TEMP%` 不存在）
/// - 自定义规则按文件顺序检查，先命中先返回
#[must_use]
pub fn is_in_suspicious_path(
    exe_path: &Path,
    user_dirs: &UserDirs,
    custom_rules: &[PathRule],
) -> Option<SuspiciousPathKind> {
    // 系统目录白名单优先：Program Files / Windows / System32 → 永不标记。
    if is_in_system_dir(user_dirs, exe_path) {
        return None;
    }

    // UserProfileDownloads 需要先有 USERPROFILE 才能拼出 Downloads。
    if let Some(userprofile) = &user_dirs.userprofile {
        let downloads = userprofile.join("Downloads");
        if path_starts_with_ci(&downloads, exe_path) {
            return Some(SuspiciousPathKind::UserProfileDownloads);
        }
    }

    // Temp 优先（malware 最爱，扣分最高 20）。
    if let Some(temp) = &user_dirs.temp
        && path_starts_with_ci(temp, exe_path)
    {
        return Some(SuspiciousPathKind::Temp);
    }

    // AppData（Roaming）必须在 LocalAppData 之前检查，因为两者都在 AppData 下。
    // C:\Users\{u}\AppData\Roaming\xxx 不能命中 LocalAppData。
    if let Some(appdata) = &user_dirs.appdata
        && path_starts_with_ci(appdata, exe_path)
    {
        return Some(SuspiciousPathKind::AppData);
    }

    if let Some(local_appdata) = &user_dirs.local_appdata
        && path_starts_with_ci(local_appdata, exe_path)
    {
        // 注意：Temp 通常位于 LocalAppData\Temp 子目录，但 Temp 检查在前已返回。
        // 这里只命中 LocalAppData 但非 Temp 的部分（如 Local\Microsoft\...）。
        return Some(SuspiciousPathKind::LocalAppData);
    }

    // 用户自定义规则：先命中先返回。
    for rule in custom_rules {
        if path_starts_with_ci(&rule.path, exe_path) {
            return Some(SuspiciousPathKind::Custom {
                name: rule.name.clone(),
                weight: rule.weight,
            });
        }
    }

    None
}

/// 用户自定义可疑目录规则。从 `~/.config/proc/path_rules.toml` 加载。
#[derive(Debug, Clone)]
pub struct PathRule {
    pub name: String,
    /// 已展开环境变量的绝对路径（展开失败规则丢弃）。
    pub path: PathBuf,
    pub weight: u32,
    pub reason: String,
}

#[derive(serde::Deserialize)]
struct PathRuleRaw {
    name: String,
    /// 支持 `%VAR%` / `$VAR` / `${VAR}` 三种占位符。stage-6.md 任务 3 示例用
    /// `%USERPROFILE%\\my_suspicious_dir`。
    path: String,
    #[serde(default = "default_custom_weight")]
    weight: u32,
    #[serde(default)]
    reason: String,
}

const fn default_custom_weight() -> u32 {
    25
}

#[derive(serde::Deserialize, Default)]
struct PathRulesFile {
    #[serde(default)]
    suspicious_dir: Vec<PathRuleRaw>,
}

/// 默认 path_rules.toml 路径：`~/.config/proc/path_rules.toml`。
#[must_use]
pub fn default_rules_path() -> PathBuf {
    crate::dirs_config_dir().join("path_rules.toml")
}

/// 从默认路径加载自定义规则。文件不存在 / 解析失败 → 空 Vec。
#[must_use]
pub fn load_path_rules() -> Vec<PathRule> {
    load_path_rules_from(&default_rules_path())
}

/// 测试 / 自定义路径入口。文件不存在 / 解析失败 → 空 Vec。
#[must_use]
pub fn load_path_rules_from(path: &Path) -> Vec<PathRule> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(parsed) = toml::from_str::<PathRulesFile>(&content) else {
        tracing::warn!("path_rules.toml 解析失败：{}", path.display());
        return Vec::new();
    };
    parsed
        .suspicious_dir
        .into_iter()
        .filter_map(|raw| {
            let expanded = expand_env_placeholders(&raw.path);
            if expanded.is_empty() {
                tracing::warn!(
                    "path rule「{}」展开后为空路径，跳过：{}",
                    raw.name,
                    raw.path
                );
                return None;
            }
            Some(PathRule {
                name: raw.name,
                path: PathBuf::from(expanded),
                weight: raw.weight,
                reason: raw.reason,
            })
        })
        .collect()
}

/// 展开 `%VAR%` / `${VAR}` / `$VAR` 三种占位符。未设置的变量保留原占位符
/// （规则仍生效但通常匹配不到，等价于禁用）。stage-6.md 任务 3 示例。
/// v0.20 stage 2 起 pub(crate)：agent.toml `[llama-cpp].search_paths` 复用。
///
/// v0.20 stage 2 修复：原实现按字节迭代 + `push(c as char)`，多字节 UTF-8
/// 路径段（如中文用户名）被逐字节拆散损坏；改为按字符推进整段拷贝。
pub(crate) fn expand_env_placeholders(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while !rest.is_empty() {
        let b = rest.as_bytes()[0];
        if b == b'%' {
            // %VAR%
            if let Some(end) = rest[1..].find('%')
                && let Ok(val) = std::env::var(&rest[1..1 + end])
            {
                out.push_str(&val);
                rest = &rest[2 + end..];
                continue;
            }
            out.push('%');
            rest = &rest[1..];
        } else if b == b'$' && rest.len() > 1 && rest.as_bytes()[1] == b'{' {
            // ${VAR}
            if let Some(end) = rest[2..].find('}') {
                let var = &rest[2..2 + end];
                if let Ok(val) = std::env::var(var) {
                    out.push_str(&val);
                    rest = &rest[3 + end..];
                    continue;
                }
            }
            out.push('$');
            rest = &rest[1..];
        } else if b == b'$' {
            // $VAR（字母数字下划线，到非标识符字符止）
            let bytes = rest.as_bytes();
            let mut end = 1;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            if end > 1 {
                let var = &rest[1..end];
                if let Ok(val) = std::env::var(var) {
                    out.push_str(&val);
                    rest = &rest[end..];
                    continue;
                }
            }
            out.push('$');
            rest = &rest[1..];
        } else {
            // 多字节 UTF-8 字符整段拷贝（防字节级 push 损坏中文路径）。
            let ch = rest.chars().next().unwrap();
            out.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    out
}

/// R18 评分入口：对每个进程检查 exe 路径是否位于可疑目录。
///
/// stage-6.md 任务 2 签名：`check_path_risk(procs: &[ProcessInfo]) -> Vec<RiskFactor>`，
/// 这里加 `user_dirs` + `custom_rules` 参数承载 scorer 持久状态（每次调用传入，
/// 避免每进程重新读环境变量）。
///
/// 返回扁平 Vec：多进程会汇总所有命中条目。score 函数对单个 proc 调用时传
/// `slice::from_ref(proc)`，最多返回 1 条（先命中先返回）。
pub fn check_path_risk(
    procs: &[ProcessInfo],
    user_dirs: &UserDirs,
    custom_rules: &[PathRule],
) -> Vec<RiskFactor> {
    procs
        .iter()
        .filter_map(|proc| {
            let exe_path = proc.exe.as_deref()?;
            let kind = is_in_suspicious_path(Path::new(exe_path), user_dirs, custom_rules)?;
            Some(RiskFactor {
                category: RiskCategory::FilePath,
                name: kind.risk_name(),
                weight: kind.default_weight(),
                description: kind.description(exe_path),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! stage-6.md 任务 5 单元测试入口。
    //! 集成测试（含 R16 协同扣分）在 `tests/test_path_rules.rs`。
    use super::*;

    fn user_dirs_fixture() -> UserDirs {
        UserDirs::new(
            Some(PathBuf::from("C:\\Users\\test\\AppData\\Local\\Temp")),
            Some(PathBuf::from("C:\\Users\\test\\AppData\\Roaming")),
            Some(PathBuf::from("C:\\Users\\test\\AppData\\Local")),
            Some(PathBuf::from("C:\\Users\\test")),
        )
    }

    #[test]
    fn temp_hit_returns_temp_kind() {
        let dirs = user_dirs_fixture();
        let kind = is_in_suspicious_path(
            Path::new("C:\\Users\\test\\AppData\\Local\\Temp\\evil.exe"),
            &dirs,
            &[],
        );
        assert_eq!(kind, Some(SuspiciousPathKind::Temp));
        assert_eq!(kind.unwrap().default_weight(), 20);
    }

    #[test]
    fn appdata_hit_returns_appdata_kind() {
        let dirs = user_dirs_fixture();
        let kind = is_in_suspicious_path(
            Path::new("C:\\Users\\test\\AppData\\Roaming\\payload\\run.exe"),
            &dirs,
            &[],
        );
        assert_eq!(kind, Some(SuspiciousPathKind::AppData));
        assert_eq!(kind.unwrap().default_weight(), 15);
    }

    #[test]
    fn local_appdata_hit_returns_local_appdata_kind() {
        let dirs = user_dirs_fixture();
        // 注意：不在 Temp 子目录，否则会被 Temp 截获。
        let kind = is_in_suspicious_path(
            Path::new("C:\\Users\\test\\AppData\\Local\\MalwareApp\\mal.exe"),
            &dirs,
            &[],
        );
        assert_eq!(kind, Some(SuspiciousPathKind::LocalAppData));
        assert_eq!(kind.unwrap().default_weight(), 15);
    }

    #[test]
    fn downloads_hit_returns_downloads_kind() {
        let dirs = user_dirs_fixture();
        let kind = is_in_suspicious_path(
            Path::new("C:\\Users\\test\\Downloads\\installer.exe"),
            &dirs,
            &[],
        );
        assert_eq!(kind, Some(SuspiciousPathKind::UserProfileDownloads));
    }

    #[test]
    fn program_files_not_hit() {
        let mut dirs = user_dirs_fixture();
        dirs.system_dirs.push(PathBuf::from("C:\\Program Files"));
        let kind =
            is_in_suspicious_path(Path::new("C:\\Program Files\\MyApp\\myapp.exe"), &dirs, &[]);
        assert!(kind.is_none());
    }

    #[test]
    fn case_insensitive_match() {
        let dirs = user_dirs_fixture();
        let kind = is_in_suspicious_path(
            Path::new("c:\\users\\TEST\\appdata\\local\\temp\\evil.exe"),
            &dirs,
            &[],
        );
        assert_eq!(kind, Some(SuspiciousPathKind::Temp));
    }

    #[test]
    fn env_var_unset_returns_none_for_that_kind() {
        // Linux-like 场景：TEMP/APPDATA 全 None
        let dirs = UserDirs::new(None, None, None, None);
        let kind = is_in_suspicious_path(Path::new("/tmp/evil.exe"), &dirs, &[]);
        assert!(kind.is_none());
    }

    #[test]
    fn no_match_for_unknown_dir() {
        let dirs = user_dirs_fixture();
        let kind = is_in_suspicious_path(Path::new("D:\\Games\\launcher.exe"), &dirs, &[]);
        assert!(kind.is_none());
    }

    #[test]
    fn custom_rule_matches() {
        let dirs = user_dirs_fixture();
        let rule = PathRule {
            name: "my_dir".to_string(),
            path: PathBuf::from("D:\\Suspicious"),
            weight: 30,
            reason: "test".to_string(),
        };
        let kind = is_in_suspicious_path(Path::new("D:\\Suspicious\\payload.exe"), &dirs, &[rule]);
        match kind {
            Some(SuspiciousPathKind::Custom { name, weight }) => {
                assert_eq!(name, "my_dir");
                assert_eq!(weight, 30);
            }
            other => panic!("expected Custom, got {other:?}"),
        }
    }

    #[test]
    fn custom_rule_does_not_match_unrelated_path() {
        let dirs = user_dirs_fixture();
        let rule = PathRule {
            name: "my_dir".to_string(),
            path: PathBuf::from("D:\\Suspicious"),
            weight: 30,
            reason: "test".to_string(),
        };
        let kind = is_in_suspicious_path(Path::new("E:\\Clean\\app.exe"), &dirs, &[rule]);
        assert!(kind.is_none());
    }

    #[test]
    fn expand_env_percent_placeholders() {
        // SAFETY: 测试专用 var，无并发竞争；同进程内串行执行。
        unsafe {
            std::env::set_var("PROC_TEST_VAR_X", "C:\\CustomPath");
        }
        let expanded = expand_env_placeholders("%PROC_TEST_VAR_X%\\sub");
        // SAFETY: 同上。
        unsafe {
            std::env::remove_var("PROC_TEST_VAR_X");
        }
        assert_eq!(expanded, "C:\\CustomPath\\sub");
    }

    #[test]
    fn expand_env_dollar_brace_placeholders() {
        // SAFETY: 同上。
        unsafe {
            std::env::set_var("PROC_TEST_VAR_Y", "D:\\Other");
        }
        let expanded = expand_env_placeholders("${PROC_TEST_VAR_Y}/bin");
        // SAFETY: 同上。
        unsafe {
            std::env::remove_var("PROC_TEST_VAR_Y");
        }
        assert_eq!(expanded, "D:\\Other/bin");
    }

    #[test]
    fn load_rules_nonexistent_returns_empty() {
        let rules = load_path_rules_from(Path::new("/nonexistent/path/path_rules.toml"));
        assert!(rules.is_empty());
    }

    #[test]
    fn load_rules_parses_valid_toml() {
        let tmp =
            std::env::temp_dir().join(format!("proc-pathrules-test-{}.toml", std::process::id()));
        std::fs::write(
            &tmp,
            r#"
[[suspicious_dir]]
name = "my_dir"
path = "D:\\StaticPath"
weight = 30
reason = "test"
"#,
        )
        .unwrap();
        let rules = load_path_rules_from(&tmp);
        let _ = std::fs::remove_file(&tmp);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "my_dir");
        assert_eq!(rules[0].weight, 30);
        assert_eq!(rules[0].path, PathBuf::from("D:\\StaticPath"));
    }

    #[test]
    fn check_path_risk_returns_factor_for_temp() {
        let dirs = user_dirs_fixture();
        let proc = crate::collect::ProcessInfo {
            pid: 1,
            name: std::sync::Arc::from("evil.exe"),
            exe: Some(std::sync::Arc::from(
                "C:\\Users\\test\\AppData\\Local\\Temp\\evil.exe",
            )),
            ..make_minimal_proc()
        };
        let factors = check_path_risk(std::slice::from_ref(&proc), &dirs, &[]);
        assert_eq!(factors.len(), 1);
        assert_eq!(factors[0].name, "suspicious_path_temp");
        assert_eq!(factors[0].weight, 20);
        assert_eq!(factors[0].category, RiskCategory::FilePath);
    }

    /// 辅助：构造一个最小可用的 ProcessInfo（check_path_risk 测试用）。
    fn make_minimal_proc() -> crate::collect::ProcessInfo {
        let name: std::sync::Arc<str> = std::sync::Arc::from("dummy");
        crate::collect::ProcessInfo {
            pid: 0,
            name: std::sync::Arc::clone(&name),
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
            parent_pid: None,
            session_id: None,
            user_id: None,
            start_time: 0,
            run_time: 0,
            name_lower: std::sync::Arc::from(name.to_lowercase().as_str()),
            throttled: crate::throttle::EcoQoSState::default(),
            signature_status: crate::security::SignatureStatus::default(),
            parent_chain: Vec::new(),
        }
    }
}
