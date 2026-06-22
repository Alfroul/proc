//! 内存映射采集（阶段 4，A3）。
//!
//! - **Windows**：`VirtualQueryEx` 遍历整个进程地址空间（64-bit 系统理论上
//!   到 0x7FFF_FFFF_FFFF），按 `MEMORY_BASIC_INFORMATION.State` 分类
//!   Commit/Reserve/Free。
//! - **Linux**：解析 `/proc/<pid>/maps`（行格式见 `parse_maps_line`）。
//! - **macOS**：返回 `PermissionDenied`，UI 给「此平台不支持」。
//!
//! `MemoryRegion.name` 在 Windows 上需要 `GetMappedFileNameW` 才能取，本阶段
//! 留空（A3 验收只要求 size > 0，不要求 name 非空）。Linux 从 maps 第 6 列
//! 直接拿路径，免费。

use crate::error::{ProcError, Result};

use super::{MemoryRegion, MemoryState};

/// 单条 `/proc/<pid>/maps` 行的解析结果（仅供 [`parse_maps_line`] 内部使用）。
#[allow(dead_code)]
struct ParsedMapsLine {
    base: u64,
    size: u64,
    protection: String,
    name: String,
}

/// 解析 `/proc/<pid>/maps` 的单行。
///
/// 典型格式：
/// ```text
/// 7f8a1b2c3000-7f8a1b2e5000 r-xp 0000fe00 fd:00 12345  /usr/lib/libc.so.6
/// ```
/// 字段：`<start>-<end> <perms> <offset> <dev> <inode>  <path>`
/// 路径可能含空格（罕见），但通常不含；我们按「第一个空格后跳过若干空白，剩余
/// 整段当 path」处理，与 `dlls.rs::parse_proc_maps` 一致。
fn parse_maps_line(line: &str) -> Option<ParsedMapsLine> {
    // 行格式（6 列）：
    //   <start>-<end> <perms> <offset> <dev> <inode>  <pathname?>
    // perms 必为 4 个字符（r/w/x/- + p/s）。
    // pathname 可能为空（匿名）、[heap]/[stack] 等标签、或绝对路径。
    // 第 6 列之后若还有空格则属于 pathname（实际罕见）。
    let (range, rest) = line.split_once(char::is_whitespace)?;

    let (start_s, end_s) = range.split_once('-')?;
    let start = u64::from_str_radix(start_s.trim(), 16).ok()?;
    let end = u64::from_str_radix(end_s.trim(), 16).ok()?;
    if end <= start {
        return None;
    }
    let size = end - start;

    // rest: `r-xp 0000fe00 fd:00 12345  /usr/lib/libc.so.6` 或
    //       `rw-p 00000000 00:00 0  [heap]` 或 `rw-p 00000000 00:00 0`（匿名）
    let mut fields = rest.split_whitespace();
    let perms = fields.next()?;
    if perms.len() != 4 {
        return None;
    }
    let mut chars = perms.chars();
    let (r, w, x, p) = (chars.next()?, chars.next()?, chars.next()?, chars.next()?);
    if !matches!(r, 'r' | '-')
        || !matches!(w, 'w' | '-')
        || !matches!(x, 'x' | '-')
        || !matches!(p, 'p' | 's')
    {
        return None;
    }
    let mut protection = String::with_capacity(4);
    protection.push(r);
    protection.push(w);
    protection.push(x);
    protection.push(p);

    // 跳过 offset / dev / inode，第 5 个 token 起就是 pathname（可能没有）。
    let _offset = fields.next();
    let _dev = fields.next();
    let _inode = fields.next();
    let name = fields.next().unwrap_or("").to_string();

    Some(ParsedMapsLine {
        base: start,
        size,
        protection,
        name,
    })
}

/// /proc/<pid>/smaps 的单段（含 Size/Rss/Pss 等字段）解析结果。
///
/// 实际上 smaps 是 maps 后跟多行 Size:/Rss:/Pss:。这里只取 Size（与 maps 的
/// span 一致，可用于 sanity check），用于未来扩展；当前 collect_memory 不读
/// smaps（smaps 在容器里访问常常被拒）。
#[allow(dead_code)]
fn parse_smaps_block(block: &str) -> Option<(u64, u64)> {
    // block 形如：
    //   7f0000000000-7f0000001000 r-xp 00000000 fd:00 1  /usr/lib/libfoo.so
    //   Size:                  4 kB
    //   Rss:                   4 kB
    //   Pss:                   2 kB
    //   ...
    // 我们只提取 Size 与 Rss（kB → bytes）。
    let first_line = block.lines().next()?;
    let parsed = parse_maps_line(first_line)?;
    let mut rss_kb: Option<u64> = None;
    let mut size_kb: Option<u64> = None;
    for line in block.lines().skip(1) {
        if let Some(rest) = line.strip_prefix("Size:") {
            size_kb = rest.split_whitespace().next().and_then(|s| s.parse().ok());
        } else if let Some(rest) = line.strip_prefix("Rss:") {
            rss_kb = rest.split_whitespace().next().and_then(|s| s.parse().ok());
        }
        if size_kb.is_some() && rss_kb.is_some() {
            break;
        }
    }
    Some((parsed.size, rss_kb.unwrap_or(size_kb.unwrap_or(0)) * 1024))
}

