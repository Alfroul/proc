//! FilterExpr 属性测试（proptest，v0.26 stage 2，ADR-0036 D3 预登记 dev-dep）。
//!
//! 两条属性（cases 256 = ADR-0036 D3 定值；parser 纯函数无 IO，16GB 机器秒级
//! ——brainstorm 风险 4 预期）：
//!
//! 1. **roundtrip 语义等价**：随机生成 AST（Process `FieldCmp`/`Regex` +
//!    Network `FieldCmp`/`Regex`/`In` 叶子 + `And`/`Or`/`Not` 复合）→ 测试侧
//!    renderer 渲染回 filter 语法 → `parse()` 重解析 → 原表达式与重解析表达式
//!    在样本求值电池（8 进程 × `apply` + 6 flow × `apply_network`）上行为逐一
//!    相同。`FilterExpr` 无 `PartialEq`（含 `Regex` 字段），语义等价用行为
//!    等价判定（比结构等价更强且可实现）。
//! 2. **乱输入不 panic**：任意字符串喂 `parse()` / `parse_frame()` 返 Ok/Err
//!    均可但绝不 panic；parse 成功的表达式对样本电池求值也不 panic
//!    （类型不匹配返 false 不炸的容错契约，filter/mod.rs apply 文档约定）。
//!
//! 生成侧约束（保证渲染串必可重解析）：数值字面量限非负整数（裸数字 / `N%`，
//! parser 数字文法 `-?\d+(\.\d+)?` + 单位后缀）、文本一律双引号包裹（字母表
//! 排除 `"` 与 `\`——quoted string 无转义文法），regex pattern 字母表限
//! `[A-Za-z0-9._-]`（无 `/` 分隔符冲突、无元字符，渲染零转义需求），
//! `case_insensitive` 经 `/i` 后缀往返（renderer 从 `re.as_str()` 剥 `(?i)`
//! 前缀还原裸 pattern——与 parser 的 `format!("(?i){pattern}")` 构造对称）。

use std::sync::Arc;

use proptest::prelude::*;

use proc::collect::ProcessInfo;
use proc::filter::{
    CmpOp, EvalCtx, Field, FilterExpr, FrameEvalCtx, NetworkEvalCtx, NetworkField, Value,
};
use proc::flow::ProcessFlow;
use proc::record::frame::{FrameAnomaly, FrameConnectionDiff, FrameNav, FrameProcess, UiFrame};

// ===========================================================================
// 样本求值电池（两个属性共用）
// ===========================================================================

fn sample_processes() -> Vec<ProcessInfo> {
    let mk = |name: &str,
              cpu: f32,
              mem: u64,
              pid: u32,
              user: Option<&str>,
              cmd: &[&str],
              disk_r: u64,
              net_s: u64| ProcessInfo {
        name: Arc::from(name),
        name_lower: Arc::from(name.to_lowercase()),
        cpu_usage: cpu,
        memory: mem,
        pid,
        user_id: user.map(Arc::from),
        cmd: Arc::from(cmd.iter().map(|s| s.to_string()).collect::<Vec<_>>()),
        disk_read_speed: disk_r,
        net_sent_rate: net_s,
        ..ProcessInfo::default()
    };
    vec![
        mk(
            "chrome.exe",
            25.5,
            800 * 1024 * 1024,
            101,
            Some("alice"),
            &["chrome", "--headless"],
            1024,
            2048,
        ),
        mk("idle", 0.0, 0, 102, None, &[], 0, 0),
        mk(
            "Svchost",
            3.25,
            120 * 1024 * 1024,
            103,
            Some("SYSTEM"),
            &[],
            0,
            0,
        ),
        mk(
            "rustc",
            99.9,
            2 * 1024 * 1024 * 1024,
            104,
            Some("bob"),
            &["rustc", "--release"],
            100_000,
            50_000,
        ),
        mk("x", 50.0, 1, 105, Some("a.b"), &["-flag"], 7, 9),
        mk(
            "user_field-1",
            12.0,
            42 * 1024 * 1024,
            106,
            Some("carol"),
            &[],
            3,
            4,
        ),
        mk(
            "mem.hog",
            1.0,
            6 * 1024 * 1024 * 1024,
            107,
            Some("dave"),
            &[],
            0,
            0,
        ),
        mk(
            "ORANGE",
            7.0,
            300 * 1024 * 1024,
            108,
            Some("eve"),
            &["a", "b"],
            11,
            22,
        ),
    ]
}

