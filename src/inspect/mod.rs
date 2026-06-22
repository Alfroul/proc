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
pub mod handles;
pub mod memory;
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

/// 进程打开的一条句柄（阶段 4 上线，阶段 1 仅声明骨架）。
///
/// 阶段 4 计划字段：
/// - `raw_handle`：Windows HANDLE / Linux fd
/// - `kind`：File / RegistryKey / Event / Semaphore / Mutant / Section / Process / Thread / ...
/// - `name`：可读对象名（NT 路径 / Win32 路径 / 注册表路径 / 空）
/// - `granted_access`：访问掩码（仅 Windows 有意义）
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HandleInfo {
    pub raw_handle: u64,
    pub kind: HandleKind,
    pub name: String,
    pub granted_access: u32,
}

/// 句柄对象分类（阶段 4 上线时填具体采集路径）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HandleKind {
    #[default]
    Unknown,
    File,
    Directory,
    RegistryKey,
    Event,
    Semaphore,
    Mutant,
    Section,
    Process,
    Thread,
    Token,
    Other,
}

impl HandleKind {
    /// UI / CLI 渲染用的稳定文字（与 InspectionTab::label 同样的设计：测试 anchor）。
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::File => "File",
            Self::Directory => "Directory",
            Self::RegistryKey => "RegistryKey",
            Self::Event => "Event",
            Self::Semaphore => "Semaphore",
            Self::Mutant => "Mutant",
            Self::Section => "Section",
            Self::Process => "Process",
            Self::Thread => "Thread",
            Self::Token => "Token",
            Self::Other => "Other",
        }
    }
}

/// 一条内存映射区域（阶段 4 上线，阶段 1 仅声明骨架）。
///
/// 阶段 4 计划字段：
/// - `base_addr` / `size`：区域范围（字节）
/// - `state`：Commit / Reserve / Free（Win32）；Linux 取自 maps 第 2 列
/// - `protection`：rwx 字符串（Linux）或 Win32 PAGE_PROTECTION_FLAGS
/// - `name`：映射名（文件路径 / heap / stack / 匿名）
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MemoryRegion {
    pub base_addr: u64,
    pub size: u64,
    pub state: MemoryState,
    pub protection: String,
    pub name: String,
}

/// 内存区域状态（阶段 4 上线时填具体采集路径）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MemoryState {
    #[default]
    Unknown,
    Commit,
    Reserve,
    Free,
    Private,
    Shared,
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
///
/// **注意**：此函数会触发一次完整 `port_map::scan_ports()`（netstat2 syscall +
/// sysinfo 全 PID 名表，几百毫秒级），主线程上调用会卡帧。TUI 主循环应改用
/// [`inspect_with_ports`] 复用 `port_panel.port_entries`。
#[must_use]
pub fn inspect(pid: u32) -> InspectionData {
    InspectionData {
        env: env::collect_env(pid).unwrap_or_default(),
        dlls: dlls::collect_dlls(pid).unwrap_or_default(),
        net: crate::port_map::find_ports_by_pid(pid).unwrap_or_default(),
    }
}

/// 与 [`inspect`] 相同，但网络分支接受调用方已采好的 `PortEntry` 切片，
/// 避免在 UI 主线程上重复 `scan_ports()`。
///
/// TUI 进详情页 / 按 `r` 刷新时应调此版本，把 `port_panel.port_entries` 传进来。
#[must_use]
pub fn inspect_with_ports(pid: u32, ports: &[crate::port_map::PortEntry]) -> InspectionData {
    InspectionData {
        env: env::collect_env(pid).unwrap_or_default(),
        dlls: dlls::collect_dlls(pid).unwrap_or_default(),
        net: ports.iter().filter(|e| e.pid == pid).cloned().collect(),
    }
}
