use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};

use crate::app_panel::{AppMode, KeyResult, Panel, PanelContext};
use crate::docker::events::{self, DockerEvent, DockerEventReceiver};
use crate::docker::images::ImageInfo;
use crate::docker::logs::LogLine;
use crate::docker::logs_worker::{self, LogChunk, LogsWorker};
use crate::docker::snapshot_worker::DockerSnapshotWorker;
use crate::docker::stats::ContainerStats;
use crate::docker::top::ContainerTopProcess;
use crate::docker::volumes::VolumeInfo;
use crate::docker::{ContainerInfo, DockerMonitor};

/// E3 — Docker 面板 3 视图：容器 / 镜像 / volume。`Tab` 循环。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DockerViewMode {
    #[default]
    Containers,
    Images,
    Volumes,
}

impl DockerViewMode {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Containers => "容器",
            Self::Images => "镜像",
            Self::Volumes => "卷",
        }
    }

    /// `Tab` 循环到下一个视图。
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Containers => Self::Images,
            Self::Images => Self::Volumes,
            Self::Volumes => Self::Containers,
        }
    }
}

/// 日志查看模式状态：是否激活 + 缓冲 + 滚动 + follow 标志。
#[derive(Debug, Default)]
pub struct LogViewer {
    /// 已收到的日志行（环形缓冲，上限 5000 行）。
    pub buffer: Vec<LogLine>,
    /// 用户当前滚动位置（从底部算起，0 = 最底）。`None` 表示自动跟随底部。
    pub scroll_from_bottom: Option<usize>,
    /// follow 模式（新日志到达时是否滚到底）。
    pub follow: bool,
    /// 当前正在跟随的容器名（None = 未跟随）。
    pub container: Option<String>,
}

impl LogViewer {
    const MAX_BUFFER_LINES: usize = 5_000;

    fn append_chunk(&mut self, chunk: LogChunk) {
        self.buffer.extend(chunk.lines);
        // 环形截断（保留最新 5000 行）。
        if self.buffer.len() > Self::MAX_BUFFER_LINES {
            let drop = self.buffer.len() - Self::MAX_BUFFER_LINES;
            self.buffer.drain(..drop);
        }
    }

    fn clear(&mut self) {
        self.buffer.clear();
        self.scroll_from_bottom = None;
    }
}

pub struct DockerPanel {
    pub monitor: Option<Arc<Mutex<DockerMonitor>>>,
    pub containers: Vec<ContainerInfo>,
    pub cursor: usize,
    pub scroll: usize,
    pub connected: bool,
    pub status_message: Option<String>,
    pub event_receiver: Option<DockerEventReceiver>,
    pub events: Vec<DockerEvent>,
    pub detail: Option<ContainerInfo>,
    pub detail_stats: Option<ContainerStats>,
    /// 后台快照 worker(在 monitor 初始化时 spawn,持有 Arc::clone)。
    /// 字段必须留在 panel 上,drop 时 worker 才会退出。
    pub snapshot_worker: Option<DockerSnapshotWorker>,

    // ───── E4：容器内进程列表 ─────
    /// 详情视图里是否展示进程子区块（按 `t` 切换）。
    pub show_top_processes: bool,
    /// 当前详情容器的进程列表（`t` 触发采集）。
    pub top_processes: Vec<ContainerTopProcess>,

    // ───── E1：日志查看模式 ─────
    /// 日志模式（按 `l` 进入 / `Esc` 退出）。
    pub log_viewer: Option<LogViewer>,
    /// 当前激活的日志 worker（与 `log_viewer.container` 对应）。
    /// Drop 时 worker 退出。
    pub logs_worker: Option<LogsWorker>,

    // ───── E3：镜像 / volume ─────
    pub view_mode: DockerViewMode,
    pub images: Vec<ImageInfo>,
    pub volumes: Vec<VolumeInfo>,
    /// 镜像/volume 视图的 cursor（与容器列表独立，切换视图保留位置）。
    pub images_cursor: usize,
    pub volumes_cursor: usize,
    /// 删除确认状态：等用户再按一次 `d` 触发真删除。
    pub delete_pending: Option<DeleteTarget>,

    // v0.6.0 阶段 3：worker panic 通知 channel。App::new 在创建 panel 后
    // 把 `Some(tx)` 设进来；lazy spawn snapshot worker 时 clone 一份传入。
    pub crash_tx: Option<std::sync::mpsc::Sender<crate::metrics::crash::WorkerCrash>>,
}

