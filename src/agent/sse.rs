//! SSE `data:` 帧分帧器（provider 无关，stage 4 从 llama_cpp_provider 抽共享）。
//!
//! LlamaCppProvider（OpenAI 协议）与 AnthropicProvider（Messages API）的流式
//! 响应都是 SSE 格式；`event:` 行忽略、事件类型从 data JSON 的 `type` 字段
//! 判别（Anthropic）或 payload 形状判别（OpenAI）。不 feature-gate——
//! anthropic-only build（`--no-default-features --features anthropic`）也可用。

/// SSE `data:` 帧提取器：feed 字节块，按空行分帧返回 data payload。
///
/// 跨 chunk 半行安全（未完整的帧留在缓冲）；`event:` / 注释行忽略；
/// 多行 `data:` 按规范以 `\n` 连接。`data: [DONE]` 哨兵原样返回，调用方判定。
#[derive(Default)]
pub struct SseFrameBuffer {
    buf: Vec<u8>,
}

impl SseFrameBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(chunk);
        let mut frames = Vec::new();
        while let Some(end) = find_frame_end(&self.buf) {
            let frame: Vec<u8> = self.buf.drain(..end).collect();
            if let Some(payload) = extract_data_payload(&frame) {
                frames.push(payload);
            }
        }
        frames
    }
}

/// 返回下一帧结束位置（含 `\n\n` / `\r\n\r\n` 分隔符）。
fn find_frame_end(buf: &[u8]) -> Option<usize> {
    if buf.len() < 2 {
        return None;
    }
    for i in 0..buf.len() - 1 {
        if buf[i] == b'\n' && buf[i + 1] == b'\n' {
            return Some(i + 2);
        }
        if buf[i] == b'\r'
            && buf[i + 1] == b'\n'
            && buf.len() >= i + 4
            && buf[i + 2] == b'\r'
            && buf[i + 3] == b'\n'
        {
            return Some(i + 4);
        }
    }
    None
}

fn extract_data_payload(frame: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(frame);
    let mut data_lines: Vec<&str> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    if data_lines.is_empty() {
        None
    } else {
        Some(data_lines.join("\n"))
    }
}
