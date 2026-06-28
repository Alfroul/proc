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

pub mod parser;

pub use parser::{ParseError, parse};

use crate::collect::ProcessInfo;

/// 过滤表达式 AST。每个节点是一个布尔判定，[`FilterExpr::apply`] 是执行入口。
///
/// `And` / `Or` / `Not` 走 `Box` 避免 enum 自引用无限大小。
#[derive(Debug, Clone)]
pub enum FilterExpr {
    /// 字段比较：`field op value`，例 `cpu > 5`。
    FieldCmp {
        field: Field,
        op: CmpOp,
        value: Value,
    },
    /// 正则匹配：`field =~ /pattern/i`。
    Regex {
        field: Field,
        re: regex::Regex,
        case_insensitive: bool,
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
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// 裸数字或字节单位已转换。比较 cpu 字段时按 %，比较 mem/disk/net 字段时按字节。
    Number(f64),
    /// 显式百分比 `5%`。与 cpu/mem 字段（其本身就是 % 或可换算）配合使用。
    Percent(f64),
    /// 文本字面量（裸字符串如 `chrome` 或带引号 `"chrome exe"`）。
    Text(String),
}

/// 字段实际取值。`extract` 后的中间态，让 apply 函数走模式匹配。
#[derive(Debug, Clone)]
pub enum FieldValue {
    Num(f64),
    Text(String),
}

/// 求值上下文。把 security_score 从 App::security_scores 单独传进来，因为
/// ProcessInfo 不持分数（分数在 App HashMap 里）。
pub struct EvalCtx<'a> {
    pub process: &'a ProcessInfo,
    pub security_score: Option<u32>,
}

impl FilterExpr {
    /// 对单个进程求值。true = 进程通过过滤器。
    ///
    /// 类型不匹配（如 `cpu > chrome`）返回 false，不报错——保留上一次成功 AST
    /// 继续过滤的设计让 UI 不至于因一个错字炸掉整列。
    #[must_use]
    pub fn apply(&self, ctx: &EvalCtx<'_>) -> bool {
        match self {
            Self::FieldCmp { field, op, value } => {
                let fv = field.extract(ctx);
                match (&fv, value) {
                    (FieldValue::Num(a), Value::Number(b)) => op.apply_num(*a, *b),
                    (FieldValue::Num(a), Value::Percent(b)) => op.apply_num(*a, *b),
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
        }
    }
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
}