fn sample_flows() -> Vec<ProcessFlow> {
    let mk = |sni: Option<&str>, dns: Option<&str>, addr: &str, port: u16, out: u64, inn: u64| {
        ProcessFlow {
            pid: 1,
            start_time: 0,
            comm: String::new(),
            local_addr: String::new(),
            remote_addr: addr.to_string(),
            remote_port: port,
            bytes_out: out,
            bytes_in: inn,
            dns_name: dns.map(str::to_string),
            sni: sni.map(str::to_string),
            first_seen: std::time::SystemTime::UNIX_EPOCH,
            last_seen: std::time::SystemTime::UNIX_EPOCH,
            exit_time: None,
        }
    };
    vec![
        mk(None, None, "1.2.3.4", 443, 0, 0),
        mk(Some("evil.com"), None, "5.6.7.8", 443, 1024, 2048),
        mk(
            Some("api.google.com"),
            Some("google.com"),
            "8.8.8.8",
            443,
            500,
            500,
        ),
        mk(None, Some("example.org"), "9.9.9.9", 80, 1, 2),
        mk(Some("a-b.io"), Some("a-b.io"), "10.0.0.1", 8080, 999_999, 0),
        mk(Some("cdn.site"), None, "0.0.0.0", 0, 0, 0),
    ]
}

fn sample_frame() -> UiFrame {
    UiFrame {
        timestamp: 1_000_000,
        mode: "ProcessList".to_string(),
        status_message: None,
        cpu_usage: 55.0,
        memory_used: 4 * 1024 * 1024 * 1024,
        memory_total: 8 * 1024 * 1024 * 1024,
        net_down: 0,
        net_up: 0,
        cpu_history: Vec::new(),
        mem_history: Vec::new(),
        processes: vec![FrameProcess {
            pid: 1,
            name: "chrome.exe".to_string(),
            cpu: 0.0,
            memory: 0,
            disk_read: 0,
            disk_write: 0,
        }],
        search_query: String::new(),
        sort_field: "Cpu".to_string(),
        process_view_mode: 0,
        tree_nodes: Vec::new(),
        port_entries: Vec::new(),
        port_view_mode: 0,
        port_process_groups: Vec::new(),
        port_remote_groups: Vec::new(),
        connection_diff: FrameConnectionDiff::default(),
        anomalies: vec![FrameAnomaly {
            rule_id: "t".to_string(),
            severity: "warning".to_string(),
            title: "t".to_string(),
            detail: String::new(),
            affected_pid: None,
            affected_ip: None,
        }],
        usb_devices: Vec::new(),
        usb_locks: Vec::new(),
        monitors: Vec::new(),
        docker_containers: Vec::new(),
        docker_events: Vec::new(),
        ops: Vec::new(),
        nav: FrameNav::default(),
    }
}

// ===========================================================================
// AST 生成策略（生成侧约束见文件头注释）
// ===========================================================================

fn process_field_strat() -> impl Strategy<Value = Field> {
    prop::sample::select(vec![
        Field::Cpu,
        Field::Mem,
        Field::Pid,
        Field::Name,
        Field::User,
        Field::Cmd,
        Field::DiskRead,
        Field::DiskWrite,
        Field::NetSent,
        Field::NetRecv,
        Field::SecurityScore,
    ])
}

fn process_text_field_strat() -> impl Strategy<Value = Field> {
    prop::sample::select(vec![Field::Name, Field::User, Field::Cmd])
}

fn network_field_strat() -> impl Strategy<Value = NetworkField> {
    prop::sample::select(vec![
        NetworkField::Sni,
        NetworkField::DnsName,
        NetworkField::RemoteAddr,
        NetworkField::RemotePort,
        NetworkField::BytesOut,
        NetworkField::BytesIn,
    ])
}

fn network_text_field_strat() -> impl Strategy<Value = NetworkField> {
    prop::sample::select(vec![
        NetworkField::Sni,
        NetworkField::DnsName,
        NetworkField::RemoteAddr,
    ])
}

