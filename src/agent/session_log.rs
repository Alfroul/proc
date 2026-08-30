//! v0.22 stage 3：session observability——JSONL 留档 + 指标提取（ADR-0032 D5）。
//!
//! session 层旁路记录：`session_loop` 在事件 send 前包装成 [`SessionLogEntry`]
//! （seq + Instant 起点相对毫秒 + 事件摘要），[`SessionRecorder`] 落盘
//! `~/.config/proc/sessions/<yyyyMMdd-HHmmss>-<provider>.jsonl`。观测是离线
//! 诉求——运行时 UI 零改动，[`super::session::SessionEvent`] 8 变体形状零改动。
//!
//! confirm 决策不在 SessionEvent 事件流里（oneshot 回传）：session 层换出
//! `req.reply` 包一层转发线程记录 [`LogEvent::ConfirmDecision`] 再转发原通道
//! （先记录后转发，保证日志序）；通道断开（面板 drop 未决策）线程干净退出。

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use super::session::SessionEvent;

/// TextDelta 聚合阈值：累计 ≥ 该 chars 落一条（brainstorm 风险 2——不逐
/// delta 落盘；聚合条目 ts 取首 delta 时间，TTFT 精度不受损）。
pub const DELTA_MERGE_CHARS: usize = 64;

/// 单条 JSONL 记录：seq（0 起，头条目是 SessionStart）+ session 起点相对
/// 毫秒（Instant 单调）+ 事件摘要。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionLogEntry {
    pub seq: u64,
    pub ts_rel_ms: u64,
    #[serde(flatten)]
    pub event: LogEvent,
}

/// 事件摘要（serde tag = "kind"，snake_case）——只记指标所需字段，不落全文。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LogEvent {
    /// seq 0 头：provider + 墙钟起点（Instant 起点 + 墙钟双时间）。
    SessionStart {
        provider: String,
        wall_start: String,
    },
    QueryStarted {
        text: String,
    },
    /// 聚合后的文本增量段（ts = 段首 delta 时间）。
    TextDelta {
        chars: usize,
    },
    ToolStart {
        name: String,
    },
    ToolFinished {
        name: String,
        is_error: bool,
        result_chars: usize,
    },
    ConfirmRequested {
        tool_name: String,
    },
    /// 决策（面板 y/n）——session 层旁路记录，不在 SessionEvent 流里。
    ConfirmDecision {
        approved: bool,
    },
    TurnFinished,
    SessionFinished {
        stop: String,
        final_chars: usize,
        final_head: String,
    },
    Error {
        message: String,
    },
}

/// session JSONL 留档器（`[session].log` 默认 true；写失败静默降级）。
///
/// v0.25 stage 2（ADR-0035 D1）延迟创建：首个非 session_start 事件到达才
/// 落盘——进面板不发问的会话不产生空文件（空会话治理主项）。
///
/// `Clone` 是廉价 Arc 共享——session_loop 主体与 sink / confirm 闭包各持一份。
#[derive(Clone)]
pub struct SessionRecorder {
    inner: Option<Arc<Mutex<RecorderInner>>>,
}

struct RecorderInner {
    /// 延迟创建目标：文件名时间戳 = 构造时刻（与 wall_start 的轻微偏差
    /// 见 ADR-0035 Consequences，无实际影响）。
    path: PathBuf,
    /// None = 文件未创建（pending_start 暂存中）或落盘失败已放弃（静默降级）。
    writer: Option<BufWriter<File>>,
    /// SessionStart 暂存（provider, wall_start）：物化时补写首行（ts 0 / seq 0）。
    pending_start: Option<(String, String)>,
    seq: u64,
    start: Instant,
    /// 聚合中的 TextDelta（首 delta ts，累计 chars）。
    pending_delta: Option<(u64, usize)>,
}

impl SessionRecorder {
    /// 按产品路径建 recorder：`dirs_config_dir()/sessions/`。
    pub fn start(provider: &str) -> Self {
        Self::start_in_dir(&crate::dirs_config_dir().join("sessions"), provider)
    }

    /// 测试注入用：指定目录。目录创建失败 → 构造即降级 disabled（`is_enabled`
    /// 语义：构造成功即 enabled，目录可写性检查保留在构造时）；文件创建延迟
    /// 到首个非 session_start 事件（ADR-0035 D1），届时失败 → 静默放弃。
    pub fn start_in_dir(dir: &Path, provider: &str) -> Self {
        if fs::create_dir_all(dir).is_err() {
            return Self::disabled();
        }
        let path = dir.join(format!(
            "{}-{provider}.jsonl",
            crate::agent::eval::runner::utc_timestamp_compact()
        ));
        Self {
            inner: Some(Arc::new(Mutex::new(RecorderInner {
                path,
                writer: None,
                pending_start: Some((
                    provider.to_string(),
                    crate::agent::eval::runner::utc_timestamp_iso(),
                )),
                seq: 0,
                start: Instant::now(),
                pending_delta: None,
            }))),
        }
    }

