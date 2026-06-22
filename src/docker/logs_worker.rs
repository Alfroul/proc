//! E1 — 容器日志后台 worker。
//!
//! 调用 bollard `logs`（`follow=true`）流式拉日志，在独立线程跑 tokio runtime，
//! 把每条 `LogOutput` 转 [`LogLine`] 攒到 chunk（最多 16 行或 4KB 字符），通过
//! `sync_channel(64)` 推给主线程。ADR-0006 背压：消费者慢时丢新 chunk 保留历史。
//!
//! 用户切换容器 / 退出日志模式时 panel 把 [`LogsWorker`] drop → Drop 触发
//! worker 退出（`shutdown_tx` drop → 主线程检测到 → 通过 `runtime` abort 停止）。

use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::docker::logs::{self, LogLine};

/// 单条 chunk 推送的最大行数。太大会延迟显示，太小频繁唤醒主线程。
const CHUNK_MAX_LINES: usize = 16;
/// 单条 chunk 推送的最大字符数。封顶避免长行（如 stacktrace）撑爆。
const CHUNK_MAX_CHARS: usize = 4 * 1024;
/// `sync_channel` 容量。主线程卡顿 ~6s 内不丢 chunk。
const LOGS_CHANNEL_CAPACITY: usize = 64;
/// shutdown 轮询间隔。
const SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// 一批日志条目。worker 攒到 [`CHUNK_MAX_LINES`] 或 [`CHUNK_MAX_CHARS`] 推一次。
#[derive(Debug, Clone, Default)]
pub struct LogChunk {
    pub lines: Vec<LogLine>,
}

/// 日志 worker 句柄。Drop 时关闭 worker。
pub struct LogsWorker {
    shutdown_tx: Option<mpsc::Sender<()>>,
    pub chunk_rx: Receiver<LogChunk>,
    thread: Option<JoinHandle<()>>,
}

impl LogsWorker {
    /// Drain 主线程在 tick 里调用一次，拿走当前所有 chunk。
    #[must_use]
    pub fn drain(&self) -> Vec<LogChunk> {
        let mut out = Vec::new();
        while let Ok(chunk) = self.chunk_rx.try_recv() {
            out.push(chunk);
        }
        out
    }
}

