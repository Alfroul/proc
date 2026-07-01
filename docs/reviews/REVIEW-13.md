# 全局 Review 报告 — v0.11.0 cycle（阶段 7）

**审查范围**：v0.11.0 cycle 阶段 1-6 全部产出
**审查日期**：2026-06-30
**审查人**：stage 7 会话
**基线测试**：`cargo test --release -q` = **1141 passed / 0 failed / 3 ignored**（v0.10.0 基线 959 → +182：v0.11 阶段 1-6 新增 6 个 test_* 文件共 89 case + 全量回归累计 +93）
**其它基线**：
- `cargo fmt --all -- --check` 干净 ✓（无输出）
- `cargo clippy --release --all-targets -- -D warnings` 0 warnings ✓
- `cargo build --release --no-default-features` 通过 ✓（2m09s，验证 cfg-gate）

---

## 摘要

- 总问题数：**P0 0 / P1 4 / P2 15**
- 阻断性问题：**0 项**（无 P0；基线三件套 + no-default-features build 全通过）
- 关键主题：**跨 FFI panic UB 风险**（DNS ETW callback）+ **跨平台评分一致性**（非 Windows 上所有进程因 Unknown 扣 5 分）+ **FilterExpr UX 反直觉**（process 字段在 Flow 视图静默无输出）+ **文档完整性缺口**（6 个 stage docs 头部缺 ✅ 标记）

**整体评价**：v0.11 cycle 6 个阶段全部交付，新增 89 个集成测试 + 数十个模块内嵌单元测试，4 个 ADR（0019/0020/0021 + 0011 增量段）文档完整，CONTEXT.md 术语 + 演进历史同步落地。架构层面 4 个新模块（restart.rs / etw.rs / lineage.rs / path_rules.rs）符合「手写 FFI + surgical 不替换 v0.6/v0.7 检查」原则。基线三件套全过，无回归。本周期发现的 P1 问题集中在 UX / 跨平台一致性 / 文档完整性，不影响核心功能。

---

## P0（阻断性，必须修才能交付 v0.11.0）

无。基线三件套全通过，跨平台编译干净，无逻辑错误导致功能不可用。

---

## P1（重要，影响质量，建议本周期或下周期修复）

### P1-1：DNS ETW callback 内 panic 跨 FFI 边界 UB 风险

**位置**：`src/dns_log/etw.rs:358-379`（`dns_event_callback`）

**现状**：

`dns_event_callback` 是 `unsafe extern "system" fn`，被 ETW 在 ProcessTrace 线程触发。callback 内调用 `parse_dns_via_tdh`（safe 函数，但仍可能因 slice 索引 / 整数溢出 panic）+ `accum.lock().expect("dns accum poisoned")`（Mutex poison 时 panic）。

```rust
unsafe extern "system" fn dns_event_callback(record: *mut EVENT_RECORD) {
    ...
    let accum_opt = CALLBACK_ACCUM.with(|cell| cell.borrow().clone());
    if let Some(accum) = accum_opt {
        let mut acc = accum.lock().expect("dns accum poisoned");  // ← panic here = UB
        acc.push(query);
    }
}
```

Rust 标准库语义：跨 FFI 边界（`extern "system"` callback）unwind panic 是 **undefined behavior**。windows-rs 的 ProcessTrace 实现不保证能捕获 Rust panic——实测大多数情况下进程会 abort，但 UB 是不可预测的（可能继续跑但状态损坏）。

**实际触发概率**：

低。`acc.push(query)` 不会 panic（Vec push 仅 OOM 时 panic，且 callback 是单线程访问 accum）；`parse_dns_via_tdh` 内部 slice 索引已用 `start.min/end.min` saturate（line 418-419），整数运算用 `saturating_add`（line 448）。但 Mutex poison 是真实风险：drain 端（worker 线程）持锁时如果 panic（如 sysinfo call 失败），accum 锁会 poison，下一次 callback lock().expect() panic → UB。

**为什么 P1（不 P0 / 不 P2）**：

- 不阻断编译 / 测试 / 启动（P0 标准）。
- 实际触发概率低（drain 端 panic 也少见），但一旦触发是 UB。
- 与项目 v0.6.0 阶段 3 catch_unwind 包 worker body 的设计原则一致——worker panic 不应跨 FFI 边界。

