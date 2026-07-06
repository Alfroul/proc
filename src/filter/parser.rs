//! 过滤表达式 parser（nom 7）。
//!
//! 递归下降：
//!
//! ```text
//! expr    := or
//! or      := and ("OR" and)*
//! and     := not ("AND" not)*
//! not     := "NOT"? primary
//! primary := "(" expr ")" | leaf
//! leaf    := field cmp_op value | field "=~" regex_lit
//! ```
//!
//! 设计要点：
//! - 关键字（AND/OR/NOT）大小写敏感，且需词边界（"OR" 不匹配 "ORANGE" 前缀）
//! - 操作符最长匹配（`>=` 在 `>` 前）
//! - 数字字面量可带单位（`b/kb/mb/gb/tb/%`），1024 进制
//! - 字符串字面量：裸字符串（非空白/括号/操作符）或双引号 `"..."`（含空格）
//! - 正则字面量：`/pattern/i?`，`i` 后缀编译为内嵌 `(?i)` 标志

use std::collections::HashSet;

use nom::{
    Err as NomErr, IResult,
    branch::alt,
    bytes::complete::{tag, take_till1, take_while1},
    character::complete::{char, digit1, multispace0},
    combinator::{cut, opt, recognize, value},
    error::{ErrorKind, ParseError as NomParseError, VerboseError, VerboseErrorKind},
    multi::many0,
    sequence::{pair, tuple},
};

use super::{CmpOp, Field, FilterExpr, FrameField, NetworkField, Value};

/// Parser 错误。`position` 是字节偏移（与 input.as_bytes()[position] 对齐）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub msg: String,
    pub position: usize,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "filter parse error at offset {}: {}",
            self.position, self.msg
        )
    }
}

impl std::error::Error for ParseError {}

type NomRes<'a, T> = IResult<&'a str, T, VerboseError<&'a str>>;

/// v0.14 阶段 3：parser 模式 — Process 模式（默认）/ Frame 模式（timeline 搜索）。
///
/// Process 模式：`cpu` / `mem` / `name` 解析成 [`Field`]（ProcessInfo 字段），
/// 用于 List / Tree / AppGroup 视图过滤，与 v0.7 阶段 4 行为一致。
/// Frame 模式：`cpu` / `mem` 解析成 [`FrameField`]（UiFrame 字段），额外支持
/// `timestamp` / `anomaly.severity` 字段；`name` 也走 [`FrameField::Name`]。
/// Network 字段在两种模式下都识别（保险，不强制 — 实际 timeline 搜索不会用
/// network 字段，但允许写 `cpu > 80 AND sni =~ /evil/` 这类混合表达式）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseMode {
    Process,
    Frame,
}

/// 入口：把整个 input 解析为单个 [`FilterExpr`]（Process 模式），trailing
/// 非空白字符算错误。
///
/// v0.14 阶段 3：本入口是 Process 模式默认入口；timeline 搜索请走 [`parse_frame`]。
pub fn parse(input: &str) -> Result<FilterExpr, ParseError> {
    parse_with_mode(input, ParseMode::Process)
}

/// v0.14 阶段 3：Frame 模式入口（timeline 搜索）。`cpu` / `mem` / `name` 解析成
/// [`FrameField`]，额外支持 `timestamp` / `anomaly.severity`。
pub fn parse_frame(input: &str) -> Result<FilterExpr, ParseError> {
    parse_with_mode(input, ParseMode::Frame)
}

fn parse_with_mode(input: &str, mode: ParseMode) -> Result<FilterExpr, ParseError> {
    match parse_expr(input, mode) {
        Ok((rest, expr)) => {
            let trimmed = rest.trim_start();
            if trimmed.is_empty() {
                Ok(expr)
            } else {
                Err(ParseError {
                    msg: format!("输入末尾出现多余内容：{:?}", trimmed),
                    position: input.len() - trimmed.len(),
                })
            }
        }
        Err(e) => Err(to_parse_error(input, e)),
    }
}

