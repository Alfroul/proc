//! GGUF scanner — 扫描本地模型目录 + 解析 GGUF KV metadata（stage 1 骨架，
//! stage 2 实装 `gguf` crate 集成 + 默认扫描路径遍历）。

use std::path::PathBuf;

/// 默认扫描路径（brainstorm 项 3）：
/// - `D:\llama.cpp\models\`（用户机器 llama.cpp 模型目录）
/// - `%USERPROFILE%\.ollama\models\blobs\`
/// - `%USERPROFILE%\.cache\huggingface\hub\`
/// - `${LOCALAPPDATA}\llama.cpp\models\`
///
/// 额外路径由 agent.toml `[llama-cpp].search_paths` 追加（`%VAR%` 占位符展开）。
pub fn default_scan_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    paths.push(PathBuf::from(r"D:\llama.cpp\models"));
    if let Some(home) = std::env::var_os("USERPROFILE") {
        let home = PathBuf::from(home);
        paths.push(home.join(r".ollama\models\blobs"));
        paths.push(home.join(r".cache\huggingface\hub"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        paths.push(PathBuf::from(local).join(r"llama.cpp\models"));
    }
    paths
}

/// 从 GGUF 文件名提取量化方式（如 `gemma-4-E2B-it-Q4_K_M.gguf` → `Q4_K_M`）。
pub fn quant_from_filename(filename: &str) -> Option<String> {
    let stem = filename.strip_suffix(".gguf")?;
    // 量化 tag 形如 IQ4_XS / Q2_K / Q8_0 / F16 / BF16，位于最后一个 '-' 后。
    stem.rsplit('-')
        .next()
        .filter(|tag| {
            let upper = tag.to_ascii_uppercase();
            upper.starts_with('Q')
                || upper.starts_with("IQ")
                || upper.starts_with('F')
                || upper.starts_with("BF")
        })
        .map(|tag| tag.to_ascii_uppercase())
}

/// 读 GGUF KV metadata（general.name / general.architecture）。
/// stage 2 实装 `gguf` crate 集成。
pub fn read_gguf_metadata(path: &std::path::Path) -> Result<GgufMetadata, std::io::Error> {
    let _ = path;
    todo!("v0.20 stage 2 落地 gguf crate 集成")
}

/// GGUF KV metadata 的 proc 关心子集。
#[derive(Debug, Clone, Default)]
pub struct GgufMetadata {
    pub general_name: Option<String>,
    pub general_architecture: Option<String>,
    pub tokenizer_model: Option<String>,
}