**建议修复方式**：

在 callback 内用 `std::panic::catch_unwind` 包裹可能 panic 的代码：

```rust
unsafe extern "system" fn dns_event_callback(record: *mut EVENT_RECORD) {
    if record.is_null() { return; }
    let record = unsafe { &*record };
    let event_id = record.EventHeader.EventDescriptor.Id;
    if event_id != DNS_EVENT_QUERY_RESPONSE && event_id != DNS_EVENT_QUERY_COMPLETED {
        return;
    }
    // catch_unwind 包裹：parse + push 任何 panic 都被吞掉（best-effort drop event），
    // 避免 panic 跨 FFI 边界 UB。与 worker.rs::SnapshotWorker::spawn catch_unwind 同款。
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let Some(query) = parse_dns_via_tdh(record) else { return };
        let accum_opt = CALLBACK_ACCUM.with(|cell| cell.borrow().clone());
        if let Some(accum) = accum_opt {
            if let Ok(mut acc) = accum.lock() {  // ← 不用 expect，poison 时静默丢
                acc.push(query);
            }
        }
    }));
}
```

同时把 `accum.lock().expect(...)` 改为 `if let Ok(mut acc) = accum.lock()`，避免 poison 时 panic。schannel_etw / disk_io_etw 的 callback 也应同款审查（可能存在同款问题，但本 cycle 不动这两个模块）。

**修复优先级**：建议阶段 8 落地（与 v0.6 catch_unwind 原则一致）。

---

### P1-2：CLI `proc flows --filter 'cpu > 5'` 静默无输出（process 字段在 Flow 视图永远 false）

**位置**：`src/cli/flows.rs:53-57` + `src/filter/mod.rs:285`（`FilterExpr::apply_network` 对 process 变体返 false）

**现状**：

`proc flows --filter` 走 `FilterExpr::apply_network`，该方法对 process 字段变体（`FieldCmp` / `Regex`）返 false。用户写 `proc flows --filter 'cpu > 5'`：

1. parser 成功（`cpu` 是合法 Process 字段）。
2. `apply_network` 对所有 flow 返 false（FieldCmp 在 network ctx 下 false）。
3. `flows.retain(|f| ...)` 把所有 flow 过滤掉。
4. 输出「当前暂无活跃 flow」。

用户困惑：filter 看起来语法正确，但所有 flow 消失。无任何 warn 提示。

**同类影响**：

- TUI Flow 子视图（`:` 激活 FilterExpr）写 `cpu > 5` 也是同款静默无输出。
- `proc flows --filter 'name = chrome'`（process 字段）也静默无输出。
- `proc flows --filter 'pid > 100'`（process 字段）也静默无输出。

**为什么 P1（不 P0 / 不 P2）**：

- 不阻断编译 / 测试 / 启动（P0 标准）。
- 不影响数据正确性（filter 语义上正确：process 字段在 flow 视图无意义）。
- 严重影响 UX：用户无法理解为什么 filter 不工作，需要读 ADR-0011 才知道 process / network 字段分离。这是 stage-3.md 明确的 surgical 选择（类型系统保证不跨 ctx 误用），但缺用户反馈。

**建议修复方式**（两选一）：

**方案 A（推荐）**：parse 成功后检查 AST 是否含 process 字段变体，是则打印 warn + 退出 1：

```rust
// src/cli/flows.rs run_flows 内
if let Some(expr_str) = filter {
    match crate::filter::parse(expr_str) {
        Ok(expr) => {
            // v0.11 阶段 7 REVIEW-13 P1-2：检测纯 process 字段表达式
            if expr.contains_process_field() {
                eprintln!(
                    "{} filter 表达式只含 process 字段（cpu/mem/name/...），\
                    在 Flow 视图下永远不命中。Flow 字段：sni/dns_name/remote_addr/\
                    remote_port/bytes_out/bytes_in/source。详见 ADR-0011。",
                    "提示:".yellow()
                );
                std::process::exit(1);
            }
            flows.retain(|f| {
                let ctx = crate::filter::NetworkEvalCtx { flow: f };
                expr.apply_network(&ctx)
            });
        }
        Err(e) => { ... }
    }
}
```

