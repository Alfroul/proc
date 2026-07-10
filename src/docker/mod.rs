pub mod events;
pub mod exec;
pub mod health;
pub mod images;
pub mod logs;
pub mod logs_worker;
pub mod snapshot_worker;
pub mod stats;
pub mod top;
pub mod volumes;

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
    #[must_use]
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
            .map_err(|e| ProcError::docker_with("无法创建 tokio 运行时", e))?;

        let mut last_err = String::new();

        // Try 1: Named pipe (Docker Desktop for Windows)
        if let Ok(docker) =
            runtime.block_on(async { bollard::Docker::connect_with_socket_defaults() })
        {
            match Self::verify_connection(docker, &runtime, "命名管道") {
                Ok(docker) => return Ok(Self { runtime, docker }),
                Err(e) => last_err = e.to_string(),
            }
        }

        // Try 2: TCP localhost:2375 (WSL Docker with TCP enabled)
        if let Ok(docker) =
            runtime.block_on(async { bollard::Docker::connect_with_http_defaults() })
        {
            match Self::verify_connection(docker, &runtime, "TCP") {
                Ok(docker) => return Ok(Self { runtime, docker }),
                Err(e) => last_err = e.to_string(),
            }
        }

        Err(ProcError::docker(format!(
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
    ) -> Result<bollard::Docker> {
        use bollard::container::ListContainersOptions;

        let opts: ListContainersOptions<String> = ListContainersOptions {
            all: false,
            ..Default::default()
        };

        runtime
            .block_on(async { docker.list_containers(Some(opts)).await })
            .map(|_| docker)
            .map_err(|e| ProcError::docker_with(format!("{label} 验证失败"), e))
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
            .map_err(|e| ProcError::docker_with("获取容器列表失败", e))?;

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
            .map_err(|e| ProcError::docker_with(format!("重启容器 {name} 失败"), e))?;
        Ok(())
    }

    /// 停止容器
    pub fn stop_container(&self, name: &str) -> Result<()> {
        self.runtime
            .block_on(async { self.docker.stop_container(name, None).await })
            .map_err(|e| ProcError::docker_with(format!("停止容器 {name} 失败"), e))?;
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

    /// E4 — 容器内进程列表（`docker top` 等价）。
    pub fn container_top(&self, name: &str) -> Result<Vec<top::ContainerTopProcess>> {
        top::get_container_top(&self.runtime, &self.docker, name)
    }

    /// E1 — 一次性拉容器日志（非流式，CLI 用）。
    pub fn collect_logs(&self, name: &str, tail: Option<&str>) -> Result<Vec<logs::LogLine>> {
        logs::collect_container_logs(&self.runtime, &self.docker, name, tail)
    }

    /// E3 — 列出本地所有镜像。
    pub fn list_images(&self) -> Result<Vec<images::ImageInfo>> {
        images::list_images(&self.runtime, &self.docker)
    }

    /// E3 — 删除镜像。
    pub fn remove_image(&self, id: &str, force: bool) -> Result<()> {
        images::remove_image(&self.runtime, &self.docker, id, force)
    }

    /// E3 — 列出所有 volume。
    pub fn list_volumes(&self) -> Result<Vec<volumes::VolumeInfo>> {
        volumes::list_volumes(&self.runtime, &self.docker)
    }

    /// E3 — 删除 volume。
    pub fn remove_volume(&self, name: &str, force: bool) -> Result<()> {
        volumes::remove_volume(&self.runtime, &self.docker, name, force)
    }

    /// v0.17 stage 6 — 删除容器（proc_docker_rm tool 用）。
    ///
    /// 与 [`Self::remove_image`] / [`Self::remove_volume`] 同款 `block_on` 模式。
    /// `force=true` 强制删（即便容器在 running 状态，先 kill 再 rm）；
    /// `volumes=true` 同时删除关联的匿名 volume（bollard 字段 `v`）。
    pub fn remove_container(&self, id: &str, force: bool, volumes: bool) -> Result<()> {
        use bollard::container::RemoveContainerOptions;

        let options = RemoveContainerOptions {
            force,
            v: volumes,
            link: false,
        };

        self.runtime
            .block_on(async { self.docker.remove_container(id, Some(options)).await })
            .map_err(|e| ProcError::docker_with(format!("删除容器 {id} 失败"), e))?;
        Ok(())
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
