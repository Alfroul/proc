# ADR-0021: Process Signature Verification — WinVerifyTrust + 6-state SignatureStatus

- **Status**: Accepted
- **Date**: 2026-06-30
- **Phase**: v0.11.0 阶段 4
- **Related**: [ADR-0008](0008-avoid-runtime-image-signing-policy.md)（self-mitigation 不开 `ProcessSignaturePolicy`）、缓存键加 start_time 防 PID 复用（代码层键控，证据见 [深挖导览条目①](../architecture-deep-dive.md)；v0.27 勘误：原引 ADR-0003 为幽灵引用——`0003-pid-reuse-start-time-key.md` 从未入库，现行 0003 是 smartctl 选型）

## 背景

Malware 常见特征之一是从 `%TEMP%` / `%LOCALAPPDATA%` 启动的**未签名 `.exe`**——合法应用几乎都有代码签名（微软签名 / 已知第三方 CA），未签名 .exe 是高置信度的可疑信号。v0.6.0 落地的 14 项 + v0.7 R15 = 15 项安全评分里**没有签名维度**，这是叙事缺口。

阶段 1 已经把字段骨架（`ProcessInfo.signature_status: SignatureStatus`，默认 `Pending`）和 `SignatureStatus` enum（v0.6.0 5 变体 + v0.11 `Pending` 共 6 变体）落地，但实际验证逻辑早就存在（`src/security/signature.rs` 的 `verify_signature` 在 v0.6.0 阶段 2 已实装）。本 ADR 记录阶段 4 把这块**单元化 + 测试可注入 + UI 显示**的决策。

## 选项

| 方案 | 优点 | 缺点 |
|---|---|---|
| **A. `sigcheck.exe`（Sysinternals）** | 微软官方工具，输出解析简单 | 外部依赖（用户必须装 Sysinternals），spawn 子进程 ~100-300ms |
| **B. `Get-AuthenticodeSignature` PowerShell** | 内置可用 | spawn 子进程慢（PowerShell 启动开销 ~300ms），高频调用不可行 |
| **C. `wintrust` crate** | 纯 Rust 调用 | 最后 release 2019，`windows-rs` 已覆盖同款 API，重复依赖 |
| **D. `rust-pkcs11` / 纯 Rust PKCS#7 解析** | 不依赖 native API | PKCS#11 是智能卡 / HSM 方向；自写 PKCS#7 验证要处理 x.509 链 + CRL + 时间戳，工程量数千行 |
| **E. `WinVerifyTrust` via `windows-rs`** | 微软官方 API（OS 自带），cat 文件 / 嵌套签名 / 时间戳都覆盖，与项目其他 windows-rs 集成一致 | 单次调用 10-50ms（含 OCSP/CRL 网络 check），必须异步 |

## 决策

**选 E**。具体设计：

### 1. `SignatureStatus` 6 状态机（非 stage-4.md 原方案的 4 状态）

`stage-4.md` 早期设计是 `Pending / Trusted / Untrusted / Unknown` 4 状态。实际落地扩到 **6 变体**，因为 v0.6.0 阶段 2 已经有 `Signed / Trusted / Unsigned / Revoked / Unknown` 5 变体（且已写入 RiskFactor 评分映射），强行收敛到 4 变体会让 v0.6 的评分分支失去区分度。

最终枚举：

| 变体 | 语义 | R16 评分（weight） |
|---|---|---|
| `Pending` | `#[default]`，尚未触发验证（ProcessInfo 初始值） | 不扣分（启动后头 1-2 个 heavy refresh 内全部 Pending，扣分会让所有进程瞬间变红） |
| `Trusted` | 签名链追溯到微软根 CA 或已知第三方 CA（DigiCert / Sectigo / Google / Mozilla / Apple / Intel / NVIDIA 等） | 不扣分 |
| `Signed` | 签名有效但不在 `TRUSTED_SIGNERS` 列表（小厂签名 / 自签名 CA） | 扣 10 分 |
| `Unsigned` | `WinVerifyTrust` 返回 `TRUST_E_SUBJECT_NOT_SIGNED` | 扣 20 分 |
| `Revoked` | `WinVerifyTrust` 返回 `CRYPT_E_REVOKED`（签名证书被 CA 吊销） | 扣 35 分（最严重） |
| `Unknown` | API 调用失败 / 链断裂 / 过期 / 非管理员 | 扣 5 分（轻扣，保留可观察性） |

