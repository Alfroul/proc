use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Instant;

use bincode::Options;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use ratatui::widgets::Widget;
use serde::{Deserialize, Serialize};

use super::encoding::options_for_version;

pub const VT100_MAGIC: &[u8; 4] = b"VT10";
pub const VT100_VERSION: u16 = 2;

const MIN_CAPTURE_MS: u64 = 200; // 5 fps

// ── Header ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VtHeader {
    pub magic: [u8; 4],
    pub version: u16,
    pub start_time: u64,
    pub width: u16,
    pub height: u16,
}

// ── Compact cell ──

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CellDump {
    pub ch: u32,
    pub fg: u32,
    pub bg: u32,
    pub flags: u8,
}

// ── Frame ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VtFrame {
    pub timestamp_ms: u64,
    pub width: u16,
    pub height: u16,
    /// Run-length encoded: (repeat_count, cell)
    pub rle: Vec<(u16, CellDump)>,
}

impl VtFrame {
    #[must_use]
    pub fn from_buffer(buffer: &Buffer, area: Rect, timestamp_ms: u64) -> Self {
        let mut rle: Vec<(u16, CellDump)> = Vec::new();

        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let dump = match buffer.cell((x, y)) {
                    Some(cell) => CellDump {
                        ch: cell.symbol().chars().next().unwrap_or(' ') as u32,
                        fg: pack_color(cell.fg),
                        bg: pack_color(cell.bg),
                        flags: pack_modifier(cell.modifier),
                    },
                    None => CellDump::default(),
                };

                if let Some((count, prev)) = rle.last_mut()
                    && *prev == dump
                {
                    *count += 1;
                    continue;
                }
                rle.push((1, dump));
            }
        }

        VtFrame {
            timestamp_ms,
            width: area.width,
            height: area.height,
            rle,
        }
    }
}

// ── Widget for replay rendering ──

pub struct VtFrameWidget<'a> {
    frame: &'a VtFrame,
}

impl<'a> VtFrameWidget<'a> {
    #[must_use]
    pub fn new(frame: &'a VtFrame) -> Self {
        Self { frame }
    }
}

impl Widget for VtFrameWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut x: u16 = 0;
        let mut y: u16 = 0;
        let fw = self.frame.width;

        for (count, cell) in &self.frame.rle {
            for _ in 0..*count {
                if y < area.height
                    && x < area.width
                    && let Some(buf_cell) = buf.cell_mut((area.x + x, area.y + y))
                {
                    let ch = char::from_u32(cell.ch).unwrap_or(' ');
                    if ch == '\0' {
                        buf_cell.set_symbol(" ");
                    } else {
                        buf_cell.set_symbol(&ch.to_string());
                    }
                    buf_cell.set_fg(unpack_color(cell.fg));
                    buf_cell.set_bg(unpack_color(cell.bg));
                    buf_cell.modifier = unpack_modifier(cell.flags);
                }
                x += 1;
                if x >= fw {
                    x = 0;
                    y += 1;
                }
            }
        }
    }
}

// ── Color packing ──
//
// 32-bit variable encoding. Bit 31 = RGB marker.
//   - Palette mode (bit 31 = 0): low bits = palette index
//       0 = Reset, 1..=16 = basic 16 colors, 17..=272 = Indexed(u8)
//   - RGB mode     (bit 31 = 1): bits 16-23 = R, 8-15 = G, 0-7 = B

const RGB_MARKER: u32 = 0x8000_0000;

#[must_use]
pub fn pack_color(color: Color) -> u32 {
    match color {
        Color::Reset => 0,
        Color::Black => 1,
        Color::Red => 2,
        Color::Green => 3,
        Color::Yellow => 4,
        Color::Blue => 5,
        Color::Magenta => 6,
        Color::Cyan => 7,
        Color::Gray => 8,
        Color::DarkGray => 9,
        Color::LightRed => 10,
        Color::LightGreen => 11,
        Color::LightYellow => 12,
        Color::LightBlue => 13,
        Color::LightMagenta => 14,
        Color::LightCyan => 15,
        Color::White => 16,
        Color::Indexed(i) => 17 + (i as u32).min(255),
        Color::Rgb(r, g, b) => RGB_MARKER | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32),
    }
}

