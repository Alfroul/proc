//! v0.24 stage 2（ADR-0034 D1/D4）：RAG 检索层——分词 / 评分 / 污染排除。
//!
//! 全部纯函数（零 IO），D1 评分规格：`score(entry) = Σ_{t ∈ set(tokens(query))}
//! idf(t) × min(tf(t, entry.query), 3)`，`idf = ln(1 + N / df)`，不做长度
//! 归一化（条目 query 均短）；D4 污染判定：exact match 或词元覆盖率
//! ≥ 0.6（去重集合口径，双向 min 分母——任一方向高度覆盖即排除）。

use std::collections::HashSet;

use super::corpus::Entry;

/// 归一化 query（exact 判定 / 去重同底）：lowercase + 去全部空白。
pub fn normalize_query(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// D1 分词：ASCII 字母数字连续段 lowercase 单 token；连续 CJK 段
/// （U+4E00~U+9FFF）长度 1 产单字 token、长度 ≥ 2 滑窗 2-gram；其余
/// 字符（含中文标点 / 空白）一律作分隔。
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut ascii = String::new();
    let mut cjk: Vec<char> = Vec::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            flush_cjk(&mut cjk, &mut tokens);
            ascii.extend(ch.to_lowercase());
        } else if is_cjk(ch) {
            flush_ascii(&mut ascii, &mut tokens);
            cjk.push(ch);
        } else {
            flush_ascii(&mut ascii, &mut tokens);
            flush_cjk(&mut cjk, &mut tokens);
        }
    }
    flush_ascii(&mut ascii, &mut tokens);
    flush_cjk(&mut cjk, &mut tokens);
    tokens
}

fn is_cjk(ch: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&ch)
}

fn flush_ascii(run: &mut String, out: &mut Vec<String>) {
    if !run.is_empty() {
        out.push(std::mem::take(run));
    }
}

fn flush_cjk(run: &mut Vec<char>, out: &mut Vec<String>) {
    match run.len() {
        0 => {}
        1 => out.push(run.drain(..).collect()),
        _ => {
            for pair in run.windows(2) {
                out.push(pair.iter().collect());
            }
            run.clear();
        }
    }
}

/// 去重 token 集合（coverage 分母口径——多重集歧义消解，见 stage doc 风险 2）。
pub fn token_set(tokens: &[String]) -> HashSet<String> {
    tokens.iter().cloned().collect()
}

/// D4 污染判定：exact match（归一化全等）或覆盖率 ≥ threshold。
///
/// `coverage = |query_set ∩ entry_set| / min(两集合大小)`——双向 min 分母，
/// 既防「同款 query 微改写」也防「长 query 包含短历史 query」型泄漏。
pub fn is_polluted(
    query_norm: &str,
    query_set: &HashSet<String>,
    entry_norm: &str,
    entry_set: &HashSet<String>,
    threshold: f64,
) -> bool {
    if query_norm == entry_norm {
        return true;
    }
    let denom = query_set.len().min(entry_set.len());
    if denom == 0 {
        return false;
    }
    let inter = query_set.intersection(entry_set).count();
    inter as f64 / denom as f64 >= threshold
}

/// D1 评分：Σ_{去重 query token} idf(t) × min(tf(t, entry_tokens), 3)。
///
/// query 侧按去重集合迭代（同 token 重复出现不重复计分）；df 缺失（语料
/// 中无条目含 t）的 token tf 恒 0，直接跳过。
pub fn score_entry(
    query_tokens: &[String],
    entry_tokens: &[String],
    df: &std::collections::HashMap<String, usize>,
    total: usize,
) -> f64 {
    let mut score = 0.0;
    for t in token_set(query_tokens) {
        let Some(&df_t) = df.get(&t) else { continue };
        let tf = entry_tokens.iter().filter(|e| **e == t).count().min(3) as f64;
        if tf > 0.0 {
            let idf = (1.0 + total as f64 / df_t as f64).ln();
            score += idf * tf;
        }
    }
    score
}