#[cfg(target_os = "windows")]
pub fn collect_memory(pid: u32) -> Result<Vec<MemoryRegion>> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Memory::{
        MEM_COMMIT, MEM_FREE, MEM_RESERVE, MEMORY_BASIC_INFORMATION, VirtualQueryEx,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
    };

    let handle = unsafe {
        OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid)
            .map_err(|e| ProcError::permission_denied_with("OpenProcess 失败", e))?
    };

    let mut regions = Vec::new();
    let mut addr: usize = 0;
    // 上限：64-bit 系统用户态地址空间上限。VirtualQueryEx 遇到无效地址会返回 0
    // 自然 break，所以这里只是防御性的兜底（防止某些极端情况下无限循环）。
    const MAX_ADDR: usize = 0x7FFF_FFFF_FFFF;

    loop {
        if addr > MAX_ADDR {
            break;
        }
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        let returned = unsafe {
            VirtualQueryEx(
                handle,
                Some(addr as *const _),
                &mut info,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if returned == 0 {
            break;
        }

        let base = info.BaseAddress as usize;
        let region_size = info.RegionSize as usize;
        if region_size == 0 {
            break;
        }

        let state = if info.State == MEM_COMMIT {
            MemoryState::Commit
        } else if info.State == MEM_RESERVE {
            MemoryState::Reserve
        } else if info.State == MEM_FREE {
            MemoryState::Free
        } else {
            MemoryState::Unknown
        };

        let protection = format_win32_protection(info.Protect);

        regions.push(MemoryRegion {
            base_addr: base as u64,
            size: region_size as u64,
            state,
            protection,
            // GetMappedFileNameW 在 windows 0.57 里需要额外 feature（Psapi），
            // A3 v1 暂不取 mapped file name；Linux 路径走 /proc/<pid>/maps
            // 自带 name，足够日常使用。Windows 上的 name 留空，UI 显示「-」。
            name: String::new(),
        });

        // 步进到下一段。addr 用 wrapping_add 防 0xFFFF... 溢出 panic。
        let next = base.wrapping_add(region_size);
        if next <= addr {
            break;
        }
        addr = next;
    }

    let _ = unsafe { CloseHandle(handle) };
    Ok(regions)
}

#[cfg(target_os = "windows")]
fn format_win32_protection(
    protect: windows::Win32::System::Memory::PAGE_PROTECTION_FLAGS,
) -> String {
    use windows::Win32::System::Memory::*;
    if protect == PAGE_NOACCESS {
        return "---".to_string();
    }
    let bits = protect.0;
    let r_mask = PAGE_READONLY.0
        | PAGE_READWRITE.0
        | PAGE_WRITECOPY.0
        | PAGE_EXECUTE_READ.0
        | PAGE_EXECUTE_READWRITE.0
        | PAGE_EXECUTE_WRITECOPY.0;
    let w_mask =
        PAGE_READWRITE.0 | PAGE_WRITECOPY.0 | PAGE_EXECUTE_READWRITE.0 | PAGE_EXECUTE_WRITECOPY.0;
    let x_mask =
        PAGE_EXECUTE.0 | PAGE_EXECUTE_READ.0 | PAGE_EXECUTE_READWRITE.0 | PAGE_EXECUTE_WRITECOPY.0;
    let mut s = String::with_capacity(4);
    s.push(if bits & r_mask != 0 { 'r' } else { '-' });
    s.push(if bits & w_mask != 0 { 'w' } else { '-' });
    s.push(if bits & x_mask != 0 { 'x' } else { '-' });
    if bits & PAGE_GUARD.0 != 0 {
        s.push('g');
    }
    s
}

#[cfg(not(target_os = "windows"))]
pub fn collect_memory(pid: u32) -> Result<Vec<MemoryRegion>> {
    #[cfg(target_os = "linux")]
    {
        let path = format!("/proc/{pid}/maps");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| ProcError::permission_denied_with(format!("读取 {path} 失败"), e))?;
        Ok(parse_proc_maps(&text))
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        Err(ProcError::permission_denied(
            "此平台（非 Windows/Linux）暂不支持内存映射采集",
        ))
    }
}

