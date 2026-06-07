use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use ratatui::widgets::Widget;
use serde::{Deserialize, Serialize};

pub const VT100_MAGIC: &[u8; 4] = b"VT10";
pub const VT100_VERSION: u16 = 1;

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
    pub fg: u8,
    pub bg: u8,
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

                if let Some((count, prev)) = rle.last_mut() {
                    if *prev == dump {
                        *count += 1;
                        continue;
                    }
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
                if y < area.height && x < area.width {
                    if let Some(buf_cell) = buf.cell_mut((area.x + x, area.y + y)) {
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

fn pack_color(color: Color) -> u8 {
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
        Color::Indexed(i) => 17 + i.min(200),
        Color::Rgb(_, _, _) => 0,
    }
}

pub fn unpack_color(packed: u8) -> Color {
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
        i => Color::Indexed(i.wrapping_sub(17)),
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
        let header_bytes = bincode::serialize(&header)?;
        file.write_all(&(header_bytes.len() as u64).to_le_bytes())?;
        file.write_all(&header_bytes)?;
        file.flush()?;

        let (tx, rx) = mpsc::channel::<RecorderMsg>();

        let thread = std::thread::Builder::new()
            .name("vt-recorder".to_string())
            .spawn(move || {
                let mut file = file;
                while let Ok(msg) = rx.recv() {
                    match msg {
                        RecorderMsg::Frame(frame) => {
                            if let Ok(bytes) = bincode::serialize(&frame) {
                                let _ = file.write_all(&(bytes.len() as u64).to_le_bytes());
                                let _ = file.write_all(&bytes);
                                let _ = file.flush();
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

    pub fn stop(mut self) -> anyhow::Result<PathBuf> {
        let _ = self.tx.send(RecorderMsg::Stop);
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
        Ok(self.path)
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn elapsed_secs(&self) -> u64 {
        self.start_time.elapsed().as_secs()
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
        let header: VtHeader = bincode::deserialize(&header_buf)?;

        if &header.magic != VT100_MAGIC {
            anyhow::bail!("无效的 VT100 录制文件");
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
            if let Ok(frame) = bincode::deserialize::<VtFrame>(&frame_buf) {
                frames.push(frame);
            }
        }

        Ok(Self {
            path,
            header,
            frames,
        })
    }

    pub fn total_frames(&self) -> usize {
        self.frames.len()
    }

    pub fn frame_at(&self, index: usize) -> Option<&VtFrame> {
        self.frames.get(index)
    }

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

    pub fn width(&self) -> u16 {
        self.header.width
    }

    pub fn height(&self) -> u16 {
        self.header.height
    }

    pub fn header(&self) -> &VtHeader {
        &self.header
    }
}

// ── Detect VT100 format ──

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
    let Ok(header) = bincode::deserialize::<VtHeader>(&header_buf) else {
        return false;
    };
    &header.magic == VT100_MAGIC
}
