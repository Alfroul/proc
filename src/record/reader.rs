use std::io::{BufReader, Read};
use std::path::PathBuf;

use super::frame::{LegacySystemFrame, RECORDING_MAGIC, RecordingHeader, UiFrame};
use crate::collect::ProcessInfo;

pub struct Player {
    #[allow(dead_code)]
    path: PathBuf,
    header: RecordingHeader,
    #[allow(dead_code)]
    frame_offsets: Vec<u64>,
    frames: Vec<UiFrame>,
}

impl Player {
    pub fn open(path: PathBuf) -> anyhow::Result<Self> {
        let file = std::fs::File::open(&path)?;
        let mut reader = BufReader::new(file);

        // Read header
        let mut len_buf = [0u8; 8];
        reader.read_exact(&mut len_buf)?;
        let header_len = u64::from_le_bytes(len_buf) as usize;

        let mut header_buf = vec![0u8; header_len];
        reader.read_exact(&mut header_buf)?;
        let header: RecordingHeader = bincode::deserialize(&header_buf)?;

        if &header.magic != RECORDING_MAGIC {
            anyhow::bail!("无效的录制文件: 魔数不匹配");
        }

        let header_end = 8 + header_len;
        let mut frame_offsets = Vec::new();
        let mut frames = Vec::new();
        let mut pos = header_end as u64;

        loop {
            let mut len_buf = [0u8; 8];
            match reader.read_exact(&mut len_buf) {
                Ok(()) => {}
                Err(_) => break,
            }
            let frame_len = u64::from_le_bytes(len_buf) as usize;

            frame_offsets.push(pos);

            let mut frame_buf = vec![0u8; frame_len];
            match reader.read_exact(&mut frame_buf) {
                Ok(()) => {}
                Err(_) => break,
            }

            // Try V2 first, fall back to V1
            let frame = if header.version >= 2 {
                bincode::deserialize::<UiFrame>(&frame_buf).ok()
            } else {
                // V1: upgrade legacy frame to UiFrame
                bincode::deserialize::<LegacySystemFrame>(&frame_buf)
                    .ok()
                    .map(legacy_to_v2)
            };

            if let Some(frame) = frame {
                frames.push(frame);
            }

            pos += 8 + frame_len as u64;
        }

        Ok(Self {
            path,
            header,
            frame_offsets,
            frames,
        })
    }

    pub fn total_frames(&self) -> usize {
        self.frames.len()
    }

    pub fn frame_at(&self, index: usize) -> Option<&UiFrame> {
        self.frames.get(index)
    }

    pub fn time_range(&self) -> (u64, u64) {
        if self.frames.is_empty() {
            (self.header.start_time, self.header.start_time)
        } else {
            (
                self.frames.first().unwrap().timestamp,
                self.frames.last().unwrap().timestamp,
            )
        }
    }

    pub fn frame_near_timestamp(&self, ts: u64) -> usize {
        if self.frames.is_empty() {
            return 0;
        }
        let mut lo = 0usize;
        let mut hi = self.frames.len();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.frames[mid].timestamp < ts {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo == 0 {
            return 0;
        }
        if lo >= self.frames.len() {
            return self.frames.len() - 1;
        }
        let diff_lo = (self.frames[lo].timestamp as i64 - ts as i64).unsigned_abs();
        let diff_prev = (self.frames[lo - 1].timestamp as i64 - ts as i64).unsigned_abs();
        if diff_prev <= diff_lo { lo - 1 } else { lo }
    }

    pub fn header(&self) -> &RecordingHeader {
        &self.header
    }
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
pub fn frame_process_to_process_info(fp: &super::frame::FrameProcess) -> ProcessInfo {
    ProcessInfo::from(fp)
}
