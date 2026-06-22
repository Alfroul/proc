//! E2 — exec 进容器（PTY 桥接）。
//!
//! 设计（ADR-0007）：在本地 portable-PTY 中 spawn `docker exec -it <container> <shell>`
//! 子进程，docker CLI 负责与 daemon 通信并在远端分配 PTY。我们只需管理本地 PTY：
//! - master.writer ← 用户按键字节（含 ANSI 转义，如 `\r` / `\x03` / `\x7f`）
//! - master.reader → 后台 reader 线程 → `sync_channel(64)` 背压 → 主线程 drain
//! - vt100 crate 把字节流解析成 `Screen`，ratatui 渲染 `terminal_state`（ADR-0006）
//!
//! 跨平台：portable-pty 自动选择 Windows ConPTY（Windows 10 1809+）/ Linux POSIX PTY。
//! Docker Desktop（命名管道）/ WSL Docker（TCP）/ Linux Docker（unix socket）三种连接
//! 方式由 docker CLI 处理，proc 不感知。
//!
//! 退出协议：用户 `exit` / `Ctrl+D` / `Ctrl+\` → docker exec 子进程退出 →
//! reader 拿到 EOF → reader thread 自然退出 → Drop 时 child.kill() + join。

use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::error::{ProcError, Result};

/// PTY reader → 主线程 chunk 通道容量。主线程卡顿 ~1s 内不丢字节。
const PTY_CHANNEL_CAPACITY: usize = 64;
/// reader 单次 read 缓冲。8KB 平衡 syscall 频次与延迟。
const PTY_READ_BUF: usize = 8 * 1024;
/// child.kill() 后等 reader 退出的最大时长。超时后放弃 join（线程泄漏可接受，
/// 因为 reader 会在 master drop 后拿到 EOF 自然退出）。
const READER_JOIN_TIMEOUT: Duration = Duration::from_millis(500);

/// portable-pty 用 anyhow::Error，把它转字符串塞进 ProcError::Docker。
/// 不用 source chain：调用方只需展示给用户。
fn pty_err(msg: impl Into<String>, e: anyhow::Error) -> ProcError {
    ProcError::docker(format!("{}: {e}", msg.into()))
}

/// 默认 PTY 尺寸（进入 exec 模式时的初始 size；第一次 tick 会用真实终端尺寸 resize）。
pub const DEFAULT_COLS: u16 = 80;
pub const DEFAULT_ROWS: u16 = 24;

/// reader 线程推过来的一批 PTY 字节。主线程 tick 时 drain 拼接后喂 vt100 parser。
#[derive(Debug, Clone, Default)]
pub struct PtyChunk {
    pub bytes: Vec<u8>,
}

/// ContainerExec 句柄。
///
/// 持有 PTY master / writer / child / reader thread。
/// Drop 时：`child.kill()` → reader 拿 EOF 退出 → join。
/// 必须在主线程同步 drop，避免 PTY fd 泄漏。
pub struct ContainerExec {
    /// PTY master。保活是必须的：master drop → slave 端的 docker exec 收到 SIGHUP 退出。
    master: Box<dyn MasterPty + Send>,
    /// writer 句柄（`MasterPty::take_writer`）。
    writer: Box<dyn Write + Send>,
    /// docker exec 子进程。
    child: Box<dyn portable_pty::Child + Send + Sync>,
    /// reader 线程 → 主线程的 chunk 通道。
    pub chunk_rx: Receiver<PtyChunk>,
    reader_thread: Option<JoinHandle<()>>,
    /// 容器名（UI 显示用 + Drop 日志）。
    pub container: String,
    /// 当前 PTY size。resize 时更新，避免重复 resize 同尺寸。
    cols: u16,
    rows: u16,
}

impl ContainerExec {
    /// 用默认 80×24 启动。`cmd` 为空时根据 `image` 推断 shell（[`detect_default_shell`]）。
    #[must_use = "句柄需保活，drop 即退出 exec"]
    pub fn start(container: &str, cmd: &[String], image: Option<&str>) -> Result<Self> {
        Self::start_with_size(container, cmd, image, DEFAULT_COLS, DEFAULT_ROWS)
    }

