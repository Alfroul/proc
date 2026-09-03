# ADR-0037：CI 修复专项（远端 workflow 转绿决策集）

**Status**: Accepted（v0.27 stage 1 Spike 落地——D1~D5 五组决策定稿，取证附录见 [`docs/stages/v0.27-stage-1.md`](../stages/v0.27-stage-1.md)）

**Date**: 2026-09-03（v0.27 cycle stage 1 Spike）

**Related**: ADR-0022（Windows-only 平台决策——D3/D5 处置的底层依据）、ADR-0027（rmcp 0.11 SSE transport 集成——D4 暴露面评估对象）、ADR-0036（badge 占位 + gate 两档——本 cycle 兑现对象）、REVIEW-v0.26 §6（主题候选来源）

## Context

v0.26 push（tag `v0.26.0`，2026-09-02）触发远端 CI 三 workflow 全红，且 `check` job 挂死 6h00m 吃满 GitHub 360 min 上限。v0.27 cycle 主题拍板为 CI 修复专项（brainstorm 5 决策，2026-09-02），stage 1 负责取证补全与本 ADR 决策集。

**stage 1 取证的三项修正性发现**（修正 brainstorm 拍板时的认知，证据链见 stage-1 doc 附录 A）：

1. **远端 CI 从未绿过**——不是「何时破损」而是「从未通过」。ci.yml 历史 18 run 全 failure；check-macos / msrv / audit 三 job 自引入起（2026-06-17，run `27682617216`）11/11 全红；miri 14/14 全红（首跑 2026-06-23，比 brainstorm 记的「07-01 起 9 连红」更早，首错根因是 `nvml_wrapper` E0433 非 `windows` crate）；release 的 update-winget job 首次执行（2026-07-04）起 6/6 全红（action 仓库 404，从未工作过）。
2. **挂死不是新问题**——`check` job test 步 6h 挂死模式自 2026-07-01（run `28492287032`，v0.11.0 push）起已存在 3 次（07-01 / 07-04 ×2，均整 6h00m 被 GitHub 上限取消）；之后 fmt/clippy 连续挡路使 test 步再未执行，直到 09-02（v0.26 stage 4 搭车修掉 2 处 clippy 漂移后）test 步再次跑到并复现挂死（`security::signature::tests::real_path_does_not_panic`，真实 WinVerifyTrust 路径在 GitHub windows runner 上无限挂起，该测试随 v0.12.0 `f56774a` 引入）。
3. **lib 在非 Windows 平台结构性编译不过，与 feature 无关**——check-macos clippy（默认 features）74 个错误中仅 3 个源于 `nvml_wrapper`（`--no-default-features` 可消），其余 **~71 个**（E0433 `windows` ×4 / E0425 windows-only 函数被非门控调用 ~35 / E0308/E0599/E0277 等 ~30）在任何 feature 档下都存在：`src/lib.rs` 43 个模块**零 cfg 门控**全量声明，estats / net_flow::windows / inspect / eject / throttle 等模块的 Windows-only 实现被非门控调用方直接引用。本地 Linux target 交叉 check 亦证实死在更早的依赖 C 工具链（reqwest→rustls 链），从未到达 proc 自身代码。**推论**：msrv job（ubuntu）即使解决依赖 MSRV 漂移，也会撞同款 ~71 个编译错误——该 job 自设计起就放错了平台。

## Decision

### D1：CI 测试策略——choke-point env-gate + ETW 类 per-test gate + timeout 兜底 ✅

**设计**（stage 2 实装，`CI=true` 只在 GitHub Actions 存在、本地不设——本地行为零变化）：

1. **choke-point 单点 gate（主修复）**：`src/security/signature.rs::verify_signature_with_policy` 入口加 `CI` env 检查 → 命中即返 `SignatureStatus::Unknown`（与本地非提权行为完全一致，本地断言全兼容）。一处覆盖整类 WinVerifyTrust 挂点：
   - lib `real_path_does_not_panic`（signature.rs:443，实证挂死）
   - `tests/test_signature.rs:497` `verify_signature_does_not_panic_on_windows`（同款真实路径，CI 上从未跑到）
   - `SecurityScorer::score()`（score.rs:249）的全部测试路径——`test_security.rs` score 系列、`test_scorer_concurrency.rs` `big_request`（200 个 synthetic 进程全带 `C:\fake\*.exe`）、`test_mcp_v0_17` worker 真实进程路径
   - 选择 choke-point 而非逐测试 gate 的理由：调用面横跨 lib + 3 个集成测试 binary 且经 scorer 间接到达，逐测试 gate 必漏；单点防御对未来新增测试同样生效。
