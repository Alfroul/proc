//! v0.20 stage 2 Slice A 集成测试（ADR-0030）。
//!
//! 覆盖：47 tool catalog 视图 / MockProvider seed fixture 回放（50 query 零
//! LLM 调用）/ record_fixture round-trip / 手写 GGUF metadata parser /
//! ModelRegistry 扫描（含 %VAR% 展开与 magic 嗅探）。

use std::path::PathBuf;

use async_trait::async_trait;
use proc::agent::ToolRegistry;
use proc::agent::mock_provider::{FixtureEntry, MockProvider, query_hash};
use proc::agent::model_registry::{ModelRegistry, ModelStatus};
use proc::agent::provider::{
    CompleteOptions, CompleteResponse, Delta, LlmError, LlmProvider, ProviderStream, StopReason,
};
use proc::agent::tools::catalog::default_registry;
use proc::agent::types::{Message, Role, ToolCall, ToolCategory, ToolSchema};

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

// ---------- 任务 1：Delta / StopReason serde ----------

#[test]
fn test_delta_serde_round_trip() {
    let deltas = vec![
        Delta::Text("查看系统".to_string()),
        Delta::ToolCall(ToolCall {
            id: "c1".to_string(),
            name: "proc_ls".to_string(),
            arguments: serde_json::json!({"sort": "cpu"}),
        }),
        Delta::EndTurn {
            stop_reason: StopReason::ToolUse,
        },
    ];
    let json = serde_json::to_string(&deltas).unwrap();
    assert!(json.contains("{\"Text\":\"查看系统\"}"));
    assert!(json.contains("\"tool_use\""));
    let back: Vec<Delta> = serde_json::from_str(&json).unwrap();
    assert_eq!(back.len(), 3);
    assert!(matches!(back[0], Delta::Text(ref t) if t == "查看系统"));
    assert!(
        matches!(&back[2], Delta::EndTurn { stop_reason } if *stop_reason == StopReason::ToolUse)
    );
}

// ---------- 任务 2：catalog ----------

#[test]
fn test_catalog_default_registry_counts() {
    let registry = default_registry();
    // 46 MCP tool + proc_help = 47；entry 4；非 entry 43。
    assert_eq!(registry.len(), 47);
    assert_eq!(registry.entry_tools().len(), 4);
    assert_eq!(registry.tools_by_category(None).len(), 43);
    for name in proc::agent::tool_registry::ENTRY_TOOL_NAMES {
        assert!(registry.get(name).is_some(), "entry tool {name} 未注册");
    }
}

#[test]
fn test_catalog_every_category_has_tools() {
    let registry = default_registry();
    for cat in [
        ToolCategory::Process,
        ToolCategory::Performance,
        ToolCategory::Docker,
        ToolCategory::Usb,
        ToolCategory::Security,
        ToolCategory::Recording,
        ToolCategory::Flow,
        ToolCategory::Monitor,
        ToolCategory::Dns,
        ToolCategory::Meta,
    ] {
        assert!(
            !registry.tools_by_category(Some(cat)).is_empty(),
            "{cat:?} 类为空"
        );
    }
}

#[test]
fn test_catalog_entry_tokens_under_budget() {
    // 两层架构 token 目标：entry 4 个 schema 合计 < 1000 token（~600 目标留余量）。
    let registry = default_registry();
    let names: Vec<String> = registry
        .entry_tools()
        .iter()
        .map(|t| t.name.clone())
        .collect();
    assert!(registry.total_tokens(&names) < 1000);
}

#[test]
fn test_proc_help_execute_with_default_registry() {
    let registry = default_registry();
    let docker_tools = proc::agent::tools::help::execute(&registry, Some(ToolCategory::Docker));
    assert_eq!(docker_tools.len(), 10);
    assert!(
        docker_tools
            .iter()
            .all(|t| t.category == ToolCategory::Docker)
    );
    let all = proc::agent::tools::help::execute(&registry, None);
    assert_eq!(all.len(), 43);
}

// ---------- 任务 3：MockProvider fixture 回放 ----------

fn fixture_files(levels: &[&str]) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir("tests/fixtures/agent")
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "jsonl")
                && levels.iter().any(|l| {
                    p.file_stem()
                        .is_some_and(|s| s.to_string_lossy().ends_with(&format!("-l{l}")))
                })
        })
        .collect();
    files.sort();
    files
}

