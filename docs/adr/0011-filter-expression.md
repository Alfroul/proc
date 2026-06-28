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

## Implementation Notes

- 入口：`src/filter/mod.rs::FilterExpr::apply(&ProcessInfo) -> bool`
- Parser：`src/filter/parser.rs::parse(&str) -> Result<FilterExpr, ParseError>`
- SearchState 接入：`src/search.rs::SearchState.mode: QueryMode`
- ProcessPanel dispatch：`src/view_models/process_panel.rs::filter`（match mode 分支）
- CLI 接入：`src/cli/ls.rs::run_ls(--filter <expr>)`
- 测试：`tests/test_filter_expr.rs`（15+ case）

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
