//! eval markdown 报告（v0.22 stage 2）——**按 ADR-0032 附录样式草案实施，
//! 不重新设计**（brainstorm 风险 4 mitigate 1）。一处小偏差：run 时间戳用
//! UTC（`Z` 后缀）非本地时区——不引 chrono，`src/cli/record.rs` 同款决策。

use super::LevelSummary;
use super::runner::EvalRunFile;

/// 失败明细表 query 列的截断长度（char 级）。
const QUERY_DISPLAY_CHARS: usize = 50;

/// 单 run 报告（`eval-<provider>-<ts>.md`）。
pub fn render_markdown(run: &EvalRunFile) -> String {
    let mut md = String::new();
    md.push_str("# proc agent eval 报告\n\n");
    md.push_str(&format!("- run: {}\n", run.meta.timestamp));
    md.push_str(&format!(
        "- provider: {} ({})\n",
        run.meta.provider, run.meta.provider_detail
    ));
    md.push_str(&format!(
        "- 参数: attempts={}, max_steps={}, git {}\n",
        run.meta.attempts, run.meta.max_steps, run.meta.git_describe
    ));
    if run.meta.quick {
        md.push_str("- 模式: QUICK（每 scenario×level 抽 1 条）\n");
    }
    md.push_str(&format!(
        "- 总时长: {}\n",
        format_duration_ms(run.report.total_duration_ms)
    ));

    // ── 通过率（per level）────────────────────────────────────────────
    md.push_str("\n## 通过率（per level）\n\n");
    md.push_str("| Level | 通过 | 总数 | 通过率 |\n|---|---|---|---|\n");
    for ls in &run.report.per_level {
        match ls.level {
            2 => {
                md.push_str(&format!(
                    "| L2 full-chain | {} | {} | {} {:.0}% |\n",
                    ls.full_chain,
                    ls.total,
                    bar(pct(ls.full_chain, ls.total), 10),
                    pct(ls.full_chain, ls.total)
                ));
                md.push_str(&format!(
                    "| L2 chain-step | {}/{} | — | {} {:.0}%（链步命中 {} / 总链步 {}） |\n",
                    ls.chain_steps_hit,
                    ls.chain_steps_total,
                    bar(pct(ls.chain_steps_hit, ls.chain_steps_total), 10),
                    pct(ls.chain_steps_hit, ls.chain_steps_total),
                    ls.chain_steps_hit,
                    ls.chain_steps_total
                ));
            }
            level => {
                md.push_str(&format!(
                    "| L{level} | {} | {} | {} {:.0}% |\n",
                    ls.passed,
                    ls.total,
                    bar(pct(ls.passed, ls.total), 10),
                    pct(ls.passed, ls.total)
                ));
            }
        }
    }

    // ── 失败模式直方图 ────────────────────────────────────────────────
    md.push_str("\n## 失败模式直方图\n\n");
    let failures: usize = run.report.failure_histogram.iter().map(|(_, n)| n).sum();
    if failures == 0 {
        md.push_str("（无失败 query）\n");
    } else {
        md.push_str("| 失败模式 | 次数 | 分布 |\n|---|---|---|\n");
        for (name, count) in &run.report.failure_histogram {
            let share = pct(*count, failures);
            md.push_str(&format!(
                "| {name} | {count} | {} {share}% |\n",
                bar(share, 20)
            ));
        }
    }

    // ── 失败 query 明细 ───────────────────────────────────────────────
    md.push_str("\n## 失败 query 明细\n\n");
    let failed: Vec<&super::QueryResult> = run.results.iter().filter(|r| !r.passed).collect();
    if failed.is_empty() {
        md.push_str("（无失败 query）\n");
    } else {
        md.push_str(
            "| # | Level | Scenario | Query | 失败模式 | 链命中 | stop | final_text（截断） |\n\
             |---|---|---|---|---|---|---|---|\n",
        );
        for (i, r) in failed.iter().enumerate() {
            let chain = if r.level == 2 {
                format!("{}/{}", r.chain_steps_hit, r.expected_tools.len())
            } else {
                "—".to_string()
            };
            md.push_str(&format!(
                "| {} | L{} | {} | {} | {} | {chain} | {} | {} |\n",
                i + 1,
                r.level,
                r.scenario,
                table_cell(&super::runner::truncate_chars(
                    &r.query,
                    QUERY_DISPLAY_CHARS
                )),
                r.failure_mode.label(),
                r.stop_cause,
                table_cell(&r.final_text_head)
            ));
        }
    }
    md
}