/// 删除确认的目标。第一次按 `d` 进入确认态，第二次按 `d` 真删，其它键取消。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteTarget {
    Image { id: String, display: String },
    Volume { name: String },
}

impl Default for DockerPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl DockerPanel {
    #[must_use]
    pub fn new() -> Self {
        Self {
            monitor: None,
            containers: Vec::new(),
            cursor: 0,
            scroll: 0,
            connected: false,
            status_message: None,
            event_receiver: None,
            events: Vec::new(),
            detail: None,
            detail_stats: None,
            snapshot_worker: None,
            show_top_processes: false,
            top_processes: Vec::new(),
            log_viewer: None,
            logs_worker: None,
            view_mode: DockerViewMode::Containers,
            images: Vec::new(),
            volumes: Vec::new(),
            images_cursor: 0,
            volumes_cursor: 0,
            delete_pending: None,
            crash_tx: None,
        }
    }

    /// v0.7.0 阶段 1 TD-5：聚合 DockerPanel 自管的 worker metrics。
    ///
    /// 返回 `docker`（snapshot worker，连上 daemon 后必有）+ `docker_logs`
    /// （logs worker，仅在日志模式激活时存在）。`App::worker_metrics` 追加到
    /// `WorkerManager::metrics_snapshot` 的输出后供 `proc diag` / `?` 帮助页消费。
    #[must_use]
    pub fn metrics(&self) -> Vec<crate::metrics::NamedWorkerStats> {
        let mut out = Vec::new();
        if let Some(w) = self.snapshot_worker.as_ref() {
            out.push(crate::metrics::NamedWorkerStats {
                name: "docker",
                stats: w.metrics.snapshot(),
            });
        }
        if let Some(w) = self.logs_worker.as_ref() {
            out.push(crate::metrics::NamedWorkerStats {
                name: "docker_logs",
                stats: w.metrics.snapshot(),
            });
        }
        out
    }

    /// 同步刷新:首次连接时初始化 `Arc<Mutex<DockerMonitor>>` 并 spawn 后台
    /// snapshot worker;无论何时都立即同步拉一次容器列表(用户按 Shift+R 触发,
    /// 期望立即响应)。后续周期性更新由 worker 异步推送,经 `poll_events`
    /// 的 `try_recv` 应用。
    pub fn refresh(&mut self) {
        if self.monitor.is_none() {
            match DockerMonitor::connect() {
                Ok(monitor) => {
                    let monitor_arc = Arc::new(Mutex::new(monitor));
                    let worker = crate::docker::snapshot_worker::spawn(
                        Arc::clone(&monitor_arc),
                        self.crash_tx.clone(),
                    );
                    self.monitor = Some(monitor_arc);
                    self.snapshot_worker = Some(worker);
                    self.connected = true;
                }
                Err(e) => {
                    self.connected = false;
                    self.status_message = Some(format!("❌ {}", e));
                    return;
                }
            }
        }
        if let Some(ref monitor) = self.monitor {
            let result: crate::error::Result<_> = monitor
                .lock()
                .map_err(|_| crate::error::ProcError::docker("DockerMonitor mutex poisoned"))
                .and_then(|m| m.list_containers(true));
            match result {
                Ok(containers) => {
                    self.containers = containers;
                    self.connected = true;
                    if self
                        .status_message
                        .as_ref()
                        .is_none_or(|m| !m.starts_with('✅'))
                    {
                        self.status_message = None;
                    }
                }
                Err(e) => {
                    self.status_message = Some(format!("❌ 获取容器列表失败: {}", e));
                }
            }
        }
    }

    fn move_cursor(&mut self, delta: i32) {
        let total = match self.view_mode {
            DockerViewMode::Containers => self.containers.len(),
            DockerViewMode::Images => self.images.len(),
            DockerViewMode::Volumes => self.volumes.len(),
        };
        if total == 0 {
            return;
        }
        let new = self.active_cursor() as i32 + delta;
        let next = if new < 0 {
            total - 1
        } else if new as usize >= total {
            0
        } else {
            new as usize
        };
        self.set_active_cursor(next);
    }

