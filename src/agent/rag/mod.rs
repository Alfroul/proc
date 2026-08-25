//! v0.24 stage 2（ADR-0034 D1~D4）：RAG 经验召回——索引组装 + 注入模板。
//!
//! 库形态自包含：语料路径参数由调用方传，config / builder / runner 接线
//! 是 stage 3（本阶段零接线——off 态行为零变更）。索引构建时机为 session
//! / eval 启动时全量重建（D3：百级文件毫秒级，不做增量——简单优先）；
//! 构建失败一次 stderr 警告 + 静默降级（`SessionRecorder::disabled()` 同款
//! 契约——绝不 panic）。

pub mod corpus;
pub mod retrieve;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use corpus::Entry;
use retrieve::{RetrievalOutcome, is_polluted, normalize_query, score_entry, token_set, tokenize};

/// 检索 + 注入参数包（定值来源：D1 top_k/min_score + D2 budget + D4
/// threshold；stage 3 `RagConfig` 映射到此包）。
#[derive(Debug, Clone, Copy)]
pub struct RagParams {
    pub top_k: usize,
    pub min_score: f64,
    pub exclude_threshold: f64,
    /// 注入段 chars 硬上限（D2：≈800 token，中文 ~1.5 chars/token 粗估）。
    pub budget_chars: usize,
}

impl Default for RagParams {
    fn default() -> Self {
        Self {
            top_k: 3,
            min_score: 1.0,
            exclude_threshold: 0.6,
            budget_chars: 1200,
        }
    }
}

/// 注入产物（D2/D4 可观测字段——stage 3 stderr 结构化输出与命中次数
/// 报告的数据源）。
#[derive(Debug, Clone, PartialEq)]
pub struct InjectedQuery {
    /// 注入后的完整 query 文本（无命中 = 原文透传，零注入痕迹）。
    pub text: String,
    pub injected: bool,
    pub injected_entries: usize,
    pub excluded_entries: usize,
    /// 注入段估算 token（round(chars / 1.5)——D2 粗估口径）。
    pub est_tokens: usize,
}

/// 经验索引（构建时预分词 + df，检索零重复计算）。
#[derive(Debug)]
pub struct RagIndex {
    entries: Vec<Entry>,
    entry_tokens: Vec<Vec<String>>,
    df: HashMap<String, usize>,
}

impl RagIndex {
    /// 全量重建：session 主语料先装载、eval bootstrap 后装载 → 归一化
    /// 去重保首见（真实语料优先）。每个失败源一次 stderr 警告后继续。
    pub fn build(session_dir: &Path, eval_json_paths: &[PathBuf]) -> Self {
        let mut entries = Vec::new();
        let (session_entries, warning) = corpus::load_session_corpus(session_dir);
        if let Some(w) = warning {
            eprintln!("[rag] {w}");
        }
        entries.extend(session_entries);
        for path in eval_json_paths {
            match std::fs::read_to_string(path) {
                Ok(content) => match corpus::entries_from_eval_json(&content) {
                    Ok(es) => entries.extend(es),
                    Err(e) => eprintln!("[rag] eval 语料 {} 跳过: {e}", path.display()),
                },
                Err(e) => eprintln!("[rag] eval 语料 {} 跳过: {e}", path.display()),
            }
        }
        Self::from_entries(corpus::dedup_entries(entries))
    }

    /// 从就绪条目建索引（预分词 + df）——测试与调用方直接注入语料用。
    pub fn from_entries(entries: Vec<Entry>) -> Self {
        let entry_tokens: Vec<Vec<String>> = entries.iter().map(|e| tokenize(&e.query)).collect();
        let mut df: HashMap<String, usize> = HashMap::new();
        for tokens in &entry_tokens {
            for t in token_set(tokens) {
                *df.entry(t).or_default() += 1;
            }
        }
        Self {
            entries,
            entry_tokens,
            df,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// 检索（D1 + D4）：污染排除 → 评分 → min_score 门槛 → 排序
    /// （score 降序，并列按 tool 链长度降序，再并列稳定保序）→ top_k 截断。
    pub fn retrieve<'a>(&'a self, query: &str, params: &RagParams) -> RetrievalOutcome<'a> {
        let query_tokens = tokenize(query);
        let query_norm = normalize_query(query);
        let query_set = token_set(&query_tokens);
        let mut excluded = 0usize;
        let mut scored: Vec<(&Entry, f64)> = Vec::new();
        for (idx, entry) in self.entries.iter().enumerate() {
            let entry_set = token_set(&self.entry_tokens[idx]);
            if is_polluted(
                &query_norm,
                &query_set,
                &normalize_query(&entry.query),
                &entry_set,
                params.exclude_threshold,
            ) {
                excluded += 1;
                continue;
            }
            let score = score_entry(
                &query_tokens,
                &self.entry_tokens[idx],
                &self.df,
                self.entries.len(),
            );
            if score >= params.min_score {
                scored.push((entry, score));
            }
        }
        scored.sort_by(|a, b| {
            b.1.total_cmp(&a.1)
                .then_with(|| b.0.tools.len().cmp(&a.0.tools.len()))
        });
        scored.truncate(params.top_k);
        RetrievalOutcome {
            hits: scored,
            excluded,
        }
    }
}

/// D2 注入：检索 → 模板渲染。条目按相关性降序逐条收录，预算（chars 硬
/// 上限）内整条放下才收，超限整条丢弃（不截半条）；无命中或预算内零条
/// 可容 → 原文透传零注入痕迹（off 态与 on-无命中态行为一致）。
pub fn inject_experience(query: &str, index: &RagIndex, params: &RagParams) -> InjectedQuery {
    let transparent = |excluded: usize| InjectedQuery {
        text: query.to_string(),
        injected: false,
        injected_entries: 0,
        excluded_entries: excluded,
        est_tokens: 0,
    };
    let outcome = index.retrieve(query, params);
    if outcome.hits.is_empty() {
        return transparent(outcome.excluded);
    }
    let header = "[历史经验参考] 以下是与当前问题相似的历史成功解法，仅供参考，不要照抄与当前问题无关的步骤：";
    let marker = "\n[当前问题] ";
    let mut used = header.chars().count() + marker.chars().count();
    let mut lines: Vec<String> = Vec::new();
    for (entry, _) in &outcome.hits {
        let line = format_entry_line(entry);
        let cost = 1 + line.chars().count(); // 行前 '\n' + 行文本
        if used + cost > params.budget_chars {
            break;
        }
        used += cost;
        lines.push(line);
    }
    if lines.is_empty() {
        return transparent(outcome.excluded);
    }
    let prefix = format!("{header}\n{}\n[当前问题] ", lines.join("\n"));
    let est_tokens = (prefix.chars().count() as f64 / 1.5).round() as usize;
    InjectedQuery {
        text: format!("{prefix}{query}"),
        injected: true,
        injected_entries: lines.len(),
        excluded_entries: outcome.excluded,
        est_tokens,
    }
}

/// D2 条目行：`- "<query>" → tool1 → tool2（结论：<head 截断 80 chars>）`。
fn format_entry_line(entry: &Entry) -> String {
    let tools = entry.tools.join(" → ");
    let head = crate::agent::eval::runner::truncate_chars(&entry.conclusion_head, 80);
    format!("- \"{}\" → {}（结论：{}）", entry.query, tools, head)
}