需在 `src/filter/mod.rs::FilterExpr` 加 `contains_process_field(&self) -> bool` 方法递归检测。

**方案 B**：parse 阶段拒绝 process 字段（更严格，但破坏 surgical 原则——TUI List 视图仍需 process 字段）。不推荐。

**修复优先级**：建议阶段 8 落地。

---

### P1-3：非 Windows 平台所有进程因 SignatureStatus::Unknown 扣 5 分

**位置**：`src/security/signature.rs:163-172`（`verify_signature_with_policy` 非 Windows stub）+ `src/security/signature.rs:258-263`（`signature_risk_factor(Unknown)` 扣 5）

**现状**：

非 Windows 上 `verify_signature_with_policy(path, None)` 永远返 `Unknown`（line 171）。BackgroundScorer 对每个进程调 `verify_signature`，结果存入 `SecurityScore.signature`。`signature_risk_factor(Unknown)` 返回扣 5 分的 RiskFactor（"签名未验证（需管理员权限）"）。

```rust
#[cfg(not(target_os = "windows"))]
{
    if let Some(result) = policy_override {
        return from_wintrust_result(result);
    }
    let _ = exe_path;
    SignatureStatus::Unknown  // ← Linux/macOS 所有进程都返这个
}
```

**后果**：

Linux / macOS 用户运行 proc 时，**所有进程都扣 5 分**（signature_unverified）。典型 workstation 200+ 进程全部命中。用户视角：「为什么我的 Linux 上所有进程都被标红？」

ADR-0021 §Consequences 负面项明确「非 elevated 时 Windows 上 verify_signature 直接返 Unknown，所有进程扣 5 分。这是已知行为——非管理员运行 proc 时签名维度降级」。**但 ADR-0021 未明确 Linux/macOS 上的同款降级行为**——这是 P1 文档 + 实装缺口。

**为什么 P1（不 P0 / 不 P2）**：

- 不阻断编译 / 测试 / 启动（P0 标准）。
- 不影响数据正确性（Unknown 扣分本身是 ADR-0021 设计意图）。
- 影响跨平台用户体验：Linux/macOS 用户所有进程被扣 5 分，与 Windows 非管理员体验一致但语义不同（Linux 根本没有 WinVerifyTrust 概念）。
- stage-7.md 任务清单第 5 项「跨平台审查」明确要求审查 cfg-gate 正确性。

**建议修复方式**（两选一）：

**方案 A（推荐）**：在 `signature_risk_factor` 内 cfg-gate，非 Windows 上 Unknown 不扣分：

```rust
#[must_use]
pub fn signature_risk_factor(status: SignatureStatus) -> Option<RiskFactor> {
    match status {
        SignatureStatus::Pending => None,
        SignatureStatus::Unsigned => Some(RiskFactor { ... weight: 20, ... }),
        SignatureStatus::Revoked => Some(RiskFactor { ... weight: 35, ... }),
        SignatureStatus::Signed => Some(RiskFactor { ... weight: 10, ... }),
        SignatureStatus::Unknown => {
            // v0.11 阶段 7 REVIEW-13 P1-3：非 Windows 上没有 WinVerifyTrust，
            // Unknown 不应扣分（Linux/macOS 用户所有进程都返 Unknown）。
            // Windows 上 Unknown 仍扣 5 分（非管理员降级行为，ADR-0021 设计）。
            #[cfg(not(target_os = "windows"))]
            { return None; }
            #[cfg(target_os = "windows")]
            { Some(RiskFactor { ... weight: 5, ... }) }
        }
        _ => None,
    }
}
```

**方案 B**：在 `verify_signature_with_policy` 非 Windows stub 直接返 `Pending`（不扣分状态）。但这破坏语义——Pending 表示「尚未触发验证」，实际是无法验证（Unknown）。

**修复优先级**：建议阶段 8 落地。

---

### P1-4：6 个 stage docs（v0.11-stage-1~6）头部缺 ✅ 已发布标记

**位置**：`docs/stages/v0.11-stage-1.md` ~ `docs/stages/v0.11-stage-6.md`

**现状**：

stage-7.md 任务清单第 7 项「完整性检查」明确要求「4 个 stage docs（v0.11-stage-1~6）头部 ✅ 已发布标记」（注：stage-7.md 字面写「4 个」实际应为 6 个，是 stage-7.md 自身笔误）。但 6 个 stage docs 头部都没有 ✅ 标记：

