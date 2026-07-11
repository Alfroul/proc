use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::mpsc;

use super::encoding::serialize_with_version;
use super::frame::{FOOTER_MAGIC, RECORDING_VERSION, RecordingFooter, RecordingHeader, UiFrame};

#[allow(clippy::large_enum_variant)]
enum WriterMsg {
    Frame(UiFrame),
    Stop,
}

pub struct Recorder {
    tx: mpsc::Sender<WriterMsg>,
    thread: Option<std::thread::JoinHandle<()>>,
    path: PathBuf,
    start_time: u64,
    stopped: bool,
}

impl Recorder {
    pub fn start(path: PathBuf) -> anyhow::Result<Self> {
        let dir = path.parent().map(|p| p.to_path_buf());
        if let Some(dir) = dir {
            std::fs::create_dir_all(&dir)?;
        }

        let header = RecordingHeader::default();
        let start_time = header.start_time;

        let mut file = BufWriter::new(File::create(&path)?);

        // v0.18 stage 2：header 永远走 fixint（与 reader.rs:69 `bincode::deserialize`
        // 配对），让 reader 拿到 header.version 后再分支选 frame/footer config。
        // 不用 serialize_with_version（version >= 4 会走 varint，与 reader fixint
        // 不匹配导致 EOF）。
        let header_bytes = bincode::serialize(&header)?;
        let header_len = header_bytes.len() as u64;
        file.write_all(&header_len.to_le_bytes())?;
        file.write_all(&header_bytes)?;
        file.flush()?;

        let (tx, rx) = mpsc::channel::<WriterMsg>();

        let thread = std::thread::Builder::new()
            .name("recorder".to_string())
            .spawn(move || {
                let mut file = file;
                let mut frames_since_log: u64 = 0;
                let mut bytes_since_log: u64 = 0;

                // v3 footer 累积状态
                let mut current_offset: u64 = 8 + header_len;
                let mut frame_offsets: Vec<u64> = Vec::new();
                let mut first_frame_ts: Option<u64> = None;
                let mut end_time: u64 = start_time;
                let mut anomaly_count: u64 = 0;
                let mut event_count: u64 = 0;
                let mut max_cpu: f32 = 0.0;
                let mut max_mem: u64 = 0;
                let mut frame_count: u64 = 0;

                while let Ok(msg) = rx.recv() {
                    match msg {
                        WriterMsg::Frame(frame) => {
                            let bytes = match serialize_with_version(RECORDING_VERSION, &frame) {
                                Ok(b) => b,
                                Err(_) => continue,
                            };
                            let len = bytes.len() as u64;

                            // 在 write 之前记录 offset（指向 8B len prefix）
                            frame_offsets.push(current_offset);
                            // 更新元数据
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

                            if file.write_all(&len.to_le_bytes()).is_err() {
                                break;
                            }
                            if file.write_all(&bytes).is_err() {
                                break;
                            }
                            let _ = file.flush();
                            current_offset += 8 + bytes.len() as u64;

                            frames_since_log += 1;
                            bytes_since_log += 8 + bytes.len() as u64;
                            if frames_since_log >= 100 {
                                tracing::debug!(
                                    frames = frames_since_log,
                                    bytes = bytes_since_log,
                                    "recorder 写入进度",
                                );
                                frames_since_log = 0;
                                bytes_since_log = 0;
                            }
                        }
                        WriterMsg::Stop => {
                            // v3 footer：写 footer + 8B footer_len + 8B FOOTER_MAGIC
                            let footer = RecordingFooter {
                                version: 1,
                                header_version: super::frame::RECORDING_VERSION,
                                start_time: first_frame_ts.unwrap_or(start_time),
                                end_time,
                                frame_count,
                                anomaly_count,
                                event_count,
                                max_cpu,
                                max_mem,
                                frame_offsets,
                            };
                            match serialize_with_version(RECORDING_VERSION, &footer) {
                                Ok(footer_bytes) => {
                                    let footer_len = footer_bytes.len() as u64;
                                    // 顺序：footer_bytes + footer_len(8B LE) + FOOTER_MAGIC(8B)
                                    let _ = file.write_all(&footer_bytes);
                                    let _ = file.write_all(&footer_len.to_le_bytes());
                                    let _ = file.write_all(&FOOTER_MAGIC);
                                    let _ = file.flush();
                                    tracing::debug!(
                                        footer_bytes = footer_bytes.len(),
                                        frame_count,
                                        "recorder footer 已写入",
                                    );
                                }
                                Err(e) => {
                                    tracing::warn!("footer 序列化失败: {e}");
                                    let _ = file.flush();
                                }
                            }
                            break;
                        }
                    }
                }
            })?;

        Ok(Self {
            tx,
            thread: Some(thread),
            path,
            start_time,
            stopped: false,
        })
    }