    fn active_cursor(&self) -> usize {
        match self.view_mode {
            DockerViewMode::Containers => self.cursor,
            DockerViewMode::Images => self.images_cursor,
            DockerViewMode::Volumes => self.volumes_cursor,
        }
    }

    fn set_active_cursor(&mut self, idx: usize) {
        match self.view_mode {
            DockerViewMode::Containers => self.cursor = idx,
            DockerViewMode::Images => self.images_cursor = idx,
            DockerViewMode::Volumes => self.volumes_cursor = idx,
        }
    }

    fn restart_selected(&mut self) {
        let name = self.containers.get(self.cursor).map(|c| c.name.clone());
        if let Some(name) = name
            && let Some(ref monitor) = self.monitor
        {
            let result: crate::error::Result<()> = monitor
                .lock()
                .map_err(|_| crate::error::ProcError::docker("DockerMonitor mutex poisoned"))
                .and_then(|m| m.restart_container(&name));
            match result {
                Ok(()) => {
                    self.status_message = Some(format!("✅ 容器 {} 已重启", name));
                    self.refresh();
                }
                Err(e) => {
                    self.status_message = Some(format!("❌ 重启失败: {}", e));
                }
            }
        }
    }

    /// v0.7 阶段 3：暴露给 App::dispatch_command_action（命令面板 Docker 重启）。
    pub fn palette_restart_selected(&mut self) {
        self.restart_selected();
    }

    /// v0.7 阶段 3：暴露给 App::dispatch_command_action（命令面板 Docker 停止）。
    pub fn palette_stop_selected(&mut self) {
        self.stop_selected();
    }

    fn stop_selected(&mut self) {
        let name = self.containers.get(self.cursor).map(|c| c.name.clone());
        if let Some(name) = name
            && let Some(ref monitor) = self.monitor
        {
            let result: crate::error::Result<()> = monitor
                .lock()
                .map_err(|_| crate::error::ProcError::docker("DockerMonitor mutex poisoned"))
                .and_then(|m| m.stop_container(&name));
            match result {
                Ok(()) => {
                    self.status_message = Some(format!("✅ 容器 {} 已停止", name));
                    self.refresh();
                }
                Err(e) => {
                    self.status_message = Some(format!("❌ 停止失败: {}", e));
                }
            }
        }
    }

    pub fn start_watching(&mut self) {
        if let Some(ref monitor) = self.monitor {
            // 持锁期间 clone docker client(bollard::Docker 内部 Arc,clone 廉价),
            // 立即释放锁,让事件监听线程独立持有。
            let docker_client = monitor.lock().ok().map(|m| m.docker());
            if let Some(docker_client) = docker_client {
                let receiver = events::spawn_event_watcher(docker_client);
                self.event_receiver = Some(receiver);
                self.status_message = Some("✅ 已开始监听容器事件".to_string());
            } else {
                self.status_message = Some("❌ Docker mutex poisoned".to_string());
            }
        } else {
            self.status_message = Some("❌ Docker 未连接，请先刷新".to_string());
        }
    }

    fn show_detail(&mut self) {
        if self.detail.is_some() {
            self.detail = None;
            self.detail_stats = None;
            self.top_processes.clear();
            self.show_top_processes = false;
            return;
        }
        let container = self.containers.get(self.cursor).cloned();
        if let Some(c) = container {
            let name = c.name.clone();
            self.detail = Some(c);
            if let Some(ref monitor) = self.monitor {
                self.detail_stats = monitor.lock().ok().and_then(|m| m.get_stats(&name).ok());
            }
        }
    }

    /// `t` 触发：在容器详情视图中切换进程区块显示。第一次开启时同步采集。
    fn toggle_top_processes(&mut self) {
        if self.detail.is_none() {
            self.status_message = Some("❌ 先按 Enter 进入容器详情".to_string());
            return;
        }
        self.show_top_processes = !self.show_top_processes;
        if self.show_top_processes {
            self.refresh_top_processes();
        } else {
            self.top_processes.clear();
        }
    }

    fn refresh_top_processes(&mut self) {
        let Some(name) = self.detail.as_ref().map(|c| c.name.clone()) else {
            return;
        };
        if let Some(ref monitor) = self.monitor {
            match monitor
                .lock()
                .ok()
                .and_then(|m| m.container_top(&name).ok())
            {
                Some(procs) => {
                    self.top_processes = procs;
                    let n = self.top_processes.len();
                    self.status_message = Some(format!("✅ 拉到 {n} 个进程"));
                }
                None => {
                    self.status_message = Some(format!("❌ 获取 {} 进程列表失败", name));
                }
            }
        }
    }