```
=== docs/stages/v0.11-stage-1.md ===
### 阶段 1：Worker Restart（TD-4）+ ProcessInfo 字段骨架

> **独立会话指令**：`阅读 CONTEXT.md 和 docs/stages/v0.11-stage-1.md，完成所有任务后确认完成`
```

对比 v0.10.0 cycle 的 stage docs（应在头部有 ✅ 标记，但实际也未见——可能是项目惯例变化）。

**为什么 P1（不 P0 / 不 P2）**：

- 不影响代码功能。
- 影响下一阶段会话判断进度：阶段 8 会话看到 6 个 stage docs 头部无 ✅ 标记，需要读全文确认是否完成，浪费 context。
- stage-7.md 任务清单明确要求，未落地是 stage 6 收尾遗漏。

**建议修复方式**：

在每个 stage doc 标题行后追加一行 `> ✅ **已完成于 stage N 会话（commit XXXXX）**` 标记。例：

```markdown
### 阶段 1：Worker Restart（TD-4）+ ProcessInfo 字段骨架

> ✅ **已完成**（阶段 1 会话产出，commit 引用见 CONTEXT.md「术语演进历史」段）

> **独立会话指令**：`阅读 CONTEXT.md 和 docs/stages/v0.11-stage-1.md，完成所有任务后确认完成`
```

同时修正 stage-7.md 第 56 行「4 个 stage docs（v0.11-stage-1~6）」为「6 个 stage docs（v0.11-stage-1~6）」。

**修复优先级**：建议阶段 8 落地（与 Cargo.toml bump / CHANGELOG 同期收尾）。

---

## P2（建议，长期改善，归档到 tech-debt）

### P2-1：DNS ETW diag JSON 输出不含 dns_collector 字段

**位置**：`src/cli/diag.rs:54`（human-readable 模式有 dns_collector 行，JSON 模式无）

**影响**：用户用 `proc diag --json` 报 bug 时附上的 JSON 缺 collector 类型信息。JSON 是 bug report 的主要格式，工程化场景几乎都用 JSON。

**修复**：在 JSON 模式输出 object 中加 `"dns_collector": "etw" | "powershell" | "none"` 字段。

---

### P2-2：worker restart spawn_one 失败时 retry_count 不增加，导致无法到达 permanent_failure

**位置**：`src/workers/manager.rs:215-220`（`try_respawn` 在 spawn_one 返 false 时仅清空 state.last_crash？实际上没有，仅不调 on_respawned）

**现状**：

如果 panic 后 `spawn_one` 失败（如 `detect_collector()` 返 None 因环境变化），`on_respawned` 不调用，retry_count 不增加。`state.last_crash` 仍在，下次 `restart_tick`（1s 后）会再次尝试 spawn_one——按 backoff 间隔（5s/30s/5min）重试。这意味着环境持续不支持该 worker 时，**永远无法到达 permanent_failure 状态**——worker 看似在重试，实际每次都失败。

**实际场景**：少见。典型发生在管理员 → 非管理员权限切换后 ETW worker panic。

**修复**：在 `spawn_one` 失败时调用 `state.on_respawn_failed(now)` 让 retry_count += 1，达到 MAX_RETRIES 后进入 permanent_failure 止损。

---

### P2-3：docker-snapshot-worker / docker-logs-worker-{name} 不在 canonical_worker_thread_name 列表

**位置**：`src/workers/manager.rs:278-288`（`canonical_worker_thread_name` 列表）+ ADR-0019 未明确文档化此例外

**现状**：

`canonical_worker_thread_name` 列出 6 个 worker（port / usb / net-flow / dns-log / disk-io-etw / schannel-etw），不含 docker-snapshot-worker / docker-logs-worker-{name}。docker worker panic 时 `WorkerManager::restart` 因 canonical 返回 None 直接返 false，docker worker 不会自动 respawn。

CONTEXT.md line 9 of manager.rs 注释明确「Docker worker 仍由 DockerPanel 自管」，但 ADR-0019 文档未明确这一例外，未来维护者会困惑。

