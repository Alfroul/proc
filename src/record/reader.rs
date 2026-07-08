//! v3 录屏文件 reader：按需加载（lazy frame_at）+ footer 元数据。
//!
//! v3 文件格式：
//! ```text
//! [8B header_len][header_bytes]
//! [8B frame_len_1][frame_bytes_1]
//! ...
//! [8B frame_len_N][frame_bytes_N]
//! [footer_bytes]
//! [8B footer_len]
//! [8B FOOTER_MAGIC]
//! ```
//!
//! reader open 流程：
//! 1. 读 header（8B len + bytes）
//! 2. seek 到 `file_size - 16` 读 trailer
//! 3. trailer 匹配 [`FOOTER_MAGIC`] → v3 路径：seek 到 footer_offset 读 footer
//! 4. 否则 → v1/v2 老文件路径：尝试加载 `.prec.idx` sidecar，失败则全量加载后写 sidecar

use std::cell::{Cell, RefCell};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use bincode::Options;

use super::frame::{
    FOOTER_MAGIC, FOOTER_TRAILER_LEN, LegacySystemFrame, RECORDING_MAGIC, RecordingFooter,
    RecordingHeader, UiFrame,
};
use super::sidecar::IdxSidecar;
use crate::collect::ProcessInfo;

pub struct Player {
    path: PathBuf,
    header: RecordingHeader,
    footer: RecordingFooter,
    file: RefCell<File>,
    /// 单帧 LRU：连续访问同一 idx（每 tick 一次）时跳过 IO + deserialize。
    cache_idx: Cell<Option<usize>>,
    cache_frame: RefCell<Option<UiFrame>>,
}

impl Player {
    pub fn open(path: PathBuf) -> anyhow::Result<Self> {
        let mut file = File::open(&path)?;
        let file_size = file.metadata()?.len();

        // Read header (8B len + bytes)
        let mut len_buf = [0u8; 8];
        file.read_exact(&mut len_buf)?;
        let header_len = u64::from_le_bytes(len_buf) as usize;

        // Sanity cap：与 v0.6 既有保护一致（reader.rs 原版本）
        const MAX_HEADER_LEN: usize = 64 * 1024;
        if header_len > MAX_HEADER_LEN {
            anyhow::bail!(
                "录制文件 header 异常大: {} bytes (上限 {})",
                header_len,
                MAX_HEADER_LEN
            );
        }

        let mut header_buf = vec![0u8; header_len];
        file.read_exact(&mut header_buf)?;
        // v0.17 stage 3 TD-45：header 自身用 fixint 反序列化（与 bincode::deserialize
        // 默认等价），拿到 header.version 后再决定后续 frame / footer 走哪个 config。
        // 当前所有版本都走 fixint（与 stage 3 前行为完全等价）。
        let header: RecordingHeader = bincode::deserialize(&header_buf)?;

        if &header.magic != RECORDING_MAGIC {
            anyhow::bail!("无效的录制文件: 魔数不匹配");
        }

        // 选中本文件用的 bincode 配置（按 header.version 决定；当前所有版本 fixint）。
        let opts = super::encoding::options_for_version(header.version);

        let header_total = 8 + header_len as u64;

        // Trailer 检测：v3 文件末尾 16B = [footer_len(8B LE)][FOOTER_MAGIC(8B)]
        let (footer, file) = if file_size >= header_total + FOOTER_TRAILER_LEN {
            let mut trailer = [0u8; 16];
            file.seek(SeekFrom::End(-16))?;
            if file.read_exact(&mut trailer).is_ok() {
                let mut magic_buf = [0u8; 8];
                magic_buf.copy_from_slice(&trailer[8..16]);
                if magic_buf == FOOTER_MAGIC {
                    // v3 路径
                    let mut footer_len_buf = [0u8; 8];
                    footer_len_buf.copy_from_slice(&trailer[0..8]);
                    let footer_len = u64::from_le_bytes(footer_len_buf) as usize;
                    let footer_start = file_size - FOOTER_TRAILER_LEN - footer_len as u64;
                    file.seek(SeekFrom::Start(footer_start))?;
                    let mut footer_bytes = vec![0u8; footer_len];
                    file.read_exact(&mut footer_bytes)?;
                    let footer: RecordingFooter = opts.deserialize(&footer_bytes)?;
                    (footer, file)
                } else {
                    // 不是 v3 trailer，老文件路径
                    let mut file = file;
                    let footer = open_legacy(&mut file, &path, &header, header_total, file_size)?;
                    (footer, file)
                }
            } else {
                let mut file = file;
                let footer = open_legacy(&mut file, &path, &header, header_total, file_size)?;
                (footer, file)
            }
        } else {
            // 文件太小不可能含 trailer，老文件路径
            let footer = open_legacy(&mut file, &path, &header, header_total, file_size)?;
            (footer, file)
        };

        Ok(Self {
            path,
            header,
            footer,
            file: RefCell::new(file),
            cache_idx: Cell::new(None),
            cache_frame: RefCell::new(None),
        })
    }

