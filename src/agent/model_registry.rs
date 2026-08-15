//! ModelRegistry — 本地 GGUF 模型扫描结果缓存 + 索引。
//!
//! 扫描 `default_scan_paths()` + agent.toml `[llama-cpp].search_paths`（`%VAR%`
//! 占位符展开），目录递归（深度 ≤ 6），magic 嗅探识别（ollama blobs 目录的
//! 文件无 .gguf 扩展名）。metadata 解析失败的文件保留条目（status = Error，
//! name fallback 文件名），不阻塞整体扫描。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::gguf_scan::{is_gguf_file, quant_from_filename, read_gguf_metadata};

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

/// 扫描目录递归深度上限（huggingface hub 快照嵌套 ~5 层）。
const MAX_SCAN_DEPTH: usize = 6;

impl ModelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 扫描给定路径（支持 `%VAR%` / `${VAR}` / `$VAR` 占位符展开）。**不合并
    /// 默认路径**——调用方（CLI）组装 `default_scan_paths()` +
    /// agent.toml `[llama-cpp].search_paths`，让本方法可测试（不依赖用户
    /// 机器真实模型目录）。单目录不存在 / 不可读 → 静默跳过。
    pub fn scan(&mut self, paths: &[String]) -> Result<(), std::io::Error> {
        self.models.clear();
        self.by_name.clear();
        for raw in paths {
            let expanded = crate::security::path_rules::expand_env_placeholders(raw);
            self.scan_dir(&PathBuf::from(expanded), 0);
        }
        Ok(())
    }

    /// 强制重扫（`proc agent models --refresh`）。
    pub fn refresh(&mut self, paths: &[String]) -> Result<(), std::io::Error> {
        self.scan(paths)
    }

    fn scan_dir(&mut self, dir: &Path, depth: usize) {
        if depth > MAX_SCAN_DEPTH {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if file_type.is_dir() {
                if name.starts_with('.') {
                    continue;
                }
                self.scan_dir(&path, depth + 1);
            } else if file_type.is_file() && is_gguf_file(&path) {
                self.add_model(&path);
            }
        }
    }

    fn add_model(&mut self, path: &Path) {
        let size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let quantization = quant_from_filename(&filename);
        let (info, status) = match read_gguf_metadata(path) {
            Ok(meta) => {
                let name = meta.general_name.unwrap_or_else(|| filename.clone());
                (
                    ModelInfo {
                        path: path.to_path_buf(),
                        name,
                        architecture: meta.general_architecture,
                        size_bytes,
                        quantization,
                        status: ModelStatus::Available,
                    },
                    None,
                )
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "GGUF metadata 解析失败");
                (
                    ModelInfo {
                        path: path.to_path_buf(),
                        name: filename,
                        architecture: None,
                        size_bytes,
                        quantization,
                        status: ModelStatus::Error,
                    },
                    Some(e.to_string()),
                )
            }
        };
        if status.is_some() {
            // Error 条目仍保留（CLI 标红提示），但不算入 by_name 索引。
            self.models.push(info);
            return;
        }
        let idx = self.models.len();
        self.models.push(info);
        let name = self.models[idx].name.clone();
        self.by_name.entry(name).or_insert(idx);
    }

    pub fn models(&self) -> &[ModelInfo] {
        &self.models
    }

    pub fn get_by_name(&self, name: &str) -> Option<&ModelInfo> {
        self.by_name.get(name).map(|&i| &self.models[i])
    }
}