**修复**：在 ADR-0019 §决策 7「不实装 ebpf_worker restart」之后追加「不实装 docker worker restart：DockerPanel 自管 worker 生命周期，独立 spawn/drop 逻辑」。或者把 docker worker 也接入 restart（需重构 DockerPanel 把 worker handle 暴露给 WorkerManager）。

---

### P2-4：HRESULT 映射不完整，CERT_E_EXPIRED / CERT_E_UNTRUSTEDROOT 都归 Unknown

**位置**：`src/security/signature.rs:83-93`（`from_wintrust_result`）

**现状**：

仅映射 3 个 HRESULT：0 → Signed / TRUST_E_SUBJECT_NOT_SIGNED → Unsigned / CRYPT_E_REVOKED → Revoked。其他都归 Unknown 扣 5 分。

未映射的关键 HRESULT：
- `CERT_E_EXPIRED` (0x800B0101) — 证书过期（应类似 Unsigned 严重）
- `CERT_E_UNTRUSTEDROOT` (0x800B0109) — 不受信根（应类似 Unsigned 严重）
- `CERT_E_WRONG_NAME` (0x800B0113) — 名称不匹配
- `TRUST_E_CERT_SIGNATURE` (0x80096010) — 签名无效
- `CERT_E_CHAINING` (0x800B010A) — 链断裂

**影响**：证书过期 / 不受信根的进程扣分偏宽松（Unknown 5 vs 应 15-20）。

**修复**：扩 from_wintrust_result 映射 + 加 SignatureStatus 变体（如 `Expired` / `UntrustedRoot`），或在 Unknown 桶内细分 weight。当前 surgical 选择优先级低。

---

### P2-5：TRUSTED_SIGNERS 列表较短，缺常见 vendor

**位置**：`src/security/signature.rs:50-59`

**现状**：仅 8 个 vendor：Microsoft / Google / Mozilla / Apple / Intel / NVIDIA。

**缺**：Adobe / Cisco / Oracle / VMWare / Docker / Red Hat / Apache Software Foundation / Python Software Foundation / Electron.js / GitHub 等。

**影响**：常见软件（如 Adobe Reader / Cisco VPN / Docker Desktop / Oracle JDK）的进程被标为 `Signed`（扣 10 分）而非 `Trusted`（不扣分），用户视角误报。

**修复**：扩列表 + 走 `path_rules.toml` 类似的用户配置入口（`trusted_signers.toml`），让用户标记自家应用。

---

### P2-6：regex 中不能 escape `/`，影响 CIDR / URL pattern

**位置**：`src/filter/parser.rs:425-431`（`parse_regex_lit` 用 `take_till1(|c| c == '/')`）

