# 阶段 2：安全 Slice — secret 脱敏 + 自我加固 + 录屏防护 + 子进程权限剥离

> **独立会话指令**：阅读 CONTEXT.md 和 docs/stages/stage-2.md，完成所有任务后确认完成
>
> **开工前只需阅读**：CONTEXT.md（领域词汇，特别是 env_reveal / self-mitigation / restricted_spawn / mask_value）、本文件、测试命令。

**目标**：v0.6.0 P0 安全 4 项全部落地。proc elevated 时不再是 credential theft 跳板；详情页 Env Tab / 录屏 / 截图分享不再泄漏凭据。

**前置依赖**：阶段 1 已完成（ADR-0008 Status: Proposed 已入仓）。

**依赖测试**（开工时跑这些测试的详情）：
- `cargo test --release --tb=no -q`（全量回归 summary，应 611 passed）
- 阶段 1 是文档/CI Spike，无 Rust 测试新增；只需保证全量回归绿

**预期代码量**：~830 行（含测试）

**任务清单**：

### 任务 1：环境变量 secret 脱敏（项 #1）

**新模块**：`src/inspect/env_mask.rs`

```rust
//! 环境变量 secret 脱敏 — 见 CONTEXT.md / ADR-0008。
//! 默认 mask，按 v 切换 env_reveal；录屏时强制 mask。

const SECRET_PATTERNS: &[&str] = &[
    "KEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD", "PWD",
    "CREDENTIAL", "PRIVATE", "AUTH", "API", "DSN", "CONNECTION_STRING",
];

/// 判断 env key 是否疑似 secret（大小写不敏感）
#[must_use]
pub fn is_secret_key(key: &str) -> bool {
    let upper = key.to_uppercase();
    SECRET_PATTERNS.iter().any(|p| upper.contains(p))
        || key.to_uppercase() == "DATABASE_URL"  // 含 :password@ 的连接串
        || key.to_uppercase().ends_with("_AUTHORIZATION")
}

/// 把 secret 值脱敏为 `前2字符***(原长 B)` 格式
#[must_use]
pub fn mask_value(val: &str) -> String {
    if val.is_empty() {
        return String::new();
    }
    let prefix: String = val.chars().take(2).collect();
    format!("{prefix}***(val.len() B)")
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn detects_common_secret_keys() {
        assert!(is_secret_key("AWS_SECRET_ACCESS_KEY"));
        assert!(is_secret_key("GITHUB_TOKEN"));
        assert!(is_secret_key("DB_PASSWORD"));
        assert!(is_secret_key("OPENAI_API_KEY"));
        assert!(is_secret_key("password"));   // 小写
        assert!(is_secret_key("Database_URL"));
    }
    
    #[test]
    fn does_not_false_positive_common_keys() {
        assert!(!is_secret_key("PATH"));
        assert!(!is_secret_key("HOME"));
        assert!(!is_secret_key("SYSTEMROOT"));
        assert!(!is_secret_key("LANG"));
        assert!(!is_secret_key("USERPROFILE"));  // USERPROFILE 含 USER 但不含 PATTERN
    }
    
    #[test]
    fn mask_value_preserves_prefix() {
        assert_eq!(mask_value(""), "");
        assert_eq!(mask_value("ab"), "ab***(2 B)");
        assert_eq!(mask_value("wJalrXUt"), "wJ***(8 B)");
        // 多字节字符取前 2 个 char（不是 byte）
        assert_eq!(mask_value("密码值123"), "密码***(12 B)");  // 12 = 3*3 + 3
    }
}
```

**改 `src/inspect/mod.rs::EnvVar`**：

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnvVar {
    pub key: String,
    pub value: String,
    /// v0.6.0 阶段 2 新增：是否被判定为 secret（不影响序列化，仅 UI 用）
    #[serde(default)]
    pub is_secret: bool,
}

impl EnvVar {
    /// 渲染值（reveal=true 显示真值，reveal=false 显示 mask）
    pub fn render_value(&self, reveal: bool) -> &str {
        if self.is_secret && !reveal {
            // 注意：这里返回 &str 需要 mask 是预计算的；改成 owned String 返回更简单
            // 实际实现：用 `render_value_owned(&self, reveal: bool) -> String`
            unimplemented!("见下方 owned 版本")
        } else {
            &self.value
        }
    }
    
