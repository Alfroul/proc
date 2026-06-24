# Contributing to proc

感谢你的兴趣！本文档说明开发流程、提交规范、门禁要求。

## 开发环境

| 工具 | 版本 | 说明 |
| --- | --- | --- |
| Rust | 1.85+（2024 Edition） | `rustup show` 检查 |
| 目标平台 | Windows 主开发平台 | Linux / macOS 可降级编译 |
| 推荐编辑器 | VS Code / RustRover | 装 rust-analyzer 扩展 |

Windows 平台依赖（自动安装）：
- `windows` crate 的 native features 通过 cargo 直接拉取
- 可选：`nvml-wrapper`（NVIDIA 监控，default feature）
- 可选：smartctl（SMART 磁盘健康，用户机器装 smartmontools 即可）

Linux 依赖（可选）：
- `nvtop`（AMD / Intel / NVIDIA GPU 监控）
- `nethogs`（per-process 网络流量）
- `smartmontools`（SMART 磁盘健康）

## 提交门禁

每次提交前必须本地跑通以下 4 项检查（CI 也跑同样的）：

```bash
cargo test --release --tb=no -q                          # 全量回归（611+ passed）
cargo clippy --release --all-targets -- -D warnings      # 0 warnings
cargo fmt --all -- --check                               # 干净
cargo build --release --no-default-features              # 跨 feature 编译通过
```

CI（`.github/workflows/ci.yml`）在 PR 上自动跑这 4 项，全绿才能 merge。

Miri（`.github/workflows/miri.yml`）会跑并发 UB 检测，仅 H4 模块（VT100 / record 序列化）相关 PR 才会触发，但触发后必须 0 UB。

## Commit message 风格

参考 CHANGELOG 的 `Added` / `Changed` / `Fixed` 分类。建议格式：

```
<type>(<scope>): <subject>

<body>
```

`type` 可选值：
- `feat`：新功能
- `fix`：bug 修复
- `perf`：性能优化
- `refactor`：重构
- `docs`：文档
- `test`：测试
- `ci`：CI/CD
- `chore`：杂项

示例：
```
feat(inspect): env mask 录屏时强制 mask 模式
fix(dns_log): DnsQuery 加 start_time 防 PID 复用串数据
perf(detail_view): Summary Tab 优先级/affinity 走缓存避免每帧 syscall
docs(readme): 删除 ADR-0006 悬空引用
```

scope 不强制，能体现修改模块即可。

## 分阶段开发

proc 采用 **分阶段开发** 模型，每个 release cycle 拆成 N 个独立可验证的阶段，每阶段在独立会话中完成。

- 当前 cycle：v0.6.0（8 阶段，详见 `docs/stages/stage-1.md` ~ `stage-8.md`）
- 上一个 cycle：v0.5.0（11 阶段）
- 阶段总览见仓库根 `plan.md`（私有）或 `docs/stages/`

新功能应归属到下一个 cycle 的某个阶段。不确定时先开 issue 讨论。

## ADR 流程（Architecture Decision Records）

**何时需要写 ADR**：
- 引入新 native 依赖
- 引入新 feature flag
- 多方案选型（必须解释为什么不选其它方案）
- 推翻既有决策（必须把旧 ADR Status 改为 `Superseded by ADR-NNNN`）

**写 ADR 的格式**：见 `docs/adr/README.md` + 既有 ADR-0001 ~ 0008 作为模板。

**编号**：紧接当前最大编号（如当前最大是 0008，下一个写 0009）。

**Status 流转**：
- `Proposed`：已提出但未落地
- `Accepted`：已落地（落地后改）
- `Superseded by ADR-NNNN`：被新决策推翻

## Issue / Pull Request

- **Issue**：bug 报告 / feature request 用 GitHub issue。先搜现有 issue 避免重复。
- **PR**：
  - 标题遵循 commit message 风格
  - body 说明改了什么 + 为什么 + 如何测试
  - 大改动（>500 行）建议先开 issue 讨论 approach，避免白做
  - 安全相关 PR 必须更新 `SECURITY.md`（如改动涉及特权模型 / 加固策略）

## License

提交即视为同意以 MIT 协议发布（仓库根 LICENSE）。

## 参考

- README.md：项目功能总览
- CHANGELOG.md：历史变更
- docs/adr/：架构决策记录
- docs/stages/：分阶段开发计划