    /// `l` 触发：进入日志模式 + 启动 worker。
    fn enter_logs_mode(&mut self) {
        let name = match self.detail.as_ref().map(|c| c.name.clone()) {
            Some(n) => n,
            None => {
                self.status_message = Some("❌ 先按 Enter 进入容器详情".to_string());
                return;
            }
        };

        // 已在跟随同一容器：无操作（避免重复 spawn worker）。
        if let Some(ref lv) = self.log_viewer
            && lv.container.as_deref() == Some(&name)
        {
            return;
        }

        // 切换容器：drop 旧 worker。
        self.logs_worker = None;

        let docker_client = self
            .monitor
            .as_ref()
            .and_then(|m| m.lock().ok())
            .map(|m| m.docker());

        let Some(docker_client) = docker_client else {
            self.status_message = Some("❌ Docker 未连接".to_string());
            return;
        };

        self.logs_worker = Some(logs_worker::spawn(docker_client, name.clone(), None));
        self.log_viewer = Some(LogViewer {
            buffer: Vec::new(),
            scroll_from_bottom: None,
            follow: true,
            container: Some(name),
        });
        self.status_message = Some("✅ 日志模式（f 切换 follow / c 清屏 / Esc 退出）".to_string());
    }

    fn exit_logs_mode(&mut self) {
        self.logs_worker = None;
        self.log_viewer = None;
    }

    /// `e` 触发：选中容器 → 设置 ctx.pending_container_exec → SwitchMode(ContainerExec)。
    /// App::switch_mode 拿到目标后启动 PTY（详见 app.rs::enter_container_exec）。
    /// 退出容器 exec 模式时同样走 ctx.pending_container_exec（None 表示「不要启动」，
    /// 但 SwitchMode 永远带 Some；进入 App 后立刻 take）。
    fn enter_exec_mode(&mut self, ctx: &mut PanelContext) -> KeyResult {
        if self.view_mode != DockerViewMode::Containers {
            self.status_message = Some("❌ exec 仅支持容器视图（Tab 切回容器）".to_string());
            return KeyResult::Consumed;
        }
        let name = match self.containers.get(self.cursor).map(|c| c.name.clone()) {
            Some(n) => n,
            None => {
                self.status_message = Some("❌ 没有选中的容器".to_string());
                return KeyResult::Consumed;
            }
        };
        let running = self
            .containers
            .get(self.cursor)
            .is_some_and(|c| c.state == "running");
        if !running {
            self.status_message = Some(format!("❌ 容器 {name} 未运行，无法 exec"));
            return KeyResult::Consumed;
        }
        *ctx.pending_container_exec = Some(name);
        KeyResult::SwitchMode(AppMode::ContainerExec)
    }

    /// `f` 切换 follow（仅日志模式有效）。
    fn toggle_logs_follow(&mut self) {
        if let Some(ref mut lv) = self.log_viewer {
            lv.follow = !lv.follow;
            if lv.follow {
                lv.scroll_from_bottom = None;
            }
            self.status_message = Some(format!(
                "✅ follow: {}",
                if lv.follow { "开" } else { "关" }
            ));
        }
    }

    /// `c` 清空日志缓冲（仅日志模式有效）。
    fn clear_logs(&mut self) {
        if let Some(ref mut lv) = self.log_viewer {
            lv.clear();
            self.status_message = Some("✅ 日志已清屏".to_string());
        }
    }

    /// `Tab` 循环视图模式。
    fn cycle_view_mode(&mut self) {
        self.view_mode = self.view_mode.next();
        self.detail = None;
        self.detail_stats = None;
        self.top_processes.clear();
        self.show_top_processes = false;
        self.exit_logs_mode();
        self.delete_pending = None;
        self.status_message = Some(format!("✅ 视图：{}", self.view_mode.label()));
        // 切到 Images/Volumes 时立刻拉一次列表。
        match self.view_mode {
            DockerViewMode::Containers => {}
            DockerViewMode::Images => self.refresh_images(),
            DockerViewMode::Volumes => self.refresh_volumes(),
        }
    }

