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

pub mod parser;

pub use parser::{ParseError, parse};

use crate::collect::ProcessInfo;
use crate::ebpf::flow::ProcessFlow;

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
    NetworkIn {
        field: NetworkField,
        values: Vec<Value>,
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
            Self::NetworkFieldCmp { .. } | Self::NetworkRegex { .. } | Self::NetworkIn { .. } => {
                false
            }
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
            // Network 变体在 ProcessInfo ctx 下无意义 → false（不报错，surgical 容错）。
            Self::NetworkFieldCmp { .. } | Self::NetworkRegex { .. } | Self::NetworkIn { .. } => {
                false
            }
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
                let fv = field.extract(ctx);
                match &fv {
                    FieldValue::Text(s) => {
                        values.iter().any(|v| matches!(v, Value::Text(t) if t == s))
                    }
                    FieldValue::Num(n) => values
                        .iter()
                        .any(|v| matches!(v, Value::Number(m) if m == n)),
                }
            }
            Self::And(l, r) => l.apply_network(ctx) && r.apply_network(ctx),
            Self::Or(l, r) => l.apply_network(ctx) || r.apply_network(ctx),
            Self::Not(e) => !e.apply_network(ctx),
            // Process-字段变体在 NetworkEvalCtx 下无意义 → false。
            Self::FieldCmp { .. } | Self::Regex { .. } => false,
        }
    }
}

/// v0.11 阶段 3：网络字段枚举。作用对象 [`ProcessFlow`]，与 [`Field`]
/// （作用对象 [`ProcessInfo`]）分离。
///
/// 文本字段：`Sni` / `DnsName` / `RemoteAddr` / `Source`（值走 [`FieldValue::Text`]）。
/// 数值字段：`RemotePort` / `BytesOut` / `BytesIn`（值走 [`FieldValue::Num`]）。
///
/// 设计取舍见 ADR-0011 v0.11 阶段 3 增量段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkField {
    /// TLS ClientHello SNI 明文（HTTPS 流量必经路径；Windows 走 Schannel ETW，
    /// Linux 走 eBPF uprobe）。None → 空字符串。
    Sni,
    /// DNS 查询关联名（DNS cache 命中时关联；HTTPS 命中 DNS cache 时关联不到）。
    /// None → 空字符串。
    DnsName,
    /// 远端 IPv4 地址字符串（`"1.2.3.4"`）。Schannel 路径可能空。
    RemoteAddr,
    /// 远端端口（host byte order）。0 = 未知（Schannel 路径不给 socket 元数据）。
    RemotePort,
    /// 出向字节数。MVP 留 0（要 hook tcp_sendmsg，留 TD-17）。
    BytesOut,
    /// 入向字节数。MVP 留 0（要 hook tcp_recvmsg，留 TD-17）。
    BytesIn,
    /// 数据来源：`"ebpf"` (Linux + ebpf feature) / `"schannel"` (Windows admin)。
    /// 与 `FlowSource` serde 序列化同款小写。
    Source,
}

impl NetworkField {
    /// 文本字段（Sni/DnsName/RemoteAddr/Source）走 [`FieldValue::Text`]；
    /// 数值字段（RemotePort/BytesOut/BytesIn）走 [`FieldValue::Num`]。
    #[must_use]
    pub fn is_text(self) -> bool {
        matches!(
            self,
            Self::Sni | Self::DnsName | Self::RemoteAddr | Self::Source
        )
    }

    /// 从 ProcessFlow 取值。`Option<String>` 字段（sni/dns_name）`None` →
    /// `Text("")`（与 `=~ /foo/` 不匹配但 `NOT sni =~ /./` 可用）。
    /// `Source` 走 `FlowSource::as_str()` 小写枚举字符串。
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
            Self::Source => FieldValue::Text(source_as_str(f.source).to_string()),
        }
    }
}

/// `FlowSource` → 小写字符串（与 serde `rename_all="lowercase"` 一致）。
/// `NetworkField::extract` 内部用。
#[must_use]
fn source_as_str(s: crate::ebpf::flow::FlowSource) -> &'static str {
    match s {
        crate::ebpf::flow::FlowSource::Ebpf => "ebpf",
        crate::ebpf::flow::FlowSource::Schannel => "schannel",
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

    // --- v0.11 阶段 3：NetworkField / apply_network ---

    use crate::ebpf::flow::{FlowSource, ProcessFlow};

    fn flow(
        sni: Option<&str>,
        dns: Option<&str>,
        addr: &str,
        port: u16,
        source: FlowSource,
    ) -> ProcessFlow {
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
            source,
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
        let f = flow(Some("evil.com"), None, "1.2.3.4", 443, FlowSource::Ebpf);
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
        let f = flow(None, None, "1.2.3.4", 443, FlowSource::Ebpf);
        let expr = FilterExpr::NetworkFieldCmp {
            field: NetworkField::Sni,
            op: CmpOp::Eq,
            value: Value::Text("evil.com".into()),
        };
        assert!(!expr.apply_network(&nctx(&f)));
    }

    #[test]
    fn network_sni_regex_matches() {
        let f = flow(
            Some("api.google.com"),
            None,
            "1.2.3.4",
            443,
            FlowSource::Ebpf,
        );
        let expr = FilterExpr::NetworkRegex {
            field: NetworkField::Sni,
            re: regex::Regex::new(r"google\.com$").unwrap(),
            case_insensitive: false,
        };
        assert!(expr.apply_network(&nctx(&f)));
    }

    #[test]
    fn network_remote_port_eq() {
        let f = flow(None, None, "1.2.3.4", 443, FlowSource::Ebpf);
        let expr = FilterExpr::NetworkFieldCmp {
            field: NetworkField::RemotePort,
            op: CmpOp::Eq,
            value: Value::Number(443.0),
        };
        assert!(expr.apply_network(&nctx(&f)));
    }

    #[test]
    fn network_remote_addr_in() {
        let f = flow(None, None, "5.6.7.8", 443, FlowSource::Ebpf);
        let expr = FilterExpr::NetworkIn {
            field: NetworkField::RemoteAddr,
            values: vec![Value::Text("1.2.3.4".into()), Value::Text("5.6.7.8".into())],
        };
        assert!(expr.apply_network(&nctx(&f)));
    }

    #[test]
    fn network_source_schannel_text_match() {
        let f = flow(Some("a.com"), None, "", 0, FlowSource::Schannel);
        let expr = FilterExpr::NetworkFieldCmp {
            field: NetworkField::Source,
            op: CmpOp::Eq,
            value: Value::Text("schannel".into()),
        };
        assert!(expr.apply_network(&nctx(&f)));
    }

    #[test]
    fn network_process_variant_returns_false() {
        // FieldCmp (process) on NetworkEvalCtx → false（network ctx 无 ProcessInfo）。
        let f = flow(Some("a.com"), None, "1.2.3.4", 443, FlowSource::Ebpf);
        let expr = FilterExpr::FieldCmp {
            field: Field::Cpu,
            op: CmpOp::Gt,
            value: Value::Number(5.0),
        };
        assert!(!expr.apply_network(&nctx(&f)));
    }

    #[test]
    fn network_and_combinator() {
        let f = flow(Some("evil.com"), None, "1.2.3.4", 443, FlowSource::Ebpf);
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
        assert!(NetworkField::Source.is_text());
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
            values: vec![Value::Text("1.2.3.4".into())],
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
