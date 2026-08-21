//! `proc agent <sub>` — v0.20 内置 AI agent CLI 入口（ADR-0030）。
//!
//! - `models`（stage 2）：GGUF scanner + ModelRegistry 表格输出
//! - `ask`（stage 3b）：单轮 query → AgentRunner ReAct loop → Markdown 输出。
//!   provider 构造链（决策 H）：CLI flag > agent.toml > 代码默认（llama-cpp）。
//! - `eval`（v0.22 stage 2）：70 query 评测 harness（ADR-0032）
//! - `session-info`（v0.22 stage 3）：session log 指标提取（ADR-0032 D5）

use colored::Colorize;

use super::def::AgentSub;
use crate::agent::config::AgentConfig;
use crate::agent::model_registry::{ModelRegistry, ModelStatus};
use crate::agent::runner::StepEvent;

/// 入口：dispatch agent 子命令。失败时打印错误并 exit 1（与既有 CLI 子命令同款）。
pub fn run_agent(sub: &AgentSub) {
    match sub {
        AgentSub::Models { refresh: _ } => {
            // 当前 scan 每次全量重扫，--refresh 仅是语义占位（无缓存层）。
            if let Err(e) = run_agent_models() {
                eprintln!("{} {}", "错误:".red(), e);
                std::process::exit(1);
            }
        }
        AgentSub::Ask {
            query,
            provider,
            model,
            max_steps,
        } => {
            if let Err(e) = run_agent_ask(query, provider.as_deref(), model.as_deref(), *max_steps)
            {
                eprintln!("{} {}", "错误:".red(), e);
                std::process::exit(1);
            }
        }
        AgentSub::Eval {
            level,
            scenario,
            quick,
            attempts,
            max_steps,
            output,
            compare,
            provider,
            model,
        } => {
            if let Err(e) = run_agent_eval(
                level.as_deref(),
                scenario,
                *quick,
                *attempts,
                *max_steps as u32,
                output.as_deref(),
                compare,
                provider.as_deref(),
                model.as_deref(),
            ) {
                eprintln!("{} {}", "错误:".red(), e);
                std::process::exit(1);
            }
        }
        AgentSub::SessionInfo { .. } => {
            // v0.22 stage 3 实装（session log 落地后可用）。
            eprintln!("proc agent session-info：v0.22 stage 3 实装（session log 落地后可用）");
        }
    }
}

// ---------------------------------------------------------------------------
// eval（v0.22 stage 2，ADR-0032）
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn run_agent_eval(
    level: Option<&str>,
    scenarios: &[String],
    quick: bool,
    attempts: u8,
    max_steps: u32,
    output: Option<&str>,
    compare: &[String],
    provider_flag: Option<&str>,
    model_flag: Option<&str>,
) -> Result<(), String> {
    use crate::agent::eval::report::{render_compare_markdown, render_markdown};
    use crate::agent::eval::runner::{self, EvalRunFile, EvalRunMeta};
    use crate::agent::eval::{build_report, parse_levels, select_queries};

    // compare 模式：读 N 份结果 JSON → 对比报告打印 stdout（不实跑）。
    if !compare.is_empty() {
        let mut runs = Vec::with_capacity(compare.len());
        for path in compare {
            let content =
                std::fs::read_to_string(path).map_err(|e| format!("读取 {} 失败: {e}", path))?;
            let run: EvalRunFile = serde_json::from_str(&content).map_err(|e| {
                format!(
                    "解析 {} 失败: {e}（应为 proc agent eval 的结果 JSON）",
                    path
                )
            })?;
            runs.push(run);
        }
        let labels: Vec<String> = compare
            .iter()
            .map(|p| {
                std::path::Path::new(p)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.clone())
            })
            .collect();
        println!("{}", render_compare_markdown(&runs, &labels));
        return Ok(());
    }

    // 实跑模式：加载 + 过滤 + 构造 runner（ask 同款 builder 链）。
    let levels = level.map(parse_levels).transpose()?;
    let all = crate::agent::eval::load_eval_queries()?;
    let selected = select_queries(&all, &levels.unwrap_or_default(), scenarios, quick)?;
    let (runner, spec) = crate::agent::builder::build_runner(provider_flag, model_flag, max_steps)?;

    let l0 = selected.iter().filter(|q| q.level == 0).count();
    let l1 = selected.iter().filter(|q| q.level == 1).count();
    let l2 = selected.iter().filter(|q| q.level == 2).count();
    eprintln!(
        "== eval: {} 模式，{} query（L0 {l0} / L1 {l1} / L2 {l2}），attempts={attempts}，max_steps={max_steps} ==",
        if quick { "QUICK" } else { "FULL" },
        selected.len(),
    );
    eprintln!("{} provider: {}", "·".dimmed(), spec.detail);

    let meta = EvalRunMeta {
        timestamp: runner::utc_timestamp_iso(),
        provider: spec.name.clone(),
        provider_detail: spec.detail.clone(),
        attempts,
        max_steps,
        git_describe: runner::git_describe(),
        quick,
        query_count: selected.len(),
    };
    let json_path = output.map(str::to_string).unwrap_or_else(|| {
        format!(
            "eval-{}-{}.json",
            spec.name,
            runner::utc_timestamp_compact()
        )
    });
    let md_path = {
        let mut stem = std::path::PathBuf::from(&json_path);
        stem.set_extension("md");
        stem.to_string_lossy().into_owned()
    };

    // CLI 自建 current_thread runtime（ask 同款）。
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio runtime 创建失败: {e}"))?;

    // 每 query 实时落盘：progress 回调全量重写 JSON（中途崩已跑数据不丢，
    // brainstorm 风险 1 mitigate 3）。
    let mut so_far: Vec<crate::agent::eval::QueryResult> = Vec::new();
    let results = rt.block_on(runner::run_eval(
        &runner,
        &selected,
        attempts,
        &mut |r, _idx, _total| {
            so_far.push(r.clone());
            eprintln!(
                "[L{}] {}: {} (tools: [{}], {} 步, stop: {}, attempt {}/{}, {:.1}s)",
                r.level,
                r.scenario,
                if r.passed { "PASS" } else { "FAIL" },
                r.actual_tools.join(","),
                r.actual_tools.len(),
                r.stop_cause,
                r.attempts_used,
                attempts,
                r.duration_ms as f64 / 1000.0,
            );
            let interim = EvalRunFile {
                meta: meta.clone(),
                results: so_far.clone(),
                report: build_report(&so_far),
            };
            if let Ok(json) = serde_json::to_string_pretty(&interim) {
                let _ = std::fs::write(&json_path, json);
            }
        },
    ));

    let run = EvalRunFile {
        meta: meta.clone(),
        report: build_report(&results),
        results,
    };
    std::fs::write(
        &json_path,
        serde_json::to_string_pretty(&run).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("写入 {} 失败: {e}", json_path))?;
    std::fs::write(&md_path, render_markdown(&run))
        .map_err(|e| format!("写入 {} 失败: {e}", md_path))?;

    for ls in &run.report.per_level {
        if ls.level == 2 {
            eprintln!(
                "===== L2: full-chain {}/{}，chain-step {}/{} =====",
                ls.full_chain, ls.total, ls.chain_steps_hit, ls.chain_steps_total
            );
        } else {
            eprintln!("===== L{}: {}/{} =====", ls.level, ls.passed, ls.total);
        }
    }
    eprintln!("{} 结果 JSON: {json_path}", "·".dimmed());
    eprintln!("{} 报告: {md_path}", "·".dimmed());
    Ok(())
}

