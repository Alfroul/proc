//! v0.24 stage 2（ADR-0034 D3）：RAG 语料层——索引单元 + 双语料源解析。
//!
//! - **session JSONL（主语料）**：`SessionLogEntry` 事件流按「成功段」状态机
//!   提取——`QueryStarted` 开段 → 段内 `ToolStart` ≥ 1 → `SessionFinished`
//!   {stop: "end_turn"} 收尾且段内无 `Error`；其余段（error / 零 tool /
//!   非 end_turn 收尾 / 未收尾）一律不产出（附录 B：96% 文件是零 query
//!   空会话，筛选是常态路径非防御性设计）。
//! - **eval run JSON（bootstrap）**：`EvalRunFile` 反序列化直读 → passed
//!   trace → 条目 `source = Eval` 标记（启用即受 D4 污染防护约束）。
//! - 聚合去重：归一化 query 全局去重保首见（先 session 后 eval——真实
//!   语料优先；附录 B「3 run 40 独立 query」口径）。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::agent::eval::runner::EvalRunFile;
use crate::agent::session_log::{LogEvent, SessionLogEntry};

/// 语料源标记（D3：bootstrap 条目 source = "eval" 明示口径）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntrySource {
    Session,
    Eval,
}

impl EntrySource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Eval => "eval",
        }
    }
}

/// RAG 索引单元：一条可检索的历史成功经验（D1/D3）。
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub query: String,
    /// 保序 tool 名链（成功段的 tool_start 序列 / eval 的 actual_tools）。
    pub tools: Vec<String>,
    /// 结论摘要（session final_head / eval final_text_head，源侧已 200 chars 截断）。
    pub conclusion_head: String,
    pub source: EntrySource,
}

