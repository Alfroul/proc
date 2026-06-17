//! 进程深挖数据采集层（v0.4.0 新增，ADR-0004）。
//!
//! 三个子模块各自跨平台实现：
//! - [`env]：进程环境变量
//! - [`dlls]：加载的 DLL / .so 列表
//! - [`net]：复用 [`crate::port_map`] 的端口/连接信息
//!
//! 顶层 [`inspect`] 聚合三份数据，TUI 层（阶段 13）按 Tab 分发渲染。

pub mod dlls;
pub mod env;
pub mod net;

/// 单条环境变量。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvVar {
    pub key: String,
    pub value: String,
}

/// 单个已加载模块（Windows DLL / Linux .so）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DllInfo {
    pub path: String,
    pub base_addr: u64,
    pub size: u64,
}

/// Inspector 一次采集的完整快照。三个 Vec 互不依赖，单 Tab 失败不影响其它。
#[derive(Debug, Clone, Default)]
pub struct InspectionData {
    pub env: Vec<EnvVar>,
    pub dlls: Vec<DllInfo>,
    pub net: Vec<crate::port_map::PortEntry>,
}

/// 同步采集 `pid` 的环境变量 / 模块 / 网络连接。
///
/// 任一子模块失败都用空 Vec 兜底，由调用方决定是否向用户展示降级提示。
/// 阶段 13 的 TUI 会把 `net.is_empty()` 等显示为「无数据 / 此平台不支持」。
#[must_use]
pub fn inspect(pid: u32) -> InspectionData {
    InspectionData {
        env: env::collect_env(pid).unwrap_or_default(),
        dlls: dlls::collect_dlls(pid).unwrap_or_default(),
        net: crate::port_map::find_ports_by_pid(pid).unwrap_or_default(),
    }
}