fn to_parse_error(input: &str, e: NomErr<VerboseError<&str>>) -> ParseError {
    match e {
        NomErr::Incomplete(_) => ParseError {
            msg: "输入不完整".to_string(),
            position: input.len(),
        },
        NomErr::Error(ve) | NomErr::Failure(ve) => {
            // VerboseError 的 frames 顺序：errors[0] = 最内层（最接近失败点），
            // errors[N-1] = 最外层。取最内层让 position 指向实际失败的字符位置，
            // 而不是整段 input 的起点（外层通常会 reset 到 input 开头）。
            let (offset, msg) = ve
                .errors
                .first()
                .map(|(sub, kind)| {
                    let off = input.len() - sub.len();
                    let m = match kind {
                        // v0.11 阶段 3：未知字段名 — 把 input[off..] 处的标识符
                        // 提取出来塞进提示，让用户一眼看出是哪个字段名拼错。
                        // parse_field 的未知分支故意把错误锚点（`sub`）设在
                        // 标识符起点（`i`，而非 `after_ident`），所以 off 就是
                        // 未知字段名的首字符位置。
                        VerboseErrorKind::Nom(ErrorKind::AlphaNumeric) => {
                            unknown_field_message(&input[off..])
                        }
                        VerboseErrorKind::Nom(k) => error_kind_to_chinese(k).to_string(),
                        VerboseErrorKind::Context(s) => (*s).to_string(),
                        VerboseErrorKind::Char(c) => char_to_chinese(*c).to_string(),
                    };
                    (off, m)
                })
                .unwrap_or((0, "解析失败".to_string()));
            ParseError {
                msg,
                position: offset,
            }
        }
    }
}

/// v0.11 阶段 3：从「未知字段名」错误锚点处抽出标识符，构造友好提示。
///
/// `rest` 是错误锚点之后的剩余输入（即从未知字段名首字符开始）。提取首个
/// `[A-Za-z0-9_]+` 段当作未知字段名。提不到（边界情况）→ 退到无标识符版本。
#[must_use]
fn unknown_field_message(rest: &str) -> String {
    const SUPPORTED: &str = "cpu/mem/pid/name/user/cmd/disk_read/disk_write/net_sent/net_recv/security_score/sni/dns_name/remote_addr/remote_port/bytes_out/bytes_in/timestamp/anomaly.severity";
    let ident: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if ident.is_empty() {
        format!("未知字段名（支持 {SUPPORTED}）")
    } else {
        format!("未知字段名：{ident}（支持 {SUPPORTED}）")
    }
}

/// nom `VerboseErrorKind::Char(c)` → 中文映射（TD-16）。
///
/// 括号 / 引号 / 斜杠这几个特殊字符直接给出语义化提示，其他字符回退到
/// `预期字符 'X'`。括号闭合错误（`Char(')')`）是 `(cpu > 5` 这类 unbalanced
/// paren 场景最常见的，必须给出「括号」字样让 UI 标题栏一眼可读。
#[must_use]
fn char_to_chinese(c: char) -> &'static str {
    match c {
        '(' | ')' => "缺少括号",
        '"' => "缺少引号",
        '/' => "缺少斜杠",
        _ => "缺少字符",
    }
}

/// nom `ErrorKind` → 中文字符串映射（TD-16）。
///
/// 用户在 TUI / CLI 看到 `expected TakeWhile1` 这种 nom 内部枚举名根本无法
/// 理解——映射成「缺少字段名/值」「缺少关键字/操作符」等可读提示。未在表内的
/// `ErrorKind` 兜底「语法错误」。
///
/// 映射覆盖范围：parser.rs 实际会触发的 ErrorKind（TakeWhile1 / Tag / AlphaNumeric /
/// Char / Digit / Verify）。其余 nom ErrorKind 在当前 parser 不会触达，但保留
/// 兜底分支以防未来扩展。
#[must_use]
fn error_kind_to_chinese(kind: &ErrorKind) -> &'static str {
    match kind {
        // take_while1 / take_till1：字段名、裸字符串、正则体等「至少 1 字符」匹配失败。
        ErrorKind::TakeWhile1 | ErrorKind::TakeTill1 => "缺少字段名/值",
        // tag：关键字（AND/OR/NOT）或操作符（=~ / >= 等）字面量未匹配。
        ErrorKind::Tag => "缺少关键字/操作符",
        // char：单字符（括号 / 引号 / 斜杠）未匹配。
        ErrorKind::Char => "缺少字符（括号/引号/斜杠）",
        // AlphaNumeric：字段名解析走 take_while1，未知字段名（如 `foo`）会落到
        // parse_field 内部 AlphaNumeric 错误分支。
        ErrorKind::AlphaNumeric => "未知字段名",
        ErrorKind::Alpha => "缺少字母",
        ErrorKind::Digit => "数字格式错误",
        // Verify：目前仅在 regex 编译失败时使用（leaf 中 regex::Regex::new 失败）。
        ErrorKind::Verify => "正则编译失败",
        // Float：parse_number_value 内部 raw.parse::<f64>() 失败时使用。
        ErrorKind::Float => "数字格式错误",
        ErrorKind::Eof => "未到输入结尾",
        ErrorKind::MultiSpace | ErrorKind::Space => "缺少空白",
        _ => "语法错误",
    }
}

// --- 递归下降 ---