#[must_use]
pub fn unpack_color(packed: u32) -> Color {
    if packed & RGB_MARKER != 0 {
        let r = ((packed >> 16) & 0xFF) as u8;
        let g = ((packed >> 8) & 0xFF) as u8;
        let b = (packed & 0xFF) as u8;
        Color::Rgb(r, g, b)
    } else {
        match packed {
            0 => Color::Reset,
            1 => Color::Black,
            2 => Color::Red,
            3 => Color::Green,
            4 => Color::Yellow,
            5 => Color::Blue,
            6 => Color::Magenta,
            7 => Color::Cyan,
            8 => Color::Gray,
            9 => Color::DarkGray,
            10 => Color::LightRed,
            11 => Color::LightGreen,
            12 => Color::LightYellow,
            13 => Color::LightBlue,
            14 => Color::LightMagenta,
            15 => Color::LightCyan,
            16 => Color::White,
            i => Color::Indexed((i - 17).min(255) as u8),
        }
    }
}

fn pack_modifier(modifier: Modifier) -> u8 {
    let mut bits = 0u8;
    if modifier.contains(Modifier::BOLD) {
        bits |= 1;
    }
    if modifier.contains(Modifier::ITALIC) {
        bits |= 2;
    }
    if modifier.contains(Modifier::UNDERLINED) {
        bits |= 4;
    }
    if modifier.contains(Modifier::REVERSED) {
        bits |= 8;
    }
    bits
}

#[must_use]
pub fn unpack_modifier(bits: u8) -> Modifier {
    let mut m = Modifier::empty();
    if bits & 1 != 0 {
        m |= Modifier::BOLD;
    }
    if bits & 2 != 0 {
        m |= Modifier::ITALIC;
    }
    if bits & 4 != 0 {
        m |= Modifier::UNDERLINED;
    }
    if bits & 8 != 0 {
        m |= Modifier::REVERSED;
    }
    m
}

// ── Recorder ──

enum RecorderMsg {
    Frame(VtFrame),
    Stop,
}

pub struct VtRecorder {
    tx: mpsc::Sender<RecorderMsg>,
    thread: Option<std::thread::JoinHandle<()>>,
    start_time: Instant,
    last_capture: Instant,
    path: PathBuf,
    stopped: bool,
    /// v0.14 stage 2：成功 capture 的帧数（writer 线程 fetch_add 后主线程 load）。
    /// 让录制中按 `b` 添加书签能拿到当前帧索引。
    frame_count: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl VtRecorder {
    pub fn start(path: PathBuf, width: u16, height: u16) -> anyhow::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }

        let header = VtHeader {
            magic: *VT100_MAGIC,
            version: VT100_VERSION,
            start_time: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs(),
            width,
            height,
        };

        let mut file = BufWriter::new(File::create(&path)?);
        let header_bytes = options_for_version(VT100_VERSION).serialize(&header)?;
        file.write_all(&(header_bytes.len() as u64).to_le_bytes())?;
        file.write_all(&header_bytes)?;
        file.flush()?;

        let (tx, rx) = mpsc::channel::<RecorderMsg>();

        // v0.14 stage 2：capture 计数共享给主线程，让录制中按 `b` 能拿到当前帧索引。
        let frame_count = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let frame_count_writer = std::sync::Arc::clone(&frame_count);

