# 阶段 8：批量修复与收尾交付 — 修 P0/P1 + README + CHANGELOG + tag v0.6.0

> **独立会话指令**：阅读 CONTEXT.md、docs/stages/stage-8.md、docs/reviews/REVIEW-7.md，修复所有 P0/P1 问题并完成 v0.6.0 项目交付

**目标**：消费 Review 报告修复阻断性问题；完善 README / CHANGELOG；打 tag v0.6.0；触发 release CI。

**前置依赖**：阶段 7（Review）已完成，`docs/reviews/REVIEW-7.md` 已产出。

**预期代码量**：视 Review 报告而定，预估 200-500 行散点修改

**任务清单**：

### 任务 1：开工前回归验证

```bash
cargo test --release --tb=no -q                       # 应 ~741 passed
cargo clippy --release --all-targets -- -D warnings   # 0 warnings
cargo fmt --all -- --check
cargo build --release --no-default-features
```

如有任何失败 → 优先修复（不在 Review 报告内的回归 bug）。

### 任务 2：消费 P0 问题

逐项阅读 `docs/reviews/REVIEW-7.md` 的 P0 段，每项：

1. 在 REVIEW 文件该项后追加 `**Status: Fixed in commit XXX**`（commit hash 完工后填）
2. 实施修复（surgical 原则，最小代码改动）
3. 跑相关测试验证
4. 跑全量回归确保不引入新问题

**修复优先级**（建议顺序）：
- 安全 P0（self_mitigation / env_mask / restricted_spawn 漏洞）→ 最优先
- 功能 P0（键位 / 数据结构破坏）→ 次优先
- 文档 P0（README 严重过时）→ 最后

### 任务 3：消费 P1 问题

同 P0 流程，逐项修复 + 标记 Status。

### 任务 4：全量回归

```bash
cargo test --release --tb=no -q                       # 应 P0/P1 修复后全绿
cargo clippy --release --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build --release --no-default-features

# 跨平台 CI 矩阵
# 通过 GitHub Actions 看 check-linux / check-macos / miri 是否全绿
```

### 任务 5：归档 P2

如果阶段 7 已建 `docs/tech-debt.md`：检查分组是否合理，补充优先级标注。

如果未建：把 REVIEW-7.md 的 P2 段抽出，按 v0.7.0 / v0.8.0+ 分组创建 `docs/tech-debt.md`。

### 任务 6：更新 README.md（完整重写或大改）

README.md 需反映 v0.6.0 全部能力：

1. **快速开始**（阶段 1 已加 binstall / winget / scoop）
2. **功能矩阵**新增段：
   - v0.6.0 安全加固（env 脱敏 / 自我加固 / 录屏防护 / 子进程权限剥离）
   - v0.6.0 可观测性（日志 rotate / crash report / worker metrics / `proc diag`）
   - v0.6.0 性能优化（ProcessInfo Arc<str> / ProcessStatus 枚举 / 搜索缓存）
3. **快捷键表**同步：
   - 详情页 `F5` 刷新（替代 'r'）
   - 详情页 `y` 复制（替代 'c'）
   - 详情页 `v` 切换 env 脱敏
   - Docker `Shift+R` 重启
4. **CLI 子命令**新增：
   - `proc diag` — worker metrics 输出
5. **FAQ** 新增：
   - "录屏会泄漏什么？"（v0.6.0 启动前确认 + 强制 mask）
   - "self-mitigation 开了哪些策略？"（链接 ADR-0008）
   - "crash report 在哪？"（`~/.config/proc/crashes/`）
   - "日志为什么不覆盖了？"（v0.6.0 起每天一个文件，保留 7 天）
6. **平台支持表**：补充 macOS ARM / Linux ARM64 已在 release CI 覆盖
7. **架构图**可选：补 v0.6.0 新增的 src/metrics/ / src/security/ / src/cli/ 模块

### 任务 7：CHANGELOG.md 定稿

把 `## [Unreleased]` 段改为 `## [0.6.0] - YYYY-MM-DD`，整理阶段 1-8 内容：

```markdown
## [0.6.0] - YYYY-MM-DD

本次发布聚焦：**安全加固 + 可观测性 + 架构债清理**。8 个阶段累积：~5000 行代码（含测试），无 API 破坏。

**最终基线**（实测）：

| 命令 | 结果 |
|---|---|
| cargo test --release | ✅ X passed / 0 failed / N ignored（baseline 611 → v0.6.0 +Y）|
| cargo clippy --release --all-targets -- -D warnings | ✅ 0 warnings |
| cargo build --release --no-default-features | ✅ 编译通过 |
| cargo fmt --all -- --check | ✅ fmt clean |
| cargo bench --bench sort_cache | ✅ 搜索性能 10x 提升 |

### 阶段 1 — 文档 + 发布基础设施
（保留原内容）

### 阶段 2 — 安全加固
（保留原内容）

...（阶段 3-6）

### 阶段 7 — Review
- 产出 `docs/reviews/REVIEW-7.md`（X P0 + Y P1 + Z P2）

### 阶段 8 — 批量修复 + 发布
- 修复所有 P0 + P1（详见 REVIEW-7.md 标记 Status: Fixed）
- 归档 P2 到 `docs/tech-debt.md`
- README.md 完整重写
- CHANGELOG.md 定稿
- tag v0.6.0

### 验证矩阵
- cargo fmt / clippy / test / build 全绿
- 5 个 release CI target 构建成功（win-x64 / linux-musl / linux-arm64 / macos-arm64 / macos-x86_64）
- cargo binstall --dry-run proc metadata 正确解析

### ADR 状态
- ADR-0008（self-mitigation policy）Status: **Accepted**（阶段 2 落地）

### 已知限制（v0.7.0+ 路线）
- ProcessSignature Policy 未开（nvml-wrapper native 依赖兼容性，ADR-0008）
- restricted_spawn 仅覆盖 PowerShell DNS 子进程（docker exec / nvtop 留 0.6.1+）
- Linux DNS 日志仍不支持（pcap/eBPF 留 v0.7+）
```