fn cmp_op_strat() -> impl Strategy<Value = CmpOp> {
    prop::sample::select(vec![
        CmpOp::Eq,
        CmpOp::Ne,
        CmpOp::Gt,
        CmpOp::Lt,
        CmpOp::Ge,
        CmpOp::Le,
    ])
}

fn value_strat() -> impl Strategy<Value = Value> {
    prop_oneof![
        (0u32..100_000).prop_map(|n| Value::Number(f64::from(n))),
        (0u32..100).prop_map(|n| Value::Percent(f64::from(n))),
        "[a-zA-Z0-9 ._-]{1,12}".prop_map(Value::Text),
    ]
}

fn expr_strat() -> impl Strategy<Value = FilterExpr> {
    let leaf = prop_oneof![
        (process_field_strat(), cmp_op_strat(), value_strat())
            .prop_map(|(field, op, value)| FilterExpr::FieldCmp { field, op, value },),
        (network_field_strat(), cmp_op_strat(), value_strat())
            .prop_map(|(field, op, value)| FilterExpr::NetworkFieldCmp { field, op, value },),
        (
            process_text_field_strat(),
            "[A-Za-z0-9._-]{1,8}",
            any::<bool>()
        )
            .prop_map(|(field, pattern, ci)| {
                let re_str = if ci {
                    format!("(?i){pattern}")
                } else {
                    pattern.clone()
                };
                FilterExpr::Regex {
                    field,
                    re: regex::Regex::new(&re_str).expect("安全字母表必可编译"),
                    case_insensitive: ci,
                }
            },),
        (
            network_text_field_strat(),
            "[A-Za-z0-9._-]{1,8}",
            any::<bool>()
        )
            .prop_map(|(field, pattern, ci)| {
                let re_str = if ci {
                    format!("(?i){pattern}")
                } else {
                    pattern.clone()
                };
                FilterExpr::NetworkRegex {
                    field,
                    re: regex::Regex::new(&re_str).expect("安全字母表必可编译"),
                    case_insensitive: ci,
                }
            },),
        (
            network_field_strat(),
            proptest::collection::vec(value_strat(), 1..4)
        )
            .prop_map(|(field, vs)| FilterExpr::NetworkIn {
                field,
                values: vs.into_iter().collect(),
            },),
    ];
    leaf.prop_recursive(2, 5, 8, |inner| {
        prop_oneof![
            (inner.clone(), inner.clone())
                .prop_map(|(l, r)| { FilterExpr::And(Box::new(l), Box::new(r)) }),
            (inner.clone(), inner.clone())
                .prop_map(|(l, r)| { FilterExpr::Or(Box::new(l), Box::new(r)) }),
            inner.prop_map(|e| FilterExpr::Not(Box::new(e))),
        ]
    })
}

// ===========================================================================
// 渲染器（AST → filter 语法；只覆盖生成策略产生的变体）
// ===========================================================================

fn field_name(f: Field) -> &'static str {
    match f {
        Field::Cpu => "cpu",
        Field::Mem => "mem",
        Field::Pid => "pid",
        Field::Name => "name",
        Field::User => "user",
        Field::Cmd => "cmd",
        Field::DiskRead => "disk_read",
        Field::DiskWrite => "disk_write",
        Field::NetSent => "net_sent",
        Field::NetRecv => "net_recv",
        Field::SecurityScore => "security_score",
    }
}

fn network_field_name(f: NetworkField) -> &'static str {
    match f {
        NetworkField::Sni => "sni",
        NetworkField::DnsName => "dns_name",
        NetworkField::RemoteAddr => "remote_addr",
        NetworkField::RemotePort => "remote_port",
        NetworkField::BytesOut => "bytes_out",
        NetworkField::BytesIn => "bytes_in",
    }
}

fn op_str(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "=",
        CmpOp::Ne => "!=",
        CmpOp::Gt => ">",
        CmpOp::Lt => "<",
        CmpOp::Ge => ">=",
        CmpOp::Le => "<=",
    }
}

