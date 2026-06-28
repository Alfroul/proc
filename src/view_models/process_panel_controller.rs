//! v0.7.0 阶段 5：ProcessPanel 的 controller 包装层（ADR-0012）。
//!
//! v0.6 App 直接持 `process_panel: ProcessPanel`，v0.7 改成持 controller。
//! controller 提供 `panel()` / `panel_mut()` 访问器，`handle_key` 把 inner
//! panel 的 `KeyResult` 翻译成 `PanelAction` 让 App 派发副作用。
//!
//! ProcessPanel 本身不重命名 / 不拆字段（surgical：保留 v0.6 公开 API 表面）。
//! 外部访问路径：`app.process_panel.cursor_index` →
//! `app.process_panel.panel().cursor_index`（或 `app.process_panel.panel.field`）。

use crossterm::event::KeyEvent;

use crate::app_panel::{Panel, PanelAction, PanelContext};
use crate::collect::ProcessInfo;
use crate::tree::TreeNode;
use crate::view_models::process_panel::ProcessPanel;

/// ProcessPanel 的 controller 包装（v0.7.0 阶段 5）。
///
/// 字段 `panel` 持原 `ProcessPanel`（不重命名 / 不拆字段），方法层只做访问器
/// 与 `handle_key` 翻译。App 字段名仍叫 `process_panel`（只换类型），让外部
/// import 路径 `app.process_panel` 不变，仅多一层 `.panel` / `.panel()`。
pub struct ProcessPanelController {
    /// 内嵌原 ProcessPanel；v0.6 字段 / 方法全保留。
    pub panel: ProcessPanel,
}

impl ProcessPanelController {
    /// 包装一个已构造好的 ProcessPanel（App::new 内继续走 v0.6 构造路径）。
    #[must_use]
    pub fn new(panel: ProcessPanel) -> Self {
        Self { panel }
    }

    /// 拆 controller 的访问器（v0.7 阶段 5 ADR-0012 约定）。`pub panel` 字段
    /// 同时支持 `c.panel.field` / `c.panel().field`，调用方按需挑。
    #[must_use]
    pub fn panel(&self) -> &ProcessPanel {
        &self.panel
    }

    #[must_use]
    pub fn panel_mut(&mut self) -> &mut ProcessPanel {
        &mut self.panel
    }

    /// v0.6 `Panel::handle_key` 的转发 + 翻译。App::handle_key ProcessList 分支
    /// 调此方法，拿到 `PanelAction` 后 dispatch。
    pub fn handle_key(&mut self, key: KeyEvent, ctx: &mut PanelContext) -> PanelAction {
        let r = self.panel.handle_key(key, ctx);
        PanelAction::from(r)
    }

    /// 同款转发 `Panel::tick`。返回 dirty 标志（数据是否变化，触发重绘）。
    pub fn tick(&mut self, ctx: &mut PanelContext) -> bool {
        self.panel.tick(ctx)
    }

    /// 透传常用方法 —— v0.6 App.rs 大量调用 `self.process_panel.init_tree(...)`
    /// 等，包了 controller 后通过这些 forward 避免每次 `.panel.` 套娃。
    /// 仅给高频 API 加 forward；冷门方法走 `c.panel().xxx()` 显式获取。
    pub fn init_tree(&mut self, processes: &[ProcessInfo], total_mem: u64) {
        self.panel.init_tree(processes, total_mem);
    }

    /// rebuild tree 后塞新 nodes（replay restore_replay_panel_data 路径用）。
    pub fn set_tree_nodes(&mut self, nodes: Vec<TreeNode>) {
        self.panel.tree_nodes = nodes;
    }
}
