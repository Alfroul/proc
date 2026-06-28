//! v0.6.0 阶段 5：详情页状态 + 数据加载逻辑封装，从 `App` 上帝对象拆出。
//!
//! 持 9 个字段（`detail_process` + 8 个 `inspection_*` / `detail_priority` /
//! `env_reveal`），暴露 `open` / `sync_detail` / `close` / `refresh_detail_priority`
//! / `handle_key`。
//!
//! `handle_key` 返回 [`InspectorAction`] 枚举：副作用（写 `status_message` /
//! `record_op` / `kill_process` / `add_monitor` / 剪贴板）通过 action 让 [`crate::app::App`]
//! 派发，避免 controller 反向依赖 App。

use crate::app_panel::InspectionTab;
use crate::collect::ProcessInfo;
use crate::inspect::{HandleInfo, InspectionData, MemoryRegion};
use crate::search::SearchState;
use crossterm::event::{KeyCode, KeyEvent};

/// 详情页状态 + 数据加载逻辑封装（v0.6.0 阶段 5 从 `App` 拆出）。
///
/// 字段名沿用 App 原名（`inspection_tab` / `detail_process` 等），让搬迁只多一层
/// `.inspector.` 前缀，tests 改造机械化。
pub struct InspectorController {
    /// 当前打开的进程（详情页主体）。`App::tick` heavy refresh 会通过
    /// [`sync_detail`] 用 `cached_processes` 重写；`PanelContext.detail_process`
    /// 也持可变引用让 ProcessPanel 在 Enter 时写入。
    pub detail_process: Option<ProcessInfo>,
    pub inspection_tab: InspectionTab,
    pub inspection_data: Option<InspectionData>,
    pub inspection_search: SearchState,
    pub inspection_scroll: usize,
    /// 阶段 4 A1/A3 采集的句柄列表（Tab=Handles 渲染用）。`None` = 未采集。
    pub inspection_handles_data: Option<Vec<HandleInfo>>,
    /// 阶段 4 A1/A3 采集的内存映射（Tab=Memory 渲染用）。`None` = 未采集。
    pub inspection_memory_data: Option<Vec<MemoryRegion>>,
    /// Summary Tab 的 (priority_label, affinity_label) 缓存（阶段 11 P1-A3）。
    /// 进入详情页 / `F5` 刷新 / `+/-` 调整 / heavy tick 4 处更新。
    pub detail_priority: Option<(String, String)>,
    /// v0.6.0 阶段 2：详情页 Env Tab 是否显示 secret 真值。默认 `false`；
    /// 按 `v` 切换；`recording_wanted=true` 时渲染层强制 mask（见
    /// [`env_render_reveal`]）。
    pub env_reveal: bool,
}

impl Default for InspectorController {
    fn default() -> Self {
        Self::new()
    }
}

impl InspectorController {
    #[must_use]
    pub fn new() -> Self {
        Self {
            detail_process: None,
            inspection_tab: InspectionTab::Summary,
            inspection_data: None,
            inspection_search: SearchState::new(),
            inspection_scroll: 0,
            inspection_handles_data: None,
            inspection_memory_data: None,
            detail_priority: None,
            env_reveal: false,
        }
    }

    /// 进入详情页时调（取代原 `App::switch_mode(ProcessDetail)` 内的初始化）。
    ///
    /// `port_entries` 复用 `port_panel.port_entries` 副本，避免在主线程上再调
    /// `scan_ports` 卡帧。失败的子项退化为空 Vec，Tab 渲染显示「采集失败」。
    /// 调用前 `detail_process` 应已被 ProcessPanel 写入（Enter 时通过
    /// `PanelContext.detail_process`）。
    pub fn open(&mut self, port_entries: &[crate::port_map::PortEntry]) {
        self.inspection_tab = InspectionTab::Summary;
        self.inspection_scroll = 0;
        self.inspection_search.clear();
        if let Some(p) = self.detail_process.as_ref() {
            self.inspection_data = Some(crate::inspect::inspect_with_ports(p.pid, port_entries));
            self.inspection_handles_data =
                Some(crate::inspect::handles::collect_handles(p.pid).unwrap_or_default());
            self.inspection_memory_data =
                Some(crate::inspect::memory::collect_memory(p.pid).unwrap_or_default());
        } else {
            self.inspection_data = None;
            self.inspection_handles_data = None;
            self.inspection_memory_data = None;
        }
        self.refresh_detail_priority();
    }

    /// heavy tick 周期：从 `cached_processes` 找最新副本覆盖 `detail_process`；
    /// 进程已死则清空。原本在 `App::tick` heavy 分支内联实现。
    pub fn sync_detail(&mut self, cached: &[ProcessInfo]) {
        let Some(detail) = self.detail_process.as_mut() else {
            return;
        };
        let pid = detail.pid;
        if let Some(latest) = cached.iter().find(|p| p.pid == pid) {
            *detail = latest.clone();
        } else {
            self.detail_process = None;
        }
    }