2. **ETW + 真实网络类 per-test gate**：`tests/test_dns_etw.rs` / `tests/test_schannel_etw.rs` 文件级 `CI=true → early-return skip`。理由：两文件的守卫是「collector 启动失败→SKIP」降级式，GitHub windows runner 默认提权 → ETW 能启动 → 走真实路径（`Resolve-DnsName example.com` / `curl https://example.com`）——非确定性（ETW 抓包漏抓重试已文档化）+ runner 代理网络下的外网依赖。
3. **timeout 兜底**：check job 加 job 级 `timeout-minutes`（60：build ~10 min + test 全量预留 ~45 min + 余量）+ test 步骤步级 `timeout-minutes: 45`——任何未知挂点从 6h 挂死止损为确定性失败。
4. **test 步骤跑全量 1744（不裁剪到 gate 快档 21 binary 名单）**：CI 与本地回归基线同口径（信号最大）；gate 快档是本地自愿防线的定位不变（ADR-0036 D2 分层延续）。
5. **自跳过类零改动留档**（附录 B 清单证明已排查）：llama_cpp_provider（server 缺失 skip）/ disk_io_etw（CI 降级路径文档化）/ gpu（NVML init Err→None）/ kill_by_name（sysinfo 未观测→跳断言）/ docker 类（daemon 缺失不 crash）/ 22 处 `#[ignore]`（真实 E2B / ANTHROPIC key）；时序断言类（test_perf_baseline <5ms / test_search_perf）不 gate，列 stage 3 远端观察项。

### D2：msrv 处置——job 迁 Windows runner + rust-version 1.85→1.88（方案 B）✅

**两步修复**（缺一不可）：

1. **job 平台修正**：msrv job `ubuntu-latest` → `windows-latest`（发现 3：ubuntu 上 lib 编译不过，该 job 在 ubuntu 从未可能绿过；MSRV 验证本就该在 crate 实际构建平台跑）。步骤不变（`cargo check --no-default-features --all-targets`——该档在 Windows 本地实测编译通过，v0.26 gate 与 ci.yml check job 的 `build (release, no default features)` 步均绿）。
   - **stage 2 实装注记（2026-09-03）**：上文「该档实测编译通过」的依据是 `build --no-default-features`（lib+bins），`--all-targets` 是未验证外推——本地 `cargo check --no-default-features --all-targets` 实测**在任何工具链（1.95 / 1.88 均验证）编不过测试目标**：`test_agent_v0_20_stage_2` / `_stage_3b` 引用 `mock-provider` / `llama-cpp` 门控模块（两者是 default feature，测试按默认 feature 写是合理设计；该失败 ubuntu 时代死于依赖解析从未暴露）。msrv job 步骤修正为 `cargo check --no-default-features`（lib+bins，与 check job 的 build 步同编译域），`cargo +1.88.0 check --no-default-features` 本地实证通过（34.6s）。
2. **MSRV 声明 B 方案（升 1.88）**：`rust-version = "1.88"`（Cargo.toml 一行）+ workflow toolchain `1.85.0` → `1.88.0`。理由：
   - **现状声明已失真**：lockfile 16 包要求 1.86-1.88（附录 C 权威清单），1.85 今天已经编不了 proc——诚实的 1.88 声明优于失真的 1.85 声明；
   - **pin 面（A 方案）是 4 条独立链 16 包**（icu×8←idna_adapter←url / image←arboard / serde_with+time←bollard-stubs / instability→darling×3←ratatui），全是活跃链，任何未来 `cargo update` 都会再漂移（brainstorm 风险 2 承认的复发面）；
   - 1.88（2025-06 发布）在 2026-09 已是 14 个月老基线，仍极保守；edition 2024 最低线 1.85 的口径不受影响（1.88 > 1.85）；
   - README 无 MSRV 声明（已核实），B 方案 doc 影响面 = Cargo.toml 一行 + ci.yml 一处 + CHANGELOG 条目。
   - A 方案完整 pin 清单留档附录 C（若用户否决 B，stage 2 按清单执行，预期 11+ 条 `cargo update --precise` + 本地装 1.85 工具链验证）。
   - **本项为 stage 2 启动前用户确认点**（brainstorm 决策表遗留的 A/B 拍板点，stage 1 给出推荐 B + 依据）。

### D3：check-macos 处置——C 方案（移除 job）✅

B 方案（job 改仅 `--no-default-features` check）被发现 3 **实证否决**：该档在 macOS 仍有 ~71 个非 feature 相关编译错误，feature 卫生检查在 macOS 从未成立也近期不可能成立（恢复需 ~10+ 模块补非 Windows 路径，即 brainstorm 选项 A 大工程，明确不在本 cycle）。

处置：删除 check-macos job + 本 ADR 注记（Q：为何 CI 矩阵无 macOS？A：ADR-0022 Windows-only + lib 模块树零 cfg 门控的实证 + job 11/11 从未绿 + `--no-default-features` 卫生已由 check job 的 Windows build 步覆盖，无检查损失）。README/亮点条若有「macOS CI」口径随 stage 2/3 一并核对修正。