fn parse_expr(i: &str, mode: ParseMode) -> NomRes<'_, FilterExpr> {
    parse_or(i, mode)
}

fn parse_or(i: &str, mode: ParseMode) -> NomRes<'_, FilterExpr> {
    let (i, first) = parse_and(i, mode)?;
    // AND/OR 关键字两侧必须吃空白 —— `cpu > 5 AND mem > 100` 中 AND 前后都有空格，
    // 否则 tag("AND") 会因 leading space 失败让 many0 提前结束。
    let (i, rest) = many0(tuple((multispace0, keyword("OR"), multispace0, |i| {
        parse_and(i, mode)
    })))(i)?;
    let result = rest.into_iter().fold(first, |acc, (_, _, _, next)| {
        FilterExpr::Or(Box::new(acc), Box::new(next))
    });
    Ok((i, result))
}

fn parse_and(i: &str, mode: ParseMode) -> NomRes<'_, FilterExpr> {
    let (i, first) = parse_not(i, mode)?;
    let (i, rest) = many0(tuple((multispace0, keyword("AND"), multispace0, |i| {
        parse_not(i, mode)
    })))(i)?;
    let result = rest.into_iter().fold(first, |acc, (_, _, _, next)| {
        FilterExpr::And(Box::new(acc), Box::new(next))
    });
    Ok((i, result))
}

fn parse_not(i: &str, mode: ParseMode) -> NomRes<'_, FilterExpr> {
    let (i, not_kw) = opt(tuple((keyword("NOT"), multispace0)))(i)?;
    let (i, primary) = parse_primary(i, mode)?;
    Ok((
        i,
        if not_kw.is_some() {
            FilterExpr::Not(Box::new(primary))
        } else {
            primary
        },
    ))
}

fn parse_primary(i: &str, mode: ParseMode) -> NomRes<'_, FilterExpr> {
    let (i, _) = multispace0(i)?;
    // 先尝试括号子表达式，再退到 leaf。
    alt((|i| paren_expr(i, mode), |i| leaf(i, mode)))(i)
}

fn paren_expr(i: &str, mode: ParseMode) -> NomRes<'_, FilterExpr> {
    let (i, _) = char('(')(i)?;
    let (i, _) = multispace0(i)?;
    let (i, expr) = parse_expr(i, mode)?;
    let (i, _) = multispace0(i)?;
    // TD-16：括号闭合失败用 cut 转 Failure，让 alt 不回退到 leaf。
    // 否则 `(cpu > 5` 缺 `)` 时 alt 会 fallback 到 leaf 解析整个 `(cpu > 5`，
    // 最内层错误变成 leaf 的 TakeWhile1（「缺少字段名/值」），丢失「括号没闭合」
    // 这条真正有用的信息。
    let (i, _) = cut(char(')'))(i)?;
    Ok((i, expr))
}

fn leaf(i: &str, mode: ParseMode) -> NomRes<'_, FilterExpr> {
    let (i, field) = parse_field_with(i, mode)?;
    let (i, _) = multispace0(i)?;
    // 先尝试 regex 路径（=~），不命中再走 in / cmp。
    let (i, regex_path) = opt(tuple((tag("=~"), multispace0, parse_regex_lit)))(i)?;
    if let Some((_, _, (pattern, ci))) = regex_path {
        let re_str = if ci {
            format!("(?i){pattern}")
        } else {
            pattern.clone()
        };
        let re = regex::Regex::new(&re_str)
            .map_err(|_| NomErr::Failure(VerboseError::from_error_kind(i, ErrorKind::Verify)))?;
        let expr = match field {
            ParsedField::Process(f) => FilterExpr::Regex {
                field: f,
                re,
                case_insensitive: ci,
            },
            ParsedField::Network(f) => FilterExpr::NetworkRegex {
                field: f,
                re,
                case_insensitive: ci,
            },
            ParsedField::Frame(f) => FilterExpr::FrameRegex {
                field: f,
                re,
                case_insensitive: ci,
            },
        };
        return Ok((i, expr));
    }
    // v0.11 阶段 3 / v0.14 阶段 3：`in` 操作符（Network + Frame 字段支持；
    // process 字段暂不支持 — surgical：仅在 Flow / Timeline view 设计文档要求）。
    let (i, in_path) = opt(tuple((keyword("in"), multispace0, parse_in_list)))(i)?;
    if let Some((_, _, values)) = in_path {
        let expr = match field {
            ParsedField::Process(_) => {
                // process 字段不支持 in（surgical：仅 network / frame 字段支持）。
                // cut → Failure 让 alt 不回退；错误锚点回退到字段起点附近。
                return Err(NomErr::Failure(VerboseError::from_error_kind(
                    i,
                    ErrorKind::Tag,
                )));
            }
            ParsedField::Network(f) => FilterExpr::NetworkIn { field: f, values },
            ParsedField::Frame(f) => FilterExpr::FrameIn { field: f, values },
        };
        return Ok((i, expr));
    }
    let (i, op) = parse_cmp_op(i)?;
    let (i, _) = multispace0(i)?;
    let (i, value) = parse_value(i)?;
    let expr = match field {
        ParsedField::Process(f) => FilterExpr::FieldCmp {
            field: f,
            op,
            value,
        },
        ParsedField::Network(f) => FilterExpr::NetworkFieldCmp {
            field: f,
            op,
            value,
        },
        ParsedField::Frame(f) => FilterExpr::FrameFieldCmp {
            field: f,
            op,
            value,
        },
    };
    Ok((i, expr))
}