fn render_value(v: &Value) -> String {
    match v {
        Value::Number(n) => format!("{n}"),
        Value::Percent(n) => format!("{n}%"),
        Value::Text(s) => format!("\"{s}\""),
    }
}

/// 从 `re.as_str()` 剥掉 `(?i)` 前缀还原裸 pattern（parser 构造对称物）。
fn strip_inline_ci(re: &regex::Regex) -> String {
    let s = re.as_str();
    s.strip_prefix("(?i)").unwrap_or(s).to_string()
}

fn render_expr(e: &FilterExpr) -> String {
    match e {
        FilterExpr::FieldCmp { field, op, value } => {
            format!(
                "{} {} {}",
                field_name(*field),
                op_str(*op),
                render_value(value)
            )
        }
        FilterExpr::Regex {
            field,
            re,
            case_insensitive,
        } => {
            let flag = if *case_insensitive { "i" } else { "" };
            format!("{} =~ /{}/{flag}", field_name(*field), strip_inline_ci(re))
        }
        FilterExpr::NetworkFieldCmp { field, op, value } => format!(
            "{} {} {}",
            network_field_name(*field),
            op_str(*op),
            render_value(value)
        ),
        FilterExpr::NetworkRegex {
            field,
            re,
            case_insensitive,
        } => {
            let flag = if *case_insensitive { "i" } else { "" };
            format!(
                "{} =~ /{}/{flag}",
                network_field_name(*field),
                strip_inline_ci(re)
            )
        }
        FilterExpr::NetworkIn { field, values } => {
            let mut rendered: Vec<String> = values.iter().map(render_value).collect();
            rendered.sort();
            format!(
                "{} in ({})",
                network_field_name(*field),
                rendered.join(", ")
            )
        }
        FilterExpr::And(l, r) => format!("({}) AND ({})", render_expr(l), render_expr(r)),
        FilterExpr::Or(l, r) => format!("({}) OR ({})", render_expr(l), render_expr(r)),
        FilterExpr::Not(inner) => format!("NOT ({})", render_expr(inner)),
        // Frame 变体不在 Process 模式 roundtrip 生成范围内（parse() 不识别
        // frame 专属字段），策略不产生——到达即生成侧约束被破坏。
        _ => unreachable!("生成策略不产生 Frame 变体"),
    }
}

// ===========================================================================
// 属性
// ===========================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// 属性 1：parse(render(ast)) 与 ast 语义等价（样本电池行为逐一相同）。
    #[test]
    fn roundtrip_semantic_equivalence(expr in expr_strat()) {
        let rendered = render_expr(&expr);
        let reparsed = proc::filter::parse(&rendered)
            .unwrap_or_else(|e| panic!("渲染串应可重解析: {rendered:?}（err: {e}）"));

        let procs = sample_processes();
        for p in &procs {
            let ctx = EvalCtx {
                process: p,
                security_score: Some(42),
                total_memory: 8 * 1024 * 1024 * 1024,
            };
            prop_assert_eq!(
                expr.apply(&ctx),
                reparsed.apply(&ctx),
                "rendered = {:?}",
                rendered
            );
        }

        let flows = sample_flows();
        for f in &flows {
            let nctx = NetworkEvalCtx { flow: f };
            prop_assert_eq!(
                expr.apply_network(&nctx),
                reparsed.apply_network(&nctx),
                "rendered = {:?}",
                rendered
            );
        }
    }

    /// 属性 2：任意字符串 parse / parse_frame 绝不 panic；成功解析的表达式
    /// 对样本电池求值也不 panic（容错契约）。
    #[test]
    fn arbitrary_input_never_panics(input in ".*") {
        if let Ok(expr) = proc::filter::parse(&input) {
            let procs = sample_processes();
            for p in &procs {
                let ctx = EvalCtx {
                    process: p,
                    security_score: None,
                    total_memory: 0,
                };
                let _ = expr.apply(&ctx);
            }
            let flows = sample_flows();
            for f in &flows {
                let _ = expr.apply_network(&NetworkEvalCtx { flow: f });
            }
        }
        if let Ok(expr) = proc::filter::parse_frame(&input) {
            let frame = sample_frame();
            let _ = expr.apply_frame(&FrameEvalCtx { frame: &frame });
        }
    }
}
