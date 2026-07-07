//! v0.13 阶段 1：criterion benchmark 共享 fixture builder。
//!
//! 设计原则（stage doc 任务 3）：
//!
//! - **不依赖运行时环境**：不调 sysinfo / 不读真实进程 / 不依赖 admin 权限。
//! - **完全 fake 数据**：任何机器 / CI / developer 上数字有可比性。
//! - **稳定分布**：进程名 / cpu / memory 用确定性公式（pid 派生）而非 RNG，
//!   避免不同次跑数字差异来自 fixture 本身。
//! - **Arc 化**：模拟 HeavyWorker 真实输出（`name` / `name_lower` / `cmd` 都是 Arc），
//!   不在 benchmark 里产生与生产路径不一致的分配模式。
//!
//! 每个 bench 文件作为独立 crate 编译，common 中未被该 bench 引用的 helper
//! 会触发 dead_code。它们是给其他 bench 用的——单 bench crate 的 known 行为，
//! 全局 allow 避免每个 bench 各自加 `#![allow(dead_code)]`。
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;

use proc::classify;
use proc::collect::{ProcessInfo, ProcessStatus, SortField};
use proc::filter::{FilterExpr, NetworkEvalCtx, parse as parse_filter_expr};
use proc::flow::ProcessFlow;
use proc::record::frame::{FrameConnectionDiff, FrameOpRecord, FrameProcess, UiFrame};

// ─── ProcessInfo fixture ───────────────────────────────────────────────────

/// 生成 N 个 fake ProcessInfo，模拟 HeavyWorker 输出分布。
///
/// 分布设计（让 filter / sort 路径有真实工作量）：
/// - **进程名**：5 类 vendor 各 N/5 个（chrome / firefox / svchost / explorer / powershell），
///   让 `name =~ /chrome/` 命中约 1/5、`name =~ /office/` 命中 0。
/// - **cpu_usage**：`pid % 30` 让 0-30 范围均匀分布，约 1/6 进程 cpu > 5（`cpu > 5` 命中）。
/// - **memory**：`pid * 8MB`，跨度足够让 `mem > 500mb` 命中部分进程。
/// - **parent_chain**：每个进程指向 pid-1 形成线性链（让 build_parent_chain 跑满深度）。
/// - **name_lower**：预算字段，与 HeavyWorker 路径一致。
#[must_use]
pub fn make_processes(n: usize) -> Vec<ProcessInfo> {
    let vendors = [
        "chrome.exe",
        "firefox.exe",
        "svchost.exe",
        "explorer.exe",
        "powershell.exe",
    ];
    let mut result = Vec::with_capacity(n);
    for i in 1..=n {
        let pid = i as u32;
        let name_str = vendors[i % vendors.len()];
        let name: Arc<str> = Arc::from(name_str);
        let name_lower: Arc<str> = Arc::from(name_str.to_lowercase().as_str());
        let cpu = (pid % 30) as f32;
        let memory = (pid as u64) * 8 * 1024 * 1024; // 8MB / pid
        let parent_chain = if i > 1 {
            vec![(
                pid - 1,
                std::sync::Arc::<str>::from(vendors[(i - 1) % vendors.len()]),
            )]
        } else {
            Vec::new()
        };
        let info = ProcessInfo {
            pid,
            name: Arc::clone(&name),
            cpu_usage: cpu,
            memory,
            virtual_memory: memory,
            disk_usage: ((pid as u64) * 1024, (pid as u64) * 512),
            disk_read_speed: (pid as u64) * 100,
            disk_write_speed: (pid as u64) * 50,
            net_sent_rate: (pid as u64) * 10,
            net_recv_rate: (pid as u64) * 20,
            status: ProcessStatus::Run,
            exe: Some(Arc::from(
                format!("C:\\Program Files\\vendor\\{name_str}").as_str(),
            )),
            cmd: Arc::from(vec![format!("{name_str} --flag-{pid}")]),
            cwd: Some(Arc::from("C:\\Users\\test")),
            parent_pid: if i > 1 { Some(pid - 1) } else { None },
            session_id: Some(1),
            user_id: Some(Arc::from("TESTUSER")),
            start_time: 1_700_000_000 + pid as u64,
            run_time: pid as u64 * 60,
            name_lower,
            parent_chain,
            ..ProcessInfo::default()
        };
        result.push(info);
    }
    result
}

/// 构造 HashMap 形式（HeavyWorker 内部 representation，build_parent_chain 用）。
#[must_use]
pub fn make_processes_map(n: usize) -> HashMap<u32, ProcessInfo> {
    make_processes(n).into_iter().map(|p| (p.pid, p)).collect()
}

// ─── FilterExpr fixture ────────────────────────────────────────────────────