    pub fn render_value_owned(&self, reveal: bool) -> String {
        if self.is_secret && !reveal {
            crate::inspect::env_mask::mask_value(&self.value)
        } else {
            self.value.clone()
        }
    }
}
```

**改 `src/inspect/env.rs::parse_utf16_env`**：

```rust
// 在 parse 时同步判定 is_secret
pub fn parse_utf16_env(raw: &[u8]) -> Vec<EnvVar> {
    // ... 原解码逻辑
    decoded.iter().map(|(k, v)| EnvVar {
        key: k.clone(),
        value: v.clone(),
        is_secret: crate::inspect::env_mask::is_secret_key(k),
    }).collect()
}
```

**改 `src/app.rs`**：

加字段：
```rust
pub struct App {
    // ... 既有字段
    /// 详情页 Env Tab 是否显示 secret 真值（默认 false；录屏时强制 false）
    pub env_reveal: bool,
}
```

`App::new()` 初始化 `env_reveal: false`。

**改 `src/tui/detail_view.rs::draw_env_tab`**：

```rust
fn draw_env_tab(f: &mut Frame, area: Rect, app: &App) {
    let data = match &app.inspection_data {
        Some(d) => &d.env,
        None => { /* 显示降级提示 */ return; }
    };
    
    // 在录屏模式下强制 mask
    let reveal = app.env_reveal && !app.recording;
    
    let rows = data.iter().map(|env| {
        let value = env.render_value_owned(reveal);
        Row::new(vec![Cell::from(env.key.as_str()), Cell::from(value)])
    });
    // ... 渲染表格
    
    // 顶部提示：当前是 mask 还是 reveal
    let badge = if reveal { "🔓env-reveal" } else { "🔒env-masked" };
    // 在表格标题栏显示 badge
}
```

`handle_detail_key` 加 `v` 分支：
```rust
KeyCode::Char('v') => {
    if app.recording {
        app.status_message = "录屏中禁止 reveal env secret".into();
    } else {
        app.env_reveal = !app.env_reveal;
        app.status_message = if app.env_reveal {
            "Env: 显示真值（仅本会话，录屏强制 mask）".into()
        } else {
            "Env: 已 mask secret".into()
        };
    }
}
```

**改 `src/tui/help_panel.rs`**：在详情页快捷键段加 `v 切换 env 脱敏`。

**测试**：
- 模块内嵌单测（上面已写）
- `tests/test_env_mask.rs`（新）：
  - `is_secret_key` 覆盖 20+ 常见 key 名 + 5 个 false positive
  - `mask_value` 边界（空 / 1 字符 / 多字节字符 / 长 token）
  - `EnvVar::render_value_owned(reveal=false)` mask；`(reveal=true)` 显示真值
  - 集成：录屏 `recording=true` + `env_reveal=true` 时仍然 mask

---

### 任务 2：进程自我加固（项 #2）

**新模块**：`src/security/self_mitigation.rs`（cfg-gate Windows）

```rust
//! 进程自我加固 — 见 CONTEXT.md / ADR-0008。
//! v0.6.0 开启: DEP(Permanent) + ASLR + ProhibitDynamicCode + DisableExtensionPoints + 远程 ImageLoad
//! 失败时 tracing::warn! 但不 panic（启动健壮性 > 完美加固）

