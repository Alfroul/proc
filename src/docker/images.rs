//! E3 — Docker 镜像列表与删除。
//!
//! 调用 bollard `list_images` / `remove_image`，把 `ImageSummary` 转成 [`ImageInfo`]。
//! `Display` 实现给 CLI / TUI 复用。

use crate::error::{ProcError, Result};
use crate::format::format_bytes;

/// 本地镜像信息。`in_use` 是从 `ImageSummary.containers` 推导的「正被使用」标记。
#[derive(Debug, Clone)]
pub struct ImageInfo {
    /// 完整 ID（`sha256:...`），删除时用。
    pub id: String,
    /// 短 ID（前 12 位），展示用。
    pub short_id: String,
    /// repo tags（如 `["nginx:latest", "nginx:1.25"]`）；`<none>:<none>` 时为空。
    pub repo_tags: Vec<String>,
    /// 创建时间（Unix 秒）。
    pub created: i64,
    /// 磁盘占用（字节）。
    pub size: u64,
    /// 使用该镜像的容器数（含 stopped）。
    pub containers: u64,
}

impl ImageInfo {
    /// 是否正被容器使用（删除前确认）。
    #[must_use]
    pub fn in_use(&self) -> bool {
        self.containers > 0
    }

    /// 显示名（第一个 tag；无 tag 时用 short_id）。
    #[must_use]
    pub fn display_name(&self) -> String {
        self.repo_tags
            .first()
            .cloned()
            .unwrap_or_else(|| format!("<none>:{}`", self.short_id))
    }
}

impl std::fmt::Display for ImageInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tags = if self.repo_tags.is_empty() {
            "<none>".to_string()
        } else {
            self.repo_tags.join(", ")
        };
        let id = self.id.strip_prefix("sha256:").unwrap_or(&self.id);
        let short = if id.len() > 12 { &id[..12] } else { id };
        write!(
            f,
            "{}  {}  {}  容器={}  创建={}",
            short,
            tags,
            format_bytes(self.size),
            self.containers,
            format_unix_seconds(self.created)
        )
    }
}

/// 列出本地所有镜像。
pub fn list_images(
    runtime: &tokio::runtime::Runtime,
    docker: &bollard::Docker,
) -> Result<Vec<ImageInfo>> {
    use bollard::image::ListImagesOptions;

    let options: ListImagesOptions<String> = ListImagesOptions {
        all: true,
        digests: false,
        ..Default::default()
    };

    let images = runtime
        .block_on(async { docker.list_images(Some(options)).await })
        .map_err(|e| ProcError::docker_with("获取镜像列表失败", e))?;

    Ok(images.iter().map(image_summary_to_info).collect())
}

/// 删除镜像。`force=true` 时强制删（即便 in_use）。
pub fn remove_image(
    runtime: &tokio::runtime::Runtime,
    docker: &bollard::Docker,
    id: &str,
    force: bool,
) -> Result<()> {
    use bollard::image::RemoveImageOptions;

    let options = RemoveImageOptions {
        force,
        noprune: false,
    };

    runtime
        .block_on(async { docker.remove_image(id, Some(options), None).await })
        .map_err(|e| ProcError::docker_with(format!("删除镜像 {id} 失败"), e))?;
    Ok(())
}

fn image_summary_to_info(s: &bollard::models::ImageSummary) -> ImageInfo {
    let id = s.id.clone();
    let stripped = id.strip_prefix("sha256:").unwrap_or(&id);
    let short_id = if stripped.len() > 12 {
        stripped[..12].to_string()
    } else {
        stripped.to_string()
    };

    ImageInfo {
        id,
        short_id,
        // bollard 给的 `<none>:<none>` tag 过滤掉，UI 不展示。
        repo_tags: s
            .repo_tags
            .iter()
            .filter(|t| !t.starts_with("<none>"))
            .cloned()
            .collect(),
        created: s.created,
        size: u64::try_from(s.size).unwrap_or(0),
        containers: u64::try_from(s.containers).unwrap_or(0),
    }
}

/// 把 Unix 秒转成简短的本地日期描述（不引入 chrono 依赖）。
fn format_unix_seconds(secs: i64) -> String {
    if secs <= 0 {
        return "-".to_string();
    }
    // 粗略：转天数。精确日期需要 chrono / time，UI 不需要精确到日。
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(secs);
    let delta_secs = now_secs.saturating_sub(secs);
    if delta_secs < 60 {
        return format!("{}秒前", delta_secs);
    }
    if delta_secs < 3600 {
        return format!("{}分前", delta_secs / 60);
    }
    if delta_secs < 86400 {
        return format!("{}时前", delta_secs / 3600);
    }
    format!("{}天前", delta_secs / 86400)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_with_tags() {
        let info = ImageInfo {
            id: "sha256:abcdef1234567890abcdef".to_string(),
            short_id: "abcdef123456".to_string(),
            repo_tags: vec!["nginx:latest".to_string()],
            created: 0,
            size: 50_000_000,
            containers: 2,
        };
        let s = format!("{info}");
        assert!(s.contains("abcdef123456"));
        assert!(s.contains("nginx:latest"));
        assert!(s.contains("容器=2"));
    }

    #[test]
    fn display_without_tags_shows_none() {
        let info = ImageInfo {
            id: "sha256:abcdef1234567890".to_string(),
            short_id: "abcdef123456".to_string(),
            repo_tags: Vec::new(),
            created: 0,
            size: 0,
            containers: 0,
        };
        let s = format!("{info}");
        assert!(s.contains("<none>"));
    }

    #[test]
    fn in_use_flag_correct() {
        let used = ImageInfo {
            id: "x".into(),
            short_id: "x".into(),
            repo_tags: vec!["a".into()],
            created: 0,
            size: 0,
            containers: 1,
        };
        let unused = ImageInfo {
            containers: 0,
            ..used.clone()
        };
        assert!(used.in_use());
        assert!(!unused.in_use());
    }

    #[test]
    fn display_name_prefers_first_tag() {
        let info = ImageInfo {
            id: "x".into(),
            short_id: "abc123def456".into(),
            repo_tags: vec!["nginx:latest".into(), "nginx:1.25".into()],
            created: 0,
            size: 0,
            containers: 0,
        };
        assert_eq!(info.display_name(), "nginx:latest");
    }

    #[test]
    fn display_name_falls_back_to_short_id() {
        let info = ImageInfo {
            id: "x".into(),
            short_id: "abc123def456".into(),
            repo_tags: Vec::new(),
            created: 0,
            size: 0,
            containers: 0,
        };
        assert!(info.display_name().contains("abc123def456"));
    }

    #[test]
    fn format_unix_seconds_handles_zero() {
        assert_eq!(format_unix_seconds(0), "-");
        assert_eq!(format_unix_seconds(-5), "-");
    }
}
