//! 持久化 UI 偏好（进程列表排序字段）。文件路径 `~/.config/proc/ui.toml`。
//!
//! 容错策略：文件缺失 / 损坏 / 字段越界 / 写入失败均静默回退到内置默认值，
//! 不阻塞启动也不打扰用户。文件格式是手写 TOML 子集（`sort_field = "memory"`），
//! 避免引入 toml crate 依赖。

use crate::collect::SortField;

fn path() -> Option<std::path::PathBuf> {
    Some(crate::dirs_config_dir().join("ui.toml"))
}

pub fn load_sort_field() -> Option<SortField> {
    let raw = std::fs::read_to_string(path()?).ok()?;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if let Some((key, val)) = trimmed.split_once('=')
            && key.trim() == "sort_field"
        {
            let v = val.trim().trim_matches('"');
            return SortField::parse_from_str(v);
        }
    }
    None
}

pub fn save_sort_field(field: SortField) {
    let Some(p) = path() else { return };
    if let Some(parent) = p.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return;
    }
    let body = format!(
        "# proc UI 偏好 — 由 proc 自动维护\nsort_field = \"{}\"\n",
        field.as_str()
    );
    let _ = std::fs::write(p, body);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_field_roundtrip() {
        for f in [
            SortField::Cpu,
            SortField::Memory,
            SortField::Pid,
            SortField::Name,
            SortField::Security,
            SortField::DiskRead,
            SortField::DiskWrite,
        ] {
            assert_eq!(SortField::parse_from_str(f.as_str()), Some(f));
        }
    }

    #[test]
    fn from_str_rejects_unknown() {
        assert_eq!(SortField::parse_from_str("unknown"), None);
        assert_eq!(SortField::parse_from_str(""), None);
    }

    /// 解析单行 sort_field = "..."（不依赖磁盘）
    fn parse_sort_field_from_text(text: &str) -> Option<SortField> {
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') || trimmed.is_empty() {
                continue;
            }
            if let Some((key, val)) = trimmed.split_once('=')
                && key.trim() == "sort_field"
            {
                let v = val.trim().trim_matches('"');
                return SortField::parse_from_str(v);
            }
        }
        None
    }

    #[test]
    fn parse_sort_field_accepts_valid() {
        assert_eq!(
            parse_sort_field_from_text("sort_field = \"memory\""),
            Some(SortField::Memory)
        );
    }

    #[test]
    fn parse_sort_field_skips_comments_and_blank() {
        let text = "# header\n\nsort_field = \"pid\"\n";
        assert_eq!(parse_sort_field_from_text(text), Some(SortField::Pid));
    }

    #[test]
    fn parse_sort_field_returns_none_when_missing() {
        assert_eq!(parse_sort_field_from_text("# nothing here\n"), None);
        assert_eq!(parse_sort_field_from_text(""), None);
    }
}
