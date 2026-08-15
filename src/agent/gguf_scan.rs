//! GGUF scanner — 扫描本地模型目录 + 解析 GGUF KV metadata。
//!
//! 手写流式 parser（stage-2.md 决策 B）：stage 1 选的 `gguf` 0.1.2 crate 把
//! header + tensor info 段绑定解析且输入不完整时丢弃已解析 header，1.6GB 模型
//! 必须全量读入内存——不可接受，故弃用（Cargo.toml 已移除 dep），改手写。
//! GGUF v2/v3 的 header/metadata 编码是公开稳定规范：magic + version +
//! tensor_count + metadata_count + 逐条 KV；metadata 段结束即停，不读 tensor 段。

use std::fs::File;
use std::io;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

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
///
/// 量化 tag 形如 Q4_K_M / IQ4_XS / Q8_0 / F16 / BF16，位于最后一个 '-' 后；
/// 必须是 Q/IQ/F/BF 后紧跟数字（防 `no-quant.gguf` 这类普通词误判）。
pub fn quant_from_filename(filename: &str) -> Option<String> {
    fn digit(b: u8) -> bool {
        b.is_ascii_digit()
    }
    let stem = filename.strip_suffix(".gguf")?;
    stem.rsplit('-')
        .next()
        .filter(|tag| {
            let upper = tag.to_ascii_uppercase();
            match upper.as_bytes() {
                [b'Q', d, ..] if digit(*d) => true,
                [b'I', b'Q', d, ..] if digit(*d) => true,
                [b'F', d, ..] if digit(*d) => true,
                [b'B', b'F', d, ..] if digit(*d) => true,
                _ => false,
            }
        })
        .map(|tag| tag.to_ascii_uppercase())
}

/// 读 GGUF KV metadata（general.name / general.architecture /
/// tokenizer.ggml.model），流式顺序读、metadata 段结束即停。
pub fn read_gguf_metadata(path: &Path) -> io::Result<GgufMetadata> {
    let mut r = BufReader::with_capacity(64 * 1024, File::open(path)?);
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic)?;
    if &magic != b"GGUF" {
        return Err(invalid_data("not a GGUF file (bad magic)"));
    }
    let version = read_u32(&mut r)?;
    if !(1..=3).contains(&version) {
        return Err(invalid_data(&format!("unsupported GGUF version {version}")));
    }
    let _tensor_count = read_u64(&mut r)?;
    let metadata_count = read_u64(&mut r)?;
    let mut out = GgufMetadata::default();
    for _ in 0..metadata_count {
        let key = read_gguf_string(&mut r)?;
        let value_type = read_u32(&mut r)?;
        if let Some(value) = read_value(&mut r, value_type)? {
            match key.as_str() {
                "general.name" => out.general_name = Some(value),
                "general.architecture" => out.general_architecture = Some(value),
                "tokenizer.ggml.model" => out.tokenizer_model = Some(value),
                _ => {}
            }
        }
    }
    Ok(out)
}

/// magic 嗅探（ollama blobs 目录的文件无 .gguf 扩展名，统一按内容识别）。
pub fn is_gguf_file(path: &Path) -> bool {
    let Ok(mut f) = File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    f.read_exact(&mut magic).is_ok() && &magic == b"GGUF"
}

/// GGUF KV metadata 的 proc 关心子集。
#[derive(Debug, Clone, Default)]
pub struct GgufMetadata {
    pub general_name: Option<String>,
    pub general_architecture: Option<String>,
    pub tokenizer_model: Option<String>,
}

// ---- 二进制 reader helpers ----

fn invalid_data(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

fn read_u32(r: &mut impl Read) -> io::Result<u32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_u64(r: &mut impl Read) -> io::Result<u64> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf))
}

/// GGUF string：u64 长度前缀 + UTF-8 字节。
fn read_gguf_string(r: &mut impl Read) -> io::Result<String> {
    let len = read_u64(r)? as usize;
    // 防异常文件声称超长 string 导致一次性巨分配（上限 16 MiB）。
    if len > 16 * 1024 * 1024 {
        return Err(invalid_data("GGUF string 长度异常"));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    String::from_utf8(buf).map_err(|_| invalid_data("GGUF string 不是合法 UTF-8"))
}

/// 读一个 metadata value；String 类型返 `Some(值)` 供上层按 key 提取，
/// 其余类型消费（定长 seek-skip / array 递归）后返 `None`。
fn read_value(r: &mut BufReader<File>, value_type: u32) -> io::Result<Option<String>> {
    match value_type {
        // Uint8 / Int8 / Bool = 1 字节；Uint16 / Int16 = 2；Uint32 / Int32 /
        // Float32 = 4；Uint64 / Int64 / Float64 = 8。
        0 | 1 | 7 => skip(r, 1).map(|_| None),
        2 | 3 => skip(r, 2).map(|_| None),
        4..=6 => skip(r, 4).map(|_| None),
        10..=12 => skip(r, 8).map(|_| None),
        8 => read_gguf_string(r).map(Some),
        9 => {
            // Array: 元素类型 u32 + 元素个数 u64 + 逐元素递归。
            let elem_type = read_u32(r)?;
            let len = read_u64(r)?;
            for _ in 0..len {
                read_value(r, elem_type)?;
            }
            Ok(None)
        }
        t => Err(invalid_data(&format!("未知 GGUF metadata value type {t}"))),
    }
}

/// BufReader::seek(Current(n)) 相对「逻辑位置」跳过（std 已处理内部缓冲折算）。
fn skip(r: &mut BufReader<File>, n: u64) -> io::Result<()> {
    r.seek(SeekFrom::Current(n as i64))?;
    Ok(())
}