    fn refresh_images(&mut self) {
        if let Some(ref monitor) = self.monitor {
            match monitor.lock().ok().and_then(|m| m.list_images().ok()) {
                Some(imgs) => {
                    self.images = imgs;
                    let total = self.images.len();
                    if self.images_cursor >= total && total > 0 {
                        self.images_cursor = total - 1;
                    }
                }
                None => {
                    self.status_message = Some("❌ 获取镜像列表失败".to_string());
                }
            }
        }
    }

    fn refresh_volumes(&mut self) {
        if let Some(ref monitor) = self.monitor {
            match monitor.lock().ok().and_then(|m| m.list_volumes().ok()) {
                Some(vols) => {
                    self.volumes = vols;
                    let total = self.volumes.len();
                    if self.volumes_cursor >= total && total > 0 {
                        self.volumes_cursor = total - 1;
                    }
                }
                None => {
                    self.status_message = Some("❌ 获取 volume 列表失败".to_string());
                }
            }
        }
    }

    /// `d` 删除当前选中的镜像 / volume（两次按键确认）。
    fn handle_delete(&mut self) {
        // 第二次按 `d`：执行删除。
        if let Some(target) = self.delete_pending.take() {
            self.execute_delete(target);
            return;
        }
        // 第一次按 `d`：进入确认态。
        match self.view_mode {
            DockerViewMode::Images => {
                if let Some(img) = self.images.get(self.images_cursor).cloned() {
                    self.delete_pending = Some(DeleteTarget::Image {
                        id: img.id.clone(),
                        display: img.display_name(),
                    });
                    self.status_message = Some(format!(
                        "⚠ 再按 d 删除镜像 {}（Esc 取消）",
                        img.display_name()
                    ));
                }
            }
            DockerViewMode::Volumes => {
                if let Some(v) = self.volumes.get(self.volumes_cursor).cloned() {
                    self.delete_pending = Some(DeleteTarget::Volume {
                        name: v.name.clone(),
                    });
                    self.status_message =
                        Some(format!("⚠ 再按 d 删除 volume {}（Esc 取消）", v.name));
                }
            }
            DockerViewMode::Containers => {
                self.status_message = Some("❌ 容器删除尚未实现，请用 docker rm".to_string());
            }
        }
    }

    fn execute_delete(&mut self, target: DeleteTarget) {
        let Some(ref monitor) = self.monitor else {
            self.status_message = Some("❌ Docker 未连接".to_string());
            return;
        };
        let result: crate::error::Result<()> = monitor
            .lock()
            .map_err(|_| crate::error::ProcError::docker("DockerMonitor mutex poisoned"))
            .and_then(|guard| match &target {
                DeleteTarget::Image { id, .. } => guard.remove_image(id, true),
                DeleteTarget::Volume { name } => guard.remove_volume(name, true),
            });
        match result {
            Ok(()) => {
                let msg = match target {
                    DeleteTarget::Image { display, .. } => format!("✅ 已删镜像 {}", display),
                    DeleteTarget::Volume { name } => format!("✅ 已删 volume {}", name),
                };
                self.status_message = Some(msg);
                match self.view_mode {
                    DockerViewMode::Images => self.refresh_images(),
                    DockerViewMode::Volumes => self.refresh_volumes(),
                    _ => {}
                }
            }
            Err(e) => {
                self.status_message = Some(format!("❌ 删除失败: {}", e));
            }
        }
    }

