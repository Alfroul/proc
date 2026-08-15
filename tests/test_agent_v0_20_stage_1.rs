//! v0.20 stage 1 Spike stub 测试（ADR-0030）。
//!
//! 验证 agent 模块骨架：编译 / serde round-trip / ToolRegistry 视图 /
//! proc_help schema / cfg-gate / CLI subcommand 注册。业务逻辑
//! （fixture 回放 / GGUF 扫描 / provider client）留 stage 2/3/4 实装测试。

use clap::{CommandFactory, Parser};

use proc::agent::ToolRegistry;
use proc::agent::provider::LlmProvider;
use proc::agent::tool_registry::ENTRY_TOOL_NAMES;
use proc::agent::types::{Message, Role, ToolCategory, ToolSchema};

#[test]
fn test_agent_module_compiles() {
    // agent 模块核心类型可正常 import + 构造（模块骨架完整）。
    let _msg = Message::new(Role::User, "我电脑为什么这么卡？");
    let _opts = proc::agent::CompleteOptions::default();
    let _config = proc::agent::AgentConfig::default();
}

#[test]
fn test_llm_provider_trait_object_works() {
    // dyn 兼容性：Box<dyn LlmProvider> 可构造（feature mock-provider 默认启用）。
    #[cfg(feature = "mock-provider")]
    {
        let provider: Box<dyn LlmProvider> = Box::new(
            proc::agent::mock_provider::MockProvider::new("tests/fixtures/agent".into()),
        );
        assert_eq!(provider.name(), "mock");
    }
    #[cfg(feature = "llama-cpp")]
    {
        let provider: Box<dyn LlmProvider> =
            Box::new(proc::agent::llama_cpp_provider::LlamaCppProvider::new(
                "llama-server.exe".into(),
                "model.gguf".into(),
            ));
        assert_eq!(provider.name(), "llama-cpp");
    }
}

#[test]
fn test_message_serialization() {
    let msg = Message::new(Role::User, "列出 CPU 占用最高的 10 个进程");
    let json = serde_json::to_string(&msg).unwrap();
    let back: Message = serde_json::from_str(&json).unwrap();
    assert_eq!(back.role, Role::User);
    assert_eq!(
        back.content.as_deref(),
        Some("列出 CPU 占用最高的 10 个进程")
    );
    assert!(back.tool_calls.is_empty());
    assert!(back.tool_results.is_empty());
}

#[test]
fn test_tool_schema_json_schema_format() {
    let schema = proc::agent::tools::help::schema();
    assert_eq!(schema.parameters["type"], "object");
    assert!(schema.parameters["properties"].is_object());
}

#[test]
fn test_tool_category_enum_serialization() {
    assert_eq!(
        serde_json::to_string(&ToolCategory::Docker).unwrap(),
        "\"docker\""
    );
    assert_eq!(
        serde_json::to_string(&ToolCategory::Meta).unwrap(),
        "\"meta\""
    );
    // round-trip
    let cat: ToolCategory = serde_json::from_str("\"performance\"").unwrap();
    assert_eq!(cat, ToolCategory::Performance);
}

#[test]
fn test_tool_registry_new_returns_empty() {
    let registry = ToolRegistry::new();
    assert!(registry.is_empty());
    assert_eq!(registry.len(), 0);
    assert!(registry.entry_tools().is_empty());
    assert!(registry.tools_by_category(None).is_empty());
    assert!(registry.get("proc_ls").is_none());
}

#[test]
fn test_tool_registry_register_and_views() {
    let mut registry = ToolRegistry::new();
    let ls = ToolSchema {
        name: "proc_ls".to_string(),
        description: "list processes".to_string(),
        parameters: serde_json::json!({"type": "object"}),
        category: ToolCategory::Process,
        estimated_tokens: 150,
    };
    let docker_ps = ToolSchema {
        name: "proc_docker_ps".to_string(),
        description: "list containers".to_string(),
        parameters: serde_json::json!({"type": "object"}),
        category: ToolCategory::Docker,
        estimated_tokens: 100,
    };
    registry.register(ls);
    registry.register(docker_ps);

    assert_eq!(registry.len(), 2);
    // entry tool 视图只含注册过的 entry 名单成员
    let entry = registry.entry_tools();
    assert_eq!(entry.len(), 1);
    assert_eq!(entry[0].name, "proc_ls");
    // Layer 1 视图：None = 全部非 entry；Some(Docker) = 按 category
    let non_entry = registry.tools_by_category(None);
    assert_eq!(non_entry.len(), 1);
    assert_eq!(non_entry[0].name, "proc_docker_ps");
    let docker_tools = registry.tools_by_category(Some(ToolCategory::Docker));
    assert_eq!(docker_tools.len(), 1);
    // token 预算监控
    assert_eq!(
        registry.total_tokens(&["proc_ls".to_string(), "proc_docker_ps".to_string()]),
        250
    );
}