        let thread = std::thread::Builder::new()
            .name("vt-recorder".to_string())
            .spawn(move || {
                let mut file = file;
                while let Ok(msg) = rx.recv() {
                    match msg {
                        RecorderMsg::Frame(frame) => {
                            if let Ok(bytes) = options_for_version(VT100_VERSION).serialize(&frame)
                            {
                                let _ = file.write_all(&(bytes.len() as u64).to_le_bytes());
                                let _ = file.write_all(&bytes);
                                let _ = file.flush();
                                // v0.14 stage 2：每写一帧 fetch_add（让主线程能 load 出当前帧数）
                                frame_count_writer
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                        RecorderMsg::Stop => {
                            let _ = file.flush();
                            break;
                        }
                    }
                }
            })?;

        Ok(Self {
            tx,
            thread: Some(thread),
            start_time: Instant::now(),
            last_capture: Instant::now() - std::time::Duration::from_millis(MIN_CAPTURE_MS),
            path,
            stopped: false,
            frame_count,
        })
    }

    pub fn try_capture(&mut self, buffer: &Buffer, area: Rect) {
        if self.last_capture.elapsed().as_millis() < MIN_CAPTURE_MS as u128 {
            return;
        }
        let ts = self.start_time.elapsed().as_millis() as u64;
        let frame = VtFrame::from_buffer(buffer, area, ts);
        let _ = self.tx.send(RecorderMsg::Frame(frame));
        self.last_capture = Instant::now();
    }

    /// v0.14 stage 2：成功写出的帧数（writer 线程 fetch_add 后主线程 load）。
    /// 让录制中按 `b` 添加书签能拿到当前帧索引。
    #[must_use]
    pub fn frame_count(&self) -> usize {
        self.frame_count.load(std::sync::atomic::Ordering::Relaxed) as usize
    }

    pub fn stop(mut self) -> anyhow::Result<PathBuf> {
        self.stop_internal();
        // Clone the path so we don't move out of `self` (Drop needs an intact
        // value to destruct). The path is short and a single allocation, so
        // the clone is negligible compared to the file join below.
        Ok(self.path.clone())
    }

    fn stop_internal(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        let _ = self.tx.send(RecorderMsg::Stop);
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
    }

    #[must_use]
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    #[must_use]
    pub fn elapsed_secs(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }
}

impl Drop for VtRecorder {
    fn drop(&mut self) {
        // If `stop()` was already called this is a no-op. Otherwise flush
        // any buffered frames via Stop + join so we never leak a thread
        // or lose the final frames when the caller drops without stopping.
        self.stop_internal();
    }
}

// ── Player ──

pub struct VtPlayer {
    #[allow(dead_code)]
    path: PathBuf,
    header: VtHeader,
    frames: Vec<VtFrame>,
}

impl VtPlayer {
    pub fn open(path: PathBuf) -> anyhow::Result<Self> {
        let file = File::open(&path)?;
        let mut reader = BufReader::new(file);

        let mut len_buf = [0u8; 8];
        reader.read_exact(&mut len_buf)?;
        let header_len = u64::from_le_bytes(len_buf) as usize;
        let mut header_buf = vec![0u8; header_len];
        reader.read_exact(&mut header_buf)?;
        let header: VtHeader = options_for_version(VT100_VERSION).deserialize(&header_buf)?;

        if &header.magic != VT100_MAGIC {
            anyhow::bail!("无效的 VT100 录制文件");
        }

        if header.version != VT100_VERSION {
            anyhow::bail!(
                "此录制使用旧版格式（v{}），需用旧版本回放。当前版本：v{}",
                header.version,
                VT100_VERSION
            );
        }

        let mut frames = Vec::new();
        loop {
            let mut len_buf = [0u8; 8];
            if reader.read_exact(&mut len_buf).is_err() {
                break;
            }
            let frame_len = u64::from_le_bytes(len_buf) as usize;
            let mut frame_buf = vec![0u8; frame_len];
            if reader.read_exact(&mut frame_buf).is_err() {
                break;
            }
            // 每次创建 opts（impl Options 不是 Copy，不能跨 loop 持有）
            if let Ok(frame) =
                options_for_version(header.version).deserialize::<VtFrame>(&frame_buf)
            {
                frames.push(frame);
            }
        }

        Ok(Self {
            path,
            header,
            frames,
        })
    }

    #[must_use]
    pub fn total_frames(&self) -> usize {
        self.frames.len()
    }

    #[must_use]
    pub fn frame_at(&self, index: usize) -> Option<&VtFrame> {
        self.frames.get(index)
    }

    #[must_use]
    pub fn time_range_ms(&self) -> (u64, u64) {
        if self.frames.is_empty() {
            (0, 0)
        } else {
            (
                self.frames.first().unwrap().timestamp_ms,
                self.frames.last().unwrap().timestamp_ms,
            )
        }
    }

    #[must_use]
    pub fn width(&self) -> u16 {
        self.header.width
    }

    #[must_use]
    pub fn height(&self) -> u16 {
        self.header.height
    }

    #[must_use]
    pub fn header(&self) -> &VtHeader {
        &self.header
    }
}

// ── Detect VT100 format ──

#[must_use]
pub fn is_vt100_file(path: &std::path::Path) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    let mut reader = BufReader::new(file);
    let mut len_buf = [0u8; 8];
    if reader.read_exact(&mut len_buf).is_err() {
        return false;
    }
    let header_len = u64::from_le_bytes(len_buf) as usize;
    if header_len > 1024 {
        return false;
    }
    let mut header_buf = vec![0u8; header_len];
    if reader.read_exact(&mut header_buf).is_err() {
        return false;
    }
    let Ok(header) = options_for_version(VT100_VERSION).deserialize::<VtHeader>(&header_buf) else {
        return false;
    };
    &header.magic == VT100_MAGIC
}

