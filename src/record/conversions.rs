//! Conversions from recording frame types (`Frame*`) back to live runtime types.
//!
//! Used by replay to reconstruct view-model state from a recorded frame without
//! scattering field-by-field mapping through `App::replay_load_current_frame`.

use crate::app_panel::OpRecord;
use crate::classify;
use crate::collect::ProcessInfo;
use crate::docker::{ContainerInfo, HealthStatus};
use crate::eject::classify::HandleRisk;
use crate::eject::{HandleLock, RemovableDevice};
use crate::port_map::{NetworkViewMode, PortEntry, Protocol};
use crate::tree::TreeNode;

use super::frame::{
    FrameContainer, FrameOpRecord, FramePortEntry, FrameProcess, FrameTreeNode, FrameUsbDevice,
    FrameUsbLock,
};

impl From<&FrameProcess> for ProcessInfo {
    fn from(fp: &FrameProcess) -> Self {
        // v0.6.0 阶段 4：FrameProcess.name 是 String（serde 兼容），转 Arc<str>
        // 触发一次分配；replay 路径调用频次低，开销可忽略。
        let name: std::sync::Arc<str> = std::sync::Arc::from(fp.name.as_str());
        let name_lower: std::sync::Arc<str> = std::sync::Arc::from(fp.name.to_lowercase().as_str());
        ProcessInfo {
            pid: fp.pid,
            name_lower,
            name,
            cpu_usage: fp.cpu,
            memory: fp.memory,
            virtual_memory: 0,
            disk_usage: (fp.disk_read, fp.disk_write),
            disk_read_speed: 0,
            disk_write_speed: 0,
            net_sent_rate: 0,
            net_recv_rate: 0,
            status: crate::collect::ProcessStatus::default(),
            exe: None,
            cmd: std::sync::Arc::from(Vec::<String>::new()),
            cwd: None,
            parent_pid: None,
            session_id: None,
            user_id: None,
            start_time: 0,
            run_time: 0,
            throttled: crate::throttle::EcoQoSState::default(),
            signature_status: crate::security::SignatureStatus::default(),
            parent_chain: Vec::new(),
        }
    }
}

impl From<&FrameTreeNode> for TreeNode {
    fn from(f: &FrameTreeNode) -> Self {
        TreeNode {
            pid: f.pid,
            name: f.name.clone(),
            cpu: f.cpu,
            memory: f.memory,
            mem_pct: 0.0,
            status: String::new(),
            disk_read_speed: 0,
            disk_write_speed: 0,
            depth: f.depth,
            children: f.children.iter().map(TreeNode::from).collect(),
            expanded: f.expanded,
            class: match f.class.as_str() {
                "UserApp" => classify::ProcessClass::UserApp,
                "SystemProcess" => classify::ProcessClass::SystemProcess,
                "WindowsService" => classify::ProcessClass::WindowsService,
                "Kernel" => classify::ProcessClass::Kernel,
                _ => classify::ProcessClass::Unknown,
            },
            is_orphan: f.is_orphan,
            is_zombie: f.is_zombie,
            is_stale: false,
            kill_safety: None,
        }
    }
}

impl From<&FramePortEntry> for PortEntry {
    fn from(e: &FramePortEntry) -> Self {
        PortEntry {
            protocol: if e.protocol.as_str() == "Udp" {
                Protocol::Udp
            } else {
                Protocol::Tcp
            },
            local_addr: e
                .local_addr
                .parse()
                .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)),
            local_port: e.local_port,
            remote_addr: e.remote_addr.as_ref().and_then(|a| a.parse().ok()),
            remote_port: e.remote_port,
            state: e.state.clone(),
            pid: e.pid,
            process_name: e.process_name.clone(),
            // 录屏文件未保存 RTT（阶段 5 之前格式），回放时一律按未知。
            rtt_ms: None,
        }
    }
}

impl From<&FrameUsbDevice> for RemovableDevice {
    fn from(d: &FrameUsbDevice) -> Self {
        RemovableDevice {
            drive_letter: d.drive_letter,
            label: d.label.clone(),
            total_size: d.total_size,
            used_size: d.used_size,
            file_system: d.file_system.clone(),
            is_occupied: d.is_occupied,
            device_path: String::new(),
        }
    }
}

/// Replay only needs the lock's basic identity; the live `port_info` and
/// `process_class` are reconstructed lazily, so we default them.
impl From<&FrameUsbLock> for HandleLock {
    fn from(l: &FrameUsbLock) -> Self {
        HandleLock {
            pid: l.pid,
            process_name: l.process_name.clone(),
            exe_path: l.exe_path.clone(),
            process_class: classify::ProcessClass::Unknown,
            port_info: Vec::new(),
        }
    }
}

impl From<&FrameUsbLock> for HandleRisk {
    fn from(l: &FrameUsbLock) -> Self {
        match l.risk.as_str() {
            "Critical" => HandleRisk::Critical,
            "Warning" => HandleRisk::Warning,
            "Safe" => HandleRisk::Safe,
            _ => HandleRisk::Harmless,
        }
    }
}

impl From<&FrameContainer> for ContainerInfo {
    fn from(c: &FrameContainer) -> Self {
        ContainerInfo {
            id: c.id.clone(),
            name: c.name.clone(),
            image: c.image.clone(),
            status: c.status.clone(),
            state: c.state.clone(),
            health: match c.health.as_str() {
                "Healthy" => HealthStatus::Healthy,
                "Unhealthy" => HealthStatus::Unhealthy,
                "Starting" => HealthStatus::Starting,
                _ => HealthStatus::NotConfigured,
            },
            cpu_percent: c.cpu_percent,
            memory_usage: c.memory_usage,
            network_in: c.network_in,
            network_out: c.network_out,
            running_since: None,
            ports: c.ports.clone(),
        }
    }
}

impl From<&FrameOpRecord> for OpRecord {
    fn from(o: &FrameOpRecord) -> Self {
        OpRecord {
            time: o.time.clone(),
            message: o.message.clone(),
        }
    }
}

/// Helper used by `replay_load_current_frame` to map the recorded port_view_mode
/// byte back to the runtime enum. Kept here so callers don't need to inline the
/// match each time.
impl NetworkViewMode {
    #[must_use]
    pub fn from_frame_code(code: u8) -> Self {
        match code {
            1 => NetworkViewMode::Process,
            2 => NetworkViewMode::Remote,
            _ => NetworkViewMode::Port,
        }
    }
}