/// parser 内部用的字段归属：process 系（作用 `ProcessInfo`）/ network 系
/// （作用 `ProcessFlow`）/ frame 系（作用 `UiFrame`，v0.14 阶段 3 新增）。
/// `leaf` 拿到归属后再决定构造 `FieldCmp` / `Regex` / `NetworkFieldCmp` /
/// `NetworkRegex` / `NetworkIn` / `FrameFieldCmp` / `FrameRegex` / `FrameIn`。
#[derive(Debug, Clone, Copy)]
enum ParsedField {
    Process(Field),
    Network(NetworkField),
    Frame(FrameField),
}

/// v0.14 阶段 3：parse_field 切到 parse_field_with(input, mode)。
/// 既有调用点（Process 模式）走 [`parse_field_with`]`(_, ParseMode::Process)`；
/// timeline 搜索走 Frame 模式。
fn parse_field_with(i: &str, mode: ParseMode) -> NomRes<'_, ParsedField> {
    let (after_ident, ident) = take_while1(|c: char| c.is_ascii_alphanumeric() || c == '_')(i)?;
    // v0.14 阶段 3：anomaly.severity 含点号 → 用专门的 take_while1 包含 `.`。
    // 但 parser 主流字段不含 `.`，特殊处理 anomaly.severity 在 take_while1 之外
    // 走显式 prefix match（让 `.` 仍是 terminator）。
    let field = match (ident, mode) {
        // 共有：network 字段（在两种模式下都识别 — 让混合表达式如 `cpu > 80 AND
        // sni =~ /evil/` 在 timeline 搜索也能写。Flow 视图与 timeline 视图都不
        // 调对方 ctx 的 apply 方法，所以网络字段在 timeline 中只会让表达式失败
        // 命中，不会跨 ctx 误用数据）
        ("sni", _) => ParsedField::Network(NetworkField::Sni),
        ("dns_name" | "dnsname", _) => ParsedField::Network(NetworkField::DnsName),
        ("remote_addr" | "remoteaddr", _) => ParsedField::Network(NetworkField::RemoteAddr),
        ("remote_port" | "remoteport", _) => ParsedField::Network(NetworkField::RemotePort),
        ("bytes_out" | "bytesout", _) => ParsedField::Network(NetworkField::BytesOut),
        ("bytes_in" | "bytesin", _) => ParsedField::Network(NetworkField::BytesIn),
        // Process 模式专属（与 v0.7 阶段 4 行为一致）
        ("cpu", ParseMode::Process) => ParsedField::Process(Field::Cpu),
        ("mem" | "memory", ParseMode::Process) => ParsedField::Process(Field::Mem),
        ("pid", ParseMode::Process) => ParsedField::Process(Field::Pid),
        ("name", ParseMode::Process) => ParsedField::Process(Field::Name),
        ("user", ParseMode::Process) => ParsedField::Process(Field::User),
        ("cmd", ParseMode::Process) => ParsedField::Process(Field::Cmd),
        ("disk_read" | "diskread", ParseMode::Process) => ParsedField::Process(Field::DiskRead),
        ("disk_write" | "diskwrite", ParseMode::Process) => ParsedField::Process(Field::DiskWrite),
        ("net_sent" | "netsent", ParseMode::Process) => ParsedField::Process(Field::NetSent),
        ("net_recv" | "netrecv", ParseMode::Process) => ParsedField::Process(Field::NetRecv),
        ("security_score" | "security", ParseMode::Process) => {
            ParsedField::Process(Field::SecurityScore)
        }
        // Frame 模式专属（v0.14 阶段 3 新增）
        ("cpu", ParseMode::Frame) => ParsedField::Frame(FrameField::Cpu),
        ("mem" | "memory", ParseMode::Frame) => ParsedField::Frame(FrameField::Mem),
        ("name", ParseMode::Frame) => ParsedField::Frame(FrameField::Name),
        ("timestamp" | "ts", ParseMode::Frame) => ParsedField::Frame(FrameField::Timestamp),
        ("severity", ParseMode::Frame) => ParsedField::Frame(FrameField::AnomalySeverity),
        _ => {
            // v0.14 阶段 3：检查是不是 `anomaly.severity`（点号需在 take_while1
            // 之外显式 match，因为 take_while1 终止于 `.`）。
            if mode == ParseMode::Frame && (ident == "anomaly") {
                // 试 `anomaly.severity`
                if let Ok((rest, _)) =
                    tag::<&str, &str, VerboseError<&str>>(".severity")(after_ident)
                {
                    return Ok((rest, ParsedField::Frame(FrameField::AnomalySeverity)));
                }
            }
            // 错误锚点放在字段名起点（i），让 to_parse_error 能从 input[off..]
            // 提取出未知字段名拼出友好提示（v0.11 阶段 3）。
            return Err(NomErr::Error(VerboseError::from_error_kind(
                i,
                ErrorKind::AlphaNumeric,
            )));
        }
    };
    Ok((after_ident, field))
}