/// 检索结果：hits 按 score 降序（并列按 tool 链长度降序，再并列稳定保序）
/// 截 top_k、逐条过 min_score 门槛；excluded = 污染排除条数（D4 命中
/// 次数报告的数据源——stage 3 stderr 结构化输出）。
#[derive(Debug)]
pub struct RetrievalOutcome<'a> {
    pub hits: Vec<(&'a Entry, f64)>,
    pub excluded: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_ascii_runs_lowercase() {
        assert_eq!(tokenize("kill 13828"), vec!["kill", "13828"]);
        assert_eq!(tokenize("Run Gamma SCAN"), vec!["run", "gamma", "scan"]);
    }

    #[test]
    fn tokenize_cjk_bigrams_and_single_char_run() {
        // 连续 CJK ≥2 → 滑窗 2-gram；单字 CJK 段 → 单字 token
        assert_eq!(tokenize("弹出"), vec!["弹出"]);
        assert_eq!(tokenize("查 CPU"), vec!["查", "cpu"]);
        // 「列出 CPU 占用最高的 3 个进程」全切分
        assert_eq!(
            tokenize("列出 CPU 占用最高的 3 个进程"),
            vec![
                "列出", "cpu", "占用", "用最", "最高", "高的", "3", "个进", "进程"
            ]
        );
    }

    #[test]
    fn tokenize_mixed_with_cjk_punctuation_as_separator() {
        // 中文标点（，）不在 U+4E00~9FFF → 作分隔
        assert_eq!(
            tokenize("列出USB盘，弹出E盘"),
            vec!["列出", "usb", "盘", "弹出", "e", "盘"]
        );
    }

    #[test]
    fn pollution_exact_match_normalized() {
        let qs = token_set(&tokenize("列出 USB 盘"));
        let es = token_set(&tokenize("列出usb  盘"));
        assert!(is_polluted("列出usb盘", &qs, "列出usb盘", &es, 0.6));
    }

    #[test]
    fn pollution_high_coverage_rewrite_excluded() {
        // 高覆盖改写（增删一两个词）典型 > 0.7 → 排除
        let q = token_set(&tokenize("列出所有 USB 设备")); // 4 tokens
        let e = token_set(&tokenize("列出 USB 设备")); // 3 tokens
        assert_eq!(q.intersection(&e).count(), 3);
        assert!(is_polluted("列出所有usb设备", &q, "列出usb设备", &e, 0.6));
    }

    #[test]
    fn pollution_same_scene_different_intent_not_excluded() {
        // 同场景异意图（ADR D4 例）：「列出 USB 盘」vs「弹出 E 盘」覆盖 < 0.4
        let q = token_set(&tokenize("弹出 E 盘"));
        let e = token_set(&tokenize("列出 USB 盘"));
        assert_eq!(q.intersection(&e).count(), 1); // 仅共享「盘」
        assert!(!is_polluted("弹出e盘", &q, "列出usb盘", &e, 0.6));
    }

    #[test]
    fn pollution_coverage_boundary_and_bidirectional_min() {
        let ts = |items: &[&str]| -> HashSet<String> {
            items.iter().map(|s| s.to_string()).collect()
        };
        // 恰 0.6 → 排除（>= 语义）；norm 占位互异以绕开 exact match 分支
        let q = ts(&["a", "b", "c", "d", "e"]);
        let e = ts(&["a", "b", "c", "x", "y"]);
        assert!(is_polluted("q", &q, "e", &e, 0.6));

        // 0.4 → 不排除
        let e2 = ts(&["a", "b", "x", "y", "z"]);
        assert!(!is_polluted("q", &q, "e2", &e2, 0.6));

        // 双向 min 分母：长 query 完全包含短 entry 的全部词元 → 覆盖 1.0 排除
        let long = ts(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]);
        let short = ts(&["a", "b", "c", "d"]);
        assert!(is_polluted("long", &long, "short", &short, 0.6));
    }

    #[test]
    fn score_rare_token_weighs_more_and_tf_caps_at_3() {
        let mut df = std::collections::HashMap::new();
        df.insert("rare".to_string(), 1);
        df.insert("common".to_string(), 10);
        let query = ["rare", "common"].map(String::from).to_vec();

        // rare（df=1/10 条）idf=ln11≈2.4 > common（df=10/10）idf=ln2≈0.69
        let entry_rare = ["rare"].map(String::from).to_vec();
        let entry_common = ["common"].map(String::from).to_vec();
        let s_rare = score_entry(&query, &entry_rare, &df, 10);
        let s_common = score_entry(&query, &entry_common, &df, 10);
        assert!(s_rare > s_common);
        assert!((s_rare - 11.0f64.ln()).abs() < 1e-9);
        assert!((s_common - 2.0f64.ln()).abs() < 1e-9);

        // tf 上限 3：token 出现 5 次按 3 计
        let entry_repeat = ["common"; 5].map(String::from).to_vec();
        let s_repeat = score_entry(&query, &entry_repeat, &df, 10);
        assert!((s_repeat - 3.0 * 2.0f64.ln()).abs() < 1e-9);
    }
}