    /// 关闭详情页：清搜索 / 句柄 / 内存（保留 env_reveal 用户偏好）。
    pub fn close(&mut self) {
        self.inspection_search.clear();
    }

    /// 阶段 11 P1-A3：刷新 `detail_priority` 缓存（priority label + affinity label）。
    /// 在 4 个点调用：进入详情页 / `F5` 刷新 / `+/-` 调整后 / heavy tick 周期。
    /// 若 `detail_process` 为 None（详情页关闭），清空缓存避免脏数据。
    pub fn refresh_detail_priority(&mut self) {
        let Some(p) = self.detail_process.as_ref() else {
            self.detail_priority = None;
            return;
        };
        let priority_label = match crate::process_control::get_priority(p.pid) {
            Ok(c) => c.label().to_string(),
            Err(_) => "-".to_string(),
        };
        let affinity_label = match crate::process_control::get_affinity(p.pid) {
            Ok(mask) => format!("0x{:X} (CPU 数: {})", mask, u64::count_ones(mask)),
            Err(_) => "-".to_string(),
        };
        self.detail_priority = Some((priority_label, affinity_label));
    }

    /// `draw_env_tab` 用：reveal 计算考虑录屏（录屏中即便 `env_reveal=true` 也
    /// 强制 mask，防录到 secret 真值）。
    #[must_use]
    pub fn env_render_reveal(&self, recording: bool) -> bool {
        self.env_reveal && !recording
    }