`stage-4.md` 原方案的「Untrusted → 扣 25 分」拆成 `Signed (10) / Unsigned (20) / Revoked (35)` 三档，更细粒度地反映风险：未签名不必然恶意（20），但被吊销一定有问题（35）。

### 2. `verify_signature_with_policy` 测试可注入

`stage-4.md` 任务 3 要求暴露内部函数供单元测试 mock。抽出 `pub(crate) fn verify_signature_with_policy(exe_path: &str, policy_override: Option<i32>) -> SignatureStatus`：

- `policy_override = None`：真实路径，调 `WinVerifyTrust`
- `policy_override = Some(hresult)`：mock 路径，直接走 `from_wintrust_result(hresult)` 不读文件——**跨平台**都能跑（非 Windows 上也能验证状态机）

`from_wintrust_result(result: i32) -> SignatureStatus` 是 pure function 把 HRESULT 映射到状态：

```text
0                              → Signed（Trusted 升级由调用方基于 CompanyName 决定）
TRUST_E_SUBJECT_NOT_SIGNED     → Unsigned
CRYPT_E_REVOKED                → Revoked
其他                           → Unknown
```

### 3. `HashReputation` LRU 缓存（替代 stage-4.md 原方案的 `SignatureCache`）

`stage-4.md` 原方案设计 `SignatureCache { HashMap<PathBuf, (SignatureStatus, SystemTime)> }`，1000 条 + 1h TTL + 路径键。**实际不重新实装**——v0.6.0 已有的 `src/security/hash_cache.rs::HashReputation` 是更先进的版本：

- **键：SHA-256 内容寻址**（路径变 / 文件改内容自动失效），而非路径
- **持久化**：`%APPDATA%/proc/sig_cache.json`，重启后复用
- **LRU + 2000 条上限**：超出按 `first_seen_epoch` 淘汰
- **MAX_HASH_BYTES = 64 MB**：避免大文件 OOM

重新实装会与现有 `HashReputation` 形成两套缓存路径，浪费且数据不一致。ADR-0021 明确：**签名缓存统一走 `HashReputation`**，stage-4.md 原方案的 `SignatureCache` 不落地。

### 4. `SecurityScorer::score` 第 1 步接入（非 stage-4.md 原方案的第 16 步 R16）

`stage-4.md` 任务 6 设计 R16 作为第 16 步（v0.6 14 项 + v0.7 R15 + v0.11 R16 = 16 项）。但 v0.6.0 阶段 2 已经把签名验证作为 `score` 函数**第 1 步**接入（cache-friendly：先查 `HashReputation.get_cached_sig`，未命中且 budget 够才调 `WinVerifyTrust`），并把结果用于：

- 步骤 1 自身（`signature_risk_factor`）
- 步骤 5（`check_network_behavior`：未签名进程监听端口额外扣分）
- 步骤 14（`HashReputation.check_hash`：已知无签名的 .exe 命中扣分）

强行降级为「第 16 步」会破坏步骤间的数据流。**ADR-0021 决策：保留第 1 步接入**，签名验证既是 R16 本身（5 档扣分），也是后续步骤的输入。`stage-4.md` 任务 6 描述的「R16 第 16 步」是设计草稿，实际落地优于原方案。

### 5. BackgroundScorer 异步集成（已在 v0.6.0 落地）

`SecurityScorer::score` 由 `BackgroundScorer`（`src/security/score.rs`）在 `security-scorer` 工作线程跑：

- 主线程 `App::tick_heavy` 把 `cached_processes` clone 到 `Arc`，通过 `BackgroundScorer::request` 发送
- 工作线程每个进程调 `score`，结果通过 `mpsc::channel` 回传
- 主线程 `App::tick_heavy` 末尾 `poll_results` 拿到 `HashMap<u32, SecurityScore>`，写入 `security_scores`

**v0.11 阶段 4 新增的反向同步**：poll 后把 `score.signature` 写回 `cached_processes[*].signature_status`，让 UI 显示最新结果而非 ProcessInfo 默认值 `Pending`（`src/app.rs::tick_heavy` poll 段）。