#[cfg(windows)]
pub fn apply_self_mitigations() {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Threading::{
        SetProcessMitigationPolicy, ProcessMitigationPolicy,
        PROCESS_MITIGATION_DEP_POLICY, PROCESS_MITIGATION_ASLR_POLICY,
        PROCESS_MITIGATION_DYNAMIC_CODE_POLICY,
        PROCESS_MITIGATION_EXTENSION_POINT_DISABLE_POLICY,
        PROCESS_MITIGATION_IMAGE_LOAD_POLICY,
    };
    
    unsafe {
        // 1. DEP 永久开启
        let dep = PROCESS_MITIGATION_DEP_POLICY {
            Enable: 1, Permanent: 1,
            ..Default::default()
        };
        if SetProcessMitigationPolicy(
            ProcessMitigationPolicy::ProcessDEPPolicy,
            &dep as *const _ as _,
            std::mem::size_of::<PROCESS_MITIGATION_DEP_POLICY>() as u32,
        ).is_err() {
            tracing::warn!("SetProcessMitigationPolicy(DEP) failed");
        }
        
        // 2. ASLR High Entropy
        let aslr = PROCESS_MITIGATION_ASLR_POLICY {
            EnableHighEntropyASLR: 1, EnableBottomUpRandomization: 1,
            ..Default::default()
        };
        if SetProcessMitigationPolicy(
            ProcessMitigationPolicy::ProcessASLRPolicy,
            &aslr as *const _ as _,
            std::mem::size_of::<PROCESS_MITIGATION_ASLR_POLICY>() as u32,
        ).is_err() {
            tracing::warn!("SetProcessMitigationPolicy(ASLR) failed");
        }
        
        // 3. ProhibitDynamicCode
        let dyn_code = PROCESS_MITIGATION_DYNAMIC_CODE_POLICY {
            ProhibitDynamicCode: 1,
            AllowThreadOptOut: 0,
            AllowRemoteDowngrade: 0,
            ..Default::default()
        };
        if SetProcessMitigationPolicy(
            ProcessMitigationPolicy::ProcessDynamicCodePolicy,
            &dyn_code as *const _ as _,
            std::mem::size_of::<PROCESS_MITIGATION_DYNAMIC_CODE_POLICY>() as u32,
        ).is_err() {
            tracing::warn!("SetProcessMitigationPolicy(DynamicCode) failed — 某些 JIT 依赖可能受影响");
        }
        
        // 4. DisableExtensionPoints (AppInit_DLLs / global hooks)
        let ext = PROCESS_MITIGATION_EXTENSION_POINT_DISABLE_POLICY {
            DisableExtensionPoints: 1,
            ..Default::default()
        };
        if SetProcessMitigationPolicy(
            ProcessMitigationPolicy::ProcessExtensionPointDisablePolicy,
            &ext as *const _ as _,
            std::mem::size_of::<PROCESS_MITIGATION_EXTENSION_POINT_DISABLE_POLICY>() as u32,
        ).is_err() {
            tracing::warn!("SetProcessMitigationPolicy(ExtensionPoint) failed");
        }
        
        // 5. ImageLoad: 禁从远程 UNC 加载
        let img = PROCESS_MITIGATION_IMAGE_LOAD_POLICY {
            NoRemoteMftImages: 1,
            NoLowMftImages: 1,
            PreferSystem32Images: 1,
            ..Default::default()
        };
        if SetProcessMitigationPolicy(
            ProcessMitigationPolicy::ProcessImageLoadPolicy,
            &img as *const _ as _,
            std::mem::size_of::<PROCESS_MITIGATION_IMAGE_LOAD_POLICY>() as u32,
        ).is_err() {
            tracing::warn!("SetProcessMitigationPolicy(ImageLoad) failed");
        }
    }
}

#[cfg(not(windows))]
pub fn apply_self_mitigations() {
    // Linux/macOS 等价物见 ADR-0008 后续工作（prctl/seccomp），v0.6.0 暂不实现
    tracing::debug!("self-mitigation: 非 Windows 平台暂未实现");
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn apply_self_mitigations_does_not_panic() {
        // 在测试进程中也调一次（Windows 上应该成功，其他平台无操作）
        apply_self_mitigations();
    }
}
```

**Cargo.toml 加 feature**：

```toml
[target.'cfg(windows)'.dependencies]
windows = { version = "0.57", features = [
    # ... 既有 features
    "Win32_System_Threading",   # 已有
] }
```

实际上 `SetProcessMitigationPolicy` 在 `Win32_System_Threading` feature 里，已经有了。无需新增。

**改 `src/main.rs::main`**：

```rust
fn main() {
    // v0.6.0 阶段 2: 最早调用 self-mitigation（早于任何 worker / FFI）
    proc::security::self_mitigation::apply_self_mitigations();
    
    // ... 既有逻辑（包括 init_tracing 等）
}
```

注意：`apply_self_mitigations` 内部用 `tracing::warn!`，但 tracing 此时还没初始化 —— warn 会丢失。

**解决方案**：让函数返回 `Result<(), String>`，main 失败时 `eprintln!`：

```rust
pub fn apply_self_mitigations() -> Vec<&'static str> {
    // 返回失败的策略名列表（空 = 全部成功）
    let mut failed = Vec::new();
    // ... 每次失败 failed.push("DEP") 等
    failed
}

