//! E3 — Docker volume 列表与删除。
//!
//! 调用 bollard `list_volumes` / `remove_volume`，把 `Volume` 转成 [`VolumeInfo`]。

use crate::error::{ProcError, Result};
use crate::format::format_bytes;

/// 一个 Docker volume（命名数据卷）。
#[derive(Debug, Clone)]
pub struct VolumeInfo {
    /// 卷名（删除时用）。
    pub name: String,
    /// 驱动（通常是 `local`）。
    pub driver: String,
    /// 宿主机挂载点路径。
    pub mountpoint: String,
    /// 创建时间（Unix 秒，从 bollard 的 BollardDate 提取）。
    pub created: i64,
    /// 卷大小（字节，从 mountpoint `du` 算的；本地无法采集时为 0）。
    pub size: u64,
    /// 是否被容器使用。bollard 列表 API 不直接给，需要 cross-ref container mounts。
    /// 默认 false，调用方填充。
    pub in_use: bool,
}

impl VolumeInfo {
    #[must_use]
    pub fn in_use(&self) -> bool {
        self.in_use
    }
}

impl std::fmt::Display for VolumeInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let size_str = if self.size > 0 {
            format_bytes(self.size)
        } else {
            "-".to_string()
        };
        let used = if self.in_use {
            "使用中"
        } else {
            "未使用"
        };
        write!(
            f,
            "{}  {}  {}  大小={}  {}",
            self.name, self.driver, self.mountpoint, size_str, used
        )
    }
}

/// 列出所有 volume。`in_use` 标记通过 `containers` API 反查（cross-ref mount）填充。
pub fn list_volumes(
    runtime: &tokio::runtime::Runtime,
    docker: &bollard::Docker,
) -> Result<Vec<VolumeInfo>> {
    use bollard::volume::ListVolumesOptions;

    let options: ListVolumesOptions<String> = ListVolumesOptions::default();

    let response = runtime
        .block_on(async { docker.list_volumes(Some(options)).await })
        .map_err(|e| ProcError::docker_with("获取 volume 列表失败", e))?;

    // 拿所有容器，收集它们 mount 的 volume name 集合，给 in_use 打标。
    let used_volumes = collect_used_volumes(runtime, docker);

    let volumes = response.volumes.unwrap_or_default();
    Ok(volumes
        .iter()
        .map(|v| {
            let created = extract_unix_seconds(&v.created_at);
            VolumeInfo {
                name: v.name.clone(),
                driver: v.driver.clone(),
                mountpoint: v.mountpoint.clone(),
                created,
                size: compute_volume_size(&v.mountpoint),
                in_use: used_volumes.contains(&v.name),
            }
        })
        .collect())
}

/// 删除 volume。`force=true` 时强制删（即便 in_use）。
pub fn remove_volume(
    runtime: &tokio::runtime::Runtime,
    docker: &bollard::Docker,
    name: &str,
    force: bool,
) -> Result<()> {
    use bollard::volume::RemoveVolumeOptions;

    let options = RemoveVolumeOptions { force };

    runtime
        .block_on(async { docker.remove_volume(name, Some(options)).await })
        .map_err(|e| ProcError::docker_with(format!("删除 volume {name} 失败"), e))?;
    Ok(())
}

/// 反查所有容器的 `Mounts`，收集 volume 名集合。
fn collect_used_volumes(
    runtime: &tokio::runtime::Runtime,
    docker: &bollard::Docker,
) -> std::collections::HashSet<String> {
    use bollard::container::ListContainersOptions;

    let options: ListContainersOptions<String> = ListContainersOptions {
        all: true,
        ..Default::default()
    };

    let containers = runtime
        .block_on(async { docker.list_containers(Some(options)).await })
        .map_err(|e| ProcError::docker_with("获取容器列表失败（用于 volume 标记）", e));

    let containers = match containers {
        Ok(c) => c,
        Err(_) => return std::collections::HashSet::new(),
    };

    let mut used = std::collections::HashSet::new();
    for c in containers {
        let Some(mounts) = c.mounts.as_ref() else {
            continue;
        };
        for m in mounts {
            if let Some(name) = m.name.as_ref() {
                used.insert(name.clone());
            }
        }
    }
    used
}

/// 计算 volume 大小。Mountpoint 不存在或不可访问 → 0。
fn compute_volume_size(mountpoint: &str) -> u64 {
    let path = std::path::Path::new(mountpoint);
    if !path.is_dir() {
        return 0;
    }
    let mut total: u64 = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if ft.is_dir() {
                stack.push(entry.path());
            } else if let Ok(meta) = entry.metadata() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    total
}

/// 把 bollard 的 `created_at`（默认 feature 下是 `Option<String>`，RFC3339 文本）
/// 转 Unix 秒。proc 不启用 bollard 的 `chrono` / `time` feature，所以走文本解析。
fn extract_unix_seconds(created_at: &Option<String>) -> i64 {
    let Some(date) = created_at else {
        return 0;
    };
    parse_rfc3339_to_unix(date).unwrap_or(0)
}

