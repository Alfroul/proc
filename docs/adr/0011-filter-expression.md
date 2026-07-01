# ADR-0011: Filter Expression via nom parser + AST

## Status

**Accepted** — v0.7.0 阶段 4 引入

## Context

v0.6.0 进程列表搜索是纯子串匹配（`name.to_lowercase().contains(&query_lower)`，见 `src/view_models/process_panel.rs:248`），无法按字段/数值过滤。用户想"看 cpu > 5 且内存 < 100MB 的进程"或"名字匹配 chrome 正则的进程"都做不到。

bottom（同类 Rust TUI）支持 `cpu > 0.5 AND mem% < 10` / `read >= 1 mb` / `user = root` / 正则，生产力刚需。btop / glances / htop 也都有类似的字段过滤能力。

## Decision

**用 `nom 7-8` parser 组合子库，定义 `FilterExpr` AST + `FilterToken` 枚举，搜索框第一字符 `:` 切换到 FilterExpr 模式，否则保留 v0.6 子串 fallback。**

具体决策：

1. **库选 `nom 7`**（不用 pest / 不用 sqlparser-rs）
   - nom 是轻量 parser combinator 库，no_std 友好
   - pest 适合完整语法（如 SQL 子集、Markdown），对小型表达式过度
   - sqlparser-rs 是 SQL parser，语义和我们要的不一致（无字段映射）

2. **语法设计**（参考 bottom）：

   ```
   filter := or_expr
   or_expr := and_expr ("OR" and_expr)*
   and_expr := not_expr ("AND" not_expr)*
   not_expr := "NOT"? primary
   primary := "(" filter ")" | leaf
   leaf := field cmp_op value | field "=~" regex_literal

   field := "cpu" | "mem" | "pid" | "name" | "user" | "cmd"
         | "disk_read" | "disk_write" | "net_sent" | "net_recv" | "security_score"
   cmp_op := "=" | "!=" | ">" | "<" | ">=" | "<="
   value := number | number_with_unit | string_literal
   regex_literal := "/" pattern "/" "i"?
   ```

3. **AST 设计**（不引入脚本能力）：
   ```rust
   pub enum FilterExpr {
       FieldCmp { field: Field, op: CmpOp, value: Value },
       Regex { field: Field, re: regex::Regex, case_insensitive: bool },
       And(Box<FilterExpr>, Box<FilterExpr>),
       Or(Box<FilterExpr>, Box<FilterExpr>),
       Not(Box<FilterExpr>),
   }
   ```
   - 无变量绑定 / 无副作用 / 无循环（不是脚本语言）
   - 字段名编译期 enum 化（不允许任意字符串字段名）

4. **切换方式：搜索框第一字符 `:`**
   - 例：`:cpu > 5 AND name =~ /chrome/` 触发 FilterExpr
   - `chrome` 走原 SearchState substring
   - 两条路径在 `view_models/process_panel.rs::filter` 内 dispatch
   - 不破坏 v0.6 任何 substring 路径行为

5. **字段映射用 `match` 不用反射**：
   ```rust
   fn extract_field(p: &ProcessInfo, f: Field) -> FieldValue {
       match f {
           Field::Cpu => FieldValue::Num(p.cpu_usage as f64),
           Field::Mem => FieldValue::Num(p.memory_bytes as f64),
           Field::Pid => FieldValue::Num(p.pid as f64),
           Field::Name => FieldValue::Text(p.name.to_string()),
           // ...
       }
   }
   ```
   - 编译期保证字段名正确
   - 字段名 typo → 编译错

6. **单位支持**（数字字面量）：
   - `5` = 5（裸数字）
   - `5.5` = 5.5
   - `5b` / `5kb` / `5mb` / `5gb` / `5tb` = 1024 进制字节
   - `5%` = 5.0（百分比，与 cpu/mem 字段配合）

7. **错误处理友好**：
   - Parser 失败 → `ParseError { msg: String, position: usize }`
   - UI 显示 "⚠ Filter syntax error at position N: <msg>"
   - 不影响 cached_sorted：保留上一次成功的 AST 继续过滤