    /// 关闭态（`[session].log = false` 或落盘失败降级）——所有 log 调用 no-op。
    pub fn disabled() -> Self {
        Self { inner: None }
    }

    pub fn is_enabled(&self) -> bool {
        self.inner.is_some()
    }

    /// 唯一事件记录入口（session_loop 在 `events_tx.send` 前调用）。
    pub fn log(&self, ev: &SessionEvent) {
        let Some(inner) = &self.inner else { return };
        let mut g = inner.lock().unwrap();
        match ev {
            SessionEvent::TextDelta(t) => {
                let chars = t.chars().count();
                let ts = elapsed_ms(&g.start);
                match &mut g.pending_delta {
                    Some((_, acc)) => {
                        *acc += chars;
                        if *acc >= DELTA_MERGE_CHARS {
                            let (ts0, total) = g.pending_delta.take().expect("pending checked");
                            g.write_entry(LogEvent::TextDelta { chars: total }, ts0);
                        }
                    }
                    None if chars >= DELTA_MERGE_CHARS => {
                        g.write_entry(LogEvent::TextDelta { chars }, ts);
                    }
                    None => g.pending_delta = Some((ts, chars)),
                }
            }
            _ => {
                g.flush_pending();
                let ts = elapsed_ms(&g.start);
                g.write_entry(summarize(ev), ts);
            }
        }
    }

    /// confirm 转发线程专用（决策旁路，不在 SessionEvent 流里）。
    pub fn log_confirm_decision(&self, approved: bool) {
        let Some(inner) = &self.inner else { return };
        let mut g = inner.lock().unwrap();
        g.flush_pending();
        let ts = elapsed_ms(&g.start);
        g.write_entry(LogEvent::ConfirmDecision { approved }, ts);
    }
}

impl RecorderInner {
    fn write_entry(&mut self, event: LogEvent, ts_rel_ms: u64) {
        // D1 延迟创建：首个非 session_start 事件到达才建文件，SessionStart
        // 届时补写首行；File::create 失败清暂存静默放弃（后续调用 no-op）。
        if self.writer.is_none() {
            let Some((provider, wall_start)) = self.pending_start.take() else {
                return;
            };
            match File::create(&self.path) {
                Ok(file) => {
                    self.writer = Some(BufWriter::new(file));
                    self.emit(
                        LogEvent::SessionStart {
                            provider,
                            wall_start,
                        },
                        0,
                    );
                }
                Err(_) => return,
            }
        }
        self.emit(event, ts_rel_ms);
    }

    fn emit(&mut self, event: LogEvent, ts_rel_ms: u64) {
        let entry = SessionLogEntry {
            seq: self.seq,
            ts_rel_ms,
            event,
        };
        self.seq += 1;
        if let (Ok(json), Some(w)) = (serde_json::to_string(&entry), self.writer.as_mut()) {
            // per-line flush：会话崩溃时已写行完整，Drop 无需特殊收尾。
            let _ = writeln!(w, "{json}");
            let _ = w.flush();
        }
    }

    fn flush_pending(&mut self) {
        if let Some((ts0, total)) = self.pending_delta.take() {
            self.write_entry(LogEvent::TextDelta { chars: total }, ts0);
        }
    }
}

fn elapsed_ms(start: &Instant) -> u64 {
    start.elapsed().as_millis() as u64
}

fn summarize(ev: &SessionEvent) -> LogEvent {
    let head = |s: &str| crate::agent::eval::runner::truncate_chars(s, 200);
    match ev {
        SessionEvent::QueryStarted(text) => LogEvent::QueryStarted { text: head(text) },
        SessionEvent::TextDelta(_) => unreachable!("TextDelta 走聚合缓冲分支"),
        SessionEvent::ToolStart { name, .. } => LogEvent::ToolStart { name: name.clone() },
        SessionEvent::ToolFinished {
            name,
            is_error,
            result_chars,
        } => LogEvent::ToolFinished {
            name: name.clone(),
            is_error: *is_error,
            result_chars: *result_chars,
        },
        SessionEvent::ConfirmRequested(req) => LogEvent::ConfirmRequested {
            tool_name: req.tool_name.clone(),
        },
        SessionEvent::TurnFinished => LogEvent::TurnFinished,
        SessionEvent::SessionFinished { final_text, stop } => LogEvent::SessionFinished {
            stop: stop.label().to_string(),
            final_chars: final_text.chars().count(),
            final_head: head(final_text),
        },
        SessionEvent::Error(message) => LogEvent::Error {
            message: head(message),
        },
    }
}