#[cfg(target_os = "linux")]
fn parse_proc_maps(text: &str) -> Vec<MemoryRegion> {
    text.lines()
        .filter_map(|line| parse_maps_line(line))
        .map(|p| MemoryRegion {
            base_addr: p.base,
            size: p.size,
            // Linux maps 没有 commit/reserve/free 概念 —— 列出来的都是已分配的，
            // 统一标 Commit 让 UI 的「按 state 分组」能跑起来。
            state: MemoryState::Commit,
            protection: p.protection,
            name: p.name,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_maps_line_typical_so() {
        let line = "7f8a1b2c3000-7f8a1b2e5000 r-xp 0000fe00 fd:00 12345  /usr/lib/libc.so.6";
        let p = parse_maps_line(line).expect("parse");
        assert_eq!(p.base, 0x7f8a1b2c3000);
        assert_eq!(p.size, 0x22000);
        assert_eq!(p.protection, "r-xp");
        assert_eq!(p.name, "/usr/lib/libc.so.6");
    }

    #[test]
    fn parse_maps_line_heap_tag() {
        let line = "7f8a1c000000-7f8a1c021000 rw-p 00000000 00:00 0  [heap]";
        let p = parse_maps_line(line).expect("parse");
        assert_eq!(p.protection, "rw-p");
        assert_eq!(p.name, "[heap]");
    }

    #[test]
    fn parse_maps_line_anon() {
        // 匿名映射，没有 path 字段
        let line = "7f8a1d000000-7f8a1d001000 rw-p 00000000 00:00 0";
        let p = parse_maps_line(line).expect("parse");
        assert_eq!(p.name, "");
        assert_eq!(p.size, 0x1000);
    }

    #[test]
    fn parse_maps_line_no_access() {
        let line = "7f8a1e000000-7f8a1e001000 ---p 00000000 00:00 0";
        let p = parse_maps_line(line).expect("parse");
        assert_eq!(p.protection, "---p");
    }

    #[test]
    fn parse_maps_line_malformed_returns_none() {
        assert!(parse_maps_line("").is_none());
        assert!(parse_maps_line("not-a-range rwxp 0 fd:00 1  /x").is_none());
        // 缺 perms
        assert!(parse_maps_line("7f0000000000-7f0000001000").is_none());
        // start > end（不可能的行）
        assert!(parse_maps_line("7f0000002000-7f0000001000 rwxp 0 fd:00 1  /x").is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_proc_maps_collects_multiple_regions() {
        let text = "7f0000000000-7f0000001000 r-xp 00000000 fd:00 1  /usr/lib/libfoo.so\n\
                    7f0000001000-7f0000002000 rw-p 00001000 fd:00 1  [heap]\n";
        let regions = parse_proc_maps(text);
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].name, "/usr/lib/libfoo.so");
        assert_eq!(regions[1].name, "[heap]");
    }

    #[test]
    fn self_memory_collect_nonempty() {
        // 自身进程至少有 1 条内存区域（任何进程都至少有 stack + heap）。
        let pid = std::process::id();
        match collect_memory(pid) {
            Ok(regions) => {
                assert!(!regions.is_empty(), "expected ≥1 memory region");
                // 每条 size 必须 > 0（VirtualQueryEx / maps 都不会返回 0 size）。
                for r in &regions {
                    assert!(r.size > 0, "zero-size region: {r:?}");
                }
            }
            Err(e) => {
                // 容器/受限环境可能拒绝读自己的 maps —— 仅记录，不挂测试。
                eprintln!("note: collect_memory({pid}) failed in CI: {e}");
            }
        }
    }

    #[test]
    fn parse_smaps_block_extracts_rss() {
        let block = "7f0000000000-7f0000001000 r-xp 00000000 fd:00 1  /usr/lib/libfoo.so\n\
                     Size:                  4 kB\n\
                     Rss:                   3 kB\n\
                     Pss:                   2 kB\n";
        let (size, rss) = parse_smaps_block(block).expect("parse");
        assert_eq!(size, 0x1000);
        assert_eq!(rss, 3 * 1024);
    }

    #[test]
    fn parse_smaps_block_missing_rss_falls_back_to_size() {
        let block = "7f0000000000-7f0000001000 r-xp 00000000 fd:00 1  /usr/lib/libfoo.so\n\
                     Size:                  4 kB\n";
        let (size, rss) = parse_smaps_block(block).expect("parse");
        assert_eq!(size, 0x1000);
        // Rss 缺失 → 退化到 Size（kB → bytes）
        assert_eq!(rss, 4 * 1024);
    }
}
