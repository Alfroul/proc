//! v0.7.0 阶段 4：进程列表过滤表达式（ADR-0011）。
//!
//! 替代 v0.6.0 的纯子串搜索。语法参考 bottom：
//!
//! ```text
//! cpu > 5 AND name =~ /chrome/i
//! mem > 100mb OR security_score < 80
//! NOT (user = root) AND (cpu > 50 OR mem > 500mb)
//! ```
//!
//! 切换方式：搜索框第一字符 `:` → FilterExpr 模式；否则保留 v0.6 substring fallback。
//! 入口：[`crate::filter::parser::parse`]。
//!
//! **v0.11.0 阶段 3 增量**：加 [`NetworkField`] / [`NetworkEvalCtx`] / 三个
//! Network 变体，让 Flow 子视图（ProcessFlow 列表）也能走 FilterExpr：
//!
//! ```text
//! sni =~ /google\.com$/
//! dns_name in ("a.com", "b.com")
//! remote_addr = 1.2.3.4 AND source = schannel
//! ```
//!
//! **v0.14.0 阶段 3 增量**：加 [`FrameField`] / [`FrameEvalCtx`] / 三个 Frame
//! 变体，让录屏时间轴搜索（[`crate::record::UiFrame`]）也能走 FilterExpr。
//! 入口 [`crate::filter::parser::parse_frame`]（Frame 模式，`cpu`/`mem` 解析
//! 成 [`FrameField`]；既有 [`crate::filter::parser::parse`] 走 Process 模式
//! 不变，向后兼容 List / Tree / Flow 视图）：
//!
//! ```text
//! cpu > 80
//! mem > 500mb
//! timestamp > 1234567890
//! name =~ /chrome/i
//! anomaly.severity = critical
//! ```

pub mod parser;

pub use parser::{ParseError, parse, parse_frame};

use std::collections::HashSet;

use crate::collect::ProcessInfo;
use crate::flow::ProcessFlow;
use crate::record::UiFrame;

/// 过滤表达式 AST。每个节点是一个布尔判定，[`FilterExpr::apply`] 是执行入口。
///
/// v0.11 阶段 3 加三个 Network 变体（[`Self::NetworkFieldCmp`] /
/// [`Self::NetworkRegex`] / [`Self::NetworkIn`]），作用对象是
/// [`ProcessFlow`]（NetworkEvalCtx）而非 [`ProcessInfo`]（EvalCtx）。
///
/// `And` / `Or` / `Not` 走 `Box` 避免 enum 自引用无限大小。
#[derive(Debug, Clone)]
pub enum FilterExpr {
    /// 字段比较：`field op value`，例 `cpu > 5`。作用对象 [`ProcessInfo`]。
    FieldCmp {
        field: Field,
        op: CmpOp,
        value: Value,
    },
    /// 正则匹配：`field =~ /pattern/i`。作用对象 [`ProcessInfo`]。
    Regex {
        field: Field,
        re: regex::Regex,
        case_insensitive: bool,
    },
    /// v0.11 阶段 3：网络字段比较 `field op value`，例 `remote_port = 443`。
    /// 作用对象 [`ProcessFlow`]（NetworkEvalCtx）。
    NetworkFieldCmp {
        field: NetworkField,
        op: CmpOp,
        value: Value,
    },
    /// v0.11 阶段 3：网络字段正则 `field =~ /pattern/i`，例 `sni =~ /google\.com$/`。
    NetworkRegex {
        field: NetworkField,
        re: regex::Regex,
        case_insensitive: bool,
    },
    /// v0.11 阶段 3：网络字段集合包含 `field in ("a", "b", ...)`，例
    /// `sni in ("a.com", "b.com")`。HashSet 查询语义。
    ///
    /// **v0.12 阶段 5（TD-29）**：内部存储从 `Vec<Value>` 改为 `HashSet<Value>`
    /// 让 apply 路径 `contains` 是 O(1) 而非 O(N)。Parser 在 `parse_in_list` 一次
    /// 性构造 HashSet；用户写 100 个值 × 1000 flows 的极端场景从 100_000 比较
    /// 降到 1_000 hash 查找。
    NetworkIn {
        field: NetworkField,
        values: HashSet<Value>,
    },
    /// v0.14 阶段 3：录屏帧字段比较，例 `cpu > 80` / `timestamp > 1234567890`。
    /// 作用对象 [`UiFrame`]（FrameEvalCtx）。文本字段（Name / AnomalySeverity）
    /// 走「帧内集合任一匹配」语义（`any_match`），数值字段（Timestamp / Cpu / Mem）
    /// 走 extract_first + apply_num。
    FrameFieldCmp {
        field: FrameField,
        op: CmpOp,
        value: Value,
    },
    /// v0.14 阶段 3：录屏帧字段正则匹配，例 `name =~ /chrome/i`。仅文本字段
    /// （Name / AnomalySeverity）支持；数值字段在该变体下永远返 false。
    FrameRegex {
        field: FrameField,
        re: regex::Regex,
        case_insensitive: bool,
    },
    /// v0.14 阶段 3：录屏帧字段 in 集合，例 `name in ("chrome", "edge")` /
    /// `anomaly.severity in ("critical", "warning")`。文本字段 only。
    FrameIn {
        field: FrameField,
        values: HashSet<Value>,
    },
    And(Box<FilterExpr>, Box<FilterExpr>),
    Or(Box<FilterExpr>, Box<FilterExpr>),
    Not(Box<FilterExpr>),
}