// main.rs:
let failed = proc::security::self_mitigation::apply_self_mitigations();
if !failed.is_empty() {
    eprintln!("warning: self-mitigation policies failed: {}", failed.join(", "));
}
```

**改 ADR-0008**：阶段 2 完工后改 Status 为 `Accepted`。

**测试**：`tests/test_self_mitigation.rs`（新）：
```rust
#[test]
fn self_mitigations_apply_without_panic() {
    let failed = proc::security::self_mitigation::apply_self_mitigations();
    // Windows 上应该全部成功（Windows 10+）；Linux/macOS 返回空 Vec
    #[cfg(windows)]
    assert!(failed.is_empty(), "Mitigation policies failed: {:?}", failed);
}
```

---

### 任务 3：录屏前 secret 防护（项 #3，依赖任务 1）

**改 `src/app.rs`**：

加字段：
```rust
pub struct App {
    // ... 既有字段
    /// 录屏启动前是否处于"待确认"状态
    pub pending_record_confirm: bool,
}
```

改 `toggle_record`（原 `R` 大写开关录屏）：
```rust
pub fn toggle_record(&mut self) {
    if self.recording {
        // 停止录屏（原逻辑）
        self.stop_recording();
        return;
    }
    // 启动前弹确认（含 secret 提示）
    self.pending_record_confirm = true;
    self.status_message = "⚠ 录屏会捕获屏幕所有内容（含 DNS 域名 / 进程 cmd）。y 确认 / n 取消".into();
}
```

改 `handle_key`（全局 R 时拦截 y/n）：
```rust
if app.pending_record_confirm {
    match key.code {
        KeyCode::Char('y') => {
            app.pending_record_confirm = false;
            app.start_recording();
            app.status_message = "录屏中... (R 停止)".into();
        }
        KeyCode::Char('n') | KeyCode::Esc => {
            app.pending_record_confirm = false;
            app.status_message = "录屏已取消".into();
        }
        _ => {}  // 其他键吞掉，等用户选 y/n
    }
    return;  // 不传递给下层 panel
}
```

**改 `draw_env_tab`（任务 1 已说明）**：录屏中 `env_reveal` 强制 false（`reveal = app.env_reveal && !app.recording`）。

**测试**：`tests/test_record_protection.rs`（新）：
- 按 R → `pending_record_confirm = true`，未开始录屏
- 按 y → `recording = true`，`pending_record_confirm = false`
- 按 n → `recording = false`，`pending_record_confirm = false`
- 录屏中 `env_reveal=true` 但 `reveal` 计算结果为 false（draw_env_tab 行为）

---

### 任务 4：子进程权限剥离（项 #4）

**新模块**：`src/security/restricted_spawn.rs`（cfg-gate Windows）

```rust
//! 子进程权限剥离 — 见 CONTEXT.md / ADR-0007 后续工作。
//! elevated proc spawn PowerShell DNS / docker exec / nvtop 时，
//! 用 CreateRestrictedToken + DISABLE_MAX_PRIVILEGE 剥离继承的 SeDebugPrivilege。

