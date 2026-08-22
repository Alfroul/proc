//! provider 构造链（v0.21 stage 2 从 `cli/agent_cmd.rs` 抽共享，ADR-0031 D1）。
//!
//! CLI ask 与 TUI AgentSession 共用：CLI flag > agent.toml > 代码默认的解析
//! 顺序 + 三 provider（llama-cpp / mock / anthropic）cfg-gate 构造 + grammar
//! 逃生舱 + 采样参数选段。`ProviderSpec` 携带一行描述（CLI stderr 打印 /
//! TUI 面板状态行）。

use std::path::PathBuf;
use std::sync::Arc;

use super::config::AgentConfig;
use super::model_registry::ModelRegistry;
use super::provider::LlmProvider;
use super::runner::{AgentOptions, AgentRunner};
use super::tool_registry::ToolRegistry;

/// 构造产物的 provider 描述（CLI 打印 / 面板状态行「provider/model」段）。
#[derive(Debug, Clone)]
pub struct ProviderSpec {
    pub name: String,
    pub detail: String,
}

/// 组装 AgentRunner（决策 H 构造链：CLI flag > agent.toml > 代码默认）。
///
/// `provider_flag` / `model_flag` 来自 CLI（TUI 场景传 `None` 走 agent.toml /
/// 代码默认）；`max_steps` 由调用方定（CLI flag 或面板默认）。
pub fn build_runner(
    provider_flag: Option<&str>,
    model_flag: Option<&str>,
    max_steps: u32,
) -> Result<(AgentRunner, ProviderSpec), String> {
    let (provider, registry, options, spec) = build_parts(provider_flag, model_flag, max_steps)?;
    Ok((AgentRunner::new(provider, registry, options), spec))
}

/// v0.21 stage 3：TUI AgentPanel 进面板时建会话（与 CLI ask 共用构造链；
/// llama-server 仍惰性 spawn 于首次 query——D5 按需 spawn 延续）。
///
/// v0.22 stage 3：按 agent.toml `[session].log`（默认 true）构造
/// SessionRecorder（ADR-0032 D5）。
pub fn build_session(
    provider_flag: Option<&str>,
    model_flag: Option<&str>,
    max_steps: u32,
) -> Result<(super::session::SessionHandle, ProviderSpec), String> {
    let (provider, registry, options, spec) = build_parts(provider_flag, model_flag, max_steps)?;
    let recorder = if AgentConfig::load().session.log {
        super::session_log::SessionRecorder::start(&spec.name)
    } else {
        super::session_log::SessionRecorder::disabled()
    };
    Ok((
        super::session::AgentSession::spawn(provider, registry, options, recorder),
        spec,
    ))
}

/// 构造链主体（build_runner / build_session 共享）：resolve provider 三件套
/// （provider / registry / options）+ 描述。`AgentRunner` 三字段私有且
/// `ToolRegistry` 非 Clone，共享 parts 是唯一不破坏既有 API 的复用路径。
fn build_parts(
    provider_flag: Option<&str>,
    model_flag: Option<&str>,
    max_steps: u32,
) -> Result<
    (
        Arc<dyn LlmProvider>,
        ToolRegistry,
        AgentOptions,
        ProviderSpec,
    ),
    String,