// ── v0.17 stage 5：VT100 → UiFrame 转码辅助 ──

/// 把 RLE cells 按行展开为字符串向量（每行一个 String）。
///
/// 给 [`extract_process_names_from_rle`] 用 —— 把 VtFrame 的 rle: Vec<(u16, CellDump)>
/// 还原为屏幕文本（按 width 换行），让后续 regex 提取按行处理。
///
/// # Parameters
///
/// - `rle`：VtFrame.rle 字段（run-length encoded cells）
/// - `width`：VtFrame.width 字段（屏幕列数）
///
/// # Returns
///
/// `Vec<String>` —— 每行一个 String（无 trailing newline），行数 = ceil(total_cells / width)。
/// 空字符（`\0`）按空格处理（与 VtFrameWidget::render 一致，src/record/vt100.rs:115）。
fn rle_to_lines(rle: &[(u16, CellDump)], width: u16) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let width = width as usize;
    let mut lines: Vec<String> = Vec::new();
    let mut current_line = String::new();
    let mut col: usize = 0;

    for (count, cell) in rle {
        for _ in 0..*count {
            let ch = char::from_u32(cell.ch).unwrap_or(' ');
            let push_ch = if ch == '\0' { ' ' } else { ch };
            current_line.push(push_ch);
            col += 1;
            if col >= width {
                lines.push(std::mem::take(&mut current_line));
                col = 0;
            }
        }
    }
    if col > 0 {
        lines.push(current_line);
    }
    lines
}

/// 从屏幕文本提取进程名候选（Windows .exe / 通用单词 pattern）。
///
/// v0.17 stage 5 落地（ADR-0028 决策 3）—— VT100 录屏的屏幕文本是终端渲染结果，
/// 无法精确还原 ProcessInfo 全字段，但能识别「name.exe」/「name」等单词作为
/// search `name =~ /<pattern>/` 命中线索。pid/cpu/memory 字段填占位值
/// （pid=0, cpu=0.0, memory=0, disk_read=0, disk_write=0）。
///
/// **提取规则**：
/// 1. 优先匹配 `\b[\w\-\.]+\.exe\b`（Windows 可执行名，如 `chrome.exe` / `code.exe`）
/// 2. fallback 匹配 `\b[\w\-]{3,30}\b`（通用单词，排除太短 / 太长噪声，仅当 .exe
///    命中数 < 3 时启用，避免单帧噪声过多）
/// 3. 同帧内 dedup（按 lowercase 归一，与 v0.6 `name_lower` 字段策略一致）
///
/// # Parameters
///
/// - `rle`：VtFrame.rle 字段
/// - `width`：VtFrame.width 字段
///
/// # Returns
///
/// `Vec<FrameProcess>` —— 去重后的进程列表（每个含 name + 占位字段）。
/// 顺序按首次出现顺序保留（与 v0.6 process list 默认顺序一致）。
#[must_use]
pub fn extract_process_names_from_rle(
    rle: &[(u16, CellDump)],
    width: u16,
) -> Vec<super::frame::FrameProcess> {
    let lines = rle_to_lines(rle, width);

    let exe_re = regex::Regex::new(r"(?i)\b[\w\-\.]+\.exe\b").expect("static regex");
    let word_re = regex::Regex::new(r"\b[\w\-]{3,30}\b").expect("static regex");

    // 第一遍：收 .exe 命中（lowercase 归一 dedup）
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut ordered: Vec<String> = Vec::new();
    for line in &lines {
        for m in exe_re.find_iter(line) {
            let lower = m.as_str().to_lowercase();
            if seen.insert(lower) {
                ordered.push(m.as_str().to_string());
            }
        }
    }

    // fallback：仅当 0 个 .exe 命中时启用通用单词匹配（避免对 chrome.exe 拆出
    // chrome / exe 子词噪声）。已有 .exe 命中时不再 fallback，让 processes 字段
    // 纯粹反映 .exe 命中（更精确，agent search 体验更可控）。
    if ordered.is_empty() {
        for line in &lines {
            for m in word_re.find_iter(line) {
                let lower = m.as_str().to_lowercase();
                // 排除已见的 / 太通用的英文虚词
                if matches!(
                    lower.as_str(),
                    "the"
                        | "and"
                        | "for"
                        | "with"
                        | "from"
                        | "this"
                        | "that"
                        | "have"
                        | "has"
                        | "was"
                        | "were"
                        | "are"
                        | "not"
                        | "but"
                        | "you"
                        | "all"
                        | "can"
                        | "her"
                        | "would"
                        | "could"
                        | "will"
                        | "they"
                        | "their"
                        | "what"
                        | "about"
                        | "which"
                        | "when"
                        | "your"
                        | "them"
                        | "into"
                        | "than"
                        | "then"
                        | "also"
                ) {
                    continue;
                }
                if seen.insert(lower) {
                    ordered.push(m.as_str().to_string());
                }
            }
        }
    }

    ordered
        .into_iter()
        .map(|name| super::frame::FrameProcess {
            pid: 0,
            name,
            cpu: 0.0,
            memory: 0,
            disk_read: 0,
            disk_write: 0,
        })
        .collect()
}

