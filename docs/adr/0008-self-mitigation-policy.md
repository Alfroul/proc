# ADR-0008: 进程自我加固策略（DEP+ASLR+DynamicCode+ExtPoint，不开 Signature）

- **Status**: Proposed（待阶段 2 落地后改 Accepted）
- **Date**: 2026-06-24
- **Phase**: v0.6.0 阶段 2

## 背景

proc 是 Windows 上要 `OpenProcess(PROCESS_VM_READ)` / `ReadProcessMemory` / 句柄枚举的工具。elevated 模式下持 `SeDebugPrivilege`，能读写任何进程内存。如果攻击者通过任何漏洞（ratatui 渲染恶意字符触发的 bug、bincode 反序列化漏洞、`vt100::Parser::process` 接受任意字节流）拿到 RCE，proc 立刻变成 credential theft 的跳板 —— 攻击者可借 proc 读 lsass.exe / 浏览器密码存储 / SSH agent。

需要选择自我加固（self-mitigation）策略，让 proc 自身在被注入 / 漏洞利用时尽可能难被滥用。

## 选项

Windows `SetProcessMitigationPolicy` 提供以下策略：

| 策略 | 作用 | 优点 | 缺点 |
|---|---|---|---|
| `ProcessDEPPolicy` (Permanent) | Data Execution Prevention 永久开启，不可逆 | 阻止栈 / 堆执行 | 现代 Windows 默认开启，显式声明只是锁死 |
| `ProcessASLRPolicy` (HighEntropy) | 地址空间随机化用高熵 | 降低 ROP 链成功率 | 与 nvml-wrapper / portable-pty 兼容 |
| `ProcessDynamicCodePolicy` (Prohibit) | **禁止动态代码生成**（JIT） | 阻止常见 shellcode 注入 | **会破坏依赖 JIT 的依赖**（proc 当前无 JIT 依赖：tokio / ratatui / sysinfo / bollard 都是 AOT） |
| `ProcessExtensionPointDisablePolicy` | 禁 AppInit_DLLs / AppVerifier / 全局 hooks | 阻止经典 DLL 注入 | 与 nvml-wrapper 兼容（其 native lib 是显式 load） |
| `ProcessSignaturePolicy` (Allow + Microsoft + Authenticode) | **仅允许已签名模块加载到 proc 自身** | 最强保护 | **nvml-wrapper native 依赖（nvml.dll）若未签名会让启动直接挂**；portable-pty ConPTY 也可能受影响 |
| `ProcessFontDisablePolicy` | 禁字体加载 | 减少攻击面 | 与渲染无关，proc 不需要 |
| `ProcessImageLoadPolicy` | 禁从 UNC / 远程路径加载 DLL | 阻止网络投递 | 与本地相对路径加载兼容 |
| `ProcessSystemCallDisablePolicy` | 禁 Win32k 系统调用 | 减少内核攻击面 | 可能影响某些 UI 调用 |

## 决策

**v0.6.0 阶段 2 开启**：DEP (Permanent) + ASLR + DynamicCode + ExtensionPointDisable + ImageLoad（远程路径）

**v0.6.0 不开**（留 ADR-0009+ 评估）：
- `ProcessSignaturePolicy`：会让 nvml-wrapper / portable-pty native 依赖加载失败，需先验证所有 native 依赖签名状态
- `ProcessSystemCallDisablePolicy`：需测试 ratatui / crossterm 是否依赖 Win32k

**配置形态**：
```rust
// src/security/self_mitigation.rs
pub fn apply_self_mitigations() {
    // Win32 SetProcessMitigationPolicy 调用，失败时 tracing::warn! 但不 panic
    // （启动健壮性 > 完美加固；用户机器可能有不同支持程度）
}
```

调用时机：`src/main.rs::main` **第一行**（早于 init_tracing / 任何 worker 启动）。

理由：

1. **不开 Signature 是兼容性取舍**：nvml-wrapper 是 proc 核心依赖（GPU 监控），其加载的 nvml.dll 签名状态不在我们控制下；强制签名可能让 0.5.0 已工作的 NVIDIA 路径挂掉
2. **DEP+ASLR+DynamicCode+ExtPoint+ImageLoad 已挡住 80% 的注入路径**：栈执行 / 堆执行 / JIT shellcode / AppInit 注入 / UNC 网络投递
3. **启动健壮性优先**：策略失败 `tracing::warn!` 不 panic（用户机器可能是 Server Core / 老 Windows 10，部分策略不支持）
4. **不破坏既有功能**：tokio / ratatui / sysinfo / bollard / nvml-wrapper / portable-pty / vt100 都是 AOT 编译，不依赖动态代码生成

## 后果

### 正面

- 进程被注入后无法执行动态生成的 shellcode（最常见的攻击路径被切断）
- AppInit_DLLs / 全局 hooks 注入失效
- 网络投递（远程 UNC DLL 加载）失效

### 负面 / 已知限制

- 不开 Signature Policy：未签名模块仍可加载（nvml-wrapper / portable-pty 兼容性权衡）
- DynamicCode 永久不可逆：未来如引入 JIT 依赖（如 pyo3 / deno_core）会失败
- 启动时如某策略失败：`tracing::warn!` 但继续启动（不是 fail-fast）

### 后续工作（v0.7.0+）

- ADR-0009+ 评估 `ProcessSignaturePolicy` 可行性：审计所有 native 依赖签名状态（nvml.dll / ConPTY 相关 / windows.dll）
- Linux 等价物（如有）：`prctl(PR_SET_NO_NEW_PRIVS)` + `seccomp` 过滤系统调用

## 验证

- 阶段 2 验收：管理员启动 proc → `Process Explorer / System Informer` 查看 proc.exe 的 Mitigation 标志位应亮 DEP/ASLR/ProhibitDynamicCode/DisableExtensionPoints
- 阶段 2 集成测试：`apply_self_mitigations()` 在 Windows 上不 panic；Linux 上 cfg-gate 返回 Ok

## 参考

- Microsoft docs: [SetProcessMitigationPolicy](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setprocessmitigationpolicy)
- 同领域参考：Microsoft Edge / Chrome / VSCode 都开启了 ProcessSignaturePolicy（但它们控制所有 native 依赖）
- v0.6.0 阶段 2 落地：plan.md / docs/stages/stage-2.md
- 相关代码：`src/security/self_mitigation.rs`（阶段 2 新增）