    /// 详情页主键盘路由。原 `App::handle_detail_key` 整体迁过来。
    ///
    /// `port_entries` 为 `port_panel.port_entries` 副本（`F5` 刷新时用）。
    /// `recording` = `App::recording_wanted`（`v` 键判断是否禁止 reveal）。
    /// 返回 [`InspectorAction`]；副作用由 App 派发。
    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        port_entries: &[crate::port_map::PortEntry],
        recording: bool,
    ) -> InspectorAction {
        // Search active → 优先吃输入；只有 Esc/Enter 走 SearchState 自带的退出。
        if self.inspection_search.is_active() {
            // 让 Esc 只退出搜索（保留 detail 视图），用户再按一次 Esc 才回 ProcessList。
            let consumed = self.inspection_search.handle_input(key);
            if consumed {
                self.inspection_scroll = 0;
                return InspectorAction::Noop;
            }
            // 搜索时 Tab/BackTab 切 Tab 无意义 → 吞掉。
            if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
                return InspectorAction::Noop;
            }
            // 其它键（Up/Down/PageUp/...）继续走常规分支，方便在过滤结果里滚动。
        }

        match key.code {
            // Tab / Shift+Tab 切换 Inspector 内部 Tab（避开 1-6 主面板切换）。
            KeyCode::Tab => {
                self.inspection_tab = self.inspection_tab.next();
                self.inspection_scroll = 0;
            }
            KeyCode::BackTab => {
                self.inspection_tab = self.inspection_tab.prev();
                self.inspection_scroll = 0;
            }
            // `F5` 强制重新采集（用户怀疑数据过期 / 权限变化时）。
            // v0.6.0 阶段 6：原 `r` 迁移到 `F5`，对齐 Mission Center / htop 刷新习惯，
            // 同时消除详情页 / Docker `r` restart / USB `r` 刷新设备三语义冲突。
            KeyCode::F(5) => {
                let Some(pid) = self.detail_process.as_ref().map(|p| p.pid) else {
                    return InspectorAction::Noop;
                };
                self.inspection_data = Some(crate::inspect::inspect_with_ports(pid, port_entries));
                // 阶段 4：handles/memory 也要刷一次（r 不应只刷新 env/dlls/net）。
                self.inspection_handles_data =
                    Some(crate::inspect::handles::collect_handles(pid).unwrap_or_default());
                self.inspection_memory_data =
                    Some(crate::inspect::memory::collect_memory(pid).unwrap_or_default());
                self.inspection_scroll = 0;
                // 阶段 11 P1-A3：r 重新采集时也刷 priority/affinity 缓存。
                self.refresh_detail_priority();
                return InspectorAction::StatusMsg(format!("已刷新 Inspector 数据 (PID {})", pid));
            }
            // `+` / `-`：阶段 4 A4，在 Summary Tab 上调高/调低进程优先级。
            // 仅详情页生效（避免与 Replay 速度调节冲突 —— replay 在另一个分支）。
            KeyCode::Char('+') | KeyCode::Char('=') => {
                if let Some(pid) = self.detail_process.as_ref().map(|p| p.pid) {
                    return InspectorAction::BumpPriority { pid, up: true };
                }
            }
            KeyCode::Char('-') => {
                if let Some(pid) = self.detail_process.as_ref().map(|p| p.pid) {
                    return InspectorAction::BumpPriority { pid, up: false };
                }
            }
            // `/` 进入搜索（Env/Dlls 数据量大时必须）。
            KeyCode::Char('/') => {
                self.inspection_search.active = true;
                self.inspection_scroll = 0;
            }
            // v0.6.0 阶段 2：v 切换 Env Tab 的 secret 脱敏；录屏中拒绝。
            KeyCode::Char('v') => {
                if recording {
                    self.env_reveal = false;
                    return InspectorAction::StatusMsg(
                        "录屏中禁止 reveal env secret（已强制 mask）".to_string(),
                    );
                }
                self.env_reveal = !self.env_reveal;
                return InspectorAction::StatusMsg(if self.env_reveal {
                    "Env: 显示真值（仅本会话，录屏强制 mask）".to_string()
                } else {
                    "Env: 已 mask secret".to_string()
                });
            }
            // 上下滚动：Summary 走整页滚动；Env/Dlls/Network 在 Tab 内部滚动。
            KeyCode::Up => {
                self.inspection_scroll = self.inspection_scroll.saturating_sub(1);
            }
            KeyCode::Down => {
                self.inspection_scroll = self.inspection_scroll.saturating_add(1);
            }
            KeyCode::PageUp => {
                self.inspection_scroll = self.inspection_scroll.saturating_sub(10);
            }
            KeyCode::PageDown => {
                self.inspection_scroll = self.inspection_scroll.saturating_add(10);
            }
            KeyCode::Home => {
                self.inspection_scroll = 0;
            }
            KeyCode::End => {
                self.inspection_scroll = usize::MAX / 2;
            }
            KeyCode::Enter | KeyCode::Esc => {
                self.inspection_search.clear();
                return InspectorAction::Close;
            }
            KeyCode::Char('k') => {
                if let Some(pid) = self.detail_process.as_ref().map(|p| p.pid) {
                    return InspectorAction::KillPid(pid);
                }
            }
            KeyCode::Char('w') => {
                if let Some(pid) = self.detail_process.as_ref().map(|p| p.pid) {
                    return InspectorAction::AddMonitor(pid);
                }
            }
            // v0.6.0 阶段 6：原 `c` 迁移到 `y`（vim yank 风格），消除
            // 全局 `c` 侧边栏折叠 / 详情页 `c` 复制双语义冲突。
            KeyCode::Char('y') => {
                if let Some(info) = self
                    .detail_process
                    .as_ref()
                    .map(crate::tree::format_process_info)
                {
                    return InspectorAction::CopyInfo(info);
                }
            }
            // v0.6.0 阶段 8（REVIEW-7.md P1-6）：保留 'r' / 'c' 兼容期 deprecation
            // warning。0.5.0 用户升级后肌肉记忆按 'r' 刷新 / 'c' 复制 → 给一句话指
            // 引到 F5 / y，避免「按了没反应」的 UX 退化。v0.7.0 计划移除。
            KeyCode::Char('r') => {
                return InspectorAction::StatusMsg(
                    "⚠ 'r' 将在 v0.7.0 移除，请用 F5 刷新".to_string(),
                );
            }
            KeyCode::Char('c') => {
                return InspectorAction::StatusMsg(
                    "⚠ 'c' 将在 v0.7.0 移除，请用 y 复制（vim yank）".to_string(),
                );
            }
            // v0.7 阶段 6：T 切换 Windows 11 EcoQoS / Efficiency Mode（ADR-0014）。
            // 当前 Eco → 切回 Normal；否则切到 Eco。非 Windows 平台 App 派发时
            // set_throttle 会返回错误并写 status_message。
            KeyCode::Char('T') => {
                if let Some(p) = self.detail_process.as_ref() {
                    return InspectorAction::ToggleEcoQoS {
                        pid: p.pid,
                        make_eco: p.throttled != crate::throttle::EcoQoSState::Eco,
                    };
                }
            }
            _ => {}
        }
        InspectorAction::Noop
    }
}

/// `InspectorController::handle_key` 的返回值。副作用（写 App 状态、kill、剪贴板）
/// 通过此枚举让 App 派发，controller 不持 App 引用。
#[derive(Debug)]
pub enum InspectorAction {
    /// 默认分支 / 已在 controller 内消化完的状态变更。
    Noop,
    /// 写 `App::status_message`（v 切换 / r 刷新结果）。
    StatusMsg(String),
    /// 关闭详情页回 ProcessList（Enter/Esc）。
    Close,
    /// `+`/`-` 调整优先级。App 收到后调 `App::bump_priority(pid, up)`。
    BumpPriority { pid: u32, up: bool },
    /// `k` 终止进程。App 调 `kill::kill_process` + `record_op` + 切回 List。
    KillPid(u32),
    /// `w` 添加监控。App 调 `monitor_panel.manager.add_monitor`。
    AddMonitor(u32),
    /// `y` 复制进程信息到剪贴板。App 调 `arboard::Clipboard::set_text`。
    /// v0.6.0 阶段 6：原 `c` 迁移到 `y`（vim yank 风格）。
    CopyInfo(String),
    /// v0.7 阶段 6：`T` 切换 EcoQoS。App 调 `throttle::set_throttle` +
    /// 写 status_message（ADR-0014）。
    ToggleEcoQoS { pid: u32, make_eco: bool },
}