#[cfg(test)]
mod vt100_extract_tests {
    use super::*;

    fn cell(ch: u32) -> CellDump {
        CellDump {
            ch,
            fg: 0,
            bg: 0,
            flags: 0,
        }
    }

    fn make_rle(text: &str, _width: u16) -> Vec<(u16, CellDump)> {
        // 简化 RLE：每字符单独一项（不合并相邻同字符，测试用足够）
        text.chars().map(|c| (1, cell(c as u32))).collect()
    }

    #[test]
    fn extract_finds_exe_names() {
        let text = "chrome.exe    code.exe     12.5%\nslack.exe";
        let rle = make_rle(text, 30);
        let procs = extract_process_names_from_rle(&rle, 30);
        let names: Vec<&str> = procs.iter().map(|p| p.name.as_str()).collect();
        assert!(names.iter().any(|n| n.to_lowercase() == "chrome.exe"));
        assert!(names.iter().any(|n| n.to_lowercase() == "code.exe"));
        assert!(names.iter().any(|n| n.to_lowercase() == "slack.exe"));
    }

    #[test]
    fn extract_dedup_within_frame() {
        let text = "chrome.exe    chrome.exe    chrome.exe";
        let rle = make_rle(text, 30);
        let procs = extract_process_names_from_rle(&rle, 30);
        let chrome_count = procs
            .iter()
            .filter(|p| p.name.to_lowercase() == "chrome.exe")
            .count();
        assert_eq!(chrome_count, 1);
    }

    #[test]
    fn extract_fallback_words_when_no_exe() {
        let text = "firefox    chrome    edge";
        let rle = make_rle(text, 30);
        let procs = extract_process_names_from_rle(&rle, 30);
        // 3 个非 .exe 单词，fallback 触发（.exe < 3）
        let names: Vec<String> = procs.iter().map(|p| p.name.to_lowercase()).collect();
        assert!(names.contains(&"firefox".to_string()));
        assert!(names.contains(&"chrome".to_string()));
        assert!(names.contains(&"edge".to_string()));
    }

    #[test]
    fn extract_skips_common_english_words() {
        let text = "the chrome and the firefox";
        let rle = make_rle(text, 30);
        let procs = extract_process_names_from_rle(&rle, 30);
        let names: Vec<String> = procs.iter().map(|p| p.name.to_lowercase()).collect();
        assert!(!names.contains(&"the".to_string()));
        assert!(!names.contains(&"and".to_string()));
        assert!(names.contains(&"chrome".to_string()));
        assert!(names.contains(&"firefox".to_string()));
    }

    #[test]
    fn extract_returns_empty_for_blank_screen() {
        let text = "        \n        ";
        let rle = make_rle(text, 8);
        let procs = extract_process_names_from_rle(&rle, 8);
        // fallback 模式下空行不应产生进程名（空字符串 regex 不匹配 \b[\w\-]{3,30}\b）
        assert!(procs.iter().all(|p| !p.name.trim().is_empty()));
    }

    #[test]
    fn extract_handles_zero_width() {
        let rle = make_rle("chrome.exe", 0);
        let procs = extract_process_names_from_rle(&rle, 0);
        assert!(procs.is_empty(), "width=0 should produce no processes");
    }

    #[test]
    fn rle_to_lines_basic() {
        let text = "hello world";
        let rle = make_rle(text, 5);
        let lines = rle_to_lines(&rle, 5);
        // 11 chars / width 5 = 3 lines (5 + 5 + 1)
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "hello");
        assert_eq!(lines[1], " worl");
        assert_eq!(lines[2], "d");
    }
}