fn load_entries(files: &[PathBuf]) -> Vec<FixtureEntry> {
    let mut entries = Vec::new();
    for file in files {
        for line in std::fs::read_to_string(file).unwrap().lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            entries.push(serde_json::from_str(line).unwrap());
        }
    }
    entries
}

/// brainstorm 测试矩阵对齐：L0 23 + L1 27 共 50 query 全部确定性回放（零 LLM 调用）。
#[test]
fn test_agent_mock_provider_replay_50_queries() {
    let files = fixture_files(&["0", "1"]);
    let entries = load_entries(&files);
    assert_eq!(entries.len(), 50, "L0+L1 seed fixture 应为 50 query");

    // stage 3b 注记：seed 已被真实 E2B 录制覆盖。真实响应绝大多数是结构化
    // tool call，个别 query 会退化为伪 tool-call 文本（Text-only + EndTurn）。
    // 回放契约按 v0.20-fixtures.md 判定：complete 不 Err + 响应非空（tool_calls
    // 或 content 至少其一）。退化率是 agent loop 验收（stage 3b
    // test_agent_stage3b_acceptance）的度量项，不在本回放测试重复断言。
    let provider = MockProvider::new("tests/fixtures/agent".into());
    rt().block_on(async {
        for entry in &entries {
            let messages = vec![Message::new(Role::User, entry.query.clone())];
            let resp = provider
                .complete(messages, Vec::new(), CompleteOptions::default())
                .await
                .unwrap_or_else(|e| panic!("query {:?} 回放失败: {e}", entry.query));
            assert_eq!(resp.message.role, Role::Assistant);
            assert!(
                !resp.message.tool_calls.is_empty()
                    || resp
                        .message
                        .content
                        .as_deref()
                        .is_some_and(|c| !c.is_empty()),
                "query {:?} 回放响应为空",
                entry.query
            );
        }
    });
}

#[test]
fn test_mock_provider_stream_yields_recorded_deltas() {
    let provider = MockProvider::new("tests/fixtures/agent".into());
    let messages = vec![Message::new(Role::User, "当前系统 CPU 和内存使用率是多少")];
    let stream = provider.stream(messages, Vec::new(), CompleteOptions::default());
    let count = rt().block_on(async {
        futures_util::StreamExt::fold(stream, 0usize, |n, item| async move {
            assert!(item.is_ok());
            n + 1
        })
        .await
    });
    // 非空校验：至少 ToolCall + EndTurn 两条。
    assert!(count >= 2);
}

#[test]
fn test_mock_provider_missing_fixture_returns_config_error() {
    let provider = MockProvider::new("tests/fixtures/agent".into());
    let messages = vec![Message::new(Role::User, "这条 query 没有任何 fixture ⚡")];
    let err = rt()
        .block_on(provider.complete(messages, Vec::new(), CompleteOptions::default()))
        .unwrap_err();
    assert!(matches!(err, LlmError::Config(ref msg) if msg.contains("fixture 缺失")));
}

#[test]
fn test_query_hash_stable_16_hex() {
    let h1 = query_hash("我电脑为什么这么卡？");
    let h2 = query_hash("我电脑为什么这么卡？");
    assert_eq!(h1, h2);
    assert_eq!(h1.len(), 16);
    assert!(h1.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(h1, query_hash("另一条 query"));
}

// ---------- 任务 3：record_fixture round-trip ----------

struct EchoProvider;

#[async_trait]
impl LlmProvider for EchoProvider {
    fn name(&self) -> &'static str {
        "echo"
    }

    async fn complete(
        &self,
        _messages: Vec<Message>,
        _tools: Vec<ToolSchema>,
        _options: CompleteOptions,
    ) -> Result<CompleteResponse, LlmError> {
        Ok(CompleteResponse {
            message: Message {
                role: Role::Assistant,
                content: Some("echo 回显".to_string()),
                tool_calls: vec![ToolCall {
                    id: "c1".to_string(),
                    name: "proc_ls".to_string(),
                    arguments: serde_json::json!({"sort": "cpu", "limit": 5}),
                }],
                tool_results: Vec::new(),
            },
            stop_reason: StopReason::ToolUse,
            usage: Default::default(),
        })
    }

    fn stream(
        &self,
        _messages: Vec<Message>,
        _tools: Vec<ToolSchema>,
        _options: CompleteOptions,
    ) -> ProviderStream<'static> {
        unimplemented!("EchoProvider 仅用于 record_fixture 测试")
    }
}