#[cfg(windows)]
pub fn spawn_with_reduced_privileges(
    program: &str,
    args: &[&str],
) -> std::io::Result<std::process::Child> {
    use std::os::windows::io::AsRawHandle;
    use std::process::Stdio;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        CreateRestrictedToken, DISABLE_MAX_PRIVILEGE, TOKEN_DUPLICATE, TOKEN_QUERY,
        GetCurrentProcess, OpenProcessToken, TOKEN_ASSIGN_PRIMARY,
    };
    use windows::Win32::System::Threading::{
        CreateProcessW, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT,
        PROCESS_CREATION_FLAGS, STARTUPINFOW,
    };
    use windows::core::PCWSTR;
    
    unsafe {
        // 1. 打开自己的 token
        let mut own_token = HANDLE::default();
        let r = OpenProcessToken(GetCurrentProcess(), TOKEN_DUPLICATE, &mut own_token);
        if r.is_err() {
            return Err(std::io::Error::other("OpenProcessToken failed"));
        }
        
        // 2. CreateRestrictedToken(DISABLE_MAX_PRIVILEGE) — 剥离所有权限
        let mut restricted = HANDLE::default();
        let r = CreateRestrictedToken(
            own_token,
            DISABLE_MAX_PRIVILEGE,
            Some(&[]),
            None,
            Some(&[]),
            &mut restricted,
        );
        let _ = CloseHandle(own_token);
        if r.is_err() {
            return Err(std::io::Error::other("CreateRestrictedToken failed"));
        }
        
        // 3. 构造命令行
        let cmd_line: Vec<u16> = format!("{} {}", program, args.join(" "))
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        
        // 4. 构造 STARTUPINFO（继承 stdio）
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        // 注：实际生产中需要从 caller 传入 stdout/stderr/stdin 配置
        
        let mut pi = std::mem::zeroed();
        
        // 5. CreateProcessAsUserW 用 restricted token
        let r = CreateProcessW(
            PCWSTR::null(),                       // lpApplicationName
            PCWSTR(cmd_line.as_ptr()),            // lpCommandLine (mut)
            None, None,                           // security attrs
            true,                                 // bInheritHandles
            CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            None,                                 // environment
            PCWSTR::null(),                       // current dir
            &si,
            &mut pi,
        );
        let _ = CloseHandle(restricted);
        if r.is_err() {
            return Err(std::io::Error::other("CreateProcessW failed"));
        }
        
        // 6. 包装成 std::process::Child —— 需要从 pi.hProcess / pi.hThread 构造
        // 简化方案：直接返回一个最小 Child，让 caller 不感知；或者保留原 std::process::Command 路径
        // 但让 caller 能 wait + kill
        
        // 实际实现用 Child::from_handle (unstable) 或 std::os::windows::process::BuilderExt
        // 这个 API 复杂，阶段 2 先实现一个简化版本（见下方简化策略）
        
        unimplemented!("见下方阶段 2 简化策略")
    }
}

#[cfg(not(windows))]
pub fn spawn_with_reduced_privileges(
    program: &str,
    args: &[&str],
) -> std::io::Result<std::process::Child> {
    // Linux/macOS: 不继承父的 capabilities —— 默认 Command::new 已经不继承 sudo
    // 但如果父是 setuid 启动，子进程也会保留 —— 简化方案：spawn 前 setuid(getuid())
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    cmd.spawn()
}
```

**阶段 2 简化策略**：

完整 `CreateProcessAsUserW` 包装复杂（需要手动管理 STARTUPINFO / pipe inheritance / handle 构造 std::process::Child）。阶段 2 先做：

1. **完整实现 Windows 路径**（`spawn_with_reduced_privileges` 用 `CreateProcessW` 拿到 `pi.hProcess`，封装为一个轻量 `RestrictedChild` 提供 `wait / kill / take_stdout` 接口）
2. **只接入 PowerShell DNS 子进程**（最高危；最常见 elevated 场景）
3. **docker exec / nvtop 留 0.6.1+**（执行 docker exec 时 elevated 通常是必需的，不能盲目 drop；需要更细粒度的 token 控制）

**改 `src/dns_log/windows_dns.rs::PowershellDnsCollector::new`**：

把 `Command::new("powershell.exe").args([...]).spawn()` 改为：
```rust
let mut child = spawn_with_reduced_privileges(
    "powershell.exe",
    &["-NoProfile", "-NonInteractive", "-Command", POWERSHELL_SCRIPT],
    // stdout piped / stderr null / stdin null
)?;
```

或者更稳妥：保留原 `Command` API，加 `pre_exec` 钩子（unstable，需要 `feature(process_pre_exec)` 不存在；Windows 上用 `CommandExt::creation_flags` + 自定义 token 不直接支持）。

**实际可行的方案**：用 `windows` crate 提供 `CommandExt::raw_attribute` 或自己包装。看 [windows-rs example](https://github.com/microsoft/windows-rs)。

考虑到复杂度，**阶段 2 实际落地**可能是：
- 实现完整 `spawn_with_reduced_privileges`
- 用它替换 PowerShell DNS 子进程
- docker exec 暂时保留原 Command，加 README 说明 elevated 风险

**测试**：`tests/test_restricted_spawn.rs`（新）：
- 非 Windows：`spawn_with_reduced_privileges("echo", &["test"])` 能正常 spawn + 收 stdout
- Windows：spawn `whoami /priv` 子进程，stdout 应该不含 `SeDebugPrivilege`

---

### 任务 5：更新 CHANGELOG + CONTEXT.md

CHANGELOG.md 在 Unreleased 段追加：
```markdown
### 阶段 2 — 安全加固

