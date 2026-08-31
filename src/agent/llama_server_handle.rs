//! LlamaServerHandle — llama-server 子进程管理（brainstorm 决策 6 + ADR-0030 D4）。
//!
//! 动态端口分配（bind 127.0.0.1:0 → OS 分配 → 立即释放）+ spawn（`--reasoning off`
//! 禁用 Gemma 4 thinking mode，ADR-0030 D6——b8685 实测无 `--no-thinks` flag，
//! 等效迁移到 `--reasoning off`）+ `/health` 轮询就绪 + Drop 时 kill 防僵尸。
//!
//! 按需 spawn 核心约束：仅用户显式跑 `proc agent ask` 时才 spawn（provider 惰性
//! 触发），命令结束（Drop）→ kill 子进程 + 释放 RAM / 端口，日常使用零影响。

use std::io::Read;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::provider::LlmError;

/// brainstorm 决策 4 拍板的默认上下文长度（agent.toml `[llama-cpp].ctx_size` 可覆盖）。
pub const DEFAULT_CTX_SIZE: u32 = 8192;

/// `/health` 轮询间隔。
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// stderr 尾部环形缓冲上限（超时诊断用，超出丢头部）。
const STDERR_TAIL_CAP: usize = 8 * 1024;

/// llama-server 启动参数（agent.toml `[llama-cpp]` 段映射）。
#[derive(Debug, Clone)]
pub struct SpawnOptions {
    /// `--ctx-size N`；None → [`DEFAULT_CTX_SIZE`]。
    pub ctx_size: Option<u32>,
    /// true（默认）→ `--reasoning off`（ADR-0030 D6 禁用 Gemma 4 thinking mode）。
    pub no_thinks: bool,
    /// `--chat-template <name>` 用户显式覆盖位；None（默认）→ 用模型 GGUF
    /// metadata 自带模板（b8685 实测：显式 gemma + `--jinja` 组合会丢 user
    /// content，决策 F）。
    pub chat_template: Option<String>,
    /// `/health` 就绪轮询上限（model load 预算）。
    pub startup_timeout: Duration,
}

impl Default for SpawnOptions {
    fn default() -> Self {
        Self {
            ctx_size: None,
            no_thinks: true,
            chat_template: None,
            startup_timeout: Duration::from_secs(120),
        }
    }
}

/// 动态端口分配：bind 127.0.0.1:0 → OS 分配 → 立即释放（决策 6 端口策略；
/// OS 保证该端口在 TIME_WAIT 之前不会再分配给其他 listener）。
pub fn allocate_port() -> std::io::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

/// 构造 llama-server 启动命令（纯函数，测试断言 flag 集合）。
///
/// flag 集：`--model` / `--host 127.0.0.1` / `--port N` / `--jinja` /
/// `--ctx-size 8192` / `--reasoning off`。
///
/// **实测更正（b8685，2026-08-15，stage 3a 决策 F）**：
/// - `--chat-template gemma` 不传——与 `--jinja` 组合会把 user content 渲染丢
///   （prompt_tokens=3、模型自由续写）；GGUF metadata 自带 gemma 模板 +
///   `--jinja` 才正确渲染。`SpawnOptions.chat_template` 保留用户显式覆盖位。
/// - `--special` 不传——副作用是 content 尾部泄漏 `<turn|>` 字面量；tool_calls
///   解析由 server 模板层处理，不依赖该 flag。
pub fn build_spawn_command(
    server_path: &Path,
    model_path: &Path,
    port: u16,
    opts: &SpawnOptions,
) -> Command {
    let mut cmd = Command::new(server_path);
    cmd.arg("--model")
        .arg(model_path)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--jinja")
        .arg("--ctx-size")
        .arg(opts.ctx_size.unwrap_or(DEFAULT_CTX_SIZE).to_string());
    if let Some(template) = &opts.chat_template {
        cmd.arg("--chat-template").arg(template);
    }
    if opts.no_thinks {
        // b8685 实测：--no-thinks 已不存在，等效 flag 是 --reasoning off。
        cmd.arg("--reasoning").arg("off");
    }
    cmd
}

/// llama-server 子进程句柄（Drop 自动 kill + wait）。
pub struct LlamaServerHandle {
    child: Child,
    port: u16,
    base_url: String,
    health_client: reqwest::Client,
}