### D4：audit 修复——permissions 块 + crossbeam update + rmcp ignore（决策 3A 兑现）✅

1. **workflow `permissions:` 块**：audit job 加 `contents: read` + `checks: write`（rustsec/audit-check action 建 check-run 所需；现状 `Resource not accessible by integration`）。
2. **crossbeam-epoch**：`cargo update -p crossbeam-epoch`（0.9.18 → ≥0.9.20，RUSTSEC-2026-0204 patched；纯 lockfile 变更，非直接依赖）。
3. **rmcp RUSTSEC-2026-0189**（决策 3A 拍板兑现）：`cargo audit --ignore RUSTSEC-2026-0189`（audit job 参数或 deny 配置）+ 立 TD 追踪 0.11→≥1.4 major。**暴露面评估**（附录 E，advisory 原文 + 代码实证）：
   - 漏洞本体：rmcp <1.4 Streamable HTTP transport 不校验 `Host` header → DNS rebinding 使恶意网页可访问受害者 loopback/私网接口上的 MCP server，枚举/调用 tool、读 resource/prompt、触发 tool 副作用；**advisory 原文明确「stdio 等非 HTTP transport 不受影响」**；
   - proc 实际暴露面：默认 `proc mcp serve` = stdio（不受影响）；SSE 路径（opt-in `--transport sse`）用 `StreamableHttpServerConfig::default()`（transport.rs:200，即漏洞配置），攻击链需同时满足「用户显式开 SSE + 服务运行中 + 受害者浏览恶意页」三条件，且 CLI 默认 `--bind-addr 127.0.0.1`（仅 loopback，全网卡需显式 opt-in）+ 写操作 tool 有 confirm 契约拦截——**窄而真实的暴露面**，升级 0.11→1.4 是 breaking major（46 MCP tool 底座），留独立 cycle（TD 追踪），ignore 期间在 ADR/README 安全叙事如实注记。

### D5：miri 移除（声明性降级）+ winget 移除 ✅

1. **miri.yml 整体删除**（声明性降级，与 unsafe-audit 口径同步改为「miri 防线已移除」）。依据：
   - runner 在 ubuntu：lib 在非 Windows 编译不过（发现 3）——job 首跑起 14/14 全红，E0433 是平台性不可修；
   - 移 windows runner 也不可行：198 处 unsafe 全在 NT FFI 层（miri 不支持 Win32 FFI 调用），且两个目标测试 binary（test_scorer_concurrency / test_workers）合计 15 处 `thread::sleep` 时序断言（最长 2s 等待 × big_request 200 进程评分）——解释器慢 20-100× 下不可行；
   - unsafe 防线的实际承担者已是 SAFETY 注释（59→63 处）+ unsafe-audit doc + 全量回归，miri 的边际价值为负维护成本。
   - `docs/unsafe-audit.md` 相应一句改写（「workflow 待修」→「已移除，理由见 ADR-0037 D5」）。
2. **release.yml update-winget job 删除**。依据：上游 action `russellbanks/release-automation-winget` 仓库已删（404），首次执行（2026-07-04）起从未成功；`microsoft/winget-pkgs` 无 `Alfroul.proc` manifest（2026-09-03 API 核实 404）——即 winget 分发从未实际存在，删除零用户破坏。实际分发渠道 = GitHub Releases（build job 绿，10/10）+ cargo-binstall（Cargo.toml `[package.metadata.binstall]` 元数据在位）。若未来要上 winget：走 winget-pkgs 手动 PR 流程（社区标准），届时再立独立 job。

## Consequences

- **正面**：远端三 workflow → 两 workflow（ci + release）且全部失败点有确定性修复路径；badge 兑现前提成立；6h 挂死止损（timeout + 挂点 gate）；面试叙事从「CI 全红」变「CI 绿 + 有记录的工程决策集」（本 ADR 五组决策全部有取证链）。
- **负面/接受**：CI 矩阵失去 macOS/miri 两个「看似多平台」的 job——用 ADR-0022 + 本 ADR 的诚实论证换掉永远红的装饰性 job；msrv 1.88 放弃 1.85 数字口径（换来声明真实性 + 摆脱 16 包 pin 的持续维护）；rmcp ignore 在 audit 输出留痕（附录 E + TD 追踪构成完整答复链）。
- **遗留**：首次 push 后仍可能有未知 CI 挂点/失败（本地无法复现类，风险 1 预案：stage 3 迭代预算）；时序断言类测试（perf_baseline 等）在 CI 慢硬件的行为是观察项；rmcp major TD（新立）；msrv 漂移探测职能由升级后的 msrv job 继续承担（1.88 声明下未来依赖要求 1.89+ 时再次报警）。