/// 对比报告（`--compare a.json b.json`）：run × 指标并列表 + 失败模式迁移
/// （首 run → 末 run；≥3 run 时同款取首末）。
pub fn render_compare_markdown(runs: &[EvalRunFile], labels: &[String]) -> String {
    let mut md = String::new();
    md.push_str("# proc agent eval 对比报告\n\n");
    md.push_str("| run | provider | L0 | L1 | L2 full-chain | L2 chain-step | OutputDegraded |\n");
    md.push_str("|---|---|---|---|---|---|---|\n");
    for (run, label) in runs.iter().zip(labels.iter()) {
        md.push_str(&format!(
            "| {label} | {} | {} | {} | {} | {} | {} |\n",
            run.meta.provider,
            level_cell(run, 0),
            level_cell(run, 1),
            level_cell(run, 2),
            chain_step_cell(run),
            histogram_count(run, "output_degraded")
        ));
    }

    if runs.len() >= 2 {
        let (first, last) = (&runs[0], &runs[runs.len() - 1]);
        md.push_str(&format!(
            "\n## 失败模式迁移（{} → {}）\n\n",
            labels[0],
            labels[labels.len() - 1]
        ));
        md.push_str(&format!(
            "| 失败模式 | {} | {} | Δ |\n|---|---|---|---|\n",
            labels[0],
            labels[labels.len() - 1]
        ));
        let mut modes: Vec<&str> = Vec::new();
        for run in [first, last] {
            for (name, _) in &run.report.failure_histogram {
                if !modes.contains(&name.as_str()) {
                    modes.push(name.as_str());
                }
            }
        }
        let mut rows: Vec<(&str, usize, usize)> = modes
            .into_iter()
            .map(|m| (m, histogram_count(first, m), histogram_count(last, m)))
            .collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.1 + r.2));
        for (mode, a, b) in rows {
            md.push_str(&format!(
                "| {mode} | {a} | {b} | {} |\n",
                b as isize - a as isize
            ));
        }
    }
    md
}

fn level_cell(run: &EvalRunFile, level: u8) -> String {
    match find_level(run, level) {
        Some(ls) if level == 2 => format!("{}/{}", ls.full_chain, ls.total),
        Some(ls) => format!("{}/{}", ls.passed, ls.total),
        None => "—".to_string(),
    }
}

fn chain_step_cell(run: &EvalRunFile) -> String {
    match find_level(run, 2) {
        Some(ls) => format!("{}/{}", ls.chain_steps_hit, ls.chain_steps_total),
        None => "—".to_string(),
    }
}

fn find_level(run: &EvalRunFile, level: u8) -> Option<&LevelSummary> {
    run.report.per_level.iter().find(|ls| ls.level == level)
}

fn histogram_count(run: &EvalRunFile, mode: &str) -> usize {
    run.report
        .failure_histogram
        .iter()
        .find(|(n, _)| n == mode)
        .map(|(_, c)| *c)
        .unwrap_or(0)
}

/// 百分比（分母 0 返 0）。
fn pct(n: usize, total: usize) -> u8 {
    if total == 0 {
        0
    } else {
        (n as f64 / total as f64 * 100.0).round() as u8
    }
}

/// ASCII 条形（`scale` 格满刻度；余数 ≥ 半格补 `▊`）。
fn bar(pct: u8, scale: usize) -> String {
    let per = 100.0 / scale as f64;
    let full = ((pct as f64 / per).floor() as usize).min(scale);
    let partial = ((pct as f64 - full as f64 * per) / per) >= 0.5 && full < scale;
    "█".repeat(full) + if partial { "▊" } else { "" }
}

/// 表格单元格净化：换行折空格、竖线转义。
fn table_cell(s: &str) -> String {
    s.replace(['\n', '\r'], " ").replace('|', "\\|")
}

/// `4h 32m 15s` / `12m 03s` / `45s` 风格时长。
fn format_duration_ms(ms: u64) -> String {
    let secs = ms / 1000;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h {m:02}m {s:02}s")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}