    /// 用指定 cols × rows 启动。
    pub fn start_with_size(
        container: &str,
        cmd: &[String],
        image: Option<&str>,
        cols: u16,
        rows: u16,
    ) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| pty_err("创建 PTY 失败", e))?;

        // 拼 `docker exec -it <container> <cmd...>`。
        // `-i` 保持 stdin 打开；`-t` 分配 TTY（远端 PTY + SIGWINCH 转发）。
        let mut builder = CommandBuilder::new("docker");
        builder.arg("exec");
        builder.arg("-it");
        builder.arg(container);
        if cmd.is_empty() {
            // 推断 shell：根据 image 名走 detect_default_shell。
            // 推断失败时退 /bin/sh（POSIX 兜底，alpine/ubuntu 都有）。
            let shell = detect_default_shell(image.unwrap_or(""));
            for token in shell.split_whitespace() {
                builder.arg(token);
            }
        } else {
            for token in cmd {
                builder.arg(token);
            }
        }

        let child = pair.slave.spawn_command(builder).map_err(|e| {
            pty_err(
                format!("spawn docker exec {container} 失败（确认 PATH 有 docker 且容器在运行）"),
                e,
            )
        })?;

        // slave 用完即 drop：slave 端已被 spawn_command 接管，重复持有会延迟 master close。
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| pty_err("PTY reader clone 失败", e))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| pty_err("PTY writer take 失败", e))?;

        let (chunk_tx, chunk_rx): (SyncSender<PtyChunk>, Receiver<PtyChunk>) =
            mpsc::sync_channel(PTY_CHANNEL_CAPACITY);
        let handle = thread::Builder::new()
            .name(format!("pty-reader-{container}"))
            .spawn(move || reader_loop(&mut reader, &chunk_tx))
            .map_err(|e| ProcError::docker(format!("spawn PTY reader thread 失败: {e}")))?;

        Ok(Self {
            master: pair.master,
            writer,
            child,
            chunk_rx,
            reader_thread: Some(handle),
            container: container.to_string(),
            cols,
            rows,
        })
    }

    /// 主线程 tick：drain 所有可用 chunk，拼接成单一字节流返回。
    ///
    /// 调用方把返回的字节喂给 vt100 parser。
    #[must_use]
    pub fn drain(&self) -> Vec<u8> {
        let mut out = Vec::new();
        while let Ok(chunk) = self.chunk_rx.try_recv() {
            out.extend_from_slice(&chunk.bytes);
        }
        out
    }

    /// 主线程：把按键转换的字节写进 PTY（容器 stdin）。
    pub fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer
            .write_all(bytes)
            .map_err(|e| ProcError::docker(format!("PTY write 失败: {e}")))?;
        self.writer
            .flush()
            .map_err(|e| ProcError::docker(format!("PTY flush 失败: {e}")))?;
        Ok(())
    }

    /// 终端 resize。同尺寸跳过；0 跳过（终端初始化阶段）。
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        if cols == 0 || rows == 0 {
            return Ok(());
        }
        if cols == self.cols && rows == self.rows {
            return Ok(());
        }
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| pty_err("PTY resize 失败", e))?;
        self.cols = cols;
        self.rows = rows;
        Ok(())
    }

    /// 子进程是否已退出（用户输入 exit / Ctrl+D / Ctrl+\）。
    pub fn is_finished(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(_) => true,
        }
    }

    /// 当前 PTY 尺寸。
    #[must_use]
    pub fn size(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }
}

impl Drop for ContainerExec {
    fn drop(&mut self) {
        // 1) kill child：触发容器内 shell 退出 → docker exec stdout 关闭 →
        //    master.reader 拿到 EOF → reader_loop 退出。
        let _ = self.child.kill();

        // 2) join reader thread。read 在 child 退出后立即返回 0（EOF），
        //    通常 <10ms。如果 reader 卡死（极端：kernel PTY bug），进程退出时
        //    OS 回收线程，可接受（不能让主线程卡住）。
        if let Some(handle) = self.reader_thread.take() {
            let join_result = wait_with_timeout(handle, READER_JOIN_TIMEOUT);
            if join_result.is_none() {
                tracing::warn!(
                    "pty-reader-{} 在 {}ms 内未退出（已 detach）",
                    self.container,
                    READER_JOIN_TIMEOUT.as_millis()
                );
            }
        }
        // 3) master / writer / child 自动 drop → PTY fd 关闭。
    }
}