**现状**：用户写 `remote_addr =~ /127\.0\.0\.1\/8/` 想匹配 CIDR `127.0.0.1/8`，但 parser 在第一个 `/` 停止，pattern 变成 `127\.0\.0\.1\`，剩余 `/8/` 被当成 trailing input 报错。

**修复**：要么支持 `\/` escape（修改 parser），要么文档建议用户用 `[\/]` character class（regex crate 支持）。

---

### P2-7：NetworkIn 用 Vec 线性查找

**位置**：`src/filter/mod.rs:270-280`（`FilterExpr::apply_network` 的 NetworkIn 分支用 `values.iter().any(...)`）

**现状**：N 个值的 in 列表，每个 flow 检查 O(N)。N 通常 < 10，但极端用户写 100 个 IP 黑名单 + 1000 个 flow → 100K 操作每 tick。

**修复**：在 FilterExpr::NetworkIn 构造时把 Vec 转为 HashSet，apply 时 O(1) 查找。改动小（~10 行），优先级低。

---

### P2-8：`%` 单位与 cpu / mem 字段交互语义不清

**位置**：`src/filter/parser.rs:406-414`（`parse_number_value` 的 `%` 分支）

**现状**：`mem > 5%` 解析为 `Value::Percent(5)`，与 `mem > 5`（字节）在 `apply_num` 下等价（5 == 5）。用户期望 `mem > 5%` 是「内存占用 > 5%」（基于总内存），实际是「内存字节数 > 5 字节」。

**影响**：用户写 `mem > 50%` 期望过滤占用 50%+ 内存的进程，实际过滤内存 > 50 字节的进程（几乎全部命中）。

**修复**：在 `Field::Mem::extract` 中把字节转 % 总内存（需 `System::total_memory()`），或者在 parser 阶段拒绝 `mem%` 组合（更严格）。

---

### P2-9：跨 ctx 表达式不支持（如 `cpu > 5 AND sni =~ /evil/`）

**位置**：`src/filter/mod.rs:204-288`（`apply` 与 `apply_network` 完全分离）

**现状**：Flow 视图调 `apply_network` 时，process 字段变体（FieldCmp / Regex）返 false。`cpu > 5 AND sni =~ /evil/` 在 Flow 视图下：`cpu > 5`（false） AND `sni =~ /evil/`（true） → false。

用户视角：「我想看 chrome 进程的 evil.com flow」无法直接表达。需先在 Process 视图找 chrome pid，再在 Flow 视图按 pid 过滤。

**修复**：在 NetworkEvalCtx 加 `process: Option<&ProcessInfo>` 字段，apply_network 对 process 变体在 process 存在时走 apply 逻辑。Flow 视图构造 ctx 时通过 pid 关联 process。改动中等（~50 行），与 surgical 原则冲突，优先级低。

---

### P2-10：R17 ScriptInterpreter 不分场景扣分（系统登录脚本也命中）

**位置**：`src/security/lineage.rs:179-182`（`detect_suspicious_chain` 的 ScriptInterpreter 优先级）

**现状**：当前进程是 wscript/cscript/mshta 即扣 15 分，不看祖先。系统登录脚本 / IT 部门部署脚本都命中。

**影响**：企业环境常见 wscript.exe 启动脚本，被标可疑。15 分扣分较低，但用户视角误报。

**修复**：增加「直接父是 services.exe / wininit.exe（系统启动）→ 不扣分」白名单。或降低 weight 到 5。

---

### P2-11：R18 + path_check 叠加扣分导致 Downloads 等合法路径扣 30 分

**位置**：`src/security/score.rs` 第 3 步（path_check）+ 第 18 步（R18）

**现状**：用户从 Downloads 运行合法安装包（如 VS Code installer），同时命中：
- v0.6 path_check downloads_dir (15)
- R18 UserProfileDownloads (15)
- 总扣分 30

CONTEXT.md 明确「surgical 原则——安全评分偏向严格」。这是设计选择，但**用户视角是误报**。

**修复**：在 path_check 内部「命中 downloads_dir 时跳过 R18 检查」（去重），或者 R18 UserProfileDownloads weight 从 15 降到 5。优先级低。

---

### P2-12：plan.md 不用 [x] checkbox 风格，stage-7.md 任务清单第 7 项描述与实际格式不匹配

**位置**：`plan.md`（表格风格）+ `docs/stages/v0.11-stage-7.md:55`（假设 checkbox 风格）

**现状**：stage-7.md 任务清单第 7 项「plan.md 中所有功能阶段已 [x]」假设 plan.md 用 checkbox 标记阶段完成情况，但 plan.md 实际是表格风格（「阶段 N 实装：...」描述）。

**修复**：要么改 plan.md 用 checkbox（破坏现有风格），要么改 stage-7.md 任务清单第 7 项描述（更准确：「plan.md 阶段表 + CONTEXT.md 演进历史段全部更新到 v0.11」）。后者更 surgical。

---

### P2-13：`property_at_index` 的 `'static` lifetime 不正确

**位置**：`src/dns_log/etw.rs:510-526`

**现状**：

```rust
fn property_at_index(
    info_ptr: *const TRACE_EVENT_INFO,
    idx: usize,
) -> Option<&'static EVENT_PROPERTY_INFO> {
    ...
    Some(unsafe { &*prop_ptr })
}
```

返回 `&'static` 但实际生命周期与 `info_ptr` 指向的 buffer 绑定（调用方 info_buf 保活）。严格说应改为 `Option<&'a EVENT_PROPERTY_INFO>` + 加生命周期参数。实际不会触发 use-after-free（info_buf 在调用栈保活），但是 API 契约不准确。

**修复**：加 lifetime parameter。优先级低（不影响正确性）。

---

### P2-14：MCP DNS tool 拿不到历史（每次调用重启 ETW session）

**位置**：`src/mcp/handler.rs:891-910`（`make_dns_json`）