/// 进程字段。编译期 enum 化（不允许任意字符串字段名），typo → 编译错。
///
/// 文本字段：`Name` / `User` / `Cmd`（值走 [`FieldValue::Text`]）。
/// 数值字段：其余 8 项（值走 [`FieldValue::Num`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Cpu,
    Mem,
    Pid,
    Name,
    User,
    Cmd,
    DiskRead,
    DiskWrite,
    NetSent,
    NetRecv,
    SecurityScore,
}

impl Field {
    /// 文本字段（Name/User/Cmd）走 [`FieldValue::Text`] 路径；其余为数值。
    #[must_use]
    pub fn is_text(self) -> bool {
        matches!(self, Self::Name | Self::User | Self::Cmd)
    }

    /// 从 ProcessInfo 取值。SecurityScore 不在 ProcessInfo 里（在 App::security_scores
    /// HashMap），通过 `ctx.security_score` 传入。
    #[must_use]
    pub fn extract(self, ctx: &EvalCtx<'_>) -> FieldValue {
        let p = ctx.process;
        match self {
            Self::Cpu => FieldValue::Num(f64::from(p.cpu_usage)),
            Self::Mem => FieldValue::Num(p.memory as f64),
            Self::Pid => FieldValue::Num(f64::from(p.pid)),
            Self::Name => FieldValue::Text((*p.name).to_string()),
            Self::User => FieldValue::Text(
                p.user_id
                    .as_ref()
                    .map(|s| (**s).to_string())
                    .unwrap_or_default(),
            ),
            Self::Cmd => FieldValue::Text(p.cmd.join(" ")),
            Self::DiskRead => FieldValue::Num(p.disk_read_speed as f64),
            Self::DiskWrite => FieldValue::Num(p.disk_write_speed as f64),
            Self::NetSent => FieldValue::Num(p.net_sent_rate as f64),
            Self::NetRecv => FieldValue::Num(p.net_recv_rate as f64),
            Self::SecurityScore => FieldValue::Num(f64::from(ctx.security_score.unwrap_or(100))),
        }
    }
}

/// 比较操作符 6 项。`=~` 走 [`FilterExpr::Regex`] 路径，不在这个枚举里。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Gt,
    Lt,
    Ge,
    Le,
}

impl CmpOp {
    /// 数值比较。`a op b`，注意 `=`/`!=` 在 f64 上是严格相等——对绝大多数过滤场景
    /// 足够（pid 是整数；cpu/mem 精确比较意义不大但用户也不会写 `cpu = 5.0001`）。
    #[must_use]
    pub fn apply_num(self, a: f64, b: f64) -> bool {
        match self {
            Self::Eq => a == b,
            Self::Ne => a != b,
            Self::Gt => a > b,
            Self::Lt => a < b,
            Self::Ge => a >= b,
            Self::Le => a <= b,
        }
    }

    /// 文本比较。`=` / `!=` 走精确匹配（大小写敏感），其余操作符在文本上无意义 → false。
    /// 大小写不敏感请用 `=~ /pattern/i`。
    #[must_use]
    pub fn apply_text(self, a: &str, b: &str) -> bool {
        match self {
            Self::Eq => a == b,
            Self::Ne => a != b,
            Self::Gt | Self::Lt | Self::Ge | Self::Le => false,
        }
    }
}

/// 字面量值。解析时单位已规范化（`5kb` → Number(5120)，`5%` → Percent(5)）。
///
/// **v0.12 阶段 5（TD-29）**：实现 `Hash + Eq` 让 [`FilterExpr::NetworkIn`] 能用
/// `HashSet<Value>` O(1) 查找（之前是 `Vec<Value>` + `iter().any()` O(N) 每 flow）。
/// f64 不内建 `Hash`（NaN 语义问题），按 `to_bits()` 实现——parser 产生的数字
/// 永远不是 NaN（合法 digit 字符串解析），所以这是安全的；测试构造的 Value 也
/// 用具体数字，无 NaN 风险。`PartialEq` 也按 `to_bits()` 让 `Hash` / `Eq` 自洽
/// （`-0.0 == +0.0` 在 IEEE 是 true 但 to_bits 不同——对 `in` 操作符语义无影响，
/// 用户不会写 `-0.0` 字面量进 `in` 列表）。
#[derive(Debug, Clone)]
pub enum Value {
    /// 裸数字或字节单位已转换。比较 cpu 字段时按 %，比较 mem/disk/net 字段时按字节。
    Number(f64),
    /// 显式百分比 `5%`。与 cpu/mem 字段（其本身就是 % 或可换算）配合使用。
    Percent(f64),
    /// 文本字面量（裸字符串如 `chrome` 或带引号 `"chrome exe"`）。
    Text(String),
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => a.to_bits() == b.to_bits(),
            (Self::Percent(a), Self::Percent(b)) => a.to_bits() == b.to_bits(),
            (Self::Text(a), Self::Text(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Value {}

impl std::hash::Hash for Value {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Self::Number(n) => {
                0u8.hash(state);
                n.to_bits().hash(state);
            }
            Self::Percent(n) => {
                1u8.hash(state);
                n.to_bits().hash(state);
            }
            Self::Text(s) => {
                2u8.hash(state);
                s.hash(state);
            }
        }
    }
}

impl Value {
    /// TD-29（v0.12 阶段 5）：把 [`FieldValue`]（apply 路径从 ProcessInfo/ProcessFlow
    /// extract 出来的中间态）转回 [`Value`]，供 `NetworkIn` 的 `HashSet<Value>`
    /// `contains` 查找。`FieldValue::Num(n)` → `Value::Number(n)`；
    /// `FieldValue::Text(s)` → `Value::Text(s)`。Percent 不出现在 FieldValue 里
    /// （Percent 是字面量标记，extract 出来的是 Num）。
    #[must_use]
    fn from_field_value(fv: FieldValue) -> Self {
        match fv {
            FieldValue::Num(n) => Self::Number(n),
            FieldValue::Text(s) => Self::Text(s),
        }
    }
}

/// 字段实际取值。`extract` 后的中间态，让 apply 函数走模式匹配。
#[derive(Debug, Clone)]
pub enum FieldValue {
    Num(f64),
    Text(String),
}

/// 求值上下文。把 security_score 从 App::security_scores 单独传进来，因为
/// ProcessInfo 不持分数（分数在 App HashMap 里）。
///
/// **v0.12 阶段 4（TD-30）**：加 `total_memory` 字段，让 `mem > 50%` 能换算成
/// 字节阈值（`mem / total_memory * 100.0` 与百分号字面量比较）。`total_memory == 0`
/// 时（测试 / unknown 容量）退回旧行为（字节值与百分号数字直接比较），保留兼容。
pub struct EvalCtx<'a> {
    pub process: &'a ProcessInfo,
    pub security_score: Option<u32>,
    pub total_memory: u64,
}

