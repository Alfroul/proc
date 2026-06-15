use crate::error::{ProcError, Result};

/// 容器健康检查详情
#[derive(Debug, Clone)]
pub enum HealthInfo {
    Healthy {
        failing_streak: i64,
        last_output: String,
    },
    Unhealthy {
        failing_streak: i64,
        last_output: String,
    },
    Starting,
    NotConfigured,
}

impl std::fmt::Display for HealthInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Healthy { failing_streak, .. } => {
                write!(f, "healthy (failing: {})", failing_streak)
            }
            Self::Unhealthy { failing_streak, .. } => {
                write!(f, "unhealthy (failing: {})", failing_streak)
            }
            Self::Starting => write!(f, "starting"),
            Self::NotConfigured => write!(f, "not configured"),
        }
    }
}

/// 查询容器健康状态
pub fn inspect_container_health(
    runtime: &tokio::runtime::Runtime,
    docker: &bollard::Docker,
    name: &str,
) -> Result<HealthInfo> {
    use bollard::container::InspectContainerOptions;

    let options = InspectContainerOptions {
        ..Default::default()
    };

    let inspect = runtime
        .block_on(async { docker.inspect_container(name, Some(options)).await })
        .map_err(|e| ProcError::Docker(format!("检查容器 {} 健康状态失败: {}", name, e)))?;

    let state = match inspect.state {
        Some(s) => s,
        None => return Ok(HealthInfo::NotConfigured),
    };

    let health = match state.health {
        Some(h) => h,
        None => return Ok(HealthInfo::NotConfigured),
    };

    let failing_streak = health.failing_streak.unwrap_or(0);
    let last_output = health
        .log
        .as_ref()
        .and_then(|logs| logs.last())
        .and_then(|entry| entry.output.clone())
        .unwrap_or_default();

    let status = health
        .status
        .unwrap_or(bollard::models::HealthStatusEnum::EMPTY);
    match status {
        bollard::models::HealthStatusEnum::HEALTHY => Ok(HealthInfo::Healthy {
            failing_streak,
            last_output,
        }),
        bollard::models::HealthStatusEnum::UNHEALTHY => Ok(HealthInfo::Unhealthy {
            failing_streak,
            last_output,
        }),
        bollard::models::HealthStatusEnum::STARTING => Ok(HealthInfo::Starting),
        _ => Ok(HealthInfo::NotConfigured),
    }
}