**现状**：MCP 每次调用 `proc_dns` 都创建一个临时 `EtwDnsCollector`（启动 ETW session + spawn ProcessTrace 线程），drain 一次拿现有数据，然后 collector drop（关闭 session）。**启动前发生的 DNS 查询无法被捕获**——session 启动后到 drain 之间的查询（短暂窗口）才能拿到。

**影响**：MCP 用户调用 `proc_dns` 通常拿到空结果或少量结果（取决于 drain 间隔）。与 v0.6 PowerShell 路径行为一致，不是 v0.11 引入。

**修复**：让 MCP handler 持有长生命的 EtwDnsCollector（与 App::workers.dns_log_worker 类似的生命周期）。改动中等（MCP handler 需 state 化），优先级低（与 v0.6 行为一致）。

---

### P2-15：`signature_risk_factor` 中 `_ => None` 通配符可能掩盖新加变体

**位置**：`src/security/signature.rs:264`

**现状**：

```rust
pub fn signature_risk_factor(status: SignatureStatus) -> Option<RiskFactor> {
    match status {
        SignatureStatus::Pending => None,
        SignatureStatus::Unsigned => Some(...),
        SignatureStatus::Revoked => Some(...),
        SignatureStatus::Signed => Some(...),
        SignatureStatus::Unknown => Some(...),
        _ => None,  // ← Trusted 走这里；但未来加新变体也走这里，可能掩盖 bug
    }
}
```

**修复**：把 `_ => None` 改为 `SignatureStatus::Trusted => None`，让编译器在新加变体时强制更新 match。

---

## 审查覆盖矩阵（按 stage 7 doc 任务清单 7 子项）

| 任务 | 覆盖情况 | 发现问题 |
|---|---|---|
| 1. 基线测试三件套 | ✅ 全过（1141 passed / fmt / clippy / no-default-features build） | 无 |
| 2. 代码质量审查 | ✅ 覆盖 worker restart / DNS ETW callback / FilterExpr v2 / BackgroundScorer / parent_chain / R16-R18 | P1-1（callback UB）+ P2-2 / P2-7 / P2-9 / P2-10 / P2-11 / P2-15 |
| 3. 架构审查 | ✅ 覆盖 4 个新模块设计 / 0 个新依赖 / serde round-trip | P2-12（plan.md 风格）+ P2-13（lifetime） |
| 4. 安全性审查 | ✅ 覆盖 worker restart 触发条件 / DNS 隐私 / 签名验证路径泄漏 / TOML 解析 | P1-3（非 Windows 评分一致性）+ P2-3（docker worker 例外） |
| 5. 跨平台审查 | ✅ cfg-gate 正确 / no-default-features build 通过 / Linux 上 schannel_etw / dns_log etw 路径为 None | P1-3（cfg-gate 缺 signature_risk_factor） |
| 6. 性能审查 | ✅ HashReputation 缓存 / parent_chain O(N*32) / DNS ETW callback worker 线程 / backoff 止损 | P2-2（spawn 失败循环）+ P2-7（Vec 线性查找） |
| 7. 完整性检查 | ✅ plan.md 全部更新 / 测试通过 / ADR + TD + CONTEXT 完整 | P1-4（stage docs 头部 ✅ 标记）|

---

## 验收对照（plan.md 阶段总览 vs 实际）

