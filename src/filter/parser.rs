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

use super::{CmpOp, Field, FilterExpr, Value};

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

/// 入口：把整个 input 解析为单个 [`FilterExpr`]，trailing 非空白字符算错误。
pub fn parse(input: &str) -> Result<FilterExpr, ParseError> {
    match parse_expr(input) {
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

fn parse_expr(i: &str) -> NomRes<'_, FilterExpr> {
    parse_or(i)
}

fn parse_or(i: &str) -> NomRes<'_, FilterExpr> {
    let (i, first) = parse_and(i)?;
    // AND/OR 关键字两侧必须吃空白 —— `cpu > 5 AND mem > 100` 中 AND 前后都有空格，
    // 否则 tag("AND") 会因 leading space 失败让 many0 提前结束。
    let (i, rest) = many0(tuple((multispace0, keyword("OR"), multispace0, parse_and)))(i)?;
    let result = rest.into_iter().fold(first, |acc, (_, _, _, next)| {
        FilterExpr::Or(Box::new(acc), Box::new(next))
    });
    Ok((i, result))
}

fn parse_and(i: &str) -> NomRes<'_, FilterExpr> {
    let (i, first) = parse_not(i)?;
    let (i, rest) = many0(tuple((multispace0, keyword("AND"), multispace0, parse_not)))(i)?;
    let result = rest.into_iter().fold(first, |acc, (_, _, _, next)| {
        FilterExpr::And(Box::new(acc), Box::new(next))
    });
    Ok((i, result))
}

fn parse_not(i: &str) -> NomRes<'_, FilterExpr> {
    let (i, not_kw) = opt(tuple((keyword("NOT"), multispace0)))(i)?;
    let (i, primary) = parse_primary(i)?;
    Ok((
        i,
        if not_kw.is_some() {
            FilterExpr::Not(Box::new(primary))
        } else {
            primary
        },
    ))
}

fn parse_primary(i: &str) -> NomRes<'_, FilterExpr> {
    let (i, _) = multispace0(i)?;
    // 先尝试括号子表达式，再退到 leaf。
    alt((paren_expr, leaf))(i)
}

fn paren_expr(i: &str) -> NomRes<'_, FilterExpr> {
    let (i, _) = char('(')(i)?;
    let (i, _) = multispace0(i)?;
    let (i, expr) = parse_expr(i)?;
    let (i, _) = multispace0(i)?;
    // TD-16：括号闭合失败用 cut 转 Failure，让 alt 不回退到 leaf。
    // 否则 `(cpu > 5` 缺 `)` 时 alt 会 fallback 到 leaf 解析整个 `(cpu > 5`，
    // 最内层错误变成 leaf 的 TakeWhile1（「缺少字段名/值」），丢失「括号没闭合」
    // 这条真正有用的信息。
    let (i, _) = cut(char(')'))(i)?;
    Ok((i, expr))
}

fn leaf(i: &str) -> NomRes<'_, FilterExpr> {
    let (i, field) = parse_field(i)?;
    let (i, _) = multispace0(i)?;
    // 先尝试 regex 路径（=~），不命中再走 cmp。
    let (i, regex_path) = opt(tuple((tag("=~"), multispace0, parse_regex_lit)))(i)?;
    if let Some((_, _, (pattern, ci))) = regex_path {
        let re_str = if ci {
            format!("(?i){pattern}")
        } else {
            pattern.clone()
        };
        let re = regex::Regex::new(&re_str)
            .map_err(|_| NomErr::Failure(VerboseError::from_error_kind(i, ErrorKind::Verify)))?;
        return Ok((
            i,
            FilterExpr::Regex {
                field,
                re,
                case_insensitive: ci,
            },
        ));
    }
    let (i, op) = parse_cmp_op(i)?;
    let (i, _) = multispace0(i)?;
    let (i, value) = parse_value(i)?;
    Ok((i, FilterExpr::FieldCmp { field, op, value }))
}

fn parse_field(i: &str) -> NomRes<'_, Field> {
    let (after_ident, ident) = take_while1(|c: char| c.is_ascii_alphanumeric() || c == '_')(i)?;
    let field = match ident {
        "cpu" => Field::Cpu,
        "mem" | "memory" => Field::Mem,
        "pid" => Field::Pid,
        "name" => Field::Name,
        "user" => Field::User,
        "cmd" => Field::Cmd,
        "disk_read" | "diskread" => Field::DiskRead,
        "disk_write" | "diskwrite" => Field::DiskWrite,
        "net_sent" | "netsent" => Field::NetSent,
        "net_recv" | "netrecv" => Field::NetRecv,
        "security_score" | "security" => Field::SecurityScore,
        _ => {
            return Err(NomErr::Error(VerboseError::from_error_kind(
                after_ident,
                ErrorKind::AlphaNumeric,
            )));
        }
    };
    Ok((after_ident, field))
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
    let (i, pattern) = take_till1(|c| c == '/')(i)?;
    let (i, _) = char('/')(i)?;
    let (i, ci) = opt(char('i'))(i)?;
    Ok((i, (pattern.to_string(), ci.is_some())))
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
}