#[test]
fn test_record_fixture_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let mut recorder = proc::agent::record_fixture::FixtureRecorder::new(
        Box::new(EchoProvider),
        dir.path().to_path_buf(),
    );
    let queries = vec![proc::agent::record_fixture::FixtureQuery {
        scenario: "echo-test",
        level: 0,
        query: "回环录制测试 query".to_string(),
    }];
    let report = rt().block_on(recorder.record_all(&queries)).unwrap();
    assert_eq!(report.recorded, 1);
    assert!(report.failed.is_empty());

    // 录制产物可被 MockProvider 直接回放（recorded query_hash 与现算一致）。
    let provider = MockProvider::new(dir.path().to_path_buf());
    let messages = vec![Message::new(Role::User, "回环录制测试 query")];
    let resp = rt()
        .block_on(provider.complete(messages, Vec::new(), CompleteOptions::default()))
        .unwrap();
    assert_eq!(resp.message.tool_calls[0].name, "proc_ls");
    assert_eq!(resp.stop_reason, StopReason::ToolUse);
}

// ---------- 任务 4：GGUF parser + ModelRegistry ----------

enum GgufVal {
    Str(&'static str),
    U32(u32),
    StrArray(&'static [&'static str]),
}

fn gguf_bytes(kvs: &[(&str, GgufVal)]) -> Vec<u8> {
    fn put_str(b: &mut Vec<u8>, s: &str) {
        b.extend_from_slice(&(s.len() as u64).to_le_bytes());
        b.extend_from_slice(s.as_bytes());
    }
    let mut b = Vec::new();
    b.extend_from_slice(b"GGUF");
    b.extend_from_slice(&3u32.to_le_bytes()); // version
    b.extend_from_slice(&0u64.to_le_bytes()); // tensor_count
    b.extend_from_slice(&(kvs.len() as u64).to_le_bytes());
    for (key, val) in kvs {
        put_str(&mut b, key);
        match val {
            GgufVal::Str(s) => {
                b.extend_from_slice(&8u32.to_le_bytes()); // String
                put_str(&mut b, s);
            }
            GgufVal::U32(v) => {
                b.extend_from_slice(&4u32.to_le_bytes()); // Uint32
                b.extend_from_slice(&v.to_le_bytes());
            }
            GgufVal::StrArray(items) => {
                b.extend_from_slice(&9u32.to_le_bytes()); // Array
                b.extend_from_slice(&8u32.to_le_bytes()); // 元素类型 String
                b.extend_from_slice(&(items.len() as u64).to_le_bytes());
                for s in *items {
                    put_str(&mut b, s);
                }
            }
        }
    }
    b
}

fn write_gguf(dir: &std::path::Path, filename: &str, kvs: &[(&str, GgufVal)]) -> PathBuf {
    let path = dir.join(filename);
    std::fs::write(&path, gguf_bytes(kvs)).unwrap();
    path
}

#[test]
fn test_read_gguf_metadata_extracts_kv_strings() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_gguf(
        dir.path(),
        "test-model-Q4_K_M.gguf",
        &[
            ("general.architecture", GgufVal::Str("gemma")),
            ("general.name", GgufVal::Str("Gemma 4 E2B")),
            ("tokenizer.ggml.model", GgufVal::Str("gemma")),
            ("general.file_type", GgufVal::U32(15)),
            (
                "tokenizer.ggml.tokens",
                GgufVal::StrArray(&["<unk>", "<s>", "</s>"]),
            ),
        ],
    );
    let meta = proc::agent::gguf_scan::read_gguf_metadata(&path).unwrap();
    assert_eq!(meta.general_name.as_deref(), Some("Gemma 4 E2B"));
    assert_eq!(meta.general_architecture.as_deref(), Some("gemma"));
    assert_eq!(meta.tokenizer_model.as_deref(), Some("gemma"));
}

#[test]
fn test_read_gguf_metadata_rejects_bad_magic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fake.gguf");
    std::fs::write(&path, b"NOPE....").unwrap();
    let err = proc::agent::gguf_scan::read_gguf_metadata(&path).unwrap_err();
    assert!(err.to_string().contains("bad magic"));
    assert!(!proc::agent::gguf_scan::is_gguf_file(&path));
}