**VERIFY_BUDGET_PER_PASS = 50**：每 pass 最多调 50 次 `WinVerifyTrust`（每次 10-50ms 含 OCSP/CRL 网络 check），避免单 pass 跑数分钟。多 pass 内会覆盖完整进程列表（顺序由 `cached_processes` 决定，随 PID/compute 顺序自然轮换）。

### 6. UI 显示

- **进程列表**（`src/tui/process_table.rs`）：name 后追加 emoji
  - `Trusted` → `🔒`
  - `Unsigned | Revoked` → `⚠️`
  - `Unknown` → `❓`
  - `Pending | Signed` → 空串（不渲染占位，避免列宽波动；与 v0.7 EcoQoS `🍃` 同款规则）
- **Inspector Summary**（`src/tui/detail_view.rs`）：显示 `ProcessInfo.signature_status`（最新值，非 score 快照），`Trusted` 时附加「(微软/已知 CA)」标注

## Consequences

- **正向**：
  - 6 状态机比 4 状态机提供更细粒度的扣分（Unsigned 20 vs Revoked 35 vs Unknown 5）
  - `verify_signature_with_policy` 抽出后，mock 路径跨平台可测（非 Windows CI 也能验证状态机）
  - `HashReputation` SHA-256 内容寻址 + 持久化让缓存命中率远高于路径键方案
  - BackgroundScorer 异步路径保证主线程 50ms tick 不被阻塞

- **负向 / 已知限制**：
  - **cat 文件签名**（`.cat`，驱动 + 系统组件走这条路）当前不区分——`WinVerifyTrust` 对 cat 签名的 .exe 会返 `TRUST_E_SUBJECT_NOT_SIGNED`，但实际 OS 信任它（通过 cat 关联）。本周期不区分，留 TD。
  - **内核态签名**（驱动签名）走不同 policy（`DriverSigningPolicy`），本周期不区分。
  - **`Pending` 持续时间**：启动后头 1-2 个 heavy refresh（~3s）内全部进程显示 `Pending`（不扣分 + 不显 emoji），用户可能误以为功能未生效。考虑 v0.12 给 `Pending` 加 `⏳` emoji 提示「正在验证」（stage-4.md 原方案就有这个 emoji，但落地后觉得启动期短暂，没必要）。
  - **非 elevated**：Windows 上 `is_elevated() == false` 时 `verify_signature` 直接返 `Unknown`（不调 `WinVerifyTrust`），所有进程扣 5 分。这是已知行为——非管理员运行 proc 时签名维度降级，符合「能力降级而非误报」原则。
  - **OCSP/CRL 延迟**：`WinVerifyTrust` 在网络不通时可能阻塞数十秒（OCSP 超时）。`VERIFY_BUDGET_PER_PASS = 50` 限制了单 pass 影响，但极端情况单 pass 仍可能跑数分钟。考虑 v0.12 加 `WTD_REVOCATION_CHECK_NONE` 选项供离线环境使用。

## Alternatives 落地说明

`stage-4.md` 任务清单第 1 项列的 4 个 alternatives（sigcheck / Get-AuthenticodeSignature / wintrust crate / rust-pkcs11）均不在本周期落地，作为决策记录保留。如果未来 `windows-rs` 弃用 `WinVerifyTrust`，备选方案是 D（自写 PKCS#7 验证），但工程量数千行，不优先。

## 测试覆盖

- `tests/test_signature.rs`（24 case）：`from_wintrust_result` 状态机 / `signature_risk_factor` 全状态 / `badge` emoji / serde round-trip + 缺字段默认 Pending / `is_trusted_signer` 已知 CA / 非 Windows stub
- `src/security/signature.rs::tests`（5 case）：`verify_signature_with_policy` mock policy 注入路径（pub(crate) 不暴露给集成测试 crate）

## 演进历史

- **v0.6.0 阶段 2**：`SignatureStatus` 5 变体（无 `Pending`）+ `verify_signature` 实装 + `HashReputation` 缓存 + `SecurityScorer::score` 第 1 步接入
- **v0.11.0 阶段 1**：加 `Pending` 变体作为 `#[default]`；`ProcessInfo.signature_status` 字段骨架（`#[serde(default)]`）
- **v0.11.0 阶段 4**（本 ADR）：`from_wintrust_result` 抽出 + `verify_signature_with_policy` 测试可注入 + UI 显示 emoji + Inspector 升级 + App 反向同步字段 + 测试套件