    pub fn poll_events(&mut self) {
        // 1) 处理事件流(docker::events::spawn_event_watcher 推送)
        if let Some(ref receiver) = self.event_receiver {
            let new_events: Vec<DockerEvent> = std::iter::from_fn(|| receiver.try_recv()).collect();
            for event in new_events {
                let action = event.action.clone();
                let container_name = event
                    .container_name
                    .clone()
                    .unwrap_or_else(|| event.container_id.clone());
                self.events.insert(0, event);
                if self.events.len() > 100 {
                    self.events.truncate(100);
                }
                match action.as_str() {
                    "die" | "stop" => {
                        crate::monitor::notify::send_toast(
                            "Docker 容器停止",
                            &format!("容器 {} 已停止", container_name),
                        )
                        .ok();
                    }
                    "start" => {
                        crate::monitor::notify::send_toast(
                            "Docker 容器启动",
                            &format!("容器 {} 已启动", container_name),
                        )
                        .ok();
                    }
                    "health_status" => {
                        crate::monitor::notify::send_toast(
                            "Docker 健康状态变化",
                            &format!("容器 {} 健康状态变化", container_name),
                        )
                        .ok();
                    }
                    _ => {}
                }
            }
        }

        // 2) 应用后台 snapshot worker 推送的容器列表(每 ~5s 一份)。
        if let Some(ref worker) = self.snapshot_worker
            && let Some(snap) = worker.try_recv_latest()
        {
            match snap.result {
                Ok(containers) => {
                    self.containers = containers;
                    self.connected = true;
                    let total = self.containers.len();
                    if total == 0 {
                        self.cursor = 0;
                    } else if self.cursor >= total {
                        self.cursor = total - 1;
                    }
                    if self
                        .status_message
                        .as_ref()
                        .is_none_or(|m| !m.starts_with('✅'))
                    {
                        self.status_message = None;
                    }
                }
                Err(e) => {
                    self.status_message = Some(format!("❌ 获取容器列表失败: {}", e));
                }
            }
        }

        // 3) 日志 worker 推送的 chunk（仅日志模式）。
        if let Some(ref worker) = self.logs_worker {
            let chunks: Vec<LogChunk> = worker.drain();
            if !chunks.is_empty()
                && let Some(ref mut lv) = self.log_viewer
            {
                for c in chunks {
                    lv.append_chunk(c);
                }
                // follow 时保持 scroll_from_bottom = None（自动到底）。
            }
        }
    }
}

impl Panel for DockerPanel {
    fn handle_key(&mut self, key: KeyEvent, ctx: &mut PanelContext) -> KeyResult {
        // 日志模式优先吃快捷键。
        if self.log_viewer.is_some() {
            match key.code {
                KeyCode::Esc => {
                    self.exit_logs_mode();
                    return KeyResult::Consumed;
                }
                KeyCode::Char('f') => {
                    self.toggle_logs_follow();
                    return KeyResult::Consumed;
                }
                KeyCode::Char('c') => {
                    self.clear_logs();
                    return KeyResult::Consumed;
                }
                KeyCode::Char('q') => return KeyResult::Quit,
                KeyCode::Up => {
                    if let Some(ref mut lv) = self.log_viewer {
                        let cur = lv.scroll_from_bottom.unwrap_or(0);
                        lv.scroll_from_bottom = Some(cur + 1);
                    }
                    return KeyResult::Consumed;
                }
                KeyCode::Down => {
                    if let Some(ref mut lv) = self.log_viewer {
                        match lv.scroll_from_bottom {
                            Some(0) | None => lv.scroll_from_bottom = None,
                            Some(n) => lv.scroll_from_bottom = Some(n.saturating_sub(1)),
                        }
                    }
                    return KeyResult::Consumed;
                }
                _ => return KeyResult::Ignored,
            }
        }

        match key.code {
            KeyCode::Char('q') => return KeyResult::Quit,
            KeyCode::Tab => self.cycle_view_mode(),
            KeyCode::Up => self.move_cursor(-1),
            KeyCode::Down => self.move_cursor(1),
            KeyCode::Enter => {
                // 镜像/volume 视图不进详情（详情视图只服务容器）。
                if self.view_mode == DockerViewMode::Containers {
                    self.show_detail();
                }
            }
            // v0.6.0 阶段 6：原 `r` 迁移到 `Shift+R`，让位详情页 `F5` 刷新；
            // 同时对齐 Mission Center / docker-compose UI「Shift+R 重启」语义。
            KeyCode::Char('R') => match self.view_mode {
                DockerViewMode::Containers => self.restart_selected(),
                DockerViewMode::Images => self.refresh_images(),
                DockerViewMode::Volumes => self.refresh_volumes(),
            },
            KeyCode::Char('s') => self.stop_selected(),
            KeyCode::Char('a') => self.start_watching(),
            KeyCode::Char('t') => self.toggle_top_processes(),
            KeyCode::Char('l') => self.enter_logs_mode(),
            KeyCode::Char('e') => return self.enter_exec_mode(ctx),
            KeyCode::Char('d') => self.handle_delete(),
            KeyCode::Esc => {
                if self.delete_pending.is_some() {
                    self.delete_pending = None;
                    self.status_message = Some("✅ 已取消删除".to_string());
                } else if self.detail.is_some() {
                    self.detail = None;
                    self.detail_stats = None;
                    self.top_processes.clear();
                    self.show_top_processes = false;
                } else {
                    self.status_message = None;
                }
            }
            _ => return KeyResult::Ignored,
        }
        KeyResult::Consumed
    }