#[test]
fn test_is_gguf_file_sniffs_magic_without_extension() {
    let dir = tempfile::tempdir().unwrap();
    // ollama blobs 场景：无 .gguf 扩展名。
    let blob = write_gguf(
        dir.path(),
        "sha256-abc123",
        &[("general.name", GgufVal::Str("blob-model"))],
    );
    assert!(proc::agent::gguf_scan::is_gguf_file(&blob));
    let plain = dir.path().join("plain.txt");
    std::fs::write(&plain, b"hello world").unwrap();
    assert!(!proc::agent::gguf_scan::is_gguf_file(&plain));
}

#[test]
fn test_model_registry_scan_finds_nested_gguf() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("model--org").join("snapshots").join("hash");
    std::fs::create_dir_all(&nested).unwrap();
    write_gguf(
        &nested,
        "gemma-4-E2B-it-Q4_K_M.gguf",
        &[
            ("general.name", GgufVal::Str("Gemma 4 E2B")),
            ("general.architecture", GgufVal::Str("gemma")),
        ],
    );
    write_gguf(
        dir.path(),
        "qwen-Q8_0.gguf",
        &[("general.name", GgufVal::Str("Qwen"))],
    );
    // 噪声文件：非 GGUF 内容。
    std::fs::write(dir.path().join("notes.gguf"), b"not really").unwrap();
    // 坏 GGUF（magic 对但版本异常）：保留 Error 条目。
    let mut bad = Vec::new();
    bad.extend_from_slice(b"GGUF");
    bad.extend_from_slice(&99u32.to_le_bytes());
    std::fs::write(dir.path().join("bad.gguf"), bad).unwrap();

    let mut registry = ModelRegistry::new();
    registry
        .scan(&[dir.path().to_string_lossy().into_owned()])
        .unwrap();
    let models = registry.models();
    assert_eq!(models.len(), 3); // 2 个可用 + 1 个 Error
    let available: Vec<_> = models
        .iter()
        .filter(|m| m.status == ModelStatus::Available)
        .collect();
    assert_eq!(available.len(), 2);
    let gemma = registry.get_by_name("Gemma 4 E2B").unwrap();
    assert_eq!(gemma.quantization.as_deref(), Some("Q4_K_M"));
    assert_eq!(gemma.architecture.as_deref(), Some("gemma"));
    // refresh 重扫幂等。
    registry
        .refresh(&[dir.path().to_string_lossy().into_owned()])
        .unwrap();
    assert_eq!(registry.models().len(), 3);
}

#[test]
fn test_model_registry_scan_expands_env_placeholder() {
    let dir = tempfile::tempdir().unwrap();
    write_gguf(
        dir.path(),
        "m-F16.gguf",
        &[("general.name", GgufVal::Str("F16Model"))],
    );
    // %TEMP% 展开到真实系统临时目录。
    let extra = format!(
        "%TEMP%\\{}",
        dir.path().file_name().unwrap().to_string_lossy()
    );
    let mut registry = ModelRegistry::new();
    registry.scan(&[extra]).unwrap();
    assert!(registry.get_by_name("F16Model").is_some());
}

#[test]
fn test_quant_from_filename_edge_cases() {
    use proc::agent::gguf_scan::quant_from_filename;
    assert_eq!(
        quant_from_filename("gemma-4-E2B-it-Q4_K_M.gguf").as_deref(),
        Some("Q4_K_M")
    );
    assert_eq!(quant_from_filename("m-Q8_0.gguf").as_deref(), Some("Q8_0"));
    assert_eq!(
        quant_from_filename("m-IQ4_XS.gguf").as_deref(),
        Some("IQ4_XS")
    );
    assert_eq!(quant_from_filename("m-F16.gguf").as_deref(), Some("F16"));
    assert_eq!(quant_from_filename("m-BF16.gguf").as_deref(), Some("BF16"));
    assert_eq!(quant_from_filename("no-quant.gguf"), None);
    assert_eq!(quant_from_filename("not-gguf.txt"), None);
}

// ---------- 任务 4：agent.toml / catalog 杂项 ----------

#[test]
fn test_agent_config_search_paths_parse() {
    let content = r#"
[llama-cpp]
search_paths = ["D:\\models", "${TEMP}\\sub"]
"#;
    let config = proc::agent::AgentConfig::from_toml(content).unwrap();
    assert_eq!(config.llama_cpp.search_paths.len(), 2);
    assert_eq!(config.llama_cpp.search_paths[0], r"D:\models");
}

#[test]
fn test_registry_catalog_names_unique() {
    let registry: ToolRegistry = default_registry();
    // HashMap key 即 name，47 = 无重名（重名会互相覆盖导致 len < 47）。
    assert_eq!(registry.len(), 47);
}