/// 给 JoinHandle 加超时：spawn 一个 detach 线程做 join，主线程在 timeout 内拿结果。
/// 超时返回 None（线程被 detach，进程退出时 OS 回收）。
fn wait_with_timeout(handle: JoinHandle<()>, timeout: Duration) -> Option<()> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = handle.join();
        let _ = tx.send(());
    });
    rx.recv_timeout(timeout).ok()
}

/// reader 线程主体：循环 read → 推 chunk。
///
/// reader.read 在以下情况返回：
/// - Ok(0)：EOF（child 退出 / master 关闭）→ 退出循环。
/// - Ok(n)：读到 n 字节 → 推 chunk。
/// - Err(Interrupted)：重试。
/// - Err(其它)：退出。
///
/// 主线程 drop chunk_rx 后，`chunk_tx.send` 返回 Err → 退出循环。
fn reader_loop(reader: &mut Box<dyn Read + Send>, chunk_tx: &SyncSender<PtyChunk>) {
    let mut buf = [0u8; PTY_READ_BUF];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let chunk = PtyChunk {
                    bytes: buf[..n].to_vec(),
                };
                if chunk_tx.send(chunk).is_err() {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

/// 根据镜像名推断默认 shell（ADR-0007）。
///
/// - alpine / busybox → `/bin/sh`（POSIX shell，alpine 的 ash 是 /bin/sh 软链）
/// - ubuntu / debian / centos / fedora / rust / golang / python / node → `/bin/bash`
/// - 其它 / 空 → `/bin/sh`（POSIX 兜底，几乎所有 Linux 镜像都有）
///
/// # 示例
///
/// ```
/// use proc::docker::exec::detect_default_shell;
/// assert_eq!(detect_default_shell("alpine:3.18"), "/bin/sh");
/// assert_eq!(detect_default_shell("ubuntu:22.04"), "/bin/bash");
/// assert_eq!(detect_default_shell("nginx:latest"), "/bin/sh");
/// ```
#[must_use]
pub fn detect_default_shell(image: &str) -> &'static str {
    let lower = image.to_lowercase();
    if lower.contains("alpine") || lower.contains("busybox") {
        "/bin/sh"
    } else if lower.contains("ubuntu")
        || lower.contains("debian")
        || lower.contains("centos")
        || lower.contains("fedora")
        || lower.contains("rust")
        || lower.contains("golang")
        || lower.contains("python")
        || lower.contains("node")
    {
        "/bin/bash"
    } else {
        "/bin/sh"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── detect_default_shell ──

    #[test]
    fn shell_alpine() {
        assert_eq!(detect_default_shell("alpine:3.18"), "/bin/sh");
        assert_eq!(detect_default_shell("ALPINE:latest"), "/bin/sh");
        assert_eq!(detect_default_shell("docker.io/library/alpine"), "/bin/sh");
    }

    #[test]
    fn shell_busybox() {
        assert_eq!(detect_default_shell("busybox:stable"), "/bin/sh");
    }

    #[test]
    fn shell_ubuntu_family() {
        assert_eq!(detect_default_shell("ubuntu:22.04"), "/bin/bash");
        assert_eq!(detect_default_shell("debian:bookworm"), "/bin/bash");
        assert_eq!(detect_default_shell("centos:stream9"), "/bin/bash");
        assert_eq!(detect_default_shell("fedora:40"), "/bin/bash");
    }

    #[test]
    fn shell_dev_images() {
        assert_eq!(detect_default_shell("rust:1.75"), "/bin/bash");
        assert_eq!(detect_default_shell("golang:1.21"), "/bin/bash");
        assert_eq!(detect_default_shell("python:3.12"), "/bin/bash");
        assert_eq!(detect_default_shell("node:20"), "/bin/bash");
    }

    #[test]
    fn shell_unknown_falls_back_to_sh() {
        assert_eq!(detect_default_shell("nginx:latest"), "/bin/sh");
        assert_eq!(detect_default_shell("postgres:16"), "/bin/sh");
        assert_eq!(detect_default_shell(""), "/bin/sh");
        assert_eq!(detect_default_shell("my-custom-image"), "/bin/sh");
    }

    // ── PtyChunk ──

    #[test]
    fn pty_chunk_default_empty() {
        let c = PtyChunk::default();
        assert!(c.bytes.is_empty());
    }
}
