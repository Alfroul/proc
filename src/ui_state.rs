//! 持久化 UI 偏好（进程列表排序字段 + 首次启动引导 + 侧边栏展开状态）。文件路径 `~/.config/proc/ui.toml`。
//!
//! 容错策略：文件缺失 / 损坏 / 字段越界 / 写入失败均静默回退到内置默认值，
//! 不阻塞启动也不打扰用户。文件格式是手写 TOML 子集
//! （`sort_field = "memory"` + `first_run = false` + `sidebar_expanded = false`），
//! 避免引入 toml crate 依赖。
//!
//! 字段向后兼容：旧 ui.toml（只有 sort_field / first_run）按缺失处理，
//! sidebar_expanded 缺失默认 false（折叠）。

use crate::collect::SortField;

fn path() -> Option<std::path::PathBuf> {
    Some(crate::dirs_config_dir().join("ui.toml"))
}

/// first_run 缺省值：文件不存在 / 字段缺失都按"首次启动"处理。
const FIRST_RUN_DEFAULT: bool = true;
/// sidebar_expanded 缺省值：折叠（保持现有 sidebar 视觉布局）。
const SIDEBAR_EXPANDED_DEFAULT: bool = false;

#[must_use]
pub fn load_sort_field() -> Option<SortField> {
    let raw = std::fs::read_to_string(path()?).ok()?;
    parse_sort_field(&raw)
}

/// 读 first_run 标记。任何异常（文件缺失/损坏/字段非法）都按 [`FIRST_RUN_DEFAULT`] 返回。
#[must_use]
pub fn load_first_run() -> bool {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| parse_first_run(&raw))
        .unwrap_or(FIRST_RUN_DEFAULT)
}

/// 读 sidebar_expanded 标记。任何异常都按 [`SIDEBAR_EXPANDED_DEFAULT`] 返回，
/// 保证老用户从老 ui.toml 升级时不会突然展开。
#[must_use]
pub fn load_sidebar_expanded() -> bool {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| parse_sidebar_expanded(&raw))
        .unwrap_or(SIDEBAR_EXPANDED_DEFAULT)
}

pub fn save_sort_field(field: SortField) {
    // 保留磁盘上已有的 first_run / sidebar_expanded，避免切排序字段时把
    // "已看过帮助"重置为 true 或把 sidebar 折叠状态归零。
    write_state(field, load_first_run(), load_sidebar_expanded());
}

/// 用户首次进入 Help（按 ?）后调用：把 first_run 标记为 false 并立即写盘，
/// 保留当前 sort_field / sidebar_expanded。first_run 已经是 false 时幂等。
pub fn mark_first_run_done() {
    write_state(
        load_sort_field().unwrap_or_default(),
        false,
        load_sidebar_expanded(),
    );
}

/// 用户按 `c` 切换 sidebar 展开后调用：把新状态写盘，保留 sort_field / first_run。
pub fn save_sidebar_expanded(expanded: bool) {
    write_state(
        load_sort_field().unwrap_or_default(),
        load_first_run(),
        expanded,
    );
}

fn write_state(field: SortField, first_run: bool, sidebar_expanded: bool) {
    let Some(p) = path() else { return };
    if let Some(parent) = p.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return;
    }
    let body = format!(
        "# proc UI 偏好 — 由 proc 自动维护\nsort_field = \"{}\"\nfirst_run = {}\nsidebar_expanded = {}\n",
        field.as_str(),
        first_run,
        sidebar_expanded
    );
    let _ = std::fs::write(p, body);
}

fn parse_sort_field(raw: &str) -> Option<SortField> {
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

fn parse_first_run(raw: &str) -> Option<bool> {
    parse_bool_field(raw, "first_run")
}

fn parse_sidebar_expanded(raw: &str) -> Option<bool> {
    parse_bool_field(raw, "sidebar_expanded")
}

fn parse_bool_field(raw: &str, key: &str) -> Option<bool> {
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if let Some((k, val)) = trimmed.split_once('=')
            && k.trim() == key
        {
            return match val.trim() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            };
        }
    }
    None
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
            SortField::NetSent,
            SortField::NetRecv,
        ] {
            assert_eq!(SortField::parse_from_str(f.as_str()), Some(f));
        }
    }

    #[test]
    fn from_str_rejects_unknown() {
        assert_eq!(SortField::parse_from_str("unknown"), None);
        assert_eq!(SortField::parse_from_str(""), None);
    }

    #[test]
    fn parse_sort_field_accepts_valid() {
        assert_eq!(
            parse_sort_field("sort_field = \"memory\""),
            Some(SortField::Memory)
        );
    }

    #[test]
    fn parse_sort_field_skips_comments_and_blank() {
        let text = "# header\n\nsort_field = \"pid\"\n";
        assert_eq!(parse_sort_field(text), Some(SortField::Pid));
    }

    #[test]
    fn parse_sort_field_returns_none_when_missing() {
        assert_eq!(parse_sort_field("# nothing here\n"), None);
        assert_eq!(parse_sort_field(""), None);
    }

    #[test]
    fn parse_first_run_accepts_bool_literals() {
        assert_eq!(parse_first_run("first_run = true"), Some(true));
        assert_eq!(parse_first_run("first_run = false"), Some(false));
    }

    #[test]
    fn parse_first_run_skips_comments_and_blank() {
        let text = "# header\n\nfirst_run = true\n";
        assert_eq!(parse_first_run(text), Some(true));
    }

    #[test]
    fn parse_first_run_returns_none_when_missing() {
        assert_eq!(parse_first_run("# nothing here\n"), None);
        assert_eq!(parse_first_run(""), None);
    }

    #[test]
    fn parse_first_run_rejects_garbage() {
        assert_eq!(parse_first_run("first_run = yes"), None);
        assert_eq!(parse_first_run("first_run = 1"), None);
    }

    #[test]
    fn parse_sort_field_ignores_first_run_line() {
        // 两种字段共存于同一文件时，sort_field 解析不应被 first_run 干扰。
        let text = "sort_field = \"cpu\"\nfirst_run = false\n";
        assert_eq!(parse_sort_field(text), Some(SortField::Cpu));
    }

    #[test]
    fn parse_first_run_ignores_sort_field_line() {
        let text = "sort_field = \"cpu\"\nfirst_run = false\n";
        assert_eq!(parse_first_run(text), Some(false));
    }

    #[test]
    fn parse_sidebar_expanded_accepts_bool_literals() {
        assert_eq!(
            parse_sidebar_expanded("sidebar_expanded = true"),
            Some(true)
        );
        assert_eq!(
            parse_sidebar_expanded("sidebar_expanded = false"),
            Some(false)
        );
    }

    #[test]
    fn parse_sidebar_expanded_returns_none_when_missing() {
        // 老 ui.toml（无 sidebar_expanded 字段）按缺失处理。
        assert_eq!(parse_sidebar_expanded("# nothing here\n"), None);
        assert_eq!(parse_sidebar_expanded("sort_field = \"cpu\"\n"), None);
    }

    #[test]
    fn parse_sidebar_expanded_rejects_garbage() {
        assert_eq!(parse_sidebar_expanded("sidebar_expanded = yes"), None);
        assert_eq!(parse_sidebar_expanded("sidebar_expanded = 1"), None);
    }

    #[test]
    fn parse_sidebar_expanded_ignores_other_lines() {
        let text = "sort_field = \"cpu\"\nfirst_run = false\nsidebar_expanded = true\n";
        assert_eq!(parse_sidebar_expanded(text), Some(true));
    }
}
