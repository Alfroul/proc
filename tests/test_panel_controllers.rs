//! v0.7.0 阶段 5 — PanelController 集成测试（ADR-0012）。
//!
//! 覆盖 5 个 controller 的：
//! 1. 包装构造：`XxxPanelController::new(XxxPanel::new())` 成功。
//! 2. 访问器：`panel()` / `panel_mut()` 返回 inner panel，可读 / 可写。
//! 3. App 集成：`App::new()` 后 `app.xxx_panel.panel` 路径访问合法。
//!
//! 不测具体 handle_key 行为（已由 test_inspector / test_command_palette 等
//! 现有测试覆盖）。本文件只验证「controller 包装层」API 表面完整。

use proc::app::App;
use proc::view_models::{
    DockerPanel, DockerPanelController, MonitorPanel, MonitorPanelController, PortPanel,
    PortPanelController, ProcessPanel, ProcessPanelController, UsbPanel, UsbPanelController,
};

// ── Test 1：ProcessPanelController ──────────────────────────────────────────

#[test]
fn process_panel_controller_wraps_panel() {
    let inner = ProcessPanel::new(&[]);
    let mut ctrl = ProcessPanelController::new(inner);
    // 访问器：读
    assert_eq!(ctrl.panel().cursor_index, 0);
    assert_eq!(ctrl.panel().scroll_offset, 0);
    // 访问器：写
    ctrl.panel_mut().cursor_index = 7;
    assert_eq!(ctrl.panel().cursor_index, 7);
    // 字段路径访问（v0.7 ADR-0012 推荐 `.panel.field`）
    ctrl.panel.cursor_index = 9;
    assert_eq!(ctrl.panel.cursor_index, 9);
}

// ── Test 2：PortPanelController ─────────────────────────────────────────────

#[test]
fn port_panel_controller_wraps_panel() {
    let inner = PortPanel::new();
    let mut ctrl = PortPanelController::new(inner);
    // PortPanel 初始 port_cursor = 0
    assert_eq!(ctrl.panel().port_cursor, 0);
    ctrl.panel_mut().port_cursor = 3;
    assert_eq!(ctrl.panel().port_cursor, 3);
    // 字段路径访问
    ctrl.panel.port_cursor = 5;
    assert_eq!(ctrl.panel.port_cursor, 5);
}

// ── Test 3：UsbPanelController ──────────────────────────────────────────────

#[test]
fn usb_panel_controller_wraps_panel() {
    let inner = UsbPanel::new();
    let ctrl = UsbPanelController::new(inner);
    // UsbPanel 初始 device_cursor = 0
    assert_eq!(ctrl.panel().device_cursor, 0);
    // 字段路径访问
    let _ = ctrl.panel.devices.len();
}

// ── Test 4：MonitorPanelController ──────────────────────────────────────────

#[test]
fn monitor_panel_controller_wraps_panel() {
    let inner = MonitorPanel::new();
    let ctrl = MonitorPanelController::new(inner);
    // MonitorPanel add_submenu 初始为 None
    assert!(ctrl.panel().add_submenu.is_none());
    // 字段路径访问
    let _ = &ctrl.panel.manager;
}

// ── Test 5：DockerPanelController ───────────────────────────────────────────

#[test]
fn docker_panel_controller_wraps_panel() {
    let inner = DockerPanel::new();
    let ctrl = DockerPanelController::new(inner);
    // DockerPanel 初始 cursor = 0、connected = false
    assert_eq!(ctrl.panel().cursor, 0);
    assert!(!ctrl.panel().connected);
    // 字段路径访问
    let _ = ctrl.panel.containers.len();
}

// ── Test 6：App 持 controller 路径 ─────────────────────────────────────────

#[test]
fn app_holds_5_controllers_via_panel_path() {
    // App::new 在 Windows 上调 sysinfo（实际采集）；Linux/macOS 也走得通。
    // 失败说明构建期 / cfg 问题，不说明 controller 包装问题。
    let app = App::new().expect("App::new");
    // 5 个 controller 字段访问路径合法（编译期保证；这里跑一次运行时确认）。
    assert_eq!(app.process_panel.panel.cursor_index, 0);
    assert_eq!(app.port_panel.panel.port_cursor, 0);
    assert_eq!(app.usb_panel.panel.device_cursor, 0);
    assert!(app.monitor_panel.panel.add_submenu.is_none());
    assert_eq!(app.docker_panel.panel.cursor, 0);
}