/// 解析单个 session JSONL 内容为成功段条目（纯函数；坏行跳过）。
pub fn entries_from_session_jsonl(content: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    // 段状态：(query text, tools, had_error)——未收尾 / 非 end_turn / 有
    // error / 零 tool 的段在收尾判定时丢弃。
    let mut cur: Option<(String, Vec<String>, bool)> = None;
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(log) = serde_json::from_str::<SessionLogEntry>(line) else {
            continue;
        };
        match log.event {
            LogEvent::QueryStarted { text } => {
                // 前段未收尾（无 session_finished）→ 丢弃，不开裂段
                cur = Some((text, Vec::new(), false));
            }
            LogEvent::ToolStart { name } => {
                if let Some((_, tools, _)) = &mut cur {
                    tools.push(name);
                }
            }
            LogEvent::Error { .. } => {
                if let Some((_, _, had_error)) = &mut cur {
                    *had_error = true;
                }
            }
            LogEvent::SessionFinished {
                stop, final_head, ..
            } => {
                if stop != "end_turn" {
                    cur = None;
                    continue;
                }
                if let Some((text, tools, had_error)) = cur.take() {
                    if !had_error && !tools.is_empty() {
                        entries.push(Entry {
                            query: text,
                            tools,
                            conclusion_head: final_head,
                            source: EntrySource::Session,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    entries
}

/// 装载 session 语料目录（`*.jsonl` 全量聚合）。
///
/// 返回 (条目, 警告)——目录缺失 / 零 jsonl 文件 / 单文件读取失败均聚合为
/// 一次警告文案（调用方打 stderr 后静默降级，`SessionRecorder::disabled()`
/// 同款契约——绝不 panic）。
pub fn load_session_corpus(dir: &Path) -> (Vec<Entry>, Option<String>) {
    let read_dir = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => {
            return (
                Vec::new(),
                Some(format!("session 语料目录不可读: {}", dir.display())),
            );
        }
    };
    let mut files: Vec<PathBuf> = read_dir
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "jsonl"))
        .collect();
    files.sort();
    if files.is_empty() {
        return (
            Vec::new(),
            Some(format!("session 语料目录无 .jsonl 文件: {}", dir.display())),
        );
    }
    let mut entries = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    for path in &files {
        match fs::read_to_string(path) {
            Ok(content) => entries.extend(entries_from_session_jsonl(&content)),
            Err(e) => failures.push(format!("{}: {e}", path.display())),
        }
    }
    let warning = (!failures.is_empty()).then(|| {
        format!(
            "session 语料 {} 个文件读取失败: {}",
            failures.len(),
            failures.join("; ")
        )
    });
    (entries, warning)
}

/// 解析单个 eval run JSON 的 passed trace 为 bootstrap 条目。
///
/// `EvalRunFile` 反序列化直读（schema 与 harness 同源，零新解析代码）；
/// 解析失败返 Err（调用方警告跳过该源）。
pub fn entries_from_eval_json(content: &str) -> Result<Vec<Entry>, String> {
    let file: EvalRunFile =
        serde_json::from_str(content).map_err(|e| format!("eval JSON 解析失败: {e}"))?;
    Ok(file
        .results
        .into_iter()
        .filter(|r| r.passed)
        .map(|r| Entry {
            query: r.query,
            tools: r.actual_tools,
            conclusion_head: r.final_text_head,
            source: EntrySource::Eval,
        })
        .collect())
}

/// 归一化 query 全局去重，保首见（调用方先 session 后 eval 装载——真实语料优先）。
pub fn dedup_entries(entries: Vec<Entry>) -> Vec<Entry> {
    let mut seen: HashSet<String> = HashSet::new();
    entries
        .into_iter()
        .filter(|e| seen.insert(super::retrieve::normalize_query(&e.query)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log_line(seq: u64, event: LogEvent) -> String {
        serde_json::to_string(&SessionLogEntry {
            seq,
            ts_rel_ms: seq * 10,
            event,
        })
        .unwrap()
    }

    fn jsonl(lines: &[String]) -> String {
        lines.join("\n")
    }

    fn session_start() -> String {
        log_line(
            0,
            LogEvent::SessionStart {
                provider: "llama-cpp".into(),
                wall_start: "2026-08-25T00:00:00Z".into(),
            },
        )
    }

    fn query(text: &str) -> String {
        log_line(1, LogEvent::QueryStarted { text: text.into() })
    }

    fn tool(name: &str) -> String {
        log_line(2, LogEvent::ToolStart { name: name.into() })
    }

    fn error(message: &str) -> String {
        log_line(
            3,
            LogEvent::Error {
                message: message.into(),
            },
        )
    }

    fn finished(stop: &str, final_head: &str) -> String {
        log_line(
            4,
            LogEvent::SessionFinished {
                stop: stop.into(),
                final_chars: final_head.chars().count(),
                final_head: final_head.into(),
            },
        )
    }

    #[test]
    fn session_state_machine_happy_path() {
        let content = jsonl(&[
            session_start(),
            query("列出 CPU 占用最高的进程"),
            tool("proc_ls"),
            finished("end_turn", "java.exe PID 13828 占用 38%"),
        ]);
        let entries = entries_from_session_jsonl(&content);
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0],
            Entry {
                query: "列出 CPU 占用最高的进程".into(),
                tools: vec!["proc_ls".into()],
                conclusion_head: "java.exe PID 13828 占用 38%".into(),
                source: EntrySource::Session,
            }
        );
    }

    #[test]
    fn session_state_machine_rejects_bad_segments() {
        // error 段 / 零 tool 段 / 非 end_turn 收尾——三种坏段全拒
        let error_seg = jsonl(&[
            query("查错误"),
            tool("proc_ls"),
            error("boom"),
            finished("end_turn", "ok"),
        ]);
        assert!(entries_from_session_jsonl(&error_seg).is_empty());

        let no_tool_seg = jsonl(&[query("零 tool"), finished("end_turn", "ok")]);
        assert!(entries_from_session_jsonl(&no_tool_seg).is_empty());

        let max_steps_seg = jsonl(&[
            query("超步数"),
            tool("proc_ls"),
            finished("max_steps", "ok"),
        ]);
        assert!(entries_from_session_jsonl(&max_steps_seg).is_empty());

        let interrupted_seg = jsonl(&[
            query("被中断"),
            tool("proc_ls"),
            finished("interrupted", "ok"),
        ]);
        assert!(entries_from_session_jsonl(&interrupted_seg).is_empty());

        // 未收尾段（query 后无 session_finished）丢弃
        let unfinished = jsonl(&[query("未收尾"), tool("proc_ls")]);
        assert!(entries_from_session_jsonl(&unfinished).is_empty());
    }

    #[test]
    fn session_state_machine_multi_segment_file() {
        let content = jsonl(&[
            session_start(),
            query("第一段"),
            tool("proc_ls"),
            tool("proc_kill"),
            finished("end_turn", "结论一"),
            query("第二段坏段"),
            finished("end_turn", "无 tool"),
            query("第三段"),
            tool("proc_dns_log"),
            finished("end_turn", "结论三"),
        ]);
        let entries = entries_from_session_jsonl(&content);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].query, "第一段");
        assert_eq!(entries[0].tools, vec!["proc_ls", "proc_kill"]);
        assert_eq!(entries[1].query, "第三段");
    }

    #[test]
    fn session_jsonl_skips_bad_lines() {
        let content = "not a json line\n".to_string()
            + &jsonl(&[query("好段"), tool("proc_ls"), finished("end_turn", "ok")]);
        let entries = entries_from_session_jsonl(&content);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].query, "好段");
    }

    #[test]
    fn eval_json_takes_passed_traces_only() {
        let json = r#"{
            "meta": {"timestamp":"t","provider":"p","provider_detail":"d","attempts":2,
                     "max_steps":10,"git_describe":"g","quick":false,"query_count":3},
            "results": [
                {"scenario":"usb","level":0,"query":"列出 USB 盘","expected_tools":["proc_usb_list"],
                 "passed":true,"failure_mode":"pass","chain_steps_hit":1,
                 "actual_tools":["proc_usb_list"],"stop_cause":"end_turn",
                 "final_text_head":"共 2 个设备","duration_ms":100,"attempts_used":1},
                {"scenario":"usb","level":0,"query":"弹盘失败","expected_tools":["proc_usb_eject"],
                 "passed":false,"failure_mode":"wrong_tool","chain_steps_hit":0,
                 "actual_tools":["proc_ls"],"stop_cause":"end_turn",
                 "final_text_head":"","duration_ms":100,"attempts_used":2}
            ],
            "report": {"per_level":[],"failure_histogram":[],"total_duration_ms":200}
        }"#;
        let entries = entries_from_eval_json(json).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].query, "列出 USB 盘");
        assert_eq!(entries[0].tools, vec!["proc_usb_list"]);
        assert_eq!(entries[0].source, EntrySource::Eval);

        assert!(entries_from_eval_json("{ bad json").is_err());
    }

    #[test]
    fn dedup_keeps_first_by_normalized_query() {
        let entries = vec![
            Entry {
                query: "查 CPU".into(),
                tools: vec!["a".into()],
                conclusion_head: String::new(),
                source: EntrySource::Eval,
            },
            Entry {
                query: "查cpu ".into(),
                tools: vec!["b".into()],
                conclusion_head: String::new(),
                source: EntrySource::Eval,
            },
            Entry {
                query: "查 内存".into(),
                tools: vec!["c".into()],
                conclusion_head: String::new(),
                source: EntrySource::Eval,
            },
        ];
        let deduped = dedup_entries(entries);
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].tools, vec!["a".to_string()]);
        assert_eq!(deduped[1].query, "查 内存");
    }
}
