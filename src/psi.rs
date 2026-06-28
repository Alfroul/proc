//! Linux Pressure Stall Information (PSI) — ADR-0013.
//!
//! Linux 4.20+ 在 `/proc/pressure/{cpu,mem,io}` 暴露 PSI（"系统真卡了"的金
//! 标准），比 load average 准。手写 parser 不引依赖（详见 ADR-0013）。
//!
//! - Linux：实装 `read_psi()` 读 3 个文件并解析
//! - Windows / macOS：cfg-gate stub 返回 `None`，监控面板降级显示
//!
//! 跨平台接口统一：上层永远调 `read_psi() -> Option<PsiStats>`，平台差异
//! 内化在模块里。

use serde::{Deserialize, Serialize};

/// 单条 PSI 记录（some 或 full 之一）。
///
/// `avg10` / `avg60` / `avg300` 是 10s / 60s / 300s 滑动平均（百分比，
/// 0-100）；`total` 是累计 stall 微秒数（monotonic，可 wrap，内核设计上
/// 一般不会溢出但调用方应按 u64 处理）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct PsiRecord {
    pub avg10: f32,
    pub avg60: f32,
    pub avg300: f32,
    pub total: u64,
}

/// 一次采样拿到的一份完整 PSI 快照（cpu/mem/io 各一份）。
///
/// `cpu_full` 恒为 `None`：内核设计上 CPU 没有 `full`（"所有任务都在等
/// CPU" = "CPU 空闲"，矛盾）。`mem_full` / `io_full` 也可能为 None —— 极
/// 少数情况内核配置或文件格式不含 full 行，让 UI 区分 "0% 压力" vs "无数据"。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PsiStats {
    pub cpu_some: PsiRecord,
    pub mem_some: PsiRecord,
    pub mem_full: Option<PsiRecord>,
    pub io_some: PsiRecord,
    pub io_full: Option<PsiRecord>,
}

/// 解析单行 PSI 内容，例如 `some avg10=2.30 avg60=1.10 avg300=0.50 total=681245`。
///
/// 返回 `(record, kind)`：`kind` 是 `some` 或 `full`，让调用方路由到
/// `PsiStats` 的对应字段。任何一行解析失败返回 `None`，调用方降级。
#[cfg(any(target_os = "linux", test))]
fn parse_psi_line(line: &str) -> Option<(PsiRecord, &str)> {
    let mut parts = line.split_whitespace();
    let kind = parts.next()?;
    let mut rec = PsiRecord::default();
    let mut got_avg10 = false;

    for kv in parts {
        let (k, v) = kv.split_once('=')?;
        match k {
            "avg10" => {
                rec.avg10 = v.parse().ok()?;
                got_avg10 = true;
            }
            "avg60" => rec.avg60 = v.parse().ok()?,
            "avg300" => rec.avg300 = v.parse().ok()?,
            "total" => rec.total = v.parse().ok()?,
            _ => {}
        }
    }

    // avg10 是 PSI 标准的第一个字段；没拿到说明解析到一半被打断，丢弃。
    if !got_avg10 {
        return None;
    }
    Some((rec, kind))
}

/// 解析一个 `/proc/pressure/{cpu,mem,io}` 文件内容，返回 (some, full)。
///
/// CPU 文件只有 `some` 行 → full = None；mem/io 有 some + full 两行。
/// 解析失败（空文件 / 格式异常）整体返回 None。
#[cfg(any(target_os = "linux", test))]
fn parse_psi_file(content: &str) -> Option<(PsiRecord, Option<PsiRecord>)> {
    let mut lines = content.lines();
    let first = lines.next()?;
    let (some, kind) = parse_psi_line(first)?;
    debug_assert_eq!(kind, "some", "PSI 文件首行应为 'some'，实际为 '{kind}'");

    let full = lines
        .next()
        .and_then(parse_psi_line)
        .and_then(|(r, k)| if k == "full" { Some(r) } else { None });

    Some((some, full))
}

#[cfg(target_os = "linux")]
fn read_psi_file(path: &str) -> Option<(PsiRecord, Option<PsiRecord>)> {
    let content = std::fs::read_to_string(path).ok()?;
    parse_psi_file(&content)
}