// ---------------------------------------------------------------------------
// 指标提取（JSONL 后处理，零运行时开销）
// ---------------------------------------------------------------------------

/// session 指标聚合（`analyze_session_log` 产物）。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SessionMetrics {
    pub provider: String,
    pub wall_start: String,
    pub total_ms: u64,
    pub queries: Vec<QueryMetrics>,
    pub totals: MetricsTotals,
}

/// 单 query 指标（QueryStarted → SessionFinished / Error 生命周期）。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct QueryMetrics {
    pub index: usize,
    pub query_head: String,
    /// TTFT：QueryStarted → 首个 TextDelta 段（无流式输出则 None）。
    pub ttft_ms: Option<u64>,
    /// 生成时长：QueryStarted → SessionFinished（未完成则 None）。
    pub duration_ms: Option<u64>,
    pub delta_events: usize,
    pub delta_chars: usize,
    pub tool_calls: usize,
    pub tool_errors: usize,
    pub turns: usize,
    pub confirms: usize,
    /// 本 query 内最大决策延迟（ConfirmRequested → ConfirmDecision）。
    pub confirm_decision_max_ms: Option<u64>,
    pub stop: Option<String>,
    pub error: Option<String>,
}

/// 跨 query 总计（含 query 外事件——如空 query 的 Error）。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct MetricsTotals {
    pub queries: usize,
    pub answered: usize,
    pub ttft_avg_ms: Option<u64>,
    pub ttft_max_ms: Option<u64>,
    pub generation_avg_ms: Option<u64>,
    pub delta_events: usize,
    pub delta_chars: usize,
    pub tool_calls: usize,
    pub tool_errors: usize,
    pub turns: usize,
    pub confirms: usize,
    pub confirm_approved: usize,
    pub confirm_denied: usize,
    pub confirm_decision_avg_ms: Option<u64>,
    pub errors: usize,
}

/// 读 JSONL 文件 → [`SessionMetrics`]（空行跳过 / 坏行报行号）。
pub fn analyze_session_log(path: &Path) -> Result<SessionMetrics, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
    let mut entries = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let entry: SessionLogEntry = serde_json::from_str(line)
            .map_err(|e| format!("解析 {} 第 {} 行失败: {e}", path.display(), idx + 1))?;
        entries.push(entry);
    }
    Ok(analyze_entries(&entries))
}

/// 纯函数聚合（测试直接喂构造 entries，不经文件）。
pub fn analyze_entries(entries: &[SessionLogEntry]) -> SessionMetrics {
    let mut m = SessionMetrics::default();
    let mut cur: Option<QueryMetrics> = None;
    let mut cur_start: u64 = 0;
    let mut ttfts: Vec<u64> = Vec::new();
    let mut durations: Vec<u64> = Vec::new();
    let mut decision_latencies: Vec<u64> = Vec::new();
    let mut pending_confirm_ts: Option<u64> = None;

    for e in entries {
        m.total_ms = e.ts_rel_ms;
        match &e.event {
            LogEvent::SessionStart {
                provider,
                wall_start,
            } => {
                m.provider = provider.clone();
                m.wall_start = wall_start.clone();
            }
            LogEvent::QueryStarted { text } => {
                if let Some(q) = cur.take() {
                    m.queries.push(q);
                }
                cur_start = e.ts_rel_ms;
                cur = Some(QueryMetrics {
                    query_head: text.clone(),
                    ..Default::default()
                });
            }
            LogEvent::TextDelta { chars } => {
                m.totals.delta_events += 1;
                m.totals.delta_chars += chars;
                if let Some(q) = &mut cur {
                    if q.ttft_ms.is_none() {
                        q.ttft_ms = Some(e.ts_rel_ms.saturating_sub(cur_start));
                    }
                    q.delta_events += 1;
                    q.delta_chars += chars;
                }
            }
            LogEvent::ToolStart { .. } => {
                m.totals.tool_calls += 1;
                if let Some(q) = &mut cur {
                    q.tool_calls += 1;
                }
            }
            LogEvent::ToolFinished { is_error, .. } => {
                if *is_error {
                    m.totals.tool_errors += 1;
                    if let Some(q) = &mut cur {
                        q.tool_errors += 1;
                    }
                }
            }
            LogEvent::ConfirmRequested { .. } => {
                m.totals.confirms += 1;
                pending_confirm_ts = Some(e.ts_rel_ms);
                if let Some(q) = &mut cur {
                    q.confirms += 1;
                }
            }
            LogEvent::ConfirmDecision { approved } => {
                if *approved {
                    m.totals.confirm_approved += 1;
                } else {
                    m.totals.confirm_denied += 1;
                }
                if let Some(t0) = pending_confirm_ts.take() {
                    let latency = e.ts_rel_ms.saturating_sub(t0);
                    decision_latencies.push(latency);
                    if let Some(q) = &mut cur {
                        q.confirm_decision_max_ms = Some(
                            q.confirm_decision_max_ms
                                .map_or(latency, |prev| prev.max(latency)),
                        );
                    }
                }
            }
            LogEvent::TurnFinished => {
                m.totals.turns += 1;
                if let Some(q) = &mut cur {
                    q.turns += 1;
                }
            }
            LogEvent::SessionFinished { stop, .. } => {
                m.totals.answered += 1;
                if let Some(mut q) = cur.take() {
                    q.duration_ms = Some(e.ts_rel_ms.saturating_sub(cur_start));
                    q.stop = Some(stop.clone());
                    q.index = m.queries.len() + 1;
                    if let Some(t) = q.ttft_ms {
                        ttfts.push(t);
                    }
                    if let Some(d) = q.duration_ms {
                        durations.push(d);
                    }
                    m.queries.push(q);
                }
            }
            LogEvent::Error { message } => {
                m.totals.errors += 1;
                if let Some(q) = &mut cur {
                    q.error = Some(message.clone());
                }
            }
        }
    }
    if let Some(q) = cur.take() {
        m.queries.push(q);
    }
    m.totals.queries = m.queries.len();
    m.totals.ttft_avg_ms = avg(&ttfts);
    m.totals.ttft_max_ms = ttfts.iter().copied().max();
    m.totals.generation_avg_ms = avg(&durations);
    m.totals.confirm_decision_avg_ms = avg(&decision_latencies);
    m
}

