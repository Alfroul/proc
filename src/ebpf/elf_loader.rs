//! v0.7 阶段 8：内核态 eBPF ELF 加载策略。
//!
//! **设计**：内核态 ELF 由 `src/ebpf/ebpf-ebpf` 独立 sub-project 编译，产物
//! `target/bpfel-unknown-none/release/proc-ebpf`。userspace 加载策略有三选：
//!
//! 1. **build.rs 嵌入（v0.8+）**：build.rs 调 `cargo build -p proc-ebpf
//!    --target bpfel-unknown-none`，把 ELF 复制到 `OUT_DIR`，userspace
//!    `include_bytes!(concat!(env!("OUT_DIR"), "/proc-ebpf"))`。
//! 2. **手动嵌入（Part A 当前）**：开发者在启用 `ebpf` feature 前先手动
//!    `cd src/ebpf/ebpf-ebpf && cargo +nightly build --target bpfel-unknown-none --release`，
//!    ELF 路径写死。简单但易出错（CI 需要 cache）。
//! 3. **运行时加载**：`std::fs::read("proc-ebpf.el")` 失败时降级。安全但
//!    分发需要额外文件。
//!
//! Part A 采用方案 2（最简单，不需要 build.rs 改动）。Linux 会话验证后
//! 可换方案 1 自动化。
//!
//! **若 ELF 文件不存在 → 编译错误**。Linux 用户启用 `ebpf` feature 前必须
//! 先编译内核态（README 已说明）。

/// 编译内核态 ELF 的相对路径。预期为 bpfel-unknown-none release 产物。
///
/// 修改这里时同时更新 README / ADR-0016 / Checkpoint。
#[cfg(all(target_os = "linux", feature = "ebpf"))]
pub const EBPF_ELF: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/ebpf/ebpf-ebpf/target/bpfel-unknown-none/release/proc-ebpf"
));
