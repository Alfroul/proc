//! v0.7.0 阶段 5：DockerPanel 的 controller 包装层（ADR-0012）。
//!
//! 同前 4 个模板，但 docker 自管 logs worker 生命周期（v0.7 阶段 1 TD-5 已暴露
//! metrics 接口）+ ContainerExec 状态。本层不动这些 —— `panel` 字段就是原
//! `DockerPanel`，logs worker 句柄 / event_receiver / metrics 都跟 panel 一起
//! 被 controller 持有，App 通过 `app.docker_panel.panel()` 访问。

use crossterm::event::KeyEvent;

use crate::app_panel::{Panel, PanelAction, PanelContext};
use crate::view_models::docker_panel::DockerPanel;

/// DockerPanel 的 controller 包装（v0.7.0 阶段 5）。
///
/// logs worker spawn / drop 时序仍由 inner `DockerPanel` 自管，controller
/// 不接管。ContainerExec 视图模式触发（`e` 键）也走 inner panel。
pub struct DockerPanelController {
    pub panel: DockerPanel,
}

impl DockerPanelController {
    #[must_use]
    pub fn new(panel: DockerPanel) -> Self {
        Self { panel }
    }

    #[must_use]
    pub fn panel(&self) -> &DockerPanel {
        &self.panel
    }

    #[must_use]
    pub fn panel_mut(&mut self) -> &mut DockerPanel {
        &mut self.panel
    }

    pub fn handle_key(&mut self, key: KeyEvent, ctx: &mut PanelContext) -> PanelAction {
        PanelAction::from(self.panel.handle_key(key, ctx))
    }

    pub fn tick(&mut self, ctx: &mut PanelContext) -> bool {
        self.panel.tick(ctx)
    }
}