/// v0.11 阶段 3：网络字段求值上下文。作用对象是 [`ProcessFlow`]（Flow 子视图 /
/// `proc flows --filter`），与 [`EvalCtx`]（ProcessInfo，List / Tree / AppGroup
/// 视图）分离。
///
/// 设计取舍（ADR-0011 v0.11 阶段 3 增量段）：分两套 ctx 而非合并，原因是
/// ProcessField 作用于 `&ProcessInfo`（List view），NetworkField 作用于
/// `&ProcessFlow`（Flow view + 进程关联 flow），**作用对象不同**。两套 ctx +
/// 两个 apply 方法（[`FilterExpr::apply`] / [`FilterExpr::apply_network`]）
/// 让类型系统保证字段不会跨 ctx 误用——`sni =~ /evil/` 在 List 视图（无 flow）
/// 不会拿到 flow 数据，因为 List 视图根本不调 apply_network。
pub struct NetworkEvalCtx<'a> {
    pub flow: &'a ProcessFlow,
}

/// v0.14 阶段 3：录屏帧求值上下文。作用对象是 [`UiFrame`]（timeline 搜索），
/// 与 [`EvalCtx`] / [`NetworkEvalCtx`] 平级分离。
///
/// 设计取舍（与 stage 2 FilterExpr 同款原则）：分一套 ctx + 一个 apply 方法
/// 让类型系统保证字段不会跨 ctx 误用——`cpu > 5` 在 timeline 搜索时是「帧整体
/// CPU」（FrameField），在 List 视图是「单进程 CPU」（Field），语义不同。
pub struct FrameEvalCtx<'a> {
    pub frame: &'a UiFrame,
}

impl FilterExpr {
    /// v0.11 阶段 8 REVIEW-13 P1-2：检测表达式是否含 process 字段变体
    /// ([`Self::FieldCmp`] / [`Self::Regex`]，作用于 [`ProcessInfo`])。
    ///
    /// 用途：`proc flows --filter` / TUI Flow 子视图 走 [`Self::apply_network`]，
    /// process 字段变体在该 ctx 下永远返 false（无 ProcessInfo），用户写
    /// `cpu > 5` 会导致所有 flow 被过滤掉。CLI / UI parse 成功后调本方法
    /// 检测，是则打印 warn 提示用户改用 network 字段（sni/dns_name/...）。
    ///
    /// And / Or / Not 递归检测任一分支；纯 network 表达式返回 false。
    #[must_use]
    pub fn contains_process_field(&self) -> bool {
        match self {
            Self::FieldCmp { .. } | Self::Regex { .. } => true,
            Self::NetworkFieldCmp { .. }
            | Self::NetworkRegex { .. }
            | Self::NetworkIn { .. }
            | Self::FrameFieldCmp { .. }
            | Self::FrameRegex { .. }
            | Self::FrameIn { .. } => false,
            Self::And(l, r) | Self::Or(l, r) => {
                l.contains_process_field() || r.contains_process_field()
            }
            Self::Not(e) => e.contains_process_field(),
        }
    }