impl LlamaServerHandle {
    /// spawn llama-server 并等待就绪（`/health` 返回 2xx）。
    ///
    /// 失败模式（均返 [`LlmError::Config`]，错误消息可诊断）：
    /// - server_path / model_path 不存在
    /// - 子进程 spawn 失败（flag 不识别 / 权限）
    /// - 子进程提前退出（模型损坏 / 显存不足，附 stderr 尾部）
    /// - startup_timeout 内 `/health` 无响应（附 stderr 尾部）
    pub async fn spawn(
        server_path: &Path,
        model_path: &Path,
        opts: SpawnOptions,
    ) -> Result<Self, LlmError> {
        if !server_path.exists() {
            return Err(LlmError::Config(format!(
                "llama-server 不存在: {}（检查 agent.toml [llama-cpp].server_path）",
                server_path.display()
            )));
        }
        if !model_path.exists() {
            return Err(LlmError::Config(format!(
                "模型文件不存在: {}（检查 agent.toml [default].model / [llama-cpp].search_paths）",
                model_path.display()
            )));
        }
        let port = allocate_port()?;
        let mut cmd = build_spawn_command(server_path, model_path, port, &opts);
        cmd.stdout(Stdio::null()).stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| {
            LlmError::Config(format!(
                "llama-server spawn 失败: {e}（路径 {}）",
                server_path.display()
            ))
        })?;
        let stderr_tail = Arc::new(Mutex::new(Vec::new()));
        if let Some(stderr) = child.stderr.take() {
            let tail = stderr_tail.clone();
            std::thread::spawn(move || drain_stderr(stderr, tail));
        }
        let base_url = format!("http://127.0.0.1:{port}");
        let health_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()?;
        let deadline = tokio::time::Instant::now() + opts.startup_timeout;
        loop {
            if let Some(status) = child.try_wait()? {
                let _ = child.kill();
                let _ = child.wait();
                return Err(LlmError::Config(format!(
                    "llama-server 提前退出（exit={:?}）stderr 尾部:\n{}",
                    status.code(),
                    stderr_tail_string(&stderr_tail)
                )));
            }
            if let Ok(resp) = health_client.get(format!("{base_url}/health")).send().await {
                if resp.status().is_success() {
                    break;
                }
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(LlmError::Config(format!(
                    "llama-server 在 {}s 内未就绪（/health 无响应，模型加载失败或启动参数有误）stderr 尾部:\n{}",
                    opts.startup_timeout.as_secs(),
                    stderr_tail_string(&stderr_tail)
                )));
            }
            tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
        }
        Ok(Self {
            child,
            port,
            base_url,
            health_client,
        })
    }

    /// 单次健康检查（`/health`，2s 超时）。
    pub async fn health_check(&self) -> Result<(), LlmError> {
        let resp = self
            .health_client
            .get(format!("{}/health", self.base_url))
            .send()
            .await?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(LlmError::Api {
                status: resp.status().as_u16(),
                body: "/health non-2xx".to_string(),
            })
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// 子进程 PID。drop 清理断言按 PID 查退出（`tasklist /FI "PID eq N"`），
    /// 不再全局扫 llama-server.exe——同 binary 内其他测试的 server 存活时
    /// 全局扫描会误报（v0.26 stage 2 R1 竞态修复）。
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

impl Drop for LlamaServerHandle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// stderr drain 线程主体：持续读防 pipe buffer 满阻塞子进程，只保留尾部。
fn drain_stderr<R: Read>(mut stderr: R, tail: Arc<Mutex<Vec<u8>>>) {
    let mut buf = [0u8; 4096];
    loop {
        match stderr.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut t = match tail.lock() {
                    Ok(t) => t,
                    Err(_) => break,
                };
                t.extend_from_slice(&buf[..n]);
                let overflow = t.len().saturating_sub(STDERR_TAIL_CAP);
                if overflow > 0 {
                    t.drain(..overflow);
                }
            }
        }
    }
}

fn stderr_tail_string(tail: &Arc<Mutex<Vec<u8>>>) -> String {
    tail.lock()
        .map(|t| String::from_utf8_lossy(&t).to_string())
        .unwrap_or_default()
}
