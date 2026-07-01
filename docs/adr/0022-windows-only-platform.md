# ADR-0022：Windows-only 平台定位

**Status**：Accepted
**Date**：2026-07-01（v0.12.0 阶段 1 落地）
**Supersedes**：ADR-0016（eBPF flow graph）、ADR-0013（PSI 监控）

## 背景（Context）

proc 自 v0.5.0 起设计为跨平台 TUI（Windows 主开发 + Linux/macOS 降级）。v0.7 阶段 8 引入 eBPF flow graph（ADR-0016）作为 Linux 杀手锏；v0.7 阶段 6 引入 PSI 监控（ADR-0013）作为 Linux 4.20+ 金标准。

**实际演进**：

- **TD-19（eBPF Linux 真实编译验证）**：自 v0.7 阶段 8 起挂账，v0.8.0 / v0.10.0 / v0.11.0 cycle 三次主动推迟。原因：用户主力 Windows 开发，Linux 真机环境（WSL2 + nightly + bpf-linker + 内核 ≥ 5.10）搭建成本高，且各 cycle 不依赖 ebpf 路径。
- **TD-17（eBPF TLS SNI / JA4 / bytes）**：v0.7 阶段 8 留下，需要 Linux 真机环境才能落地。
- **Linux 路径维护成本**：v0.11 REVIEW-13 多个 P2 涉及 cfg gate / cross-platform 一致性（P1-3 非 Windows signature_risk_factor / P2-13 property_at_index lifetime / P2-3 docker worker restart 例外等），占 review 精力但用户从未在 Linux 上跑过 proc。
- **v0.11 cycle 收尾用户决策**：v0.11.0 阶段 8 后用户明确「这个只作为 Windows 系统的应用」，要求移除 Linux 端代码。

## 决策（Decision）

**v0.12.0 起 proc 转为 Windows-only 应用**：

- **平台支持范围**：Windows 10 1809+（build 17763+）/ Windows 11，x64 架构
- **代码库清理**：
  - 移除 `src/ebpf/` 整模块（6 files，含 ebpf-ebpf 内核态子项目）
  - 移除 `src/psi.rs`（Linux PSI 监控）
  - 移除 `~30 文件 cfg(not(target_os="windows"))` / `cfg(target_os="linux")` 分支
  - 移除 `tests/test_linux_stubs.rs` / `tests/test_psi.rs` / `tests/test_ebpf_flow.rs`
  - 移除 `EBPF_ENABLED` 常量 + 所有 `FlowSource::Ebpf` 引用（ProcessFlow 简化为仅 Schannel 路径）
  - 移除 NvtopProvider（Linux nvtop 子进程）/ NethogsCollector（Linux nethogs 子进程）
- **Cargo.toml 简化**：
  - 删 `[target.'cfg(not(target_os="windows"))'.dependencies]` 段（libc 等仅 Linux 用）
  - 删 `[target.'cfg(target_os="linux")'.dependencies]` 段（aya / aya-log）
  - 删 `[features] ebpf / nvtop / nethogs` feature flag
  - 删 `[workspace] members = ["src/ebpf/ebpf-ebpf"]` 段
  - 删 Rust toolchain nightly 引用
- **CI / Release 简化**：
  - 删 `.github/workflows/ci.yml` 的 `check-linux` job
  - 删 release CI 的 linux-musl / linux-arm / linux-ebpf / macOS target build step
  - 仅保留 `x86_64-pc-windows-msvc` target
- **ADR 状态调整**：
  - ADR-0016（eBPF flow graph）Status 改 `Superseded by ADR-0022`
  - ADR-0013（PSI 监控）Status 改 `Deprecated (v0.12 移除)`
  - 其他 ADR（0009 MCP / 0011 FilterExpr / 0019 worker restart / 0020 DNS ETW / 0021 signature）保持 Accepted
- **用户迁移路径**：
  - 旧 Linux/macOS 用户：`git checkout v0.11.0` + `cargo build` 仍可用，但不接受新功能
  - 如有强烈 Linux 需求：欢迎 fork（v0.11.0 是最后含 Linux 代码的 release）

