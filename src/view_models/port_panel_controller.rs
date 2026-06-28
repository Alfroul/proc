//! v0.7.0 阶段 5：PortPanel 的 controller 包装层（ADR-0012）。
//!
//! 同 [`crate::view_models::process_panel_controller`] 模板：包装 `PortPanel`
//! 不动其字段 / 方法，提供 `panel()` / `panel_mut()` 访问器 + `handle_key`
//! forward。App 字段名仍叫 `port_panel`，仅类型从 `PortPanel` 换成 controller。

use crossterm::event::KeyEvent;

use crate::app_panel::{Panel, PanelAction, PanelContext};
use crate::view_models::port_panel::PortPanel;

/// PortPanel 的 controller 包装（v0.7.0 阶段 5）。
pub struct PortPanelController {
    pub panel: PortPanel,
}

impl PortPanelController {
    #[must_use]
    pub fn new(panel: PortPanel) -> Self {
        Self { panel }
    }

    #[must_use]
    pub fn panel(&self) -> &PortPanel {
        &self.panel
    }

    #[must_use]
    pub fn panel_mut(&mut self) -> &mut PortPanel {
        &mut self.panel
    }

    pub fn handle_key(&mut self, key: KeyEvent, ctx: &mut PanelContext) -> PanelAction {
        PanelAction::from(self.panel.handle_key(key, ctx))
    }

    pub fn tick(&mut self, ctx: &mut PanelContext) -> bool {
        self.panel.tick(ctx)
    }
}