impl Drop for LogsWorker {
    fn drop(&mut self) {
        // 1) drop shutdown_tx → worker 主循环 recv 返回 Err → 触发退出。
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
            drop(tx);
        }
        // 2) join 线程。
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

/// 启动日志 worker。
///
/// `docker` 由调用方 `clone()` 传入（bollard 内部 Arc，廉价）。
/// `name` 是容器名。`tail` 是初始尾随（`Some("100")` 从最后 100 行开始，`None` 全部）。
#[must_use]
pub fn spawn(docker: bollard::Docker, name: String, tail: Option<String>) -> LogsWorker {
    let (chunk_tx, chunk_rx) = mpsc::sync_channel(LOGS_CHANNEL_CAPACITY);
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();

    let handle = thread::Builder::new()
        .name(format!("docker-logs-worker-{name}"))
        .spawn(move || {
            worker_body(docker, name, tail, chunk_tx, shutdown_rx);
        })
        .expect("spawn docker-logs-worker");

    LogsWorker {
        shutdown_tx: Some(shutdown_tx),
        chunk_rx,
        thread: Some(handle),
    }
}

fn worker_body(
    docker: bollard::Docker,
    name: String,
    tail: Option<String>,
    chunk_tx: SyncSender<LogChunk>,
    shutdown_rx: Receiver<()>,
) {
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            tracing::warn!("docker-logs-worker tokio 起不来: {e:?}");
            return;
        }
    };

    let options = logs::make_follow_options(tail.as_deref());

    // 策略：在主线程上跑 stream consumer + 周期性检查 shutdown。
    // `docker.logs` 返回的 stream 是非阻塞的；我们用 try_next + 短 timeout 轮询，
    // 每 SHUTDOWN_POLL_INTERVAL 检查一次 shutdown_rx，避免无限阻塞。
    runtime.block_on(async move {
        use futures_util::stream::StreamExt;

        let mut stream = docker.logs(&name, Some(options)).fuse();
        let mut buf: Vec<LogLine> = Vec::with_capacity(CHUNK_MAX_LINES);
        let mut buf_chars: usize = 0;

        loop {
            if is_shutdown(&shutdown_rx) {
                break;
            }
            // 短 timeout 让 select 周期性回到 shutdown 检查。
            tokio::select! {
                item = stream.next() => {
                    match item {
                        Some(Ok(log)) => {
                            push_log_line(&log, &mut buf, &mut buf_chars, &chunk_tx);
                        }
                        Some(Err(e)) => {
                            tracing::warn!("docker-logs-worker ({name}) stream err: {e:?}");
                            break;
                        }
                        None => {
                            // stream 结束（容器停止 / docker daemon 断开）。
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep(SHUTDOWN_POLL_INTERVAL) => {
                    // timeout 回到循环顶部检查 shutdown。
                }
            }
        }
        flush(&mut buf, &mut buf_chars, &chunk_tx);
    });
}

/// 检查 shutdown 信号是否触发（主线程 drop 或 send(())）。
fn is_shutdown(shutdown_rx: &Receiver<()>) -> bool {
    match shutdown_rx.try_recv() {
        Ok(_) => true,
        Err(mpsc::TryRecvError::Empty) => false,
        Err(mpsc::TryRecvError::Disconnected) => true,
    }
}

fn push_log_line(
    log: &bollard::container::LogOutput,
    buf: &mut Vec<LogLine>,
    buf_chars: &mut usize,
    chunk_tx: &SyncSender<LogChunk>,
) {
    use bollard::container::LogOutput;

    let raw = String::from_utf8_lossy(log.as_ref()).into_owned();
    let is_stderr = matches!(log, LogOutput::StdErr { .. });

    for piece in raw.split_inclusive('\n') {
        let (timestamp, message) = logs::parse_log_timestamp(piece);
        if timestamp.is_none() && message.is_empty() {
            continue;
        }
        buf.push(LogLine {
            timestamp,
            message,
            is_stderr,
        });
        *buf_chars += piece.len();
        if buf.len() >= CHUNK_MAX_LINES || *buf_chars >= CHUNK_MAX_CHARS {
            flush(buf, buf_chars, chunk_tx);
        }
    }
}

fn flush(buf: &mut Vec<LogLine>, buf_chars: &mut usize, chunk_tx: &SyncSender<LogChunk>) {
    if buf.is_empty() {
        return;
    }
    let chunk = LogChunk {
        lines: std::mem::take(buf),
    };
    *buf_chars = 0;
    match chunk_tx.try_send(chunk) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => {
            tracing::warn!(
                "docker-logs-worker chunk channel 已满（{}），丢新 chunk",
                LOGS_CHANNEL_CAPACITY
            );
        }
        Err(TrySendError::Disconnected(_)) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker::logs::LogLine;

    #[test]
    fn chunk_default_empty() {
        let c = LogChunk::default();
        assert!(c.lines.is_empty());
    }

    #[test]
    fn log_chunk_capacity_constants_reasonable() {
        const { assert!(CHUNK_MAX_LINES <= 64) };
        const { assert!(CHUNK_MAX_LINES > 0) };
        const { assert!(LOGS_CHANNEL_CAPACITY >= 16) };
    }

    #[test]
    fn flush_skips_empty_buffer() {
        let (tx, rx) = mpsc::sync_channel::<LogChunk>(4);
        let mut buf = Vec::new();
        let mut chars = 0;
        flush(&mut buf, &mut chars, &tx);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn flush_drains_buffer_and_sends_chunk() {
        let (tx, rx) = mpsc::sync_channel::<LogChunk>(4);
        let mut buf = vec![LogLine {
            timestamp: None,
            message: "hi".to_string(),
            is_stderr: false,
        }];
        let mut chars = 2;
        flush(&mut buf, &mut chars, &tx);
        let chunk = rx.try_recv().expect("should have received");
        assert_eq!(chunk.lines.len(), 1);
        assert_eq!(chunk.lines[0].message, "hi");
        assert!(buf.is_empty());
        assert_eq!(chars, 0);
    }

    #[test]
    fn is_shutdown_empty_returns_false() {
        let (_tx, rx) = mpsc::channel::<()>();
        assert!(!is_shutdown(&rx));
    }

    #[test]
    fn is_shutdown_disconnected_returns_true() {
        let (tx, rx) = mpsc::channel::<()>();
        drop(tx);
        // give the OS a moment to propagate
        assert!(is_shutdown(&rx));
    }

    #[test]
    fn is_shutdown_signaled_returns_true() {
        let (tx, rx) = mpsc::channel::<()>();
        tx.send(()).expect("send");
        assert!(is_shutdown(&rx));
    }
}
