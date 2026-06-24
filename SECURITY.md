# Security Policy

## Supported Versions

仅最新 release 接收安全修复。老版本不维护，请升级到最新 release。

| 版本 | 支持状态 |
| --- | --- |
| 最新 release（`master` 分支 tag `v*`） | ✅ 接收安全修复 |
| 老版本 / 未发布分支 | ❌ 不维护 |

## Reporting a Vulnerability

**请勿在公开 issue 中提交安全漏洞。**

- 邮箱：`security@alfroul.example`（**占位，发布前替换为真实邮箱**）
- 响应承诺：72 小时内确认收到，7 天内给出修复计划
- 修复后会发布 patch release + 在 CHANGELOG 中说明（不主动披露利用细节，等用户有时间升级）

如果漏洞涉及提权 / RCE / 数据泄漏，请附带复现步骤 + 受影响版本 + 影响评估。

## Privilege Model

**默认（非管理员）**：
- 仅读自己有权限访问的进程信息（sysinfo 走标准 process snapshot）
- 句柄枚举 / `ReadProcessMemory` 等操作对受保护进程会失败，自动降级

**Elevated（管理员）**：
- 持 `SeDebugPrivilege`，可枚举所有进程的句柄 / 内存 / 模块
- 进程级带宽监控（`GetPerTcpConnectionEStats`）需要 elevated
- v0.6.0 阶段 2 起：elevated 时 spawn 子进程（PowerShell DNS / docker exec / nvtop）会调 `CreateRestrictedToken` 剥离继承的 `SeDebugPrivilege`，避免子进程被滥用为 credential theft 跳板

## Hardening（v0.6.0 阶段 2 起）

`proc` 启动时最早调用 `apply_self_mitigations()`，通过 `SetProcessMitigationPolicy` 给自己上以下保护：

| 策略 | 作用 |
| --- | --- |
| `ProcessDEPPolicy`（Permanent） | 数据执行防护永久开启，不可逆 |
| `ProcessASLRPolicy`（HighEntropy） | 高熵地址空间随机化 |
| `ProcessDynamicCodePolicy`（Prohibit） | 禁止动态代码生成（挡 JIT shellcode） |
| `ProcessExtensionPointDisablePolicy` | 禁 AppInit_DLLs / 全局 hooks |

策略调用失败时 `tracing::warn!` 但**不 panic**（用户机器可能是 Server Core 或老版 Windows，部分策略不支持）。

详见 `docs/adr/0008-self-mitigation-policy.md`。

## 已知限制

- **未启用 `ProcessSignaturePolicy`**：会让 `nvml-wrapper` 等 native 依赖（NVML 库）因签名检查加载失败。等 ADR-0009+ 评估所有 native 依赖签名状态后再考虑。
- **`ProcessSystemCallDisablePolicy` 未启用**：需测试 ratatui / crossterm 是否依赖 Win32k 系统调用。
- Linux 平台暂无等价 self-mitigation（未来 v0.7.0+ 评估 `prctl(PR_SET_NO_NEW_PRIVS)` + seccomp）。
- 进程注入防御是降低概率，不是绝对防护。如 proc 在 elevated 模式下被攻陷，仍可能成为攻击跳板（受限于上述 4 项策略外的攻击面）。

## 隐私承诺

- **DNS 查询日志**：仅在内存中保留最近 1000 条，**永不持久化**到磁盘。退出 proc 即丢失。
- **环境变量**：详情页 Env Tab 默认 mask 显示（`{前2字符}***(原长B)`），录屏时强制 mask 模式。
- **录屏**（VT100 recording）：用户主动按 `R` 触发，触发时先弹确认对话框（警告会捕获屏幕所有内容含 DNS 域名 / 进程 cmd），按 `y` 确认。
- **崩溃报告**：写到 `~/.config/proc/crashes/crash-{timestamp}.txt`，含 panic info + backtrace，**不上传任何位置**。用户报 bug 时手动附上。

## 参考

- GitHub 推荐 SECURITY.md 模板：https://docs.github.com/en/code-security/getting-started/adding-a-security-policy-to-your-repository
- Microsoft docs：[SetProcessMitigationPolicy](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setprocessmitigationpolicy)
- ADR-0008：自我加固策略选型（含未启 Signature Policy 的理由）