8. **CLI 接入**：`proc ls --filter 'cpu > 5 AND name =~ /chrome/'`
   - 与 TUI 同款 parser，CLI 走 `crate::filter::parse` 入口

## Alternatives Considered

### A. 用 pest（PEG parser generator）

**否决理由**：
- pest 需要单独 `.pest` 语法文件 + 编译时代码生成
- 对 ~50 行的小型表达式语法，nom 更直接（直接在 Rust 里写组合子）
- pest 适合 Markdown / JSON 这种完整语法的语言

### B. 用 sqlparser-rs（SQL 子集）

**否决理由**：
- SQL 语义过重（SELECT/FROM/WHERE/JOIN），proc 不需要
- 字段映射需要 runtime schema，与编译期 enum 化冲突
- 引入 ~2MB 依赖过重

### C. 沿用 v0.6 substring（不做表达式）

**否决理由**：
- bottom / btop / glances 都有表达式过滤，用户期望
- 进程数 500+ 时 substring 找不到精确目标（如"只看 chrome 主进程，不看 GPU 进程"）

### D. 引入 JavaScript / Lua 引擎（rhai / mlua）

**否决理由**：
- 引入脚本能力 = 引入安全风险（用户可写任意脚本）
- 命令注入风险（如果脚本 spawn 子进程）
- 编译时无法静态分析表达式正确性

### E. 完全替换 substring（不留 fallback）

**否决理由**：
- 简单搜索场景（如只敲 `chrome`）用户期望直接 substring
- 强制 `:name =~ /chrome/` 太啰嗦

## Consequences

### 正面

- **生产力**：bottom 式过滤，用户能精确定位进程
- **CLI 与 TUI 一致**：`proc ls --filter` 和 TUI 内 `:filter` 用同一 parser
- **编译期字段安全**：enum 化字段名，typo 编译错
- **不破坏 v0.6**：substring 路径完全保留

### 负面

- **依赖增加**：nom ~200KB + regex（可能已有）
- **SearchState 加 mode 字段**：`enum QueryMode { Substring, FilterExpr(FilterExpr) }`
- **cached_sorted 缓存键扩展**：原 `(sort_field, query)` → `(sort_field, query, mode)`，mode 变化触发重建
- **parser 维护成本**：~300 行 parser 代码 + 20+ 测试 case

### 缓解

- nom 是成熟库，社区文档充足
- AST 是简单枚举，没有递归下降的隐藏复杂度
- parser 单元测试 20+ case 覆盖合法/非法/边界

### v0.8.0 阶段 2 增量：错误信息中文化（TD-16）

nom 默认错误信息直出内部 `ErrorKind` 枚举名（`TakeWhile1` / `Tag` / `AlphaNumeric` 等），中文用户看不懂。v0.8.0 阶段 2 加两层中文映射：

- `error_kind_to_chinese(&ErrorKind) -> &'static str`：覆盖 parser 实际会触达的 9 个 ErrorKind 变体，未匹配兜底「语法错误」。
- `char_to_chinese(char) -> &'static str`：括号 / 引号 / 斜杠给出语义化提示（「缺少括号」/「缺少引号」/「缺少斜杠」），其他字符回退「缺少字符」。

同时把 `paren_expr` 的闭合 `)` 改成 `cut(char(')'))` —— 括号闭合失败转 `Err::Failure`，alt 不再回退到 leaf。否则 `(cpu > 5` 缺 `)` 时 alt 会 fallback，最内层错误从 `Char(')')` 退化为 leaf 的 `TakeWhile1`，丢失「括号没闭合」这条真正有用的提示。

代价：parser 测试需要锁死中文映射对外的字面量（`tests/test_filter_expr.rs::err_chinese_*`），映射表改名时需要同步更新。

### v0.8.0 阶段 3 增量：FilterExpr 扩 Tree / AppGroup view（TD-15）

v0.7 阶段 4 FilterExpr 只接入 List view；Tree / AppGroup 视图保留 substring。v0.8 阶段 3 把两个视图同款接入：