    /// 对单个进程求值。true = 进程通过过滤器。
    ///
    /// 类型不匹配（如 `cpu > chrome`）返回 false，不报错——保留上一次成功 AST
    /// 继续过滤的设计让 UI 不至于因一个错字炸掉整列。
    ///
    /// v0.11 阶段 3：本方法只处理 process-字段变体（[`Self::FieldCmp`] /
    /// [`Self::Regex`]）。Network 变体（[`Self::NetworkFieldCmp`] 等）在本 ctx
    /// 下返回 false——调用方应改用 [`Self::apply_network`]。
    #[must_use]
    pub fn apply(&self, ctx: &EvalCtx<'_>) -> bool {
        match self {
            Self::FieldCmp { field, op, value } => {
                let fv = field.extract(ctx);
                match (&fv, value) {
                    (FieldValue::Num(a), Value::Number(b)) => op.apply_num(*a, *b),
                    (FieldValue::Num(a), Value::Percent(b)) => {
                        // TD-30（v0.12 阶段 4）：mem + % 语义修复——按 total_memory
                        // 换算成「mem 占总内存百分比」与百分号字面量比较。cpu 自身
                        // 就是 0-100 标度，`cpu > 5%` 与 `cpu > 5` 等价（不变）；
                        // disk_read/write / net_sent/recv 字段没有自然除数，退回
                        // 旧行为（字节值直接与百分号数字比较，语义可疑但 surgical
                        // 原则下不引入未定义除数）；total_memory == 0（测试 / 未知容量）
                        // 也退回旧行为，避免 div by zero。
                        if matches!(field, Field::Mem) && ctx.total_memory > 0 {
                            let pct = *a / ctx.total_memory as f64 * 100.0;
                            op.apply_num(pct, *b)
                        } else {
                            op.apply_num(*a, *b)
                        }
                    }
                    (FieldValue::Text(a), Value::Text(b)) => op.apply_text(a, b),
                    // 类型不匹配（数值字段对文本字面量、文本字段对数字）→ 不命中
                    _ => false,
                }
            }
            Self::Regex { field, re, .. } => {
                // case_insensitive 在 parser 端通过 `(?i)` 内嵌标志编译进同一个 Regex，
                // apply 这里不需要再分支。
                let fv = field.extract(ctx);
                match &fv {
                    FieldValue::Text(s) => re.is_match(s),
                    FieldValue::Num(_) => false,
                }
            }
            Self::And(l, r) => l.apply(ctx) && r.apply(ctx),
            Self::Or(l, r) => l.apply(ctx) || r.apply(ctx),
            Self::Not(e) => !e.apply(ctx),
            // Network / Frame 变体在 ProcessInfo ctx 下无意义 → false（不报错，surgical 容错）。
            Self::NetworkFieldCmp { .. }
            | Self::NetworkRegex { .. }
            | Self::NetworkIn { .. }
            | Self::FrameFieldCmp { .. }
            | Self::FrameRegex { .. }
            | Self::FrameIn { .. } => false,
        }
    }

    /// v0.11 阶段 3：对单个 ProcessFlow 求值。true = flow 通过过滤器。
    ///
    /// 与 [`Self::apply`] 对称：network 变体正常求值；process 变体（[`Self::FieldCmp`]
    /// / [`Self::Regex`]）在本 ctx 下返回 false（NetworkEvalCtx 不持 ProcessInfo）。
    /// And / Or / Not 递归两边都走 apply_network，让用户能写
    /// `sni =~ /evil/ AND remote_port = 443` 这类纯网络表达式。
    #[must_use]
    pub fn apply_network(&self, ctx: &NetworkEvalCtx<'_>) -> bool {
        match self {
            Self::NetworkFieldCmp { field, op, value } => {
                let fv = field.extract(ctx);
                match (&fv, value) {
                    (FieldValue::Num(a), Value::Number(b)) => op.apply_num(*a, *b),
                    (FieldValue::Num(a), Value::Percent(b)) => op.apply_num(*a, *b),
                    (FieldValue::Text(a), Value::Text(b)) => op.apply_text(a, b),
                    _ => false,
                }
            }
            Self::NetworkRegex { field, re, .. } => {
                let fv = field.extract(ctx);
                match &fv {
                    FieldValue::Text(s) => re.is_match(s),
                    FieldValue::Num(_) => false,
                }
            }
            Self::NetworkIn { field, values } => {
                // TD-29（v0.12 阶段 5）：HashSet O(1) contains 替代 Vec.iter().any() O(N)。
                // extract 出来的 FieldValue 转 Value 后直接 contains——HashSet 用我们
                // 手写的 Hash+Eq（to_bits() 对 f64），与 parser 构造的 Value 自洽。
                let fv = field.extract(ctx);
                values.contains(&Value::from_field_value(fv))
            }
            Self::And(l, r) => l.apply_network(ctx) && r.apply_network(ctx),
            Self::Or(l, r) => l.apply_network(ctx) || r.apply_network(ctx),
            Self::Not(e) => !e.apply_network(ctx),
            // Process / Frame 变体在 NetworkEvalCtx 下无意义 → false。
            Self::FieldCmp { .. }
            | Self::Regex { .. }
            | Self::FrameFieldCmp { .. }
            | Self::FrameRegex { .. }
            | Self::FrameIn { .. } => false,
        }
    }

