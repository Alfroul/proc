//! macOS / Linux / 其它平台的占位 collector：返回空 Vec。
//!
//! 阶段 8 Linux 原计划走 systemd-resolved DBus，但 systemd-resolved 的 DBus
//! 接口只暴露配置 / 状态 / ResolveHostname 调用，**不暴露 per-query 信号**
//! （详见 `docs/adr/0006-dns-subprocess-not-etw-dbus.md`）。pcap + DNS 协议
//! 解析 + PID 关联工程量超出单 stage 范围，留作未来 feature flag。
//!
//! 在非 Windows 平台，[`crate::dns_log::detect_collector`] 返回 `None`，
//! worker 不启动，UI 显示空列表 + 提示。

use crate::dns_log::{DnsLogCollector, DnsQuery};
use crate::error::{ProcError, Result};

pub struct PowershellDnsCollector;

impl PowershellDnsCollector {
    #[allow(clippy::unnecessary_wraps)]
    pub fn new() -> Result<Self> {
        Err(ProcError::monitor(
            "PowershellDnsCollector 仅 Windows 支持；此平台无 DNS 查询日志采集",
        ))
    }
}

impl DnsLogCollector for PowershellDnsCollector {
    fn drain(&mut self) -> Vec<DnsQuery> {
        Vec::new()
    }
    fn provider_name(&self) -> &'static str {
        "unsupported"
    }
}