#[test]
fn test_entry_tool_names_constant() {
    assert_eq!(ENTRY_TOOL_NAMES.len(), 4);
    assert!(ENTRY_TOOL_NAMES.contains(&"proc_help"));
}

#[test]
fn test_proc_help_schema_returns_meta_category() {
    let schema = proc::agent::tools::help::schema();
    assert_eq!(schema.name, "proc_help");
    assert_eq!(schema.category, ToolCategory::Meta);
    assert!(schema.estimated_tokens > 0);
}

#[test]
fn test_proc_help_execute_on_empty_registry() {
    let registry = ToolRegistry::new();
    let tools = proc::agent::tools::help::execute(&registry, Some(ToolCategory::Docker));
    assert!(tools.is_empty());
}

#[test]
fn test_grammars_embedded_in_binary() {
    let grammar = proc::agent::grammars::TOOL_CALL_GRAMMAR;
    assert!(grammar.contains("tool_calls"));
    assert!(grammar.contains("root ::="));
}

#[test]
fn test_system_prompt_embedded_with_snapshot_placeholder() {
    let prompt = proc::agent::prompts::SYSTEM_PROMPT;
    assert!(prompt.contains("{{SYSTEM_SNAPSHOT}}"));
    // 3 个 few-shot 示例（附录 C）
    assert!(prompt.contains("示例 1"));
    assert!(prompt.contains("示例 2"));
    assert!(prompt.contains("示例 3"));
}

#[test]
fn test_agent_config_from_toml() {
    let content = r#"
[default]
provider = "llama-cpp"
model = "gemma-4-E2B-it-Q4_K_M"
max_steps = 10

[llama-cpp]
ctx_size = 8192
no_thinks = true
"#;
    let config = proc::agent::AgentConfig::from_toml(content).unwrap();
    assert_eq!(config.default.provider.as_deref(), Some("llama-cpp"));
    assert_eq!(
        config.default.model.as_deref(),
        Some("gemma-4-E2B-it-Q4_K_M")
    );
    assert_eq!(config.default.max_steps, Some(10));
    assert_eq!(config.llama_cpp.ctx_size, Some(8192));
    assert!(config.llama_cpp.no_thinks);
}

#[test]
fn test_agent_config_llama_cpp_no_thinks_defaults_true() {
    // agent.toml 缺 [llama-cpp] 段时 no_thinks 默认 true（ADR-0030 D6 强制禁用）。
    let config = proc::agent::AgentConfig::from_toml("").unwrap();
    assert!(config.llama_cpp.no_thinks);
}

#[cfg(feature = "mock-provider")]
#[test]
fn test_mock_provider_compiles_with_feature_gate() {
    let provider = proc::agent::mock_provider::MockProvider::new("fixtures".into());
    assert_eq!(provider.name(), "mock");
}

#[cfg(feature = "llama-cpp")]
#[test]
fn test_llama_cpp_provider_compiles_with_feature_gate() {
    let provider = proc::agent::llama_cpp_provider::LlamaCppProvider::new(
        "llama-server.exe".into(),
        "model.gguf".into(),
    );
    assert_eq!(provider.name(), "llama-cpp");
}

#[cfg(feature = "anthropic")]
#[test]
fn test_anthropic_provider_compiles_with_feature_gate() {
    // ANTHROPIC_API_KEY 未设置时 friendly error（不 panic）。
    std::env::remove_var("ANTHROPIC_API_KEY");
    let result = proc::agent::anthropic_provider::AnthropicProvider::from_env(
        "claude-sonnet-4-6".into(),
        4096,
    );
    assert!(result.is_err());
}

#[test]
fn test_cli_agent_subcommand_registered() {
    // `proc agent --help` 不报 unrecognized subcommand（subcommand 已注册）。
    let cmd = proc::cli::Cli::command();
    assert!(cmd.find_subcommand("agent").is_some());
    let agent = cmd.find_subcommand("agent").unwrap();
    assert!(agent.find_subcommand("ask").is_some());
    assert!(agent.find_subcommand("models").is_some());
}

#[test]
fn test_cli_agent_ask_parses() {
    let cli = proc::cli::Cli::try_parse_from(["proc", "agent", "ask", "列出 top 10 进程"]).unwrap();
    match cli.command {
        Some(proc::cli::Command::Agent {
            sub:
                proc::cli::def::AgentSub::Ask {
                    ref query,
                    max_steps,
                    ..
                },
        }) => {
            assert_eq!(query, "列出 top 10 进程");
            assert_eq!(max_steps, 10);
        }
        other => panic!("expected Agent(Ask), got {other:?}"),
    }
}
