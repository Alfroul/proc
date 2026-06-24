# 阶段 1：架构 Spike — 文档 + 发布基础设施

> **独立会话指令**：阅读 CONTEXT.md 和 docs/stages/stage-1.md，完成所有任务后确认完成
>
> **开工前只需阅读**：项目结构、CONTEXT.md、本文件、测试命令。不需要读其他阶段。

**目标**：铺设发布与文档基础设施，让 v0.6.0 后续阶段产出的代码可被 cargo-binstall / winget / scoop 安装；让 ADR 文档入仓，贡献者可读。

**前置依赖**：无（v0.5.0 已发布，基线 611 测试 / 0 warnings）

**预期代码量**：~400 行（主要是 CI yml + 文档；少量 Rust 代码改动只有 Cargo.toml metadata）

**任务清单**：

### 1. 改 `.gitignore`（已预先完成，验收时确认）

本会话开工前已改好：
- 移除 `docs/` 整体排除
- 改为只排除 `docs/handoff-*.md`（Checkpoint 文件）和 `docs/internal/`（如有）
- `CONTEXT.md` / `plan.md` 保持私有

验收命令：
```bash
git check-ignore docs/adr/0008-self-mitigation-policy.md  # 应该没有输出（未忽略）
git check-ignore docs/handoff-stage-5-checkpoint.md       # 应该输出该路径（已忽略）
git check-ignore CONTEXT.md                                # 应该输出该路径（已忽略）
```

### 2. ADR 文档入仓（已预先完成 8 个，验收时确认）

已写入 `docs/adr/`：
- `README.md`（索引）
- `0001-phased-project-adoption.md` ~ `0007-container-exec-pty-bridge.md`（从 CHANGELOG 引用回填）
- `0008-self-mitigation-policy.md`（v0.6.0 新决策，Status: Proposed → 阶段 2 落地后改 Accepted）

验收命令：
```bash
ls docs/adr/ | wc -l   # 应该输出 9（README + 0001-0008）
grep "Status: Accepted" docs/adr/0001-*.md docs/adr/0002-*.md docs/adr/0003-*.md docs/adr/0004-*.md docs/adr/0005-*.md docs/adr/0006-*.md docs/adr/0007-*.md  # 7 个 Accepted
grep "Status: Proposed" docs/adr/0008-*.md  # 1 个 Proposed（阶段 2 落地后改 Accepted）
```

### 3. SECURITY.md（仓库根目录）

新建 `SECURITY.md`：
- Supported Versions（仅最新 release）
- Reporting a Vulnerability（邮箱占位 `<待用户填>`，72 小时响应承诺）
- Privilege Model（默认无特权；elevated 时持 SeDebugPrivilege；v0.6.0 阶段 2 起子进程剥离）
- Hardening（v0.6.0+ 启动调 `apply_self_mitigations`：DEP/ASLR/ProhibitDynamicCode/DisableExtensionPoints，见 ADR-0008）
- 已知限制：未开 ProcessSignaturePolicy（兼容 nvml-wrapper）