fn avg(values: &[u64]) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    Some(values.iter().sum::<u64>() / values.len() as u64)
}

// ---------------------------------------------------------------------------
// session-info 格式化
// ---------------------------------------------------------------------------

/// `proc agent session-info` 的输出（纯函数，测试断言锚）。
pub fn format_session_metrics(m: &SessionMetrics) -> String {
    let mut out = String::new();
    out.push_str(&format!("session 观测: {}\n", m.provider));
    if !m.wall_start.is_empty() {
        out.push_str(&format!(
            "起点: {}（时长 {}）\n",
            m.wall_start,
            fmt_duration(m.total_ms)
        ));
    }
    out.push_str(&format!(
        "query 数: {}（完成 {} / 错误 {}）\n\n",
        m.totals.queries, m.totals.answered, m.totals.errors
    ));

    out.push_str("总指标:\n");
    out.push_str(&format!(
        "  TTFT       {} / max {}\n",
        opt_duration(m.totals.ttft_avg_ms),
        opt_duration(m.totals.ttft_max_ms),
    ));
    out.push_str(&format!(
        "  生成时长   avg {}\n",
        opt_duration(m.totals.generation_avg_ms)
    ));
    out.push_str(&format!(
        "  文本增量   {} 段 / {} chars\n",
        m.totals.delta_events, m.totals.delta_chars
    ));
    out.push_str(&format!(
        "  tool 轮数  {}（error {}）/ turn {}\n",
        m.totals.tool_calls, m.totals.tool_errors, m.totals.turns
    ));
    out.push_str(&format!(
        "  confirm    {}（approved {} / denied {}），决策延迟 avg {}\n",
        m.totals.confirms,
        m.totals.confirm_approved,
        m.totals.confirm_denied,
        opt_duration(m.totals.confirm_decision_avg_ms),
    ));

    if !m.queries.is_empty() {
        out.push_str("\nper-query:\n");
        out.push_str("  #  stop          ttft     生成     tools  delta        confirm  query\n");
        for q in &m.queries {
            out.push_str(&format!(
                "  {:<2} {:<13} {:<8} {:<8} {:<6} {:<12} {:<8} {}\n",
                q.index,
                q.stop
                    .as_deref()
                    .unwrap_or(if q.error.is_some() { "error" } else { "-" }),
                opt_duration(q.ttft_ms),
                opt_duration(q.duration_ms),
                q.tool_calls,
                format!("{}/{}", q.delta_events, q.delta_chars),
                q.confirms,
                q.query_head,
            ));
        }
    }
    out
}

fn fmt_duration(ms: u64) -> String {
    if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{}m {}s", ms / 60_000, (ms % 60_000) / 1000)
    }
}

fn opt_duration(ms: Option<u64>) -> String {
    ms.map(fmt_duration).unwrap_or_else(|| "-".to_string())
}
