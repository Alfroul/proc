use crate::error::{ProcError, Result};

/// 容器资源统计
#[derive(Debug, Clone, Default)]
pub struct ContainerStats {
    pub cpu_percent: f64,
    pub memory_usage: u64,
    pub memory_limit: u64,
    pub network_in: u64,
    pub network_out: u64,
}

/// 获取容器资源统计（单次快照）
pub fn get_container_stats(
    runtime: &tokio::runtime::Runtime,
    docker: &bollard::Docker,
    name: &str,
) -> Result<ContainerStats> {
    use bollard::container::StatsOptions;
    use futures_util::stream::TryStreamExt;

    let options = StatsOptions {
        stream: false,
        one_shot: false,
    };

    let stat = runtime
        .block_on(async { docker.stats(name, Some(options)).try_next().await })
        .map_err(|e| ProcError::Docker(format!("获取容器 {} 统计失败: {}", name, e)))?;

    let stat = match stat {
        Some(s) => s,
        None => return Ok(ContainerStats::default()),
    };

    let cpu_percent = calculate_cpu_percent(&stat);
    let (memory_usage, memory_limit) = extract_memory(&stat);
    let (network_in, network_out) = extract_network(&stat);

    Ok(ContainerStats {
        cpu_percent,
        memory_usage,
        memory_limit,
        network_in,
        network_out,
    })
}

// CPU: (cpu_delta / system_delta) * num_cpus * 100
fn calculate_cpu_percent(stats: &bollard::container::Stats) -> f64 {
    let cpu_delta = stats.cpu_stats.cpu_usage.total_usage
        .saturating_sub(stats.precpu_stats.cpu_usage.total_usage) as f64;

    let system_delta = match (
        stats.cpu_stats.system_cpu_usage,
        stats.precpu_stats.system_cpu_usage,
    ) {
        (Some(curr), Some(prev)) => curr.saturating_sub(prev) as f64,
        _ => return 0.0,
    };

    if system_delta <= 0.0 {
        return 0.0;
    }

    let num_cpus = stats
        .cpu_stats
        .online_cpus
        .unwrap_or(
            stats
                .cpu_stats
                .cpu_usage
                .percpu_usage
                .as_ref()
                .map(|v| v.len() as u64)
                .unwrap_or(1),
        ) as f64;

    (cpu_delta / system_delta) * num_cpus * 100.0
}

fn extract_memory(stats: &bollard::container::Stats) -> (u64, u64) {
    (
        stats.memory_stats.usage.unwrap_or(0),
        stats.memory_stats.limit.unwrap_or(0),
    )
}

fn extract_network(stats: &bollard::container::Stats) -> (u64, u64) {
    match &stats.networks {
        Some(networks) => {
            let mut rx = 0u64;
            let mut tx = 0u64;
            for ns in networks.values() {
                rx += ns.rx_bytes;
                tx += ns.tx_bytes;
            }
            (rx, tx)
        }
        None => (0, 0),
    }
}