// ---------------------------------------------------------------------------
// ask（stage 3b）
// ---------------------------------------------------------------------------

fn run_agent_ask(
    query: &str,
    provider_flag: Option<&str>,
    model_flag: Option<&str>,
    max_steps: u32,
) -> Result<(), String> {
    // provider 构造链抽共享（v0.21 stage 2）：CLI 与 TUI AgentSession 共用
    // `agent::builder::build_runner`（CLI flag > agent.toml > 代码默认）。
    let (runner, spec) = crate::agent::builder::build_runner(provider_flag, model_flag, max_steps)?;
    eprintln!("{} provider: {}", "·".dimmed(), spec.detail);

    // CLI 自建 current_thread runtime（agent 不走 MCP server runtime，风险 6 规避）。
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio runtime 创建失败: {e}"))?;

    let outcome = rt
        .block_on(runner.run_with_progress(query, &|ev| match ev {
            StepEvent::LlmTurn(n) => {
                eprintln!("{} LLM 第 {} 轮", "·".dimmed(), n + 1);
            }
            StepEvent::ToolStart(name, args) => {
                eprintln!("{} {name} {}", "→".cyan(), args.to_string().dimmed());
            }
        }))
        .map_err(|e| format!("agent 运行失败: {e}"))?;

    if outcome.stop != crate::agent::runner::StopCause::EndTurn {
        eprintln!(
            "{} 终止原因: {}（{} 步 / {} tool call）",
            "⚠".yellow(),
            outcome.stop.label(),
            outcome.steps.len(),
            outcome.steps.len()
        );
    }
    println!("{}", outcome.final_text);
    Ok(())
}

fn run_agent_models() -> Result<(), String> {
    let config = AgentConfig::load();
    // 默认扫描路径 + agent.toml 自定义路径（ModelRegistry::scan 只扫传入路径，
    // 占位符在 scan 内展开）。
    let mut paths: Vec<String> = crate::agent::gguf_scan::default_scan_paths()
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    paths.extend(config.llama_cpp.search_paths.iter().cloned());
    let mut registry = ModelRegistry::new();
    registry
        .scan(&paths)
        .map_err(|e| format!("模型扫描失败: {e}"))?;

    let models = registry.models();
    if models.is_empty() {
        println!("{}", "未检测到本地 GGUF 模型".yellow());
        println!();
        println!("默认扫描路径：");
        for path in crate::agent::gguf_scan::default_scan_paths() {
            println!("  {}", path.display());
        }
        println!();
        println!(
            "可在 {} 的 [llama-cpp] search_paths 中追加自定义路径（支持 %VAR% 占位符）。",
            "~/.config/proc/agent.toml".bold()
        );
        return Ok(());
    }

    println!("检测到 {} 个本地模型：\n", models.len().to_string().green());
    println!(
        "{:<36} {:>10}  {:<8} {:<12} {}",
        "NAME".bold(),
        "SIZE".bold(),
        "QUANT".bold(),
        "ARCH".bold(),
        "PATH".bold()
    );
    for model in models {
        let status_mark = match model.status {
            ModelStatus::Available => String::new(),
            _ => " [metadata 解析失败]".red().to_string(),
        };
        println!(
            "{:<36} {:>10}  {:<8} {:<12} {}{}",
            model.name,
            format_size(model.size_bytes),
            model.quantization.as_deref().unwrap_or("-"),
            model.architecture.as_deref().unwrap_or("-"),
            model.path.display(),
            status_mark
        );
    }
    Ok(())
}

fn format_size(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GIB {
        format!("{:.1}G", b / GIB)
    } else if b >= MIB {
        format!("{:.0}M", b / MIB)
    } else {
        format!("{bytes}B")
    }
}