    /// v0.14 阶段 3：对单个 UiFrame 求值。true = 该帧通过过滤器（命中）。
    ///
    /// 与 [`Self::apply`] / [`Self::apply_network`] 对称：Frame 变体正常求值；
    /// Process / Network 变体在本 ctx 下返 false（FrameEvalCtx 不持 ProcessInfo
    /// / ProcessFlow）。And / Or / Not 递归两边都走 apply_frame。
    ///
    /// 文本字段（Name / AnomalySeverity）走「帧内集合任一匹配」（[`FrameField::any_match`]），
    /// 数值字段（Timestamp / Cpu / Mem）走 extract_first + apply_num。
    #[must_use]
    pub fn apply_frame(&self, ctx: &FrameEvalCtx<'_>) -> bool {
        match self {
            Self::FrameFieldCmp { field, op, value } => {
                if field.is_text() {
                    // 文本字段（Name / AnomalySeverity）：帧内集合任一 Eq/Ne 匹配。
                    // Gt/Lt/Ge/Le 在文本上无意义 → false（与 CmpOp::apply_text 一致）。
                    let Value::Text(target) = value else {
                        return false;
                    };
                    field.any_match_text(ctx.frame, |s| op.apply_text(s, target))
                } else {
                    // 数值字段（Timestamp / Cpu / Mem）：extract_first + apply_num。
                    // Percent 在 frame ctx 下没有自然除数（与 disk_read/write 同款理由），
                    // 走「数值直接比较」（cpu 自身就是 0-100 标度，`cpu > 5%` 与 `cpu > 5` 等价）。
                    let fv = field.extract_first(ctx.frame);
                    match (&fv, value) {
                        (FieldValue::Num(a), Value::Number(b)) => op.apply_num(*a, *b),
                        (FieldValue::Num(a), Value::Percent(b)) => op.apply_num(*a, *b),
                        _ => false,
                    }
                }
            }
            Self::FrameRegex { field, re, .. } => {
                // case_insensitive 在 parser 端通过 `(?i)` 内嵌标志编译进同一个 Regex。
                if !field.is_text() {
                    return false;
                }
                field.any_match_text(ctx.frame, |s| re.is_match(s))
            }
            Self::FrameIn { field, values } => {
                if !field.is_text() {
                    return false;
                }
                field.any_match_text(ctx.frame, |s| values.contains(&Value::Text(s.to_string())))
            }
            Self::And(l, r) => l.apply_frame(ctx) && r.apply_frame(ctx),
            Self::Or(l, r) => l.apply_frame(ctx) || r.apply_frame(ctx),
            Self::Not(e) => !e.apply_frame(ctx),
            // Process / Network 变体在 FrameEvalCtx 下无意义 → false。
            Self::FieldCmp { .. }
            | Self::Regex { .. }
            | Self::NetworkFieldCmp { .. }
            | Self::NetworkRegex { .. }
            | Self::NetworkIn { .. } => false,
        }
    }

    /// v0.14 阶段 3：检测表达式是否含 Frame 变体（作用于 [`UiFrame`]）。
    /// 让 timeline 搜索 / List 视图 detect 是否走 [`Self::apply_frame`] 路径。
    /// 与 [`Self::contains_process_field`] 同款递归实现。
    #[must_use]
    pub fn contains_frame_field(&self) -> bool {
        match self {
            Self::FrameFieldCmp { .. } | Self::FrameRegex { .. } | Self::FrameIn { .. } => true,
            Self::FieldCmp { .. }
            | Self::Regex { .. }
            | Self::NetworkFieldCmp { .. }
            | Self::NetworkRegex { .. }
            | Self::NetworkIn { .. } => false,
            Self::And(l, r) | Self::Or(l, r) => {
                l.contains_frame_field() || r.contains_frame_field()
            }
            Self::Not(e) => e.contains_frame_field(),
        }
    }
}

/// v0.11 阶段 3：网络字段枚举。作用对象 [`ProcessFlow`]，与 [`Field`]
/// （作用对象 [`ProcessInfo`]）分离。
///
/// 文本字段：`Sni` / `DnsName` / `RemoteAddr`（值走 [`FieldValue::Text`]）。
/// 数值字段：`RemotePort` / `BytesOut` / `BytesIn`（值走 [`FieldValue::Num`]）。
///
/// 设计取舍见 ADR-0011 v0.11 阶段 3 增量段。
///
/// v0.12 阶段 2：移除 `Source` 字段（Windows-only 后唯一来源是 Schannel）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkField {
    /// TLS ClientHello SNI 明文（HTTPS 流量必经路径；Windows 走 Schannel ETW）。
    /// None → 空字符串。
    Sni,
    /// DNS 查询关联名（DNS cache 命中时关联；HTTPS 命中 DNS cache 时关联不到）。
    /// None → 空字符串。
    DnsName,
    /// 远端 IPv4 地址字符串（`"1.2.3.4"`）。Schannel 路径常空。
    RemoteAddr,
    /// 远端端口（host byte order）。0 = 未知（Schannel 路径不给 socket 元数据）。
    RemotePort,
    /// 出向字节数。Schannel 路径常 0。
    BytesOut,
    /// 入向字节数。Schannel 路径常 0。
    BytesIn,
}

impl NetworkField {
    /// 文本字段（Sni/DnsName/RemoteAddr）走 [`FieldValue::Text`]；
    /// 数值字段（RemotePort/BytesOut/BytesIn）走 [`FieldValue::Num`]。
    #[must_use]
    pub fn is_text(self) -> bool {
        matches!(self, Self::Sni | Self::DnsName | Self::RemoteAddr)
    }

    /// 从 ProcessFlow 取值。`Option<String>` 字段（sni/dns_name）`None` →
    /// `Text("")`（与 `=~ /foo/` 不匹配但 `NOT sni =~ /./` 可用）。
    #[must_use]
    pub fn extract(self, ctx: &NetworkEvalCtx<'_>) -> FieldValue {
        let f = ctx.flow;
        match self {
            Self::Sni => FieldValue::Text(f.sni.clone().unwrap_or_default()),
            Self::DnsName => FieldValue::Text(f.dns_name.clone().unwrap_or_default()),
            Self::RemoteAddr => FieldValue::Text(f.remote_addr.clone()),
            Self::RemotePort => FieldValue::Num(f64::from(f.remote_port)),
            Self::BytesOut => FieldValue::Num(f.bytes_out as f64),
            Self::BytesIn => FieldValue::Num(f.bytes_in as f64),
        }
    }
}