/// 典型 FilterExpr AST：`cpu > 5 AND name =~ /chrome/`。
///
/// `parse_filter_expr` 失败时 panic——本 fixture 仅用于 benchmark，parser 自身
/// 有专属单元测试覆盖。
#[must_use]
pub fn make_filter_expr() -> FilterExpr {
    parse_filter_expr("cpu > 5 AND name =~ /chrome/").expect("fixture filter expr should parse")
}

/// 多档 FilterExpr，覆盖 4 类典型表达式。
#[must_use]
pub fn make_filter_expr_set() -> Vec<(&'static str, FilterExpr)> {
    vec![
        ("cpu_gt", parse_filter_expr("cpu > 5").expect("parse")),
        (
            "name_regex",
            parse_filter_expr("name =~ /chrome/i").expect("parse"),
        ),
        (
            "cpu_and_mem",
            parse_filter_expr("cpu > 5 AND mem > 100mb").expect("parse"),
        ),
        (
            "complex",
            parse_filter_expr("cpu > 5 AND name =~ /chrome/ OR security_score < 80")
                .expect("parse"),
        ),
    ]
}

// ─── ProcessFlow fixture（Flow view / NetworkEvalCtx 用） ──────────────────

/// 生成 N 个 fake ProcessFlow。`sni` 字段按 pid % 4 命中 4 个常见域名让
/// `sni in (...)` 命中 1/4。
#[must_use]
pub fn make_flows(n: usize) -> Vec<ProcessFlow> {
    let snis = [
        Some("www.google.com".to_string()),
        Some("www.github.com".to_string()),
        Some("www.microsoft.com".to_string()),
        None,
    ];
    let now = std::time::SystemTime::now();
    (1..=n)
        .map(|i| {
            let pid = i as u32;
            ProcessFlow {
                pid,
                start_time: 1_700_000_000 + pid as u64,
                comm: format!("proc-{pid}"),
                local_addr: "127.0.0.1".to_string(),
                remote_addr: format!("1.2.3.{}", pid % 256),
                remote_port: 443,
                bytes_out: (pid as u64) * 100,
                bytes_in: (pid as u64) * 500,
                dns_name: Some(format!("dns-{pid}.example.com")),
                sni: snis[i % snis.len()].clone(),
                first_seen: now,
                last_seen: now,
                exit_time: None,
            }
        })
        .collect()
}

// ─── NetworkEvalCtx factory（per-flow 求值用） ─────────────────────────────

#[must_use]
pub fn network_eval_ctx<'a>(flow: &'a ProcessFlow) -> NetworkEvalCtx<'a> {
    NetworkEvalCtx { flow }
}

// ─── SortField fixture ─────────────────────────────────────────────────────

/// 默认 SortField（与生产 List view 默认一致）。
#[must_use]
pub fn default_sort_field() -> SortField {
    SortField::Cpu
}

// ─── classify 暴露（rebuild_sorted_cache 核心算法 benchmark 用） ───────────

#[must_use]
pub fn classify(proc: &ProcessInfo) -> classify::ProcessClass {
    classify::classify_process(proc)
}

// ─── UiFrame fixture（bench_record_serialize 用） ──────────────────────────

/// 构造典型录屏帧。N 个进程填进 `processes` 字段（FrameProcess 简化结构）。
#[must_use]
pub fn make_ui_frame(n_processes: usize) -> UiFrame {
    let processes: Vec<FrameProcess> = (1..=n_processes)
        .map(|i| {
            let pid = i as u32;
            FrameProcess {
                pid,
                name: format!("proc-{pid}"),
                cpu: (pid % 30) as f32,
                memory: (pid as u64) * 8 * 1024 * 1024,
                disk_read: (pid as u64) * 1024,
                disk_write: (pid as u64) * 512,
            }
        })
        .collect();

    UiFrame {
        timestamp: 1_700_000_000,
        mode: "ProcessList".to_string(),
        status_message: Some("ok".to_string()),
        cpu_usage: 12.5,
        memory_used: 8 * 1024 * 1024 * 1024,
        memory_total: 16 * 1024 * 1024 * 1024,
        net_down: 1024 * 100,
        net_up: 1024 * 50,
        cpu_history: (0..60).map(|i| (i + 1) as u64).collect(),
        mem_history: (0..60).map(|i| (i + 1) as u64 * 100).collect(),
        processes,
        search_query: "chrome".to_string(),
        sort_field: "Cpu".to_string(),
        process_view_mode: 0,
        tree_nodes: Vec::new(),
        port_entries: Vec::new(),
        port_view_mode: 0,
        port_process_groups: Vec::new(),
        port_remote_groups: Vec::new(),
        connection_diff: FrameConnectionDiff::default(),
        anomalies: Vec::new(),
        usb_devices: Vec::new(),
        usb_locks: Vec::new(),
        monitors: Vec::new(),
        docker_containers: Vec::new(),
        docker_events: Vec::new(),
        ops: vec![FrameOpRecord {
            time: "00:00:01".to_string(),
            message: "started".to_string(),
        }],
        nav: Default::default(),
    }
}