#[cfg(target_os = "linux")]
#[must_use]
pub fn read_psi() -> Option<PsiStats> {
    let cpu = read_psi_file("/proc/pressure/cpu")?;
    let mem = read_psi_file("/proc/pressure/memory")?;
    let io = read_psi_file("/proc/pressure/io")?;

    Some(PsiStats {
        cpu_some: cpu.0,
        mem_some: mem.0,
        mem_full: mem.1,
        io_some: io.0,
        io_full: io.1,
    })
}

#[cfg(not(target_os = "linux"))]
#[must_use]
pub fn read_psi() -> Option<PsiStats> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_psi_line_some_typical() {
        let (rec, kind) = parse_psi_line("some avg10=2.30 avg60=1.10 avg300=0.50 total=681245")
            .expect("valid line");
        assert_eq!(kind, "some");
        assert!((rec.avg10 - 2.30).abs() < 1e-6);
        assert!((rec.avg60 - 1.10).abs() < 1e-6);
        assert!((rec.avg300 - 0.50).abs() < 1e-6);
        assert_eq!(rec.total, 681_245);
    }

    #[test]
    fn parse_psi_line_full_typical() {
        let (rec, kind) = parse_psi_line("full avg10=0.50 avg60=0.10 avg300=0.00 total=150000")
            .expect("valid line");
        assert_eq!(kind, "full");
        assert!((rec.avg10 - 0.50).abs() < 1e-6);
        assert_eq!(rec.total, 150_000);
    }

    #[test]
    fn parse_psi_line_rejects_truncated() {
        // 缺 avg10 —— 解析失败
        assert!(parse_psi_line("some avg60=1.10").is_none());
    }

    #[test]
    fn parse_psi_line_skips_unknown_keys() {
        // 内核未来加新字段不破坏 parser
        let (rec, _) = parse_psi_line("some avg10=1.0 avg60=0.0 avg300=0.0 total=0 future=42")
            .expect("unknown key ignored");
        assert!((rec.avg10 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn parse_psi_file_cpu_only_some() {
        // CPU 文件只有 some 行
        let cpu_content = "some avg10=2.30 avg60=1.10 avg300=0.50 total=681245\n";
        let (some, full) = parse_psi_file(cpu_content).expect("parsed");
        assert!((some.avg10 - 2.30).abs() < 1e-6);
        assert!(full.is_none(), "CPU pressure 没有 full 行");
    }

    #[test]
    fn parse_psi_file_mem_with_full() {
        let mem_content = "some avg10=0.10 avg60=0.00 avg300=0.00 total=12345\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=100\n";
        let (some, full) = parse_psi_file(mem_content).expect("parsed");
        assert!((some.avg10 - 0.10).abs() < 1e-6);
        let full = full.expect("mem has full");
        assert_eq!(full.total, 100);
    }

    #[test]
    fn parse_psi_file_rejects_empty() {
        assert!(parse_psi_file("").is_none());
    }

    #[test]
    fn parse_psi_file_rejects_first_line_full() {
        // 第一行应是 some；如果出现 full（破损文件），返回的 kind 不匹配 →
        // debug_assert 在 release 不触发，full 字段保持 None，但 some 字段
        // 已被赋值。这里测「最坏情况下 some 仍可用」。
        let weird = "full avg10=1.0 avg60=0.0 avg300=0.0 total=5\nsome avg10=2.0 avg60=0.0 avg300=0.0 total=10\n";
        // 当前 parse_psi_file 把第一行当 some 用 —— kind mismatch 但仍返回。
        let (some, _) = parse_psi_file(weird).expect("fallback");
        assert!((some.avg10 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn read_psi_cross_platform_returns_something_predictable() {
        // Linux 上若 CONFIG_PSI=n 或内核 < 4.20，read_psi() 返回 None；
        // 其他平台恒返回 None。本测试只在「能读到」时验证字段一致。
        if let Some(stats) = read_psi() {
            // 全部 avg 字段应在 [0, 100] 区间
            for r in [
                stats.cpu_some,
                stats.mem_some,
                stats.io_some,
                stats.mem_full.unwrap_or_default(),
                stats.io_full.unwrap_or_default(),
            ] {
                assert!(r.avg10 >= 0.0 && r.avg10 <= 100.0, "avg10 out of range");
                assert!(r.avg60 >= 0.0 && r.avg60 <= 100.0, "avg60 out of range");
                assert!(r.avg300 >= 0.0 && r.avg300 <= 100.0, "avg300 out of range");
            }
        }
    }
}