    pub fn submit_frame(&self, frame: UiFrame) {
        let _ = self.tx.send(WriterMsg::Frame(frame));
    }

    pub fn stop(mut self) -> anyhow::Result<()> {
        self.stop_internal();
        tracing::info!("录制已保存到: {}", self.path.display());
        Ok(())
    }

    fn stop_internal(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        let _ = self.tx.send(WriterMsg::Stop);
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
    }

    #[must_use]
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    #[must_use]
    pub fn start_time(&self) -> u64 {
        self.start_time
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // Mirror VtRecorder: if `stop()` was already called this is a no-op,
        // otherwise flush + join so a forgotten Recorder still persists its
        // buffered frames and never leaks the writer thread.
        self.stop_internal();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::encoding::deserialize_with_version;
    use crate::record::frame::FrameProcess;

    fn make_test_frame(timestamp: u64, cpu: f32, anomalies: usize) -> UiFrame {
        UiFrame {
            timestamp,
            mode: "ProcessList".to_string(),
            status_message: None,
            cpu_usage: cpu,
            memory_used: 1024,
            memory_total: 4096,
            net_down: 0,
            net_up: 0,
            cpu_history: vec![],
            mem_history: vec![],
            processes: vec![FrameProcess {
                pid: 1,
                name: "p".to_string(),
                cpu,
                memory: 1024,
                disk_read: 0,
                disk_write: 0,
            }],
            search_query: String::new(),
            sort_field: "Cpu".to_string(),
            process_view_mode: 0,
            tree_nodes: vec![],
            port_entries: vec![],
            port_view_mode: 0,
            port_process_groups: vec![],
            port_remote_groups: vec![],
            connection_diff: Default::default(),
            anomalies: (0..anomalies)
                .map(|i| super::super::frame::FrameAnomaly {
                    rule_id: format!("r{i}"),
                    severity: "Warning".to_string(),
                    title: "t".to_string(),
                    detail: "d".to_string(),
                    affected_pid: None,
                    affected_ip: None,
                })
                .collect(),
            usb_devices: vec![],
            usb_locks: vec![],
            monitors: vec![],
            docker_containers: vec![],
            docker_events: vec![],
            ops: vec![],
            nav: Default::default(),
        }
    }

    #[test]
    fn v3_writer_appends_footer_trailer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v3.prec");

        let recorder = Recorder::start(path.clone()).unwrap();
        for i in 0..5u64 {
            recorder.submit_frame(make_test_frame(1000 + i, 10.0 + i as f32, i as usize));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        recorder.stop().unwrap();

        let bytes = std::fs::read(&path).unwrap();
        // trailer：末 16B = [8B footer_len LE][8B FOOTER_MAGIC]
        let n = bytes.len();
        assert!(n >= 16, "file too small: {n}");
        let magic = &bytes[n - 8..n];
        assert_eq!(
            magic,
            FOOTER_MAGIC,
            "footer magic mismatch: got {:?}",
            std::str::from_utf8(magic).ok()
        );
        let mut len_buf = [0u8; 8];
        len_buf.copy_from_slice(&bytes[n - 16..n - 8]);
        let footer_len = u64::from_le_bytes(len_buf) as usize;
        assert!(footer_len > 0 && footer_len < n - 16);

        // footer deserialize
        let footer_start = n - 16 - footer_len;
        let footer: RecordingFooter = deserialize_with_version(
            RECORDING_VERSION,
            &bytes[footer_start..footer_start + footer_len],
        )
        .unwrap();
        assert_eq!(footer.frame_count, 5);
        assert_eq!(footer.frame_offsets.len(), 5);
        assert_eq!(footer.start_time, 1000);
        assert_eq!(footer.end_time, 1004);
        // frame i 携带 i 个 anomaly（i=0..4），总和 = 0+1+2+3+4 = 10
        assert_eq!(footer.anomaly_count, (0..=4u64).sum::<u64>());
        assert!((footer.max_cpu - 14.0).abs() < 0.01);
        assert_eq!(footer.max_mem, 1024);
    }

    #[test]
    fn v3_writer_handles_empty_recording() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.prec");

        let recorder = Recorder::start(path.clone()).unwrap();
        recorder.stop().unwrap();

        let bytes = std::fs::read(&path).unwrap();
        let n = bytes.len();
        assert!(n >= 16);
        assert_eq!(&bytes[n - 8..n], FOOTER_MAGIC);
        let mut len_buf = [0u8; 8];
        len_buf.copy_from_slice(&bytes[n - 16..n - 8]);
        let footer_len = u64::from_le_bytes(len_buf) as usize;
        let footer_start = n - 16 - footer_len;
        let footer: RecordingFooter = deserialize_with_version(
            RECORDING_VERSION,
            &bytes[footer_start..footer_start + footer_len],
        )
        .unwrap();
        assert_eq!(footer.frame_count, 0);
        assert!(footer.frame_offsets.is_empty());
    }
}
