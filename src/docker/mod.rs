pub mod events;
pub mod health;
pub mod stats;

use std::time::SystemTime;

use crate::error::{ProcError, Result};

/// 容器健康状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    Unhealthy,
    Starting,
    NotConfigured,
}

impl std::fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Healthy => write!(f, "healthy"),
            Self::Unhealthy => write!(f, "unhealthy"),
            Self::Starting => write!(f, "starting"),
            Self::NotConfigured => write!(f, "-"),
        }
    }
}

impl HealthStatus {
    /// 从 Docker ContainerSummary.Status 字符串解析健康状态
    pub fn from_status(status: &str) -> Self {
        if status.contains("(healthy)") {
            Self::Healthy
        } else if status.contains("(unhealthy)") {
            Self::Unhealthy
        } else if status.contains("(health: starting)") {
            Self::Starting
        } else {
            Self::NotConfigured
        }
    }
}

/// 容器简要信息
#[derive(Debug, Clone)]
pub struct ContainerInfo {
    pub id: String,
    pub name: String,
    pub image: String,
    pub status: String,
    pub state: String,
    pub health: HealthStatus,
    pub cpu_percent: f64,
    pub memory_usage: u64,
    pub network_in: u64,
    pub network_out: u64,
    pub running_since: Option<SystemTime>,
    pub ports: String,
}

/// Docker 监控管理器
pub struct DockerMonitor {
    runtime: tokio::runtime::Runtime,
    docker: bollard::Docker,
}

impl DockerMonitor {
    /// 连接 Docker Engine，依次尝试命名管道（Docker Desktop）和 TCP（WSL Docker），
    /// 用 list_containers API 调用验证连接真实可用。
    pub fn connect() -> Result<Self> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| ProcError::Docker(format!("无法创建 tokio 运行时: {}", e)))?;

        let mut last_err = String::new();

        // Try 1: Named pipe (Docker Desktop for Windows)
        if let Ok(docker) =
            runtime.block_on(async { bollard::Docker::connect_with_socket_defaults() })
        {
            match Self::verify_connection(docker, &runtime, "命名管道") {
                Ok(docker) => return Ok(Self { runtime, docker }),
                Err(e) => last_err = e,
            }
        }

        // Try 2: TCP localhost:2375 (WSL Docker with TCP enabled)
        if let Ok(docker) =
            runtime.block_on(async { bollard::Docker::connect_with_http_defaults() })
        {
            match Self::verify_connection(docker, &runtime, "TCP") {
                Ok(docker) => return Ok(Self { runtime, docker }),
                Err(e) => last_err = e,
            }
        }

        Err(ProcError::Docker(format!(
            "Docker 未运行或未安装 (已尝试命名管道和 TCP 连接{})",
            if last_err.is_empty() {
                String::new()
            } else {
                format!(": {}", last_err)
            }
        )))
    }

    fn verify_connection(
        docker: bollard::Docker,
        runtime: &tokio::runtime::Runtime,
        label: &str,
    ) -> std::result::Result<bollard::Docker, String> {
        use bollard::container::ListContainersOptions;

        let opts: ListContainersOptions<String> = ListContainersOptions {
            all: false,
            ..Default::default()
        };

        runtime
            .block_on(async { docker.list_containers(Some(opts)).await })
            .map(|_| docker)
            .map_err(|e| format!("{} 验证失败: {}", label, e))
    }

    /// 获取 Docker 客户端的克隆（用于事件监听线程）
    pub fn docker(&self) -> bollard::Docker {
        self.docker.clone()
    }

    /// 列出所有容器
    pub fn list_containers(&self, include_stopped: bool) -> Result<Vec<ContainerInfo>> {
        use bollard::container::ListContainersOptions;

        let options: ListContainersOptions<String> = ListContainersOptions {
            all: include_stopped,
            ..Default::default()
        };

        let containers = self
            .runtime
            .block_on(async { self.docker.list_containers(Some(options)).await })
            .map_err(|e| ProcError::Docker(format!("获取容器列表失败: {}", e)))?;

        let mut result = Vec::new();
        for c in containers {
            let id = c.id.unwrap_or_default();
            let short_id = if id.len() > 12 { &id[..12] } else { &id };

            let name = c
                .names
                .as_ref()
                .and_then(|names| names.first())
                .map(|n| n.trim_start_matches('/').to_string())
                .unwrap_or_else(|| short_id.to_string());

            let image = c.image.unwrap_or_default();
            // image 可能是 sha256:xxx 形式，取短 ID
            let image_display = if let Some(hash) = image.strip_prefix("sha256:") {
                if hash.len() > 12 { &hash[..12] } else { hash }.to_string()
            } else {
                image.clone()
            };

            let state = c.state.unwrap_or_default();
            let status = c.status.unwrap_or_default();
            let health = HealthStatus::from_status(&status);

            let running_since = if state == "running" {
                // Docker 返回 created 为 Unix 时间戳（秒）
                c.created.map(|ts| {
                    let dur = std::time::Duration::from_secs(ts as u64);
                    SystemTime::UNIX_EPOCH + dur
                })
            } else {
                None
            };

            let ports = format_ports(&c.ports);

            result.push(ContainerInfo {
                id: short_id.to_string(),
                name,
                image: image_display,
                status,
                state,
                health,
                cpu_percent: 0.0,
                memory_usage: 0,
                network_in: 0,
                network_out: 0,
                running_since,
                ports,
            });
        }

        // 运行中的容器排在前面
        result.sort_by(|a, b| {
            let a_running = a.state == "running";
            let b_running = b.state == "running";
            b_running.cmp(&a_running).then_with(|| a.name.cmp(&b.name))
        });

        Ok(result)
    }

    /// 重启容器
    pub fn restart_container(&self, name: &str) -> Result<()> {
        self.runtime
            .block_on(async { self.docker.restart_container(name, None).await })
            .map_err(|e| ProcError::Docker(format!("重启容器 {} 失败: {}", name, e)))?;
        Ok(())
    }

    /// 停止容器
    pub fn stop_container(&self, name: &str) -> Result<()> {
        self.runtime
            .block_on(async { self.docker.stop_container(name, None).await })
            .map_err(|e| ProcError::Docker(format!("停止容器 {} 失败: {}", name, e)))?;
        Ok(())
    }

    /// 查询容器健康状态
    pub fn inspect_health(&self, name: &str) -> Result<health::HealthInfo> {
        health::inspect_container_health(&self.runtime, &self.docker, name)
    }

    /// 获取容器资源统计
    pub fn get_stats(&self, name: &str) -> Result<stats::ContainerStats> {
        stats::get_container_stats(&self.runtime, &self.docker, name)
    }
}

fn format_ports(ports: &Option<Vec<bollard::models::Port>>) -> String {
    let Some(ports) = ports else {
        return String::new();
    };
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for p in ports {
        if let Some(public_port) = p.public_port {
            let key = format!("{}:{}", public_port, p.private_port);
            if seen.insert(key.clone()) {
                result.push(key);
            }
        }
    }
    result.join(", ")
}