参考模板：[GitHub SECURITY.md 推荐](https://docs.github.com/en/code-security/getting-started/adding-a-security-policy-to-your-repository)

### 4. CONTRIBUTING.md（仓库根目录）

新建 `CONTRIBUTING.md`：
- 开发环境（Rust 1.85+ / Windows 主开发平台 / Linux 可降级）
- 提交流程：
  - `cargo test --release` 全绿
  - `cargo clippy --release --all-targets -- -D warnings` 0 warnings
  - `cargo fmt --all -- --check` 干净
  - `cargo build --release --no-default-features` 编译通过
- Commit message 风格（看 CHANGELOG 的 Added/Changed/Fixed）
- ADR 流程（新决策追加 `docs/adr/NNNN-标题.md`；推翻旧决策改 Status 为 `Superseded by ADR-NNNN`）
- 分阶段开发：见 `plan.md`（私有）/ `docs/stages/`
- Issue / PR 模板（简单说明）

### 5. `.github/workflows/release.yml`（新增）

新建 release workflow，tag 触发，cross 构建 5 个 target：

```yaml
name: release
on:
  push:
    tags: ['v*']
  workflow_dispatch:

permissions:
  contents: write

jobs:
  build:
    strategy:
      fail-fast: false
      matrix:
        include:
          - { os: windows-latest, target: x86_64-pc-windows-msvc, archive: zip, archive_cmd: '7z a -tzip' }
          - { os: ubuntu-latest,  target: x86_64-unknown-linux-musl, archive: tar.gz, archive_cmd: 'tar czf' }
          - { os: ubuntu-24.04-arm, target: aarch64-unknown-linux-gnu, archive: tar.gz, archive_cmd: 'tar czf' }
          - { os: macos-14,       target: aarch64-apple-darwin, archive: tar.gz, archive_cmd: 'tar czf' }
          - { os: macos-13,       target: x86_64-apple-darwin, archive: tar.gz, archive_cmd: 'tar czf' }
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - uses: Swatinem/rust-cache@v2
        with:
          key: release-${{ matrix.target }}
      - name: Install musl tools (linux)
        if: matrix.target == 'x86_64-unknown-linux-musl'
        run: sudo apt-get install -y musl-tools
      - name: Build
        run: cargo build --release --target ${{ matrix.target }}
      - name: Package
        shell: bash
        run: |
          mkdir -p proc-${{ matrix.target }}
          cp target/${{ matrix.target }}/release/proc* proc-${{ matrix.target }}/
          # Windows 二进制是 proc.exe，Linux/mac 是 proc
          # 用通配符 proc* 同时覆盖两种情况，但要清理掉非二进制（如有）
          cp README.md LICENSE CHANGELOG.md proc-${{ matrix.target }}/
          ${{ matrix.archive_cmd }} proc-${{ matrix.target }}.${{ matrix.archive }} proc-${{ matrix.target }}
      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: proc-${{ matrix.target }}
          path: proc-${{ matrix.target }}.${{ matrix.archive }}
      - name: Upload to Release
        if: startsWith(github.ref, 'refs/tags/')
        uses: softprops/action-gh-release@v2
        with:
          files: proc-${{ matrix.target }}.${{ matrix.archive }}
          generate_release_notes: true
          draft: true   # 草稿状态，给用户审核后手动 publish
```

**注意**：`fail-fast: false` 让单个 target 失败不阻断其他；`draft: true` 让用户手动审核后 publish（防误发）。

### 6. `Cargo.toml` 加 cargo-binstall metadata

在 `[package]` 段后追加：

```toml
[package.metadata.binstall]
pkg-url = "{ repo }/releases/download/{ version }/proc-{ target }{ archive-suffix }"
bin-dir = "proc-{ target }/proc{ binary-ext }"
pkg-fmt = "tar.gz"

[package.metadata.binstall.overrides.x86_64-pc-windows-msvc]
pkg-fmt = "zip"
```

验收命令：
```bash
cargo install cargo-binstall
cargo binstall --dry-run proc    # 应该能解析 metadata，不实际下载（仓库未发 release 时报 404 是正常）
```

### 7. winget manifest 模板

新建 `winget-pkgs-templates/Alfroul.proc.template.yaml`（占位，正式版用户自己 PR 到 microsoft/winget-pkgs）：

```yaml
# Winget Manifest Template for Alfroul.proc
# 实际 PR 时复制到 microsoft/winget-pkgs/manifests/p/proc/Alfroul/<version>/
PackageIdentifier: Alfroul.proc
PackageName: proc
Publisher: Alfroul
License: MIT
ShortDescription: 交互式 TUI 系统进程管理器
Tags:
  - tui
  - process-manager
  - sysinfo
  - monitoring
  - terminal
Installers:
  - Architecture: x64
    InstallerType: zip
    InstallerUrl: https://github.com/Alfroul/proc/releases/download/vVERSION/proc-x86_64-pc-windows-msvc.zip
    InstallerSha256: REPLACE_AT_RELEASE
    NestedInstallerType: portable
    NestedInstallerFiles:
      - RelativeFilePath: proc-x86_64-pc-windows-msvc/proc.exe
        PortableCommandAlias: proc
```

### 8. scoop bucket

新建 `scoop/proc.json`：

```json
{
    "version": "0.6.0",
    "description": "交互式 TUI 系统进程管理器",
    "homepage": "https://github.com/Alfroul/proc",
    "license": "MIT",
    "architecture": {
        "64bit": {
            "url": "https://github.com/Alfroul/proc/releases/download/v0.6.0/proc-x86_64-pc-windows-msvc.zip",
            "hash": "REPLACE_AT_RELEASE"
        }
    },
    "bin": "proc-x86_64-pc-windows-msvc/proc.exe",
    "checkver": "github",
    "autoupdate": {
        "architecture": {
            "64bit": {
                "url": "https://github.com/Alfroul/proc/releases/download/v$version/proc-x86_64-pc-windows-msvc.zip"
            }
        }
    }
}
```

### 9. 在 release.yml 里追加自动 PR winget 的 step（可选，但推荐）

在 `release.yml` 末尾加：
```yaml
  update-winget:
    needs: build
    if: startsWith(github.ref, 'refs/tags/')
    runs-on: ubuntu-latest
    steps:
      - uses: russellbanks/release-automation-winget@v1
        with:
          identifier: Alfroul.proc
          version: ${{ github.ref_name }}
          installers: https://github.com/Alfroul/proc/releases/download/${{ github.ref_name }}/proc-x86_64-pc-windows-msvc.zip
          token: ${{ secrets.WINGET_TOKEN }}
```

注：用户需手动在仓库 Settings → Secrets 配置 `WINGET_TOKEN`（一个有 winget-pkgs PR 权限的 PAT）。

### 10. 更新 README.md

在 README.md 「快速开始」段加：
```markdown
## 安装

```bash
# 方式 1：cargo binstall（5 秒装预编译版，推荐）
cargo install cargo-binstall
cargo binstall proc

# 方式 2：从源码编译（5 分钟）
git clone https://github.com/Alfroul/proc.git
cd proc
cargo build --release
./target/release/proc

# 方式 3：Windows 包管理器（v0.6.0+）
winget install Alfroul.proc
scoop install proc
```
```

### 11. CHANGELOG 追加 v0.6.0 Unreleased 段

在 `## [Unreleased]` 下追加：
```markdown
## [Unreleased] — v0.6.0

### 阶段 1 — 文档 + 发布基础设施

- Added: `docs/adr/` 入仓（0001-0007 从私有 docs 移入 + 0008-self-mitigation-policy 新增 Proposed）
- Added: `SECURITY.md`（vulnerability reporting policy + privilege model + hardening 说明）
- Added: `CONTRIBUTING.md`（开发流程 + 提交规范 + ADR 流程）
- Added: `.github/workflows/release.yml`（tag 触发，cross 构建 5 个 target：win-x64 / linux-musl / linux-arm64 / macos-arm64 / macos-x86_64）
- Added: `Cargo.toml` `[package.metadata.binstall]`（cargo-binstall 支持）
- Added: `scoop/proc.json` + `winget-pkgs-templates/Alfroul.proc.template.yaml`
- Changed: `.gitignore` 放行 `docs/`（保留 `docs/handoff-*.md` 私有）
- Changed: README.md「快速开始」段加 binstall / winget / scoop 安装方式
```

### 12. 验收测试

```bash
# 1. 全量回归（不破坏既有功能）
cargo test --release --tb=no -q                                  # 应 611 passed
cargo clippy --release --all-targets -- -D warnings             # 应 0 warnings
cargo fmt --all -- --check                                       # 应干净
cargo build --release --no-default-features                      # 应编译通过

# 2. CI workflow 语法验证
# 用 actionlint（如装了）：
actionlint .github/workflows/release.yml

# 3. cargo-binstall metadata 验证（仓库无 release，会 404，但能解析说明 metadata 正确）
cargo binstall --dry-run proc 2>&1 | grep "Resolved"

# 4. SECURITY.md / CONTRIBUTING.md 存在
test -f SECURITY.md && test -f CONTRIBUTING.md && echo "OK"
```

### 13. 提交并打阶段完成标记

```bash
git add docs/ SECURITY.md CONTRIBUTING.md .github/workflows/release.yml \
        scoop/ winget-pkgs-templates/ Cargo.toml README.md CHANGELOG.md .gitignore
git commit -m "$(cat <<'EOF'
docs(stage-1): v0.6.0 阶段 1 — 文档 + 发布基础设施

- ADR 入仓: 0001-0007 + 0008-self-mitigation-policy (Proposed)
- SECURITY.md + CONTRIBUTING.md
- release.yml: 5 target cross build (win/linux-musl/linux-arm/macos-arm/macos-intel)
- cargo-binstall metadata
- scoop manifest + winget template
EOF
)"
```

**验收标准**：
- 全量回归 611 passed / 0 failed
- clippy / fmt / no-default-features 编译通过
- `docs/adr/` 入仓且 `git status` 显示这些文件不再被忽略
- SECURITY.md / CONTRIBUTING.md 存在且内容完整
- release.yml 通过 actionlint 校验（或至少 yaml 语法合法）
- cargo-binstall metadata 已配置
- README.md 安装段已更新
- CHANGELOG.md Unreleased 段已加阶段 1 内容