- **Tree view**：`get_filtered_tree_visible(&self, cached_processes: &[ProcessInfo])` 加 `cached_processes` 参数。FilterExpr 分支建 `pid → &ProcessInfo` HashMap，按 visible TreeNode 的 pid 取原始 ProcessInfo 再 `FilterExpr::apply`。Substring 分支保留 v0.6 name.lower().contains 行为。
- **AppGroup view**：`app_group_filtered_visual_items(&self, cached_processes: &[ProcessInfo])` 同款扩参。FilterExpr 分支两套 apply 语义：
  - **Header 项（聚合）**：用 group 的 `total_cpu` / `total_memory` + `display_name` 构造合成 ProcessInfo，apply 时 `cpu > 50` 表示「该 .exe 总 cpu > 50」。Header 命中 → 整组保留。
  - **Child 项（单进程）**：Header 不命中时按 pid 查 cached_processes 取原始 ProcessInfo，命中的 child 保留并自动展开该组。

设计取舍：选「传 cached_processes 参数」（不动 TreeNode / AppGroupProcess 结构），保持 v0.7 阶段 5 拆分边界（ADR-0012）。AppGroup Header 用合成 ProcessInfo（`..ProcessInfo::default()`）而非新增字段，避免 AppGroupProcess 持 `Arc<ProcessInfo>` 引发构造点 + 序列化连锁修改。

代价：内部 helper（`tree_move_cursor` / `tree_toggle_select` / `tree_initiate_kill` / `tree_select_orphans` / `tree_select_stale` / `app_group_move_cursor` / `app_group_toggle_expand` / `app_group_toggle_select` / `app_group_initiate_kill`）签名都加 `cached_processes: &[ProcessInfo]`；外部调用点（`src/app.rs::handle_scroll` / `src/tui/process_tree.rs::draw` / `src/tui/app_group_view.rs::draw`）传 `&app.cached_processes[..]`。

### v0.11.0 阶段 3 增量：FilterExpr v2 网络字段（ADR-0011 v2）

v0.10 阶段 3 落地的 `ProcessFlow.sni` / `dns_name` / `remote_addr` 字段没有过滤入口，用户在 Flow 子视图（按 `F` 进入）只能逐行扫；CLI `proc flows` 也没法按字段过滤。v0.11 阶段 3 把 FilterExpr AST 扩出网络字段，让 Flow 视图（TUI + CLI）与 List / Tree / AppGroup 视图享有同款过滤体验。

具体增量：

1. **新枚举 `NetworkField`**：`Sni` / `DnsName` / `RemoteAddr` / `RemotePort` / `BytesOut` / `BytesIn` / `Source`。与现有 `Field`（process 系）平级，不合并。`is_text()` / `extract(&NetworkEvalCtx) -> FieldValue` 同款 API。

2. **三个 Network 变体加进 `FilterExpr`**：`NetworkFieldCmp { field, op, value }` / `NetworkRegex { field, re, case_insensitive }` / `NetworkIn { field, values }`。`NetworkIn` 是 NetworkField 独有的（`in` 操作符），process 系暂不支持（surgical：仅在 Flow 视图要求）。

3. **`NetworkEvalCtx`**：与 `EvalCtx` 平级的新 ctx，持 `&ProcessFlow` 而非 `&ProcessInfo`。

4. **`FilterExpr::apply_network(&NetworkEvalCtx) -> bool`**：与 `apply` 对称的执行入口。And/Or/Not 递归两边都走 apply_network；process 系变体（`FieldCmp` / `Regex`）在本 ctx 下返 false（NetworkEvalCtx 不持 ProcessInfo）；network 系变体在 `apply`（process ctx）下也返 false。这样类型系统保证字段不会跨 ctx 误用。

5. **设计取舍：分两个变体而非合并到 `FieldCmp`**：ProcessField 作用于 `&ProcessInfo`（List view），NetworkField 作用于 `&ProcessFlow`（Flow view），**作用对象不同**。合并需要泛型 Field / Value 提取器，复杂度跳一档；分两个 ctx + 两个 apply 方法让类型系统保证字段不会跨 ctx 误用——`sni =~ /evil/` 在 List 视图（无 flow）不会拿到 flow 数据，因为 List 视图根本不调 apply_network。

