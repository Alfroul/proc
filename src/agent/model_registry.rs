//! ModelRegistry — 本地 GGUF 模型扫描结果缓存 + 索引（stage 1 骨架，
//! stage 2 实装扫描 + `proc agent models` CLI 输出）。

use std::collections::HashMap;
use std::path::PathBuf;

/// 单个检测到的本地模型。
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub path: PathBuf,
    /// GGUF KV `general.name`（解析失败时 fallback 文件名）
    pub name: String,
    /// GGUF KV `general.architecture`（如 `gemma`）
    pub architecture: Option<String>,
    pub size_bytes: u64,
    /// 量化方式（从文件名提取，如 `Q4_K_M`）
    pub quantization: Option<String>,
    /// available / loading / error
    pub status: ModelStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelStatus {
    Available,
    Loading,
    Error,
}

/// 扫描结果缓存 + 按 path / name / size 索引。
#[derive(Debug, Default)]
pub struct ModelRegistry {
    models: Vec<ModelInfo>,
    by_name: HashMap<String, usize>,
}

impl ModelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 扫描默认路径 + agent.toml `[llama-cpp].search_paths`（stage 2 实装）。
    pub fn scan(&mut self) -> Result<(), std::io::Error> {
        todo!("v0.20 stage 2 落地 GGUF 扫描")
    }

    /// 强制重扫（`proc agent models --refresh`）。
    pub fn refresh(&mut self) -> Result<(), std::io::Error> {
        self.scan()
    }

    pub fn models(&self) -> &[ModelInfo] {
        &self.models
    }

    pub fn get_by_name(&self, name: &str) -> Option<&ModelInfo> {
        self.by_name.get(name).map(|&i| &self.models[i])
    }
}