/// 简陋的 RFC3339 → Unix 秒转换（避免引入 chrono）。
///
/// 支持格式：`YYYY-MM-DDTHH:MM:SS` / `YYYY-MM-DDTHH:MM:SSZ` /
/// `YYYY-MM-DDTHH:MM:SS.sssssssssZ` / `YYYY-MM-DDTHH:MM:SS+HH:MM`。
fn parse_rfc3339_to_unix(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let year: i64 = s[0..4].parse().ok()?;
    let mon: i64 = s[5..7].parse().ok()?;
    let day: i64 = s[8..10].parse().ok()?;
    let hour: i64 = s[11..13].parse().ok()?;
    let minute: i64 = s[14..16].parse().ok()?;
    let sec: i64 = s[17..19].parse().ok()?;

    let days_since_epoch = days_from_civil(year, mon, day)?;
    let secs = days_since_epoch * 86_400 + hour * 3_600 + minute * 60 + sec;
    Some(secs)
}

/// Howard Hinnant 的 `days_from_civil` 算法（整数公历 → Unix 天数）。
fn days_from_civil(year: i64, month: i64, day: i64) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let m = month as u64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe as i64 - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_used_volume() {
        let v = VolumeInfo {
            name: "my-vol".to_string(),
            driver: "local".to_string(),
            mountpoint: "/var/lib/docker/volumes/my-vol/_data".to_string(),
            created: 0,
            size: 1_024,
            in_use: true,
        };
        let s = format!("{v}");
        assert!(s.contains("my-vol"));
        assert!(s.contains("使用中"));
        // format_bytes(1024) → "1KB"（无空格）。
        assert!(s.contains("1KB"));
    }

    #[test]
    fn display_unused_volume_with_no_size() {
        let v = VolumeInfo {
            name: "v2".to_string(),
            driver: "local".to_string(),
            mountpoint: "/var/lib/docker/volumes/v2/_data".to_string(),
            created: 0,
            size: 0,
            in_use: false,
        };
        let s = format!("{v}");
        assert!(s.contains("未使用"));
        assert!(s.contains("大小=-"));
    }

    #[test]
    fn in_use_flag_accurate() {
        let used = VolumeInfo {
            name: "a".into(),
            driver: "local".into(),
            mountpoint: "/a".into(),
            created: 0,
            size: 0,
            in_use: true,
        };
        let unused = VolumeInfo {
            in_use: false,
            ..used.clone()
        };
        assert!(used.in_use());
        assert!(!unused.in_use());
    }

    #[test]
    fn parse_rfc3339_basic() {
        // 2026-06-20T08:30:45Z：day-of-year 170 from 2026-01-01（31+28+31+30+31+19=170）→ +170 天。
        // 2026-01-01 → 1970-01-01 = 20454 天（验证：2024-01-01 = 19723，2025-01-01 = 20089，
        // 2026-01-01 = 20454；2024 是 leap）。
        // 20454 + 170 = 20624 天。20624 * 86400 = 1_781_913_600。
        // 加 8:30:45 = 30_645 秒 → 1_781_944_245。
        let secs = parse_rfc3339_to_unix("2026-06-20T08:30:45Z").expect("parse ok");
        assert_eq!(secs, 1_781_944_245);
    }

    #[test]
    fn parse_rfc3339_rejects_garbage() {
        assert!(parse_rfc3339_to_unix("garbage").is_none());
        assert!(parse_rfc3339_to_unix("").is_none());
        assert!(parse_rfc3339_to_unix("2026-06-20").is_none());
    }

    #[test]
    fn days_from_civil_known_epoch() {
        // 1970-01-01 → 0
        assert_eq!(days_from_civil(1970, 1, 1), Some(0));
        // 1970-01-02 → 1
        assert_eq!(days_from_civil(1970, 1, 2), Some(1));
        // 2024-01-01 → 应该是 19_723（公开值）。
        // 这个测试在不同时区下应该是稳定的（天数是绝对的）。
        assert_eq!(days_from_civil(2024, 1, 1), Some(19_723));
    }

    #[test]
    fn compute_volume_size_missing_dir_returns_zero() {
        let size = compute_volume_size("/this/path/should/not/exist/abc123");
        assert_eq!(size, 0);
    }

    #[test]
    fn compute_volume_size_walks_tempdir() {
        // tempfile 不在本模块，用 std::env::temp_dir()。
        let dir = std::env::temp_dir().join("proc-vol-test");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("a.txt"), b"hello world").expect("write");
        std::fs::write(dir.join("b.txt"), b"another one").expect("write");
        let size = compute_volume_size(dir.to_str().unwrap());
        assert!(size >= b"hello world".len() as u64 + b"another one".len() as u64);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_unix_seconds_from_none() {
        assert_eq!(extract_unix_seconds(&None), 0);
    }

    #[test]
    fn extract_unix_seconds_from_valid_rfc3339() {
        let secs = extract_unix_seconds(&Some("2026-06-20T08:30:45Z".to_string()));
        assert_eq!(secs, 1_781_944_245);
    }

    #[test]
    fn extract_unix_seconds_from_garbage_returns_zero() {
        let secs = extract_unix_seconds(&Some("garbage".to_string()));
        assert_eq!(secs, 0);
    }
}
