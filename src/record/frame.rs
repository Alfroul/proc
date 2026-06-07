use serde::{Deserialize, Serialize};

pub const RECORDING_MAGIC: &[u8; 4] = b"PREC";
pub const RECORDING_VERSION: u16 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingHeader {
    pub magic: [u8; 4],
    pub version: u16,
    pub start_time: u64,
    pub hostname: String,
}

impl Default for RecordingHeader {
    fn default() -> Self {
        Self {
            magic: *RECORDING_MAGIC,
            version: RECORDING_VERSION,
            start_time: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            hostname: std::env::var("COMPUTERNAME")
                .or_else(|_| std::env::var("HOSTNAME"))
                .unwrap_or_else(|_| "unknown".to_string()),
        }
    }
}

// --- V1 frame (kept for backward-compatible reading) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacySystemFrame {
    pub timestamp: u64,
    pub cpu_usage: f32,
    pub memory_used: u64,
    pub memory_total: u64,
    pub net_down: u64,
    pub net_up: u64,
    pub processes: Vec<FrameProcess>,
}

// --- V2 full UI frame ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiFrame {
    pub timestamp: u64,

    // Current UI state
    pub mode: String,
    pub status_message: Option<String>,

    // System metrics
    pub cpu_usage: f32,
    pub memory_used: u64,
    pub memory_total: u64,
    pub net_down: u64,
    pub net_up: u64,
    pub cpu_history: Vec<u64>,
    pub mem_history: Vec<u64>,

    // Process list
    pub processes: Vec<FrameProcess>,
    pub search_query: String,
    pub sort_field: String,

    // Process tree
    pub tree_nodes: Vec<FrameTreeNode>,

    // Port map
    pub port_entries: Vec<FramePortEntry>,
    pub port_view_mode: u8,
    pub port_process_groups: Vec<FrameProcessNetGroup>,
    pub port_remote_groups: Vec<FrameRemoteGroup>,
    pub connection_diff: FrameConnectionDiff,
    pub anomalies: Vec<FrameAnomaly>,

    // USB
    pub usb_devices: Vec<FrameUsbDevice>,
    pub usb_locks: Vec<FrameUsbLock>,

    // Monitor
    pub monitors: Vec<FrameMonitorEntry>,

    // Docker
    pub docker_containers: Vec<FrameContainer>,
    pub docker_events: Vec<FrameDockerEvent>,

    // Operation history since recording started
    pub ops: Vec<FrameOpRecord>,

    // UI navigation state
    #[serde(default)]
    pub nav: FrameNav,
}

/// Captured UI navigation state for faithful replay.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FrameNav {
    // ProcessList
    pub cursor: usize,
    pub scroll: usize,
    pub selected: Vec<usize>,

    // ProcessTree
    pub tree_cursor: usize,
    pub tree_scroll: usize,
    pub tree_selected: Vec<usize>,

    // PortMap
    pub port_cursor: usize,
    pub port_scroll: usize,
    pub port_process_cursor: usize,
    pub port_process_scroll: usize,
    pub port_remote_cursor: usize,
    pub port_remote_scroll: usize,

    // UsbAssistant
    pub usb_device_cursor: usize,

    // MonitorPanel
    pub monitor_cursor: usize,

    // DockerPanel
    pub docker_cursor: usize,
    pub docker_scroll: usize,
}

// --- Helper frame types ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameProcess {
    pub pid: u32,
    pub name: String,
    pub cpu: f32,
    pub memory: u64,
    pub disk_read: u64,
    pub disk_write: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameTreeNode {
    pub pid: u32,
    pub name: String,
    pub cpu: f32,
    pub memory: u64,
    pub depth: usize,
    pub expanded: bool,
    pub is_orphan: bool,
    pub is_zombie: bool,
    pub class: String,
    pub children: Vec<FrameTreeNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FramePortEntry {
    pub protocol: String,
    pub local_addr: String,
    pub local_port: u16,
    pub remote_addr: Option<String>,
    pub remote_port: Option<u16>,
    pub state: Option<String>,
    pub pid: u32,
    pub process_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameProcessNetGroup {
    pub pid: u32,
    pub process_name: String,
    pub tcp_count: usize,
    pub udp_count: usize,
    pub established: usize,
    pub listening: usize,
    pub time_wait: usize,
    pub close_wait: usize,
    pub down_speed: u64,
    pub up_speed: u64,
    pub total_down: u64,
    pub total_up: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameRemoteGroup {
    pub remote_addr: String,
    pub ip_class: String,
    pub cloud_provider: Option<String>,
    pub process_names: Vec<String>,
    pub established: usize,
    pub listening: usize,
    pub time_wait: usize,
    pub close_wait: usize,
    pub connection_count: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FrameConnectionDiff {
    pub new_count: usize,
    pub closed_count: usize,
    pub active_count: usize,
    pub close_wait_count: usize,
    pub time_wait_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameAnomaly {
    pub rule_id: String,
    pub severity: String,
    pub title: String,
    pub detail: String,
    pub affected_pid: Option<u32>,
    pub affected_ip: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameUsbDevice {
    pub drive_letter: char,
    pub label: String,
    pub total_size: u64,
    pub used_size: u64,
    pub file_system: String,
    pub is_occupied: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameUsbLock {
    pub pid: u32,
    pub process_name: String,
    pub exe_path: Option<String>,
    pub risk: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameMonitorEntry {
    pub id: u32,
    pub target: String,
    pub pid: Option<u32>,
    pub status: String,
    pub crash_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameContainer {
    pub id: String,
    pub name: String,
    pub image: String,
    pub status: String,
    pub state: String,
    pub health: String,
    pub cpu_percent: f64,
    pub memory_usage: u64,
    pub network_in: u64,
    pub network_out: u64,
    pub ports: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameDockerEvent {
    pub action: String,
    pub container_id: String,
    pub container_name: Option<String>,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameOpRecord {
    pub time: String,
    pub message: String,
}
