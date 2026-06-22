//! macOS / 其它平台的占位 collector：返回空 Vec，UI net 列保持 0。
//!
//! 不抛错、不警告日志（每次 poll 都 warn 会刷屏）。stage 7 的非 Windows / Linux
//! 降级路径走 [`crate::net_flow::detect_collector`] 返回 None；这个模块主要是
//! 给 cfg-gated `pub use unsupported as windows` 在非 Windows 平台提供一个
//! 「`IphelperCollector` 类型存在但 `new()` 总失败」的占位实现。

use crate::error::{ProcError, Result};
use crate::net_flow::{NetFlowCollector, ProcessNetRate};

pub struct IphelperCollector;

impl IphelperCollector {
    #[allow(clippy::unnecessary_wraps)]
    pub fn new() -> Result<Self> {
        Err(ProcError::monitor(
            "IphelperCollector 仅 Windows 支持；此平台无 per-process 网络观测",
        ))
    }
}

impl NetFlowCollector for IphelperCollector {
    fn per_process_rates(&mut self) -> Vec<ProcessNetRate> {
        Vec::new()
    }
    fn provider_name(&self) -> &'static str {
        "unsupported"
    }
}