/// v0.14 阶段 3：录屏帧字段枚举。作用对象 [`UiFrame`]，与 [`Field`]
/// （[`ProcessInfo`]）/ [`NetworkField`]（[`ProcessFlow`]）平级分离。
///
/// UiFrame 含两类数据：(1) 帧级标量（timestamp / cpu_usage / memory_used）；
/// (2) 帧内集合（processes / anomalies）。本枚举对应 timeline 搜索的 5 个维度。
///
/// **文本字段**：`Name`（任一进程名匹配）/ `AnomalySeverity`（任一异常严重度匹配）—
/// 走「帧内集合任一匹配」（[`Self::any_match_text`]）。
/// **数值字段**：`Timestamp` / `Cpu` / `Mem` — 走 extract_first + apply_num。
///
/// 设计取舍（与 stage 2 FilterExpr 同款原则）：分一套 ctx + 一个 apply 方法
/// 让类型系统保证字段不会跨 ctx 误用——`cpu > 5` 在 timeline 搜索时是「帧整体
/// CPU」（FrameField::Cpu），在 List 视图是「单进程 CPU」（Field::Cpu），
/// Parser 通过 `parse_frame()` vs `parse()` 入口区分语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameField {
    /// 帧绝对时间戳（unix epoch 秒）。数值字段。
    Timestamp,
    /// 帧整体 CPU 占用百分比（0-100，[`UiFrame::cpu_usage`]）。数值字段。
    Cpu,
    /// 帧整体内存（字节，[`UiFrame::memory_used`]）。数值字段。
    Mem,
    /// 帧内任一进程名匹配（[`crate::record::FrameProcess::name`]）。文本字段。
    Name,
    /// 帧内任一 anomaly 的 severity（[`crate::record::FrameAnomaly::severity`]）。
    /// 典型值 `info` / `warning` / `critical`。文本字段。
    AnomalySeverity,
}

impl FrameField {
    /// 文本字段（Name / AnomalySeverity）走 [`FieldValue::Text`]；
    /// 数值字段（Timestamp / Cpu / Mem）走 [`FieldValue::Num`]。
    #[must_use]
    pub fn is_text(self) -> bool {
        matches!(self, Self::Name | Self::AnomalySeverity)
    }

    /// 数值字段从 UiFrame 取单一值。文本字段调用方应改用 [`Self::any_match_text`]。
    #[must_use]
    pub fn extract_first(self, frame: &UiFrame) -> FieldValue {
        match self {
            Self::Timestamp => FieldValue::Num(frame.timestamp as f64),
            Self::Cpu => FieldValue::Num(f64::from(frame.cpu_usage)),
            Self::Mem => FieldValue::Num(frame.memory_used as f64),
            Self::Name => FieldValue::Text(
                frame
                    .processes
                    .first()
                    .map(|p| p.name.clone())
                    .unwrap_or_default(),
            ),
            Self::AnomalySeverity => FieldValue::Text(
                frame
                    .anomalies
                    .first()
                    .map(|a| a.severity.clone())
                    .unwrap_or_default(),
            ),
        }
    }

    /// 文本字段在帧内集合中**任一**项匹配（`pred` 返 true）。数值字段调用本方法
    /// 返 false（不应到达此路径——apply_frame 在数值字段下走 extract_first）。
    /// 让 [`FilterExpr::FrameFieldCmp`] / [`FilterExpr::FrameRegex`] / [`FilterExpr::FrameIn`]
    /// 共用一套「帧内集合扫描」逻辑。
    #[must_use]
    pub fn any_match_text(self, frame: &UiFrame, pred: impl Fn(&str) -> bool) -> bool {
        match self {
            Self::Name => frame.processes.iter().any(|p| pred(&p.name)),
            Self::AnomalySeverity => frame.anomalies.iter().any(|a| pred(&a.severity)),
            _ => false,
        }
    }
}

