//! 非 Linux / 无 `ebpf` feature 平台的 [`EbisuBpfWorker`] 占位。
//!
//! 保持与 Linux 实装一致的 API（`try_recv_events` / 字段名），让上游
//! `WorkerManager` / App 的字段类型统一。`try_spawn` 在 [`crate::ebpf`]
//! 根直接返回 `None`，stub 类型本身不会被构造。

/// Stub worker。仅作为类型占位；构造路径（`try_spawn`）不会返回它。
pub struct EbisuBpfWorker;

impl EbisuBpfWorker {
    /// stub 平台永远不应该被调用（`try_spawn` 返回 None），保留方法
    /// 让上游代码在 cfg-gate 切换时不出现 "method not found"。
    #[must_use]
    pub fn try_recv_events(&self) -> Vec<crate::ebpf::flow::FlowEvent> {
        Vec::new()
    }
}