| 阶段 | plan.md 计划 | 实际产出 | 验收 |
|---|---|---|---|
| 阶段 1 | WorkerManager::restart + ProcessInfo 字段骨架 + crash banner 升级 | `src/workers/{restart.rs(新), manager.rs}` + `src/app.rs::{poll_crashes, restart_tick, App::crash_tx}` + `src/tui/layout.rs::{draw_crash_banner, restart_label_for, restart_style_for}` + `src/security/signature.rs`（Pending 变体）+ `src/collect.rs`（2 字段骨架）+ 12 case test_worker_restart + ADR-0019 | ✅ |
| 阶段 2 | DNS ETW provider（Microsoft-Windows-DNS-Client）+ PowerShell fallback + DnsCollectorKind | `src/dns_log/{etw.rs(新 ~470 行), mod.rs(detect_collector tuple + DnsCollectorKind), windows_dns.rs(PidNameLookup pub(crate))}` + `src/workers/manager.rs(dns_collector_kind 字段)` + `src/cli/{diag.rs(dns_collector 行), dns.rs(tuple 解构)}` + `src/mcp/handler.rs` + 8 case test_dns_etw + ADR-0020 | ✅ |
| 阶段 3 | FilterExpr v2 + NetworkField + apply_network + parser 扩展 + CLI / TUI 接入 | `src/filter/{mod.rs(NetworkField + NetworkEvalCtx + 3 个 FilterExpr 新变体 + apply_network + 10 unit test), parser.rs(ParsedField dispatch + parse_in_list + 未知字段锚点 + 11 unit test)}` + `src/cli/{def.rs(Flows --filter), mod.rs(dispatch), flows.rs(run_flows filter)}` + `src/view_models/port_panel.rs(flow_search + flow_filtered_indices + handle_flow_view_key)` + `src/tui/port_table.rs(draw_flow_view)` + `src/app_panel.rs(PanelContext.flows)` + `src/app.rs(2 处 PanelContext 构造)` + 25 case test_filter_expr_v2 + ADR-0011 v0.11 增量段 | ✅ |
| 阶段 4 | 进程签名验证 + R16 未签名进程 | `src/security/{mod.rs(暴露 from_wintrust_result), signature.rs(from_wintrust_result + verify_signature_with_policy pub(crate) + SignatureStatus::badge + 5 unit test)}` + `src/app.rs(tick_heavy poll 反向同步 signature_status)` + `src/tui/{process_table.rs(name 后渲染 signature_status.badge), detail_view.rs(Summary 签名行)}` + 25 case test_signature + ADR-0021 | ✅ |
| 阶段 5 | parent_chain 字段填实 + R17 可疑父子链 + 自定义规则 | `src/security/{lineage.rs(新 ~555 行 + 16 unit test), mod.rs(pub mod lineage + re-export), score.rs(SecurityScorer 加 lineage_rules 字段 + score 第 17 步接入)}` + `src/collect.rs(HeavyWorker 批量填 parent_chain)` + `src/tui/detail_view.rs(draw_summary 加 R17 警告 + 父进程/祖父链显示)` + 10 case test_lineage | ✅ |
| 阶段 6 | R18 可疑启动路径 + 用户配置 + R16+R18 协同扣分 | `src/security/{path_rules.rs(新 ~640 行 + 15 unit test), mod.rs(pub mod path_rules + re-export), score.rs(SecurityScorer 加 user_dirs + path_rules 字段 + score 第 18 步接入 + r18_cooperation_factor + 7 unit test)}` + `src/tui/detail_view.rs(draw_summary 加 R18 警告 + 可执行行 [⚠ 可疑位置] 标记)` + 9 case test_path_rules | ✅ |

**全部 6 个阶段验收通过**，无未交付项。

---

## 阶段 7 自身合规性

- ✅ **本阶段未修改任何代码**（git diff 仅新增 `docs/reviews/REVIEW-13.md`，源代码 / 测试 / 其他文档零改动）
- ✅ **基线三件套全过**（1141 passed / fmt / clippy / no-default-features build）
- ✅ **REVIEW-13.md 已产出**（按 P0/P1/P2 三档分级 + 覆盖矩阵 + 验收对照）
- ✅ **任务清单 7 子项全覆盖**（基线 / 代码质量 / 架构 / 安全 / 跨平台 / 性能 / 完整性）

---

## 阶段 8 修复优先级建议

按 P1 顺序：

1. **P1-1**（callback UB）：DNS ETW callback 包 catch_unwind + accum.lock() 改为非 expect。同时审查 schannel_etw / disk_io_etw callback。
2. **P1-3**（非 Windows 评分一致性）：`signature_risk_factor` cfg-gate，Linux/macOS Unknown 不扣分。
3. **P1-2**（FilterExpr UX）：CLI flows filter 加 process 字段检测 + warn。
4. **P1-4**（stage docs 标记）：6 个 stage docs 头部加 ✅ 标记 + stage-7.md 第 56 行「4 个」改「6 个」。

P2 共 15 项，建议归档到 tech-debt.md，按优先级在 v0.12+ cycle 评估。本周期（v0.11.0 阶段 8）可不修。