- Added (#1): `src/inspect/env_mask.rs` 新模块 — `is_secret_key` + `mask_value`。`EnvVar` 加 `is_secret: bool` 字段，详情页 Env Tab 默认 mask（前 2 字符 + `***` + 原长），按 `v` 切换 `env_reveal`；录屏时强制 mask。20+ 单测覆盖常见 key 名 + 边界。
- Added (#2): `src/security/self_mitigation.rs` — `apply_self_mitigations()` 调 `SetProcessMitigationPolicy` 开 DEP(Permanent) + ASLR(HighEntropy) + ProhibitDynamicCode + DisableExtensionPoints + 禁远程 ImageLoad。`main.rs` 最早调用（早于 tracing init）；失败 `eprintln!` 不 panic。
- Added (#3): 录屏启动前确认弹窗 — `App::pending_record_confirm`，按 `R` 弹警告"会捕获屏幕所有内容含 DNS 域名 / 进程 cmd"，`y` 确认 / `n` 取消。
- Added (#4): `src/security/restricted_spawn.rs` — `spawn_with_reduced_privileges()` 用 `CreateRestrictedToken(DISABLE_MAX_PRIVILEGE)` 剥离子进程继承的 SeDebug。接入 PowerShell DNS 子进程（最高危）；docker exec / nvtop 留 0.6.1+。
- Changed: ADR-0008 Status 改 `Accepted`（阶段 2 落地后）。
- Docs: SECURITY.md 补充 hardening 说明（阶段 1 模板，本阶段填实）。
```

CONTEXT.md：术语演进历史段追加（实际新增的字段 / 模块）。

---

### 验收命令

```bash
cargo test --release --tb=no -q                       # 611 + 阶段 2 新增（预计 +30）= ~641
cargo clippy --release --all-targets -- -D warnings  # 0 warnings
cargo fmt --all -- --check
cargo build --release --no-default-features

# 阶段 2 特殊验证：
# 1. Windows: 启动 proc，用 Process Explorer 查看 proc.exe Mitigation 标志位
#    应该亮 DEP/ASLR/ProhibitDynamicCode/DisableExtensionPoints
# 2. 录屏测试: 启动 proc → 进详情页 → 按 v 切换 → 看到真值；再按 R 录屏 → 
#    按 v 不再切换（强制 mask）；录屏文件 replay → grep 'password\|token\|aws' = 0 命中
# 3. elevated 测试: 管理员启动 proc → DNS 子进程在 Process Explorer 中查 powershell.exe 
#    的 token → 应该不含 SeDebugPrivilege
```

**验收标准**：
- 全量回归通过（611 + 新增 ~30 = ~641）
- clippy / fmt / no-default-features 编译通过
- ADR-0008 Status 改 Accepted
- CHANGELOG.md Unreleased 段加阶段 2 内容
- 4 个新模块（env_mask / self_mitigation / restricted_spawn + 测试文件）入仓
- 真实 Windows 环境验证 Mitigation 标志位（如可用）
- 录屏 grep secret 0 命中

**容量预警**：本阶段代码量 ~830 行（含测试），未超 1500 上限，但 Win32 FFI 部分（restricted_spawn）调试可能耗时。如上下文消耗 > 600K，生成 Checkpoint 中断。

**主修改区域**：
- `src/inspect/{env_mask.rs(新), env.rs, mod.rs}`
- `src/security/{self_mitigation.rs(新), restricted_spawn.rs(新)}`
- `src/app.rs`（加 env_reveal / pending_record_confirm 字段 + handle 逻辑）
- `src/tui/detail_view.rs`（draw_env_tab 改 reveal 计算）
- `src/main.rs`（最早调 apply_self_mitigations）
- `src/dns_log/windows_dns.rs`（接入 restricted_spawn）
- `tests/test_env_mask.rs(新)` + `tests/test_self_mitigation.rs(新)` + `tests/test_record_protection.rs(新)` + `tests/test_restricted_spawn.rs(新)`
- `docs/adr/0008-self-mitigation-policy.md`（Status 改 Accepted）
- `CHANGELOG.md` / `CONTEXT.md` 更新