/// v0.11 阶段 3：`in (v1, v2, ...)` 列表解析。至少 1 个值；trailing `,` 不允许。
/// 闭合 `)` 用 `cut` 让错误不被 alt 吞掉（与 paren_expr 同款）。
///
/// **v0.12 阶段 5（TD-29）**：返回 `HashSet<Value>` 而非 `Vec<Value>`，让
/// `FilterExpr::NetworkIn` apply 路径 `contains` 是 O(1)。重复值会被去重
/// （`sni in ("a", "a")` 等价于 `sni in ("a")`），与 `in` 集合语义一致。
fn parse_in_list(i: &str) -> NomRes<'_, HashSet<Value>> {
    let (i, _) = char('(')(i)?;
    let (i, _) = multispace0(i)?;
    let (i, first) = parse_value(i)?;
    let (i, rest) = many0(tuple((multispace0, char(','), multispace0, parse_value)))(i)?;
    let (i, _) = multispace0(i)?;
    let (i, _) = cut(char(')'))(i)?;
    let mut values = HashSet::with_capacity(rest.len() + 1);
    values.insert(first);
    for (_, _, _, v) in rest {
        values.insert(v);
    }
    Ok((i, values))
}

fn parse_cmp_op(i: &str) -> NomRes<'_, CmpOp> {
    alt((
        value(CmpOp::Ge, tag(">=")),
        value(CmpOp::Le, tag("<=")),
        value(CmpOp::Ne, tag("!=")),
        value(CmpOp::Eq, tag("=")),
        value(CmpOp::Gt, tag(">")),
        value(CmpOp::Lt, tag("<")),
    ))(i)
}

fn parse_value(i: &str) -> NomRes<'_, Value> {
    let starts_digit = i
        .chars()
        .next()
        .map(|c| c.is_ascii_digit() || c == '-')
        .unwrap_or(false);
    if starts_digit {
        let (i, val) = parse_number_value(i)?;
        return Ok((i, val));
    }
    if i.starts_with('"') {
        let (i, s) = parse_quoted_string(i)?;
        return Ok((i, Value::Text(s)));
    }
    let (i, s) = bare_string_token(i)?;
    Ok((i, Value::Text(s.to_string())))
}

fn parse_number_value(i: &str) -> NomRes<'_, Value> {
    let (i, raw) = recognize(tuple((
        opt(char('-')),
        digit1,
        opt(pair(char('.'), digit1)),
    )))(i)?;
    let num: f64 = raw
        .parse()
        .map_err(|_| NomErr::Error(VerboseError::from_error_kind(i, ErrorKind::Digit)))?;
    // 单位后缀：1-2 字母或 %。注意 `b` 必须放最后，否则 `kb/gb/mb/tb` 的 `b` 会被
    // 错误识别。alt 短路求值，所以先长后短。
    let (i, unit) = opt(alt((
        tag("%"),
        tag("tb"),
        tag("gb"),
        tag("mb"),
        tag("kb"),
        tag("b"),
    )))(i)?;
    let value = match unit {
        None | Some("b") => Value::Number(num),
        Some("%") => Value::Percent(num),
        Some("kb") => Value::Number(num * 1024.0),
        Some("mb") => Value::Number(num * 1024.0_f64.powi(2)),
        Some("gb") => Value::Number(num * 1024.0_f64.powi(3)),
        Some("tb") => Value::Number(num * 1024.0_f64.powi(4)),
        _ => unreachable!("unit alt covers all branches"),
    };
    Ok((i, value))
}