    #[must_use]
    pub fn total_frames(&self) -> usize {
        self.footer.frame_count as usize
    }

    /// 按需加载：返回 owned UiFrame。开销 = 1 次 seek + 8 + N 字节 read +
    /// bincode deserialize（@ 1000 进程 = 165 µs，PERF-BASELINE-v0.13 §4 实测）。
    /// 单帧 LRU 缓存命中时跳过 IO + deserialize。
    #[must_use]
    pub fn frame_at(&self, index: usize) -> Option<UiFrame> {
        if index >= self.footer.frame_count as usize {
            return None;
        }
        // LRU 命中
        if self.cache_idx.get() == Some(index) {
            return self.cache_frame.borrow().clone();
        }

        let offset = *self.footer.frame_offsets.get(index)?;
        let mut file = self.file.borrow_mut();
        file.seek(SeekFrom::Start(offset)).ok()?;
        let mut len_buf = [0u8; 8];
        if file.read_exact(&mut len_buf).is_err() {
            return None;
        }
        let frame_len = u64::from_le_bytes(len_buf) as usize;
        // sanity cap：单帧不应超过文件大小
        let file_size = file.metadata().map(|m| m.len()).unwrap_or(0);
        if frame_len as u64 > file_size {
            return None;
        }
        let mut frame_buf = vec![0u8; frame_len];
        if file.read_exact(&mut frame_buf).is_err() {
            return None;
        }

        // Try V2 first, fall back to V1
        // v0.17 stage 3 TD-45：用 options_for_version(self.header.version) 选 bincode
        // 配置（当前所有版本 fixint，与 stage 3 前行为完全等价）。
        let opts = super::encoding::options_for_version(self.header.version);
        let frame = if self.header.version >= 2 {
            opts.deserialize::<UiFrame>(&frame_buf).ok()?
        } else {
            // V1: upgrade legacy frame to UiFrame
            let legacy = opts.deserialize::<LegacySystemFrame>(&frame_buf).ok()?;
            legacy_to_v2(legacy)
        };

        self.cache_idx.set(Some(index));
        *self.cache_frame.borrow_mut() = Some(frame.clone());
        Some(frame)
    }

    #[must_use]
    pub fn time_range(&self) -> (u64, u64) {
        (self.footer.start_time, self.footer.end_time)
    }

    /// 在 footer.start_time / end_time 范围内二分查找最接近 `ts` 的帧 idx。
    /// 不需要 deserialize 帧数据（v0.6 老版本是 `self.frames[mid].timestamp`）。
    #[must_use]
    pub fn frame_near_timestamp(&self, ts: u64) -> usize {
        let n = self.footer.frame_count as usize;
        if n == 0 {
            return 0;
        }
        if self.footer.frame_count == 1 {
            return 0;
        }
        // 假设线性（实际 frame timestamp 单调递增；个别 gap 不影响）
        let start = self.footer.start_time;
        let end = self.footer.end_time;
        if ts <= start {
            return 0;
        }
        if ts >= end {
            return n - 1;
        }
        let span = end.saturating_sub(start).max(1);
        let approx = ((ts.saturating_sub(start)) as u128 * (n - 1) as u128 / span as u128) as usize;
        // 估算点附近 ±2 帧 refine（拉回两个候选比较 timestamp）
        let mut best = approx.min(n - 1);
        let mut best_diff = u64::MAX;
        let lo = best.saturating_sub(2);
        let hi = (best + 2).min(n - 1);
        for i in lo..=hi {
            if let Some(f) = self.frame_at(i) {
                let diff = (f.timestamp as i64 - ts as i64).unsigned_abs();
                if diff < best_diff {
                    best_diff = diff;
                    best = i;
                }
            }
        }
        best
    }

    #[must_use]
    pub fn header(&self) -> &RecordingHeader {
        &self.header
    }