## 备选方案（Alternatives）

### (a) 继续双平台，接受维护成本

**否决理由**：
- TD-19 三次推迟证明 Linux 真机验证成本无法摊销
- 用户主力 Windows，Linux 代码路径从未被用户实际使用
- v0.11 REVIEW-13 多个 P2 与 Linux 路径相关，浪费 review 精力
- 双平台代码库让 Windows 用户体验受损（cfg gate 静默 skip 风险）

### (b) 仅移除 ebpf（最不成熟部分），保留 sysinfo Linux 路径

**否决理由**：
- 部分措施，无法彻底降低维护成本
- sysinfo Linux 路径仍需 cfg gate，未来仍会触发 P2
- 用户明确要求 Windows-only，部分移除违背指令

### (c) fork 给社区维护 Linux 版本

**推迟**：
- 当前没有社区贡献者主动接手
- v0.11.0 tag 已锁住 Linux 代码，未来 fork 随时可恢复
- 如有社区需求欢迎，但本 cycle 不主动做 fork

## 结果（Consequences）

### 正面

- **代码库简化**：~1000 行 Linux 代码删除，~30 文件 cfg gate 清理，Cargo.toml 显著瘦身
- **维护成本降低**：未来不再有 Linux 相关 P2（cfg gate / cross-platform 一致性 / lifetime 问题）
- **CI / Release 简化**：5 target → 1 target，build 时间和复杂度降低
- **聚焦 Windows 用户场景**：开发精力全部投入 Windows 平台深度（签名 / ETW / Schannel / EcoQoS / WMI 等方向）
- **工具链简化**：不再需要 nightly + bpf-linker（Linux ebpf 编译工具链）

### 负面

- **Linux/macOS 用户流失**：v0.12+ 不再支持，用户需停留在 v0.11.0 或 fork
- **ADR-0016（eBPF flow graph）设计投入沉没**：v0.7 阶段 8 落地的 ebpf 模块完全废弃
- **跨平台学习价值降低**：proc 之前作为「Rust 跨平台 TUI 范例」的价值减弱
- **未来若想恢复 Linux 支持**：需从 v0.11.0 cherry-pick + 重新合并（成本不低）

### 中性

- **FilterExpr FlowSource 字段移除**：v0.10 落地的 `ProcessFlow.source: FlowSource` 简化（仅 Schannel 路径），serde `#[serde(default)]` 保旧录屏兼容
- **CONTEXT.md 术语段瘦身**：移除 `EBPF_ENABLED` / `EbisuBpfWorker` / `FlowSource` / `PSI` / `NvtopProvider` / `NethogsCollector` 等 Linux 相关术语

## Migration path

| 用户类型 | 推荐操作 |
|---|---|
| Windows 用户（主） | 升级到 v0.12+，享受更稳定的 Windows-only 版本 |
| Linux/macOS 用户 | 停留在 v0.11.0（`git checkout v0.11.0`）；如需新功能欢迎 fork |
| 跨平台 IDE 用户（Cursor / VSCode Remote） | 在 Linux 远端用 v0.11.0；本地 Windows 用 v0.12+ |
| proc contributor | clone master 分支仅 Windows 编译；旧 ADR-0016 / 0013 保留作历史参考 |

## 相关 ADR

- **ADR-0016（eBPF flow graph）**：Superseded by ADR-0022（v0.12 移除 ebpf 路径）
- **ADR-0013（PSI 监控）**：Deprecated（v0.12 移除，Linux-only 特性）
- **ADR-0018（Windows Schannel ETW SNI）**：保留 Accepted，v0.12 后成为唯一 TLS SNI 路径
- **ADR-0011（FilterExpr）**：保留 Accepted，FlowSource 字段移除不影响 FilterExpr 设计
- **ADR-0019（worker restart）/ 0020（DNS ETW）/ 0021（signature）**：保留 Accepted，与平台决策无关