6. **Parser 扩展**：`parse_field` 返回 `ParsedField` 枚举（Process(Field) | Network(NetworkField)），dispatch 在 `leaf` 内部按 ParsedField 变体构造对应 FilterExpr 变体。`in` 操作符走新 `parse_in_list` parser（`("(" val ("," val)* ")"`），闭合 `)` 用 `cut` 让错误不被 alt 吞。

7. **错误中文化增强**：未知字段错误（`AlphaNumeric` ErrorKind）锚点从 `after_ident` 改回 `i`（字段名起点），`to_parse_error` 在该 ErrorKind 上从 `input[off..]` 提取未知标识符拼出友好提示：「未知字段名：xxx（支持 cpu/mem/.../sni/dns_name/remote_addr/...）」。原 v0.8 阶段 2 的 `error_kind_to_chinese` 兜底映射保留。

8. **CLI 接入**：`proc flows --filter '<expr>'` 与 `proc ls --filter` 同款 parser，但走 `apply_network`。语义：先 collect 全部 → filter → truncate limit（典型「找前 N 个命中 X 的 flow」）。

9. **TUI 接入**：PortPanel 加 `flow_search: SearchState` 字段。Flow 子视图（`F` 进入）按 `:` 激活 FilterExpr 模式（与 List / Tree / AppGroup 视图同款 UI 契约）；`flow_filtered_indices(flows)` 按 mode 分支返回可见索引；标题栏显示当前表达式 + parse 错误（与 List view 同款）。

代价：

- FilterExpr enum 加 3 个变体；NetworkField enum 7 个变体；mod.rs +90 行；parser.rs +120 行（含 in 操作符 + 测试）。
- PanelContext 加 `flows: &'a [ProcessFlow]` 字段，App 两处 PanelContext 构造点同步。
- PortPanel 加 `flow_search` 字段 + `flow_filtered_indices` 方法（`pub`，集成测试需要）。
- `flow_clamp_cursor` 调用点改用过滤后总数（搜索 / FilterExpr 收窄后光标不越界）。
- 新增 `tests/test_filter_expr_v2.rs`（25 case）覆盖 parser + apply_network + PortPanel 集成。

## Implementation Notes

- 入口：`src/filter/mod.rs::FilterExpr::apply(&ProcessInfo) -> bool`
- v0.11 阶段 3：`src/filter/mod.rs::FilterExpr::apply_network(&NetworkEvalCtx) -> bool`
- Parser：`src/filter/parser.rs::parse(&str) -> Result<FilterExpr, ParseError>`
- SearchState 接入：`src/search.rs::SearchState.mode: QueryMode`
- ProcessPanel dispatch：`src/view_models/process_panel.rs::filter`（match mode 分支）
- v0.11 阶段 3 Flow 子视图接入：`src/view_models/port_panel.rs::{handle_flow_view_key, flow_filtered_indices}`
- CLI 接入：`src/cli/ls.rs::run_ls(--filter <expr>)`
- v0.11 阶段 3 CLI 接入：`src/cli/flows.rs::run_flows(filter: Option<&str>)`
- 测试：`tests/test_filter_expr.rs`（15+ case）+ `tests/test_filter_expr_v2.rs`（v0.11 阶段 3 新增 25 case）

## Examples

```text
# 简单字段比较
cpu > 5
mem > 100mb
security_score < 80

# 字符串相等
user = root
name = chrome.exe

# 正则
name =~ /chrome/i
cmd =~ /--headless/

# 布尔组合
cpu > 5 AND mem < 100mb
(cpu > 50 OR mem > 500mb) AND NOT user = root
name =~ /chrome/i OR name =~ /firefox/i

# 切换语法
:cpu > 5          # FilterExpr 模式
chrome            # Substring 模式（v0.6 兼容）
```

## References

- [nom 7 docs](https://docs.rs/nom)
- [bottom filter syntax](https://github.com/ClementTsang/bottom/blob/main/docs/content/usage/general-usage.md)
- proc v0.6.0 `src/view_models/process_panel.rs:248`（substring 现状）
- proc v0.6.0 `src/search.rs::SearchState`（mode 字段加在这里）