    fn tick(&mut self, _ctx: &mut PanelContext) -> bool {
        self.poll_events();
        // 容器消失后（被删 / 停 → list 不再返回）cursor 必须收紧，避免越界渲染。
        let total = match self.view_mode {
            DockerViewMode::Containers => self.containers.len(),
            DockerViewMode::Images => self.images.len(),
            DockerViewMode::Volumes => self.volumes.len(),
        };
        if total == 0 {
            self.set_active_cursor(0);
        } else if self.active_cursor() >= total {
            self.set_active_cursor(total - 1);
        }
        false
    }

    fn cursor(&self) -> usize {
        self.cursor
    }

    fn scroll(&self) -> usize {
        self.scroll
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_mode_cycle_wraps() {
        assert_eq!(DockerViewMode::Containers.next(), DockerViewMode::Images);
        assert_eq!(DockerViewMode::Images.next(), DockerViewMode::Volumes);
        assert_eq!(DockerViewMode::Volumes.next(), DockerViewMode::Containers);
    }

    #[test]
    fn view_mode_default_is_containers() {
        assert_eq!(DockerViewMode::default(), DockerViewMode::Containers);
    }

    #[test]
    fn view_mode_labels_distinct() {
        let labels: Vec<_> = [
            DockerViewMode::Containers,
            DockerViewMode::Images,
            DockerViewMode::Volumes,
        ]
        .iter()
        .map(|m| m.label())
        .collect();
        assert_eq!(labels, ["容器", "镜像", "卷"]);
    }

    #[test]
    fn log_viewer_append_chunk_grows_buffer() {
        let mut lv = LogViewer::default();
        let chunk = LogChunk {
            lines: vec![
                LogLine {
                    timestamp: None,
                    message: "a".to_string(),
                    is_stderr: false,
                },
                LogLine {
                    timestamp: None,
                    message: "b".to_string(),
                    is_stderr: true,
                },
            ],
        };
        lv.append_chunk(chunk);
        assert_eq!(lv.buffer.len(), 2);
    }

    #[test]
    fn log_viewer_caps_at_max_lines() {
        let mut lv = LogViewer::default();
        for _ in 0..(LogViewer::MAX_BUFFER_LINES + 100) {
            lv.append_chunk(LogChunk {
                lines: vec![LogLine {
                    timestamp: None,
                    message: "x".to_string(),
                    is_stderr: false,
                }],
            });
        }
        assert_eq!(lv.buffer.len(), LogViewer::MAX_BUFFER_LINES);
    }

    #[test]
    fn log_viewer_clear_empties_buffer() {
        let mut lv = LogViewer {
            buffer: vec![LogLine {
                timestamp: None,
                message: "a".to_string(),
                is_stderr: false,
            }],
            ..Default::default()
        };
        lv.clear();
        assert!(lv.buffer.is_empty());
    }

    #[test]
    fn delete_target_image_eq() {
        let a = DeleteTarget::Image {
            id: "sha256:abc".to_string(),
            display: "nginx:latest".to_string(),
        };
        let b = DeleteTarget::Image {
            id: "sha256:abc".to_string(),
            display: "nginx:latest".to_string(),
        };
        assert_eq!(a, b);
    }

    #[test]
    fn new_panel_has_clean_state() {
        let p = DockerPanel::new();
        assert!(p.monitor.is_none());
        assert!(p.containers.is_empty());
        assert!(!p.show_top_processes);
        assert!(p.top_processes.is_empty());
        assert!(p.log_viewer.is_none());
        assert!(p.logs_worker.is_none());
        assert_eq!(p.view_mode, DockerViewMode::Containers);
        assert!(p.images.is_empty());
        assert!(p.volumes.is_empty());
        assert!(p.delete_pending.is_none());
    }

    #[test]
    fn metrics_empty_when_no_workers() {
        // v0.7.0 阶段 1 TD-5：未连接 daemon 时 snapshot/logs worker 都是 None，
        // metrics() 应返回空 vec。连接后由 docker daemon 决定，集成测试覆盖。
        let p = DockerPanel::new();
        let m = p.metrics();
        assert!(m.is_empty(), "fresh panel should expose no worker metrics");
    }
}
