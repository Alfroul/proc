use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::mpsc;

use super::frame::{RecordingHeader, UiFrame};

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
                while let Ok(msg) = rx.recv() {
                    match msg {
                        WriterMsg::Frame(frame) => {
                            if let Ok(bytes) = bincode::serialize(&frame) {
                                let len = bytes.len() as u64;
                                if file.write_all(&len.to_le_bytes()).is_err() {
                                    break;
                                }
                                if file.write_all(&bytes).is_err() {
                                    break;
                                }
                                let _ = file.flush();
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
                        }
                        WriterMsg::Stop => {
                            let _ = file.flush();
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
        })
    }

    pub fn submit_frame(&self, frame: UiFrame) {
        let _ = self.tx.send(WriterMsg::Frame(frame));
    }

    pub fn stop(mut self) -> anyhow::Result<()> {
        let _ = self.tx.send(WriterMsg::Stop);
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
        tracing::info!("录制已保存到: {}", self.path.display());
        Ok(())
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn start_time(&self) -> u64 {
        self.start_time
    }
}