fn parse_quoted_string(i: &str) -> NomRes<'_, String> {
    let (i, _) = char('"')(i)?;
    let (i, s) = take_till1(|c| c == '"')(i)?;
    let (i, _) = char('"')(i)?;
    Ok((i, s.to_string()))
}

fn parse_regex_lit(i: &str) -> NomRes<'_, (String, bool)> {
    let (i, _) = char('/')(i)?;
    // v0.12 阶段 4（TD-28）：支持 `\/` 转义，让 `/192\.168\.1\.0\/24/` 这类
    // 含 `/` 的 pattern（CIDR / URL / 路径）能写出来。状态机扫描：
    //   - 遇 `\` → 看下一字符：
    //       - 是 `/` → pattern 追加单 `/`（去掉转义反斜杠，regex 不需要 `\/`）；
    //       - 其他字符（`.` / `d` / `w` / `s` 等 regex 元字符）→ `\X` 原样保留，
    //         让 regex crate 自行解释（如 `\d` 是数字字符类）。
    //   - 遇非转义 `/` → pattern 结束；
    //   - 其他字符 → 原样追加。
    // 兼容性：旧表达式（无 `\/`）行为不变；`\` 后再无字符 → 非法（未闭合）。
    let mut pattern = String::new();
    let mut rest = i;
    let mut closed = false;
    loop {
        match rest.chars().next() {
            None => break,
            Some('/') => {
                rest = &rest[1..];
                closed = true;
                break;
            }
            Some('\\') => {
                rest = &rest[1..];
                match rest.chars().next() {
                    Some('/') => {
                        // 用户转义的是 regex literal 的分隔符 `/`，传给 regex
                        // crate 时是字面 `/`（不是 `\/`——后者会被 regex 拒绝
                        // 为「无效转义」）。
                        pattern.push('/');
                        rest = &rest[1..];
                    }
                    Some(c2) => {
                        // 其他 `\X` 转义（`\d` `\.` `\w` 等）原样保留。
                        pattern.push('\\');
                        pattern.push(c2);
                        rest = &rest[c2.len_utf8()..];
                    }
                    None => {
                        // `\` 在末尾 → 仍未闭合，让下面的 closed=false 走错误路径。
                        break;
                    }
                }
            }
            Some(c) => {
                pattern.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    if !closed {
        return Err(NomErr::Error(VerboseError::from_error_kind(
            rest,
            ErrorKind::Char,
        )));
    }
    let (i, ci) = opt(char('i'))(rest)?;
    Ok((i, (pattern, ci.is_some())))
}

/// 裸字符串 token：读到空白 / 括号 / 操作符字符为止。允许 `.`/`-`/`_` 等进程名
/// 常见字符。包含空格的字符串需用 `"..."`。
fn bare_string_token(i: &str) -> NomRes<'_, &str> {
    take_while1(|c: char| !c.is_whitespace() && !matches!(c, '(' | ')' | '=' | '!' | '>' | '<'))(i)
}

/// 关键字匹配：tag(kw) + 词边界检查（下一字符不能是 alphanumeric/_）。
/// 这样 `OR` 不会匹配 `ORANGE` 前缀；同时大小写敏感（`or` 不匹配 `OR`）。
fn keyword(kw: &'static str) -> impl Fn(&str) -> NomRes<'_, &str> + Copy {
    move |i: &str| {
        let (rest, matched) = tag(kw)(i)?;
        let next_is_word = rest
            .chars()
            .next()
            .map(|c| c.is_alphanumeric() || c == '_')
            .unwrap_or(false);
        if next_is_word {
            return Err(NomErr::Error(VerboseError::from_error_kind(
                rest,
                ErrorKind::Tag,
            )));
        }
        Ok((rest, matched))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_cmp() {
        let e = parse("cpu > 5").unwrap();
        match e {
            FilterExpr::FieldCmp {
                field: Field::Cpu,
                op: CmpOp::Gt,
                value: Value::Number(n),
            } => assert!((n - 5.0).abs() < 1e-9),
            other => panic!("expected FieldCmp, got {:?}", other),
        }
    }

    #[test]
    fn parse_and() {
        let e = parse("cpu > 5 AND mem > 100").unwrap();
        assert!(matches!(e, FilterExpr::And(_, _)));
    }

    #[test]
    fn parse_or() {
        let e = parse("cpu > 5 OR cpu < 1").unwrap();
        assert!(matches!(e, FilterExpr::Or(_, _)));
    }

    #[test]
    fn parse_not() {
        let e = parse("NOT cpu > 5").unwrap();
        assert!(matches!(e, FilterExpr::Not(_)));
    }

    #[test]
    fn parse_parens_with_complex() {
        let e = parse("(cpu > 5 OR mem > 100) AND NOT user = root").unwrap();
        // 不爆 panic 就过；进一步断言最外层是 And。
        assert!(matches!(e, FilterExpr::And(_, _)));
    }

    #[test]
    fn parse_regex_with_flag() {
        let e = parse("name =~ /chrome/i").unwrap();
        match e {
            FilterExpr::Regex {
                case_insensitive, ..
            } => assert!(case_insensitive),
            other => panic!("expected Regex, got {:?}", other),
        }
    }

    #[test]
    fn parse_regex_no_flag() {
        let e = parse("cmd =~ /--headless/").unwrap();
        assert!(matches!(e, FilterExpr::Regex { .. }));
    }

    #[test]
    fn parse_unit_mb() {
        let e = parse("mem > 100mb").unwrap();
        match e {
            FilterExpr::FieldCmp {
                value: Value::Number(n),
                ..
            } => assert!((n - 100.0 * 1024.0 * 1024.0).abs() < 1.0),
            other => panic!("expected FieldCmp Number, got {:?}", other),
        }
    }

    #[test]
    fn parse_unit_gb() {
        let e = parse("mem > 1gb").unwrap();
        if let FilterExpr::FieldCmp {
            value: Value::Number(n),
            ..
        } = e
        {
            assert!((n - 1024.0_f64.powi(3)).abs() < 1.0);
        }
    }

    #[test]
    fn parse_percent() {
        let e = parse("cpu > 5%").unwrap();
        match e {
            FilterExpr::FieldCmp {
                value: Value::Percent(n),
                ..
            } => assert!((n - 5.0).abs() < 1e-9),
            other => panic!("expected FieldCmp Percent, got {:?}", other),
        }
    }

    #[test]
    fn parse_quoted_string() {
        let e = parse(r#"name = "chrome exe""#).unwrap();
        if let FilterExpr::FieldCmp {
            value: Value::Text(s),
            ..
        } = e
        {
            assert_eq!(s, "chrome exe");
        }
    }

    #[test]
    fn parse_bare_string_with_dot() {
        let e = parse("name = chrome.exe").unwrap();
        if let FilterExpr::FieldCmp {
            value: Value::Text(s),
            ..
        } = e
        {
            assert_eq!(s, "chrome.exe");
        }
    }

    #[test]
    fn parse_keyword_word_boundary() {
        // ORANGE 不应被识别为 OR + ANGE。
        assert!(parse("name = ORANGE").is_ok());
        // 中间的 AND 必须是独立 token。
        let e = parse("cpu > 5 AND mem > 100").unwrap();
        assert!(matches!(e, FilterExpr::And(_, _)));
    }

    #[test]
    fn err_double_gt() {
        assert!(parse("cpu >> 5").is_err());
    }

    #[test]
    fn err_missing_value() {
        assert!(parse("cpu >").is_err());
    }

    #[test]
    fn err_missing_op() {
        assert!(parse("cpu 5").is_err());
    }

    #[test]
    fn err_unbalanced_paren() {
        assert!(parse("(cpu > 5").is_err());
    }

    #[test]
    fn err_unbalanced_paren_close_only() {
        assert!(parse("cpu > 5)").is_err());
    }

    #[test]
    fn err_unknown_field() {
        assert!(parse("foo > 5").is_err());
    }

    #[test]
    fn err_trailing_input() {
        assert!(parse("cpu > 5 extra").is_err());
    }

    #[test]
    fn err_invalid_regex() {
        // 不闭合的正则：`/chrome` 后没有 `/`
        assert!(parse("name =~ /chrome").is_err());
    }

    #[test]
    fn err_bad_regex_pattern() {
        // 非法正则：(unclosed
        assert!(parse("name =~ /(unclosed/").is_err());
    }

    #[test]
    fn case_sensitive_keywords() {
        // 小写 and 不应被识别为关键字 AND。
        let r = parse("cpu > 5 and mem > 100");
        // 这会读成 cpu > 5，然后 and mem > 100 是 trailing —— 报错。
        assert!(r.is_err());
    }

    #[test]
    fn empty_input_errors() {
        assert!(parse("").is_err());
    }

    #[test]
    fn whitespace_only_errors() {
        assert!(parse("   ").is_err());
    }

    #[test]
    fn error_has_position() {
        // 缺值：错位应在 input 末尾。
        let e = parse("cpu >").unwrap_err();
        assert!(e.position > 0);
        assert!(!e.msg.is_empty());
    }

    // --- v0.11 阶段 3：network 字段 + in 操作符 ---

    #[test]
    fn parse_network_sni_regex() {
        let e = parse("sni =~ /google\\.com$/").unwrap();
        match e {
            FilterExpr::NetworkRegex { field, .. } => {
                assert_eq!(field, NetworkField::Sni);
            }
            other => panic!("expected NetworkRegex, got {:?}", other),
        }
    }

    #[test]
    fn parse_network_dns_name_eq() {
        // v0.7 语法约定：相等用 `=`（与 `user = root` / `name = chrome.exe` 同款）。
        let e = parse("dns_name = \"example.com\"").unwrap();
        match e {
            FilterExpr::NetworkFieldCmp {
                field,
                op: CmpOp::Eq,
                value: Value::Text(s),
            } => {
                assert_eq!(field, NetworkField::DnsName);
                assert_eq!(s, "example.com");
            }
            other => panic!("expected NetworkFieldCmp, got {:?}", other),
        }
    }

    #[test]
    fn parse_network_remote_addr_in() {
        let e = parse(r#"remote_addr in ("1.2.3.4", "5.6.7.8")"#).unwrap();
        match e {
            FilterExpr::NetworkIn { field, values } => {
                assert_eq!(field, NetworkField::RemoteAddr);
                assert_eq!(values.len(), 2);
            }
            other => panic!("expected NetworkIn, got {:?}", other),
        }
    }

    #[test]
    fn parse_network_remote_port_cmp() {
        let e = parse("remote_port = 443").unwrap();
        match e {
            FilterExpr::NetworkFieldCmp {
                field,
                op: CmpOp::Eq,
                value: Value::Number(n),
            } => {
                assert_eq!(field, NetworkField::RemotePort);
                assert!((n - 443.0).abs() < 1e-9);
            }
            other => panic!("expected NetworkFieldCmp, got {:?}", other),
        }
    }

    #[test]
    fn parse_network_source_field_now_unknown() {
        // v0.12 阶段 2：source 字段已删除（Windows-only 后唯一来源是 Schannel），
        // parse 应返未知字段错误。
        let result = parse("source = schannel");
        assert!(result.is_err(), "source 字段已移除，parse 应失败");
    }

    #[test]
    fn parse_network_bytes_unit() {
        // bytes_out 字段允许字节单位字面量。
        let e = parse("bytes_out > 1kb").unwrap();
        match e {
            FilterExpr::NetworkFieldCmp {
                field,
                value: Value::Number(n),
                ..
            } => {
                assert_eq!(field, NetworkField::BytesOut);
                assert!((n - 1024.0).abs() < 1e-9);
            }
            other => panic!("expected NetworkFieldCmp, got {:?}", other),
        }
    }

    #[test]
    fn parse_network_in_single_value() {
        // 单值 in 也合法（语义等价于 =，但语法允许）。
        let e = parse(r#"sni in ("a.com")"#).unwrap();
        assert!(matches!(e, FilterExpr::NetworkIn { .. }));
    }

    #[test]
    fn parse_network_combined_with_and() {
        // 网络表达式 AND 组合，确认网络字段在布尔结构里也能解析。
        let e = parse(r#"sni =~ /evil\.com/ AND remote_port = 443"#).unwrap();
        assert!(matches!(e, FilterExpr::And(_, _)));
    }

    #[test]
    fn parse_backward_compat_old_expr_still_works() {
        // v0.7/v0.8 契约：纯 process 字段表达式不变。
        let e = parse("cpu > 5 AND name =~ /chrome/").unwrap();
        assert!(matches!(e, FilterExpr::And(_, _)));
    }

    #[test]
    fn err_unknown_field_message_contains_supported_list() {
        // 未知字段错误信息含「未知字段名」+ 支持列表（v0.11 阶段 3 增强）。
        let e = parse("foo > 5").unwrap_err();
        assert!(
            e.msg.contains("未知"),
            "expected 含「未知」的提示, got: {}",
            e.msg
        );
        assert!(
            e.msg.contains("sni"),
            "expected 含支持列表（含 sni）, got: {}",
            e.msg
        );
        assert!(
            e.msg.contains("cpu"),
            "expected 含支持列表（含 cpu）, got: {}",
            e.msg
        );
        assert!(
            !e.msg.contains("AlphaNumeric"),
            "must not leak nom ErrorKind: {}",
            e.msg
        );
    }

    #[test]
    fn err_in_on_process_field_rejected() {
        // `in` 仅支持 network 字段，process 字段报错。
        assert!(parse(r#"name in ("chrome")"#).is_err());
    }

    #[test]
    fn err_in_unclosed_paren() {
        assert!(parse(r#"sni in ("a.com", "b.com""#).is_err());
    }
}
