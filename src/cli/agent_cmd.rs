//! `proc agent <sub>` — v0.20 内置 AI agent CLI 入口（ADR-0030）。
//!
//! - `models`（stage 2）：GGUF scanner + ModelRegistry 表格输出
//! - `ask`（stage 3b）：单轮 query → AgentRunner ReAct loop → Markdown 输出。
//!   provider 构造链（决策 H）：CLI flag > agent.toml > 代码默认（llama-cpp）。

use std::path::PathBuf;
use std::sync::Arc;

use colored::Colorize;

use super::def::AgentSub;
use crate::agent::config::AgentConfig;
use crate::agent::model_registry::{ModelRegistry, ModelStatus};
use crate::agent::runner::{AgentOptions, AgentRunner, StepEvent};

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
    }
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
    let config = AgentConfig::load();
    let provider_name = provider_flag
        .map(str::to_string)
        .or_else(|| config.default.provider.clone())
        .unwrap_or_else(|| "llama-cpp".to_string());

    // 决策 H：anthropic 拦到 stage 4（provider 本体 stage 4 实装，先拦防 todo!() panic）。
    if provider_name == "anthropic" {
        return Err(
            "AnthropicProvider 在 v0.20 stage 4 落地（cargo build --features anthropic + \
             ANTHROPIC_API_KEY）"
                .to_string(),
        );
    }

    // 决策 C：grammar 逃生舱——agent.toml [llama-cpp] grammar_file = "tool_call"
    // 显式启用才进 ReAct 循环（默认不启用，OpenAI tools 协议解析可靠）。
    let grammar = (config.llama_cpp.grammar_file.as_deref() == Some("tool_call"))
        .then(|| crate::agent::grammars::TOOL_CALL_GRAMMAR.to_string());

    let options = AgentOptions {
        max_steps,
        grammar,
        temperature: config.llama_cpp.temperature,
        top_p: config.llama_cpp.top_p,
        top_k: config.llama_cpp.top_k,
    };

    // 显式类型标注：--no-default-features 下所有分支都提前 return Err，
    // match 各臂 divergent 时类型推断缺锚点。
    let runner: AgentRunner = match provider_name.as_str() {
        "mock" => {
            #[cfg(feature = "mock-provider")]
            {
                let dir = config
                    .mock
                    .fixtures_dir
                    .clone()
                    .unwrap_or_else(|| "tests/fixtures/agent".to_string());
                let provider = crate::agent::mock_provider::MockProvider::new(PathBuf::from(dir));
                AgentRunner::new(
                    Arc::new(provider),
                    crate::agent::tools::catalog::default_registry(),
                    options,
                )
            }
            #[cfg(not(feature = "mock-provider"))]
            {
                return Err(
                    "mock-provider feature 未启用（默认 build 已含，检查 build 配置）".to_string(),
                );
            }
        }
        "llama-cpp" => {
            #[cfg(feature = "llama-cpp")]
            {
                let server_path = resolve_server_path(&config)?;
                let model_path = resolve_model_path(&config, model_flag)?;
                eprintln!(
                    "{} llama-server: {}\n{} model: {}",
                    "·".dimmed(),
                    server_path.display(),
                    "·".dimmed(),
                    model_path.display()
                );
                let spawn_opts = crate::agent::llama_server_handle::SpawnOptions {
                    ctx_size: config.llama_cpp.ctx_size,
                    no_thinks: config.llama_cpp.no_thinks,
                    chat_template: None,
                    ..Default::default()
                };
                let provider = crate::agent::llama_cpp_provider::LlamaCppProvider::with_options(
                    server_path,
                    model_path,
                    spawn_opts,
                );
                AgentRunner::new(
                    Arc::new(provider),
                    crate::agent::tools::catalog::default_registry(),
                    options,
                )
            }
            #[cfg(not(feature = "llama-cpp"))]
            {
                return Err(
                    "llama-cpp feature 未启用（默认 build 已含；最小化 build 请用 --provider mock)"
                        .to_string(),
                );
            }
        }
        other => {
            return Err(format!(
                "未知 provider '{other}'（合法值：llama-cpp / mock / anthropic）"
            ));
        }
    };

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

/// llama-server 路径解析：agent.toml `[llama-cpp].server_path` > PATH 查找 > 报错。
fn resolve_server_path(config: &AgentConfig) -> Result<PathBuf, String> {
    if let Some(p) = &config.llama_cpp.server_path {
        let path = PathBuf::from(p);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "agent.toml [llama-cpp] server_path 指向的文件不存在: {}",
            path.display()
        ));
    }
    if let Some(found) = find_llama_server_in_path() {
        return Ok(found);
    }
    Err(
        "未找到 llama-server：请在 ~/.config/proc/agent.toml 的 [llama-cpp] 配置 \
         server_path，或把 llama-server.exe 加入 PATH"
            .to_string(),
    )
}

fn find_llama_server_in_path() -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(exe))
        .find(|candidate| candidate.is_file())
}

/// 模型路径解析（决策 H）：`--model` > agent.toml `[default].model` > 代码默认
/// `gemma-4-E2B-it-Q4_K_M`（brainstorm 决策 9）。名称匹配 GGUF general.name
/// 或文件 stem；用户未显式指定且只扫到 1 个模型时自动选用（单模型机器开箱即用）。
fn resolve_model_path(config: &AgentConfig, model_flag: Option<&str>) -> Result<PathBuf, String> {
    let explicitly_set = model_flag.is_some() || config.default.model.is_some();
    let wanted = model_flag
        .map(str::to_string)
        .or_else(|| config.default.model.clone())
        .unwrap_or_else(|| "gemma-4-E2B-it-Q4_K_M".to_string());

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
    let matched = models.iter().find(|m| {
        m.name.eq_ignore_ascii_case(&wanted)
            || m.path
                .file_stem()
                .is_some_and(|s| s.eq_ignore_ascii_case(std::ffi::OsStr::new(&wanted)))
    });
    if let Some(m) = matched {
        return Ok(m.path.clone());
    }
    if !explicitly_set && models.len() == 1 {
        eprintln!(
            "{} 未配置默认模型，自动选用唯一检测到的模型: {}",
            "·".dimmed(),
            models[0].name
        );
        return Ok(models[0].path.clone());
    }
    let list = models
        .iter()
        .map(|m| format!("  - {} ({})", m.name, m.path.display()))
        .collect::<Vec<_>>()
        .join("\n");
    Err(format!(
        "未找到模型 '{wanted}'。检测到以下模型：\n{list}\n\
         可用 --model <name> 指定，或在 ~/.config/proc/agent.toml 的 [default] 配置 model。"
    ))
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
