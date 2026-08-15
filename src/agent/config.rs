//! `~/.config/proc/agent.toml` 配置解析（ADR-0030，brainstorm 决策 9 拍板）。
//!
//! 配置优先级（高 → 低）：CLI flag（`--provider` / `--model` / `--max-steps`）→
//! 配置文件 → 代码默认值（隐私架构：本地 llama-cpp + Gemma 4 E2B）。
//! 文件缺失 / 解析失败 → 静默降级到代码默认值（与 `ui.toml` 同款契约）。

use serde::Deserialize;

/// agent 配置根（agent.toml 多 section）。
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    #[serde(default)]
    pub default: DefaultConfig,
    /// TOML 段名是 `[llama-cpp]`（连字符），字段名是 snake_case。
    #[serde(default, rename = "llama-cpp")]
    pub llama_cpp: LlamaCppConfig,
    #[serde(default)]
    pub anthropic: AnthropicConfig,
    #[serde(default)]
    pub mock: MockConfig,
}

/// `[default]` — 用户长期偏好（provider / 模型 / loop 深度）。
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct DefaultConfig {
    /// `llama-cpp` / `anthropic` / `mock`
    pub provider: Option<String>,
    /// llama-cpp 路径 = GGUF 文件名 / anthropic 路径 = 模型 ID
    pub model: Option<String>,
    pub max_steps: Option<u32>,
}

/// `[llama-cpp]` — llama-server 启动参数。
///
/// 手写 Default（`no_thinks: true`）：derive Default 会把 bool 置 false，
/// 与 ADR-0030 D6「thinking mode 强制禁用默认开」冲突。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlamaCppConfig {
    pub server_path: Option<String>,
    #[serde(default)]
    pub search_paths: Vec<String>,
    pub ctx_size: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    /// 强制禁用 Gemma 4 thinking mode（ADR-0030 D6，默认开）。
    #[serde(default = "default_true")]
    pub no_thinks: bool,
    /// GBNF grammar 名（嵌入 binary，决策 8）。
    #[serde(default)]
    pub grammar_file: Option<String>,
}

/// `[anthropic]` — 云端对照路径（opt-in feature）。
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AnthropicConfig {
    pub model: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    // API key 从 ANTHROPIC_API_KEY env 读，不写配置文件（避免泄漏）。
}

/// `[mock]` — fixture 回放配置。
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct MockConfig {
    pub fixtures_dir: Option<String>,
}

fn default_true() -> bool {
    true
}

impl Default for LlamaCppConfig {
    fn default() -> Self {
        Self {
            server_path: None,
            search_paths: Vec::new(),
            ctx_size: None,
            temperature: None,
            top_p: None,
            top_k: None,
            no_thinks: true,
            grammar_file: None,
        }
    }
}

impl AgentConfig {
    /// 读 `~/.config/proc/agent.toml`；不存在 / 解析失败 → 代码默认值。
    pub fn load() -> Self {
        let path = crate::dirs_config_dir().join("agent.toml");
        match std::fs::read_to_string(&path) {
            Ok(content) => Self::from_toml(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn from_toml(content: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(content)
    }
}
