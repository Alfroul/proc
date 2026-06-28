# ADR-0010: Shell completion via clap_complete + Command palette via nucleo

## Status

**Accepted** — v0.7.0 阶段 3 引入

## Context

v0.6.0 的 proc 已经有 17+ CLI 子命令 + 6 面板 + 6 Tab + 14 项安全检查 + 9 字段排序，键位已经多到 `?` 帮助页拥挤。两个独立但相关的 UX 问题：

1. **Shell completion 缺失**：用户敲 `proc d<tab>` 不补全 `docker/dns/diag`。Cargo.toml 没引 `clap_complete`，release CI 不产出 completion 文件。同类工具（lazydocker / bottom / glances）都有。

2. **键位爆炸**：每个面板都有 5-10 个键位，详情页 6 个 Tab 各自有快捷键。新增功能（v0.7 各阶段）会继续加键位。`?` 帮助页能列出来，但用户记不住也不愿学。

## Decision

**两个独立子决策：**

### A. Shell completion：用 `clap_complete 4`，子命令在线生成（不用 build.rs）

具体决策：

1. **库选 `clap_complete 4`**
   - clap 官方配套，与 clap 4 API 100% 兼容
   - 支持 bash / zsh / fish / powershell / elvish 5 种 shell

2. **`proc completions --shell <SHELL>` 子命令**（在线生成，不用 build.rs）
   - 理由：build.rs 编译时耦合 clap_complete 到 build 阶段，让 cargo build 不必要的依赖增加
   - 在线生成更灵活：用户可只生成自己用的 shell

3. **release CI 打包预生成 completion**
   - `completions/` 目录下放 4 个文件（bash / zsh / fish / powershell），让 winget / scoop 包默认带上
   - 用户也可手动 `proc completions --shell bash > ~/.bash_completion.d/proc`

4. **不动态生成**（动态生成需要 proc 在 shell 启动时跑一次，慢）
   - 静态预生成 + 用户自己 `source` 是标准做法

### B. 命令面板 Ctrl+P：用 `nucleo` fuzzy + `tui-input`，modal 浮层

具体决策：

1. **库选 `nucleo`**（Helix 编辑器 fuzzy 库）
   - 性能极佳（Helix 大文件 fuzzy 都靠它）
   - rust-code（已知项目）用 nucleo 做文件搜索 Ctrl+P，验证可行

2. **库选 `tui-input 0.15`**（150 万下载）
   - 标准 ratatui 输入库，支持 unicode / 多 backend

3. **modal 浮层 + AppLayer 状态机**
   - 新增 `enum AppLayer { Normal, Search, Palette }`
   - Ctrl+P 激活：`active_layer = Palette`，拦截所有按键不传给面板/搜索/详情页
   - Esc / Enter 退出回 Normal
   - 解决 v0.6 单层 dispatch 引发的键位冲突（如详情页 `c` vs 全局 `c`）

4. **不引入 trait object**（CommandItem 都是具体类型）
   - 性能 + 编译期类型安全
   - 30-50 个命令项 hardcode 在 source，新增功能时手动注册

5. **不替代 `?` 帮助页**
   - 命令面板是"快速执行"，帮助页是"学习键位"，两者互补

## Alternatives Considered

### A. Shell completion

#### A1. 用 build.rs 编译时生成

**否决理由**：
- 增加 build 时依赖（clap_complete 进 build-deps）
- 每次编译都重新生成（即使是 `cargo check`）
- 编译时生成的 completion 文件没地方放（只能写到 `OUT_DIR`，用户拿不到）

#### A2. 引 `clap_complete_nushell` / `clap_complete_fig` 等扩展

**否决理由**：
- nushell / fig 用户量小，v0.7 不必要
- 标准 5 shell 覆盖 99% 用户

#### A3. 手写 completion 脚本

**否决理由**：
- 维护成本高（每次新增子命令都要手改 5 个文件）
- 容易和 clap derive 不同步

### B. 命令面板

#### B1. 用 ratatui-windowed / tui-rs-popup

**否决理由**：
- ratatui-windowed 不存在（编造）
- tui-popup 是简单浮层，无 fuzzy
- 命令面板需要 fuzzy + 输入框 + 列表，浮层只是基础

#### B2. 用 trait object + Box<dyn CommandItem>

**否决理由**：
- 命令面板每帧 fuzzy 1000 项，trait object 的 v-table dispatch 不必要
- 编译期类型安全更好

#### B3. 替代 `?` 帮助页

**否决理由**：
- 帮助页是"学习键位"（用户主动查），命令面板是"快速执行"（用户已知道想干什么）
- 两者用途不同，互补不替代

#### B4. 不做命令面板，继续加键位 + 扩帮助页

**否决理由**：
- 键位空间已耗尽（`1`-`6` 面板 / `a`-`z` 大部分被占 / `Ctrl+*` 部分 / `Shift+*` 部分）
- v0.7+ 还有更多功能要加（PSI / EcoQoS / Flow 子视图 / FilterExpr 模式切换）
- 用户记忆负担过重

## Consequences

### 正面

- **completion**：用户敲 tab 自动补全，是成熟 CLI 的标志
- **命令面板**：解决键位爆炸，新功能加命令项而非加键位
- **AppLayer 状态机**：为未来扩展（如自定义 modal）打好基础
- **可发现性**：用户可在命令面板里 fuzzy 搜"kill"，不需要记 `k` / `K` / `Space+k` 的区别

### 负面

- **依赖增加**：nucleo ~200KB + tui-input ~50KB + clap_complete ~100KB
- **App::handle_key 加一层 dispatch**：先 match AppLayer，再加一层间接
- **命令项维护**：新增功能需要手动注册 CommandItem（30-50 项）

### 缓解

- nucleo 性能足以实时 fuzzy 1000 项（Helix 验证）
- AppLayer 是 enum，match 编译期检查穷尽
- 命令项注册集中在一处（`src/tui/command_palette.rs::default_items()`），新增时改一处

## Implementation Notes

- Shell completion 入口：`src/cli/completions.rs::run_completions(shell)`
- 命令面板：`src/tui/command_palette.rs::{CommandPalette, CommandItem, CommandAction}`
- AppLayer：`src/app_panel.rs::AppLayer`
- 测试：`tests/test_command_palette.rs`（7 个 case，含与现有搜索的互斥）
- 预生成 completion：`completions/` 目录下 4 文件，release.yml 打包

## References

- [clap_complete docs](https://docs.rs/clap_complete)
- [nucleo (Helix fuzzy)](https://github.com/helix-editor/nucleo)
- [tui-input](https://crates.io/crates/tui-input)
- [rust-code Ctrl+P 实现参考](https://github.com/fortunto2/rust-code)
- VSCode / zed 命令面板交互参考