> {
    let config = AgentConfig::load();
    let provider_name = provider_flag
        .map(str::to_string)
        .or_else(|| config.default.provider.clone())
        .unwrap_or_else(|| "llama-cpp".to_string());

    // 决策 C：grammar 逃生舱——agent.toml [llama-cpp] grammar_file = "tool_call"
    // 显式启用才进 ReAct 循环（默认不启用，OpenAI tools 协议解析可靠）。
    let grammar = (config.llama_cpp.grammar_file.as_deref() == Some("tool_call"))
        .then(|| super::grammars::TOOL_CALL_GRAMMAR.to_string());

    // anthropic 时 temperature 用 [anthropic] 段（其余采样参数 provider 侧按
    // Anthropic「至多一个」约束取 temperature 优先，top_p / top_k 不会外发）。
    let temperature = if provider_name == "anthropic" {
        #[cfg(feature = "anthropic")]
        {
            config.anthropic.temperature
        }
        #[cfg(not(feature = "anthropic"))]
        {
            config.llama_cpp.temperature
        }
    } else {
        config.llama_cpp.temperature
    };

    let options = AgentOptions {
        max_steps,
        grammar,
        temperature,
        top_p: config.llama_cpp.top_p,
        top_k: config.llama_cpp.top_k,
    };

    let registry = super::tools::catalog::default_registry();

    // 显式类型标注：--no-default-features 下所有分支都提前 return Err，
    // match 各臂 divergent 时类型推断缺锚点。
    let provider: Arc<dyn LlmProvider> = match provider_name.as_str() {
        "mock" => {
            #[cfg(feature = "mock-provider")]
            {
                let dir = config
                    .mock
                    .fixtures_dir
                    .clone()
                    .unwrap_or_else(|| "tests/fixtures/agent".to_string());
                let provider = super::mock_provider::MockProvider::new(PathBuf::from(dir));
                Arc::new(provider)
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
                let spec = ProviderSpec {
                    name: provider_name.clone(),
                    detail: format!(
                        "llama-server: {} | model: {}",
                        server_path.display(),
                        model_path.display()
                    ),
                };
                let spawn_opts = super::llama_server_handle::SpawnOptions {
                    ctx_size: config.llama_cpp.ctx_size,
                    no_thinks: config.llama_cpp.no_thinks,
                    chat_template: None,
                    ..Default::default()
                };
                let provider = super::llama_cpp_provider::LlamaCppProvider::with_options(
                    server_path,
                    model_path,
                    spawn_opts,
                );
                return Ok((Arc::new(provider), registry, options, spec));
            }
            #[cfg(not(feature = "llama-cpp"))]
            {
                return Err(
                    "llama-cpp feature 未启用（默认 build 已含；最小化 build 请用 --provider mock)"
                        .to_string(),
                );
            }
        }
        "anthropic" => {
            #[cfg(feature = "anthropic")]
            {
                // model 解析（决策 H 同款链）：--model > [anthropic].model >
                // [default].model > 代码默认。
                let model = model_flag
                    .map(str::to_string)
                    .or_else(|| config.anthropic.model.clone())
                    .or_else(|| config.default.model.clone())
                    .unwrap_or_else(|| super::anthropic_provider::DEFAULT_MODEL.to_string());
                let provider = super::anthropic_provider::AnthropicProvider::from_env(
                    model.clone(),
                    config.anthropic.max_tokens.unwrap_or(4096),
                )
                .map_err(|e| e.to_string())?;
                let spec = ProviderSpec {
                    name: provider_name.clone(),
                    detail: model,
                };
                return Ok((Arc::new(provider), registry, options, spec));
            }
            #[cfg(not(feature = "anthropic"))]
            {
                return Err(
                    "anthropic feature 未启用（cargo build --release --features anthropic + \
                     ANTHROPIC_API_KEY）"
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

    // mock 分支（llama-cpp / anthropic 已在各自分支内 return）。
    let spec = ProviderSpec {
        name: provider_name,
        detail: "mock fixtures 回放".to_string(),
    };
    Ok((provider, registry, options, spec))
}

/// llama-server 路径解析：agent.toml `[llama-cpp].server_path` > PATH 查找 > 报错。
pub fn resolve_server_path(config: &AgentConfig) -> Result<PathBuf, String> {
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
pub fn resolve_model_path(
    config: &AgentConfig,
    model_flag: Option<&str>,
) -> Result<PathBuf, String> {
    let explicitly_set = model_flag.is_some() || config.default.model.is_some();
    let wanted = model_flag
        .map(str::to_string)
        .or_else(|| config.default.model.clone())
        .unwrap_or_else(|| "gemma-4-E2B-it-Q4_K_M".to_string());

    let mut paths: Vec<String> = super::gguf_scan::default_scan_paths()
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
