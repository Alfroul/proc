//! v0.7.0 阶段 5：MonitorPanel 的 controller 包装层（ADR-0012）。
//!
//! 同 process / port / usb 模板。monitor 涉及 watchdog spawn，与 v0.6 阶段 2
//! 的 restricted_spawn 有交互（已在阶段 1 TD-10 / TD-11 处理），本层不动 spawn
//! 逻辑，仅做字段 / 方法包装。

use crossterm::event::KeyEvent;

use crate::app_panel::{Panel, PanelAction, PanelContext};
use crate::view_models::monitor_panel::MonitorPanel;

/// MonitorPanel 的 controller 包装（v0.7.0 阶段 5）。
pub struct MonitorPanelController {
    pub panel: MonitorPanel,
}

impl MonitorPanelController {
    #[must_use]
    pub fn new(panel: MonitorPanel) -> Self {
        Self { panel }
    }

    #[must_use]
    pub fn panel(&self) -> &MonitorPanel {
        &self.panel
    }

    #[must_use]
    pub fn panel_mut(&mut self) -> &mut MonitorPanel {
        &mut self.panel
    }

    pub fn handle_key(&mut self, key: KeyEvent, ctx: &mut PanelContext) -> PanelAction {
        PanelAction::from(self.panel.handle_key(key, ctx))
    }

    pub fn tick(&mut self, ctx: &mut PanelContext) -> bool {
        self.panel.tick(ctx)
    }
}