    #[must_use]
    pub fn meta(&self) -> &RecordingFooter {
        &self.footer
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// v1/v2 老文件加载（无 footer）：先尝试 sidecar，失败则全量加载 + 写 sidecar。
fn open_legacy(
    file: &mut File,
    path: &Path,
    header: &RecordingHeader,
    header_total: u64,
    file_size: u64,
) -> anyhow::Result<RecordingFooter> {
    // 1. 尝试 sidecar 快路径
    if let Some(sidecar) = IdxSidecar::try_load(path) {
        // 校验 sidecar 的 header 与文件 header 一致
        if sidecar.header.magic == header.magic
            && sidecar.header.version == header.version
            && sidecar.header.start_time == header.start_time
        {
            return Ok(sidecar.footer);
        }
    }

    // 2. fallback：全量加载（保留 v0.6 行为）+ 构造 footer
    let mut frame_offsets: Vec<u64> = Vec::new();
    let mut first_frame_ts: Option<u64> = None;
    let mut end_time: u64 = header.start_time;
    let mut anomaly_count: u64 = 0;
    let mut event_count: u64 = 0;
    let mut max_cpu: f32 = 0.0;
    let mut max_mem: u64 = 0;
    let mut frame_count: u64 = 0;

    file.seek(SeekFrom::Start(header_total))?;
    let mut pos = header_total;
    while pos + 8 <= file_size {
        let mut len_buf = [0u8; 8];
        if file.read_exact(&mut len_buf).is_err() {
            break;
        }
        let frame_len = u64::from_le_bytes(len_buf) as usize;
        if pos + 8 + frame_len as u64 > file_size {
            break;
        }

        frame_offsets.push(pos);

        let mut frame_buf = vec![0u8; frame_len];
        if file.read_exact(&mut frame_buf).is_err() {
            break;
        }

        // 解析帧提取元数据（v2 走 UiFrame；v1 走 LegacySystemFrame）
        // v0.17 stage 3 TD-45：用 options_for_version 选 bincode 配置（每次创建新实例，
        // impl Options 不是 Copy 不能跨 loop 持有）。
        if header.version >= 2 {
            if let Ok(frame) = super::encoding::options_for_version(header.version)
                .deserialize::<UiFrame>(&frame_buf)
            {
                first_frame_ts.get_or_insert(frame.timestamp);
                end_time = frame.timestamp;
                anomaly_count += frame.anomalies.len() as u64;
                event_count += frame.docker_events.len() as u64;
                event_count += frame.ops.len() as u64;
                if frame.cpu_usage > max_cpu {
                    max_cpu = frame.cpu_usage;
                }
                if frame.memory_used > max_mem {
                    max_mem = frame.memory_used;
                }
                frame_count += 1;
            }
        } else if let Ok(legacy) = super::encoding::options_for_version(header.version)
            .deserialize::<LegacySystemFrame>(&frame_buf)
        {
            first_frame_ts.get_or_insert(legacy.timestamp);
            end_time = legacy.timestamp;
            if legacy.cpu_usage > max_cpu {
                max_cpu = legacy.cpu_usage;
            }
            if legacy.memory_used > max_mem {
                max_mem = legacy.memory_used;
            }
            frame_count += 1;
        }

        pos += 8 + frame_len as u64;
    }

    let footer = RecordingFooter {
        version: 1,
        header_version: header.version,
        start_time: first_frame_ts.unwrap_or(header.start_time),
        end_time,
        frame_count,
        anomaly_count,
        event_count,
        max_cpu,
        max_mem,
        frame_offsets,
    };

    // 3. 写 sidecar（失败静默）
    let sidecar = IdxSidecar::from_legacy(path, header.clone(), footer.clone());
    sidecar.write(path);

    Ok(footer)
}

fn legacy_to_v2(legacy: LegacySystemFrame) -> UiFrame {
    UiFrame {
        timestamp: legacy.timestamp,
        mode: "ProcessList".to_string(),
        status_message: None,
        cpu_usage: legacy.cpu_usage,
        memory_used: legacy.memory_used,
        memory_total: legacy.memory_total,
        net_down: legacy.net_down,
        net_up: legacy.net_up,
        cpu_history: Vec::new(),
        mem_history: Vec::new(),
        processes: legacy.processes,
        search_query: String::new(),
        sort_field: "Cpu".to_string(),
        process_view_mode: 0,
        tree_nodes: Vec::new(),
        port_entries: Vec::new(),
        port_view_mode: 0,
        port_process_groups: Vec::new(),
        port_remote_groups: Vec::new(),
        connection_diff: super::frame::FrameConnectionDiff {
            new_count: 0,
            closed_count: 0,
            active_count: 0,
            close_wait_count: 0,
            time_wait_count: 0,
        },
        anomalies: Vec::new(),
        usb_devices: Vec::new(),
        usb_locks: Vec::new(),
        monitors: Vec::new(),
        docker_containers: Vec::new(),
        docker_events: Vec::new(),
        ops: Vec::new(),
        nav: super::frame::FrameNav::default(),
    }
}

/// Legacy entry point kept for test compatibility. Delegates to the
/// `From<&FrameProcess>` impl in `super::conversions`.
#[must_use]
pub fn frame_process_to_process_info(fp: &super::frame::FrameProcess) -> ProcessInfo {
    ProcessInfo::from(fp)
}
