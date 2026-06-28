//! v0.7.0 阶段 5：UsbPanel 的 controller 包装层（ADR-0012）。
//!
//! 同 process / port 模板。App 字段名仍叫 `usb_panel`，仅类型换成 controller。

use crossterm::event::KeyEvent;

use crate::app_panel::{Panel, PanelAction, PanelContext};
use crate::view_models::usb_panel::UsbPanel;

/// UsbPanel 的 controller 包装（v0.7.0 阶段 5）。
pub struct UsbPanelController {
    pub panel: UsbPanel,
}

impl UsbPanelController {
    #[must_use]
    pub fn new(panel: UsbPanel) -> Self {
        Self { panel }
    }

    #[must_use]
    pub fn panel(&self) -> &UsbPanel {
        &self.panel
    }

    #[must_use]
    pub fn panel_mut(&mut self) -> &mut UsbPanel {
        &mut self.panel
    }

    pub fn handle_key(&mut self, key: KeyEvent, ctx: &mut PanelContext) -> PanelAction {
        PanelAction::from(self.panel.handle_key(key, ctx))
    }

    pub fn tick(&mut self, ctx: &mut PanelContext) -> bool {
        self.panel.tick(ctx)
    }
}