### 任务 8：Cargo.toml 版本号

`Cargo.toml` 改：
```toml
version = "0.6.0"
```

### 任务 9：手动启动验证

按 README.md「快速开始」章节命令完整跑一遍：

```bash
# 1. 从源码构建
cargo build --release
./target/release/proc                # TUI 启动，6 面板全部正常

# 2. CLI 子命令
./target/release/proc ls --sort cpu --limit 20
./target/release/proc ls --sort net_recv --limit 10
./target/release/proc port 8080 --stats
./target/release/proc smart
./target/release/proc dns --tail &
./target/release/proc diag
./target/release/proc docker ps
./target/release/proc export --format json --limit 5

# 3. TUI 关键路径
./target/release/proc
# 进详情页 → F5 刷新 → 'y' 复制 → 'v' 切 env reveal
# Docker 面板 → Shift+R 重启容器
# 按 R 录屏 → 弹确认 → y 确认 → 操作 → R 停止 → replay

# 4. 跨平台（如可用）
# Linux: SSH 到 ubuntu-latest 验证 stub 行为
# macOS: 验证基础进程列表 + Docker
```

任何路径失败 → 修复后重新验证。

### 任务 10：打 tag + 推送 release

```bash
# 确认所有改动已 commit
git status   # 应该 clean

# 打 tag
git tag v0.6.0
git push origin v0.6.0   # 触发 release.yml workflow
```

**注意**：release.yml 是 `draft: true`，构建完成后需要用户在 GitHub Release 页面手动审核 + publish。

### 任务 11：验证 release CI 产出

GitHub Actions 跑完后：
1. 进入仓库 Releases 页面，应该有 draft release `v0.6.0`
2. 检查 5 个 target 的资产是否齐全：
   - `proc-x86_64-pc-windows-msvc.zip`
   - `proc-x86_64-unknown-linux-musl.tar.gz`
   - `proc-aarch64-unknown-linux-gnu.tar.gz`
   - `proc-aarch64-apple-darwin.tar.gz`
   - `proc-x86_64-apple-darwin.tar.gz`
3. 手动测试 `cargo binstall proc`（需先 publish release）
4. 手动测试 `winget install Alfroul.proc`（需 winget-pkgs PR 已合并）

### 任务 12：清理临时文件（如有）

```bash
# 删除开发期临时调试文件（perf.log / .db 等）
git status   # 确认 .gitignore 工作正常

# 删除 plan.md / CONTEXT.md / docs/handoff-*.md 等私有文件（如果不再需要）
# 注：CONTEXT.md / plan.md 在 .gitignore 中本来就不入仓
```

### 验收命令

```bash
cargo test --release --tb=no -q
cargo clippy --release --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build --release --no-default-features
cargo bench --bench sort_cache -- --baseline v0.5.0   # 性能对比（如有保存）

# 发版前最终检查
git log --oneline v0.5.0..HEAD   # 看 v0.6.0 周期所有 commit
git tag                          # 确认 v0.6.0 tag 存在
```

**验收标准**：
- 所有 P0/P1 问题已修复（REVIEW-7.md 中标记 Status: Fixed）
- 全量测试通过，无回归
- README.md 完整反映 v0.6.0 能力
- CHANGELOG.md 定稿（v0.5.0 段后追加 v0.6.0 段）
- Cargo.toml version = "0.6.0"
- 启动验证全部通过（CLI 子命令 + TUI 关键路径）
- tag v0.6.0 已打并推送
- release CI 触发，5 个 target 资产构建成功（draft 状态待用户 publish）
- P2 已归档到 docs/tech-debt.md

**主修改区域**：
- 全仓库散点（根据 REVIEW-7.md P0/P1 清单）
- `README.md`（重写或大改）
- `CHANGELOG.md`（v0.5.0 后加 v0.6.0 段）
- `Cargo.toml`（version = "0.6.0"）
- `docs/reviews/REVIEW-7.md`（标记 Status: Fixed）
- `docs/tech-debt.md`（归档 P2，如未在阶段 7 创建）

**完工后输出**：项目交付完成报告（无下一阶段启动指令包，因为这是最后阶段）。报告包含：
- v0.6.0 最终交付物清单（README / CHANGELOG / tag / 测试覆盖率 / tech-debt）
- 各阶段 commit 数 / 代码量统计
- 已知限制（来自 tech-debt）
- v0.7.0 路线图建议（来自 tech-debt）