/// v0.14 阶段 3：构造 substring 搜索表达式（timeline 搜索 `:` 前缀外的 fallback）。
///
/// 用户在 timeline 输入 `chrome`（无 `:` 前缀）→ 等价 `name =~ /chrome/i`。
/// `regex::escape` 转义元字符让 `.` `*` `+` 等当字面量（避免用户输入 `chrome.exe`
/// 时被解释为「chrome + 任意字符 + exe」匹配到 `chromexexe`）。
///
/// 输入为空时返 Err（让调用方短路，不进入 apply_frame 路径）。
pub fn build_frame_substring_expr(input: &str) -> Result<FilterExpr, ParseError> {
    if input.trim().is_empty() {
        return Err(ParseError {
            msg: "搜索内容为空".to_string(),
            position: 0,
        });
    }
    let escaped = regex::escape(input);
    let re = regex::Regex::new(&format!("(?i){escaped}")).map_err(|_| ParseError {
        msg: "正则编译失败".to_string(),
        position: 0,
    })?;
    Ok(FilterExpr::FrameRegex {
        field: FrameField::Name,
        re,
        case_insensitive: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(name: &str, cpu: f32, mem: u64, pid: u32) -> ProcessInfo {
        ProcessInfo {
            name: std::sync::Arc::from(name),
            name_lower: std::sync::Arc::from(name.to_lowercase()),
            cpu_usage: cpu,
            memory: mem,
            pid,
            ..ProcessInfo::default()
        }
    }

    fn ctx<'a>(p: &'a ProcessInfo, score: Option<u32>) -> EvalCtx<'a> {
        EvalCtx {
            process: p,
            security_score: score,
            total_memory: 0,
        }
    }

    #[test]
    fn cpu_gt_match() {
        let p = proc("chrome", 10.0, 0, 1);
        let expr = FilterExpr::FieldCmp {
            field: Field::Cpu,
            op: CmpOp::Gt,
            value: Value::Number(5.0),
        };
        assert!(expr.apply(&ctx(&p, None)));
    }

    #[test]
    fn cpu_gt_no_match() {
        let p = proc("idle", 1.0, 0, 2);
        let expr = FilterExpr::FieldCmp {
            field: Field::Cpu,
            op: CmpOp::Gt,
            value: Value::Number(5.0),
        };
        assert!(!expr.apply(&ctx(&p, None)));
    }

    #[test]
    fn mem_mb_unit() {
        let p = proc("big", 0.0, 200 * 1024 * 1024, 3);
        let expr = FilterExpr::FieldCmp {
            field: Field::Mem,
            op: CmpOp::Gt,
            value: Value::Number(100.0 * 1024.0 * 1024.0),
        };
        assert!(expr.apply(&ctx(&p, None)));
    }

    #[test]
    fn name_eq_text() {
        let p = proc("chrome.exe", 0.0, 0, 4);
        let expr = FilterExpr::FieldCmp {
            field: Field::Name,
            op: CmpOp::Eq,
            value: Value::Text("chrome.exe".to_string()),
        };
        assert!(expr.apply(&ctx(&p, None)));
    }

    #[test]
    fn and_combinator() {
        let p = proc("chrome", 10.0, 100 * 1024 * 1024, 5);
        let expr = FilterExpr::And(
            Box::new(FilterExpr::FieldCmp {
                field: Field::Cpu,
                op: CmpOp::Gt,
                value: Value::Number(5.0),
            }),
            Box::new(FilterExpr::FieldCmp {
                field: Field::Mem,
                op: CmpOp::Lt,
                value: Value::Number(500.0 * 1024.0 * 1024.0),
            }),
        );
        assert!(expr.apply(&ctx(&p, None)));
    }

    #[test]
    fn or_combinator() {
        let p = proc("chrome", 1.0, 0, 6);
        let expr = FilterExpr::Or(
            Box::new(FilterExpr::FieldCmp {
                field: Field::Cpu,
                op: CmpOp::Gt,
                value: Value::Number(5.0),
            }),
            Box::new(FilterExpr::FieldCmp {
                field: Field::Name,
                op: CmpOp::Eq,
                value: Value::Text("chrome".to_string()),
            }),
        );
        assert!(expr.apply(&ctx(&p, None)));
    }

    #[test]
    fn not_combinator() {
        let p = proc("chrome", 1.0, 0, 7);
        let expr = FilterExpr::Not(Box::new(FilterExpr::FieldCmp {
            field: Field::Cpu,
            op: CmpOp::Gt,
            value: Value::Number(5.0),
        }));
        assert!(expr.apply(&ctx(&p, None)));
    }

    #[test]
    fn regex_match() {
        let p = proc("chrome.exe", 0.0, 0, 8);
        let re = regex::Regex::new("chrom").unwrap();
        let expr = FilterExpr::Regex {
            field: Field::Name,
            re,
            case_insensitive: false,
        };
        assert!(expr.apply(&ctx(&p, None)));
    }

    #[test]
    fn type_mismatch_returns_false() {
        let p = proc("chrome", 10.0, 0, 9);
        let expr = FilterExpr::FieldCmp {
            field: Field::Cpu,
            op: CmpOp::Gt,
            value: Value::Text("chrome".to_string()),
        };
        assert!(!expr.apply(&ctx(&p, None)));
    }

    #[test]
    fn security_score_default_100() {
        let p = proc("safe", 0.0, 0, 10);
        let expr = FilterExpr::FieldCmp {
            field: Field::SecurityScore,
            op: CmpOp::Eq,
            value: Value::Number(100.0),
        };
        assert!(expr.apply(&ctx(&p, None)));
    }

    #[test]
    fn security_score_explicit_low() {
        let p = proc("sus", 0.0, 0, 11);
        let expr = FilterExpr::FieldCmp {
            field: Field::SecurityScore,
            op: CmpOp::Lt,
            value: Value::Number(80.0),
        };
        assert!(expr.apply(&ctx(&p, Some(50))));
    }

    // --- v0.11 阶段 3：NetworkField / apply_network ---

    use crate::flow::ProcessFlow;

    fn flow(sni: Option<&str>, dns: Option<&str>, addr: &str, port: u16) -> ProcessFlow {
        ProcessFlow {
            pid: 1,
            start_time: 0,
            comm: String::new(),
            local_addr: String::new(),
            remote_addr: addr.into(),
            remote_port: port,
            bytes_out: 0,
            bytes_in: 0,
            dns_name: dns.map(str::to_string),
            sni: sni.map(str::to_string),
            first_seen: std::time::SystemTime::UNIX_EPOCH,
            last_seen: std::time::SystemTime::UNIX_EPOCH,
            exit_time: None,
        }
    }

    fn nctx<'a>(f: &'a ProcessFlow) -> NetworkEvalCtx<'a> {
        NetworkEvalCtx { flow: f }
    }

    #[test]
    fn network_sni_eq_matches() {
        let f = flow(Some("evil.com"), None, "1.2.3.4", 443);
        let expr = FilterExpr::NetworkFieldCmp {
            field: NetworkField::Sni,
            op: CmpOp::Eq,
            value: Value::Text("evil.com".into()),
        };
        assert!(expr.apply_network(&nctx(&f)));
    }

    #[test]
    fn network_sni_none_returns_empty() {
        // sni = None → Text("") → 与任何非空 sni 字面量不匹配。
        let f = flow(None, None, "1.2.3.4", 443);
        let expr = FilterExpr::NetworkFieldCmp {
            field: NetworkField::Sni,
            op: CmpOp::Eq,
            value: Value::Text("evil.com".into()),
        };
        assert!(!expr.apply_network(&nctx(&f)));
    }

    #[test]
    fn network_sni_regex_matches() {
        let f = flow(Some("api.google.com"), None, "1.2.3.4", 443);
        let expr = FilterExpr::NetworkRegex {
            field: NetworkField::Sni,
            re: regex::Regex::new(r"google\.com$").unwrap(),
            case_insensitive: false,
        };
        assert!(expr.apply_network(&nctx(&f)));
    }

    #[test]
    fn network_remote_port_eq() {
        let f = flow(None, None, "1.2.3.4", 443);
        let expr = FilterExpr::NetworkFieldCmp {
            field: NetworkField::RemotePort,
            op: CmpOp::Eq,
            value: Value::Number(443.0),
        };
        assert!(expr.apply_network(&nctx(&f)));
    }

    #[test]
    fn network_remote_addr_in() {
        let f = flow(None, None, "5.6.7.8", 443);
        let expr = FilterExpr::NetworkIn {
            field: NetworkField::RemoteAddr,
            values: [Value::Text("1.2.3.4".into()), Value::Text("5.6.7.8".into())]
                .into_iter()
                .collect(),
        };
        assert!(expr.apply_network(&nctx(&f)));
    }

    #[test]
    fn network_process_variant_returns_false() {
        // FieldCmp (process) on NetworkEvalCtx → false（network ctx 无 ProcessInfo）。
        let f = flow(Some("a.com"), None, "1.2.3.4", 443);
        let expr = FilterExpr::FieldCmp {
            field: Field::Cpu,
            op: CmpOp::Gt,
            value: Value::Number(5.0),
        };
        assert!(!expr.apply_network(&nctx(&f)));
    }

    #[test]
    fn network_and_combinator() {
        let f = flow(Some("evil.com"), None, "1.2.3.4", 443);
        let expr = FilterExpr::And(
            Box::new(FilterExpr::NetworkRegex {
                field: NetworkField::Sni,
                re: regex::Regex::new("evil").unwrap(),
                case_insensitive: false,
            }),
            Box::new(FilterExpr::NetworkFieldCmp {
                field: NetworkField::RemotePort,
                op: CmpOp::Eq,
                value: Value::Number(443.0),
            }),
        );
        assert!(expr.apply_network(&nctx(&f)));
    }

    #[test]
    fn network_field_is_text_classification() {
        assert!(NetworkField::Sni.is_text());
        assert!(NetworkField::DnsName.is_text());
        assert!(NetworkField::RemoteAddr.is_text());
        assert!(!NetworkField::RemotePort.is_text());
        assert!(!NetworkField::BytesOut.is_text());
        assert!(!NetworkField::BytesIn.is_text());
    }

    // --- v0.11 阶段 8 REVIEW-13 P1-2：contains_process_field ---

    #[test]
    fn contains_process_field_pure_process() {
        let expr = FilterExpr::FieldCmp {
            field: Field::Cpu,
            op: CmpOp::Gt,
            value: Value::Number(5.0),
        };
        assert!(expr.contains_process_field());
    }

    #[test]
    fn contains_process_field_pure_network() {
        let expr = FilterExpr::NetworkRegex {
            field: NetworkField::Sni,
            re: regex::Regex::new("evil").unwrap(),
            case_insensitive: false,
        };
        assert!(!expr.contains_process_field());
    }

    #[test]
    fn contains_process_field_mixed_and() {
        // `cpu > 5 AND sni =~ /evil/` —— 含 process 字段，应返 true
        let expr = FilterExpr::And(
            Box::new(FilterExpr::FieldCmp {
                field: Field::Cpu,
                op: CmpOp::Gt,
                value: Value::Number(5.0),
            }),
            Box::new(FilterExpr::NetworkRegex {
                field: NetworkField::Sni,
                re: regex::Regex::new("evil").unwrap(),
                case_insensitive: false,
            }),
        );
        assert!(expr.contains_process_field());
    }

    #[test]
    fn contains_process_field_network_in_only() {
        let expr = FilterExpr::NetworkIn {
            field: NetworkField::RemoteAddr,
            values: [Value::Text("1.2.3.4".into())].into_iter().collect(),
        };
        assert!(!expr.contains_process_field());
    }

    #[test]
    fn contains_process_field_not_network() {
        // NOT (sni =~ /evil/) —— 纯 network，应返 false
        let expr = FilterExpr::Not(Box::new(FilterExpr::NetworkRegex {
            field: NetworkField::Sni,
            re: regex::Regex::new("evil").unwrap(),
            case_insensitive: false,
        }));
        assert!(!expr.contains_process_field());
    }
}
