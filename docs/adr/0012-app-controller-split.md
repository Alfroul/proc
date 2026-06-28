# ADR-0012: App split into 5 panel controllers

## Status

**Accepted** — v0.7.0 阶段 5 引入（落地 TD-6）

## Context

v0.6.0 阶段 5 已拆出 `InspectorController` / `ReplayController` / `WorkerManager`（共 15 字段），但 `src/app.rs` 仍 **1715 行 / 67 函数 / 40+ 字段**。剩余 5 个 panel 字段（`process_panel` / `port_panel` / `usb_panel` / `monitor_panel` / `docker_panel`）仍直接持在 App：

```rust
pub struct App {
    // ...
    pub process_panel: ProcessPanel,
    pub port_panel: PortPanel,
    pub usb_panel: UsbPanel,
    pub monitor_panel: MonitorPanel,
    pub docker_panel: DockerPanel,
    // ... 40+ 字段
}
```

问题：

1. **App 仍是协调器 + 部分状态混合**：新功能加 panel 时 App 字段继续膨胀
2. **handle_key 单一长 match**：每个 mode 都有 inline 逻辑，难并行开发
3. **测试访问路径深**：`tests/test_inspector.rs` 直接访问 `app.process_panel.cursor_index` 等，迁移成本高但必须做

## Decision

**把 5 个 panel 字段各自包装成 Controller（`ProcessPanelController` 等），App 只持 controller 引用 + 全局状态。`handle_key` 返回 `PanelAction` 枚举让 App 派发副作用（与 v0.6 阶段 5 InspectorController 同款 event-based 模式）。**

具体决策：

1. **5 个 controller 全是具体类型**（不引入 `trait PanelController` + Box）
   - App 字段：`process: ProcessPanelController` / `port: PortPanelController` / `usb: UsbPanelController` / `monitor: MonitorPanelController` / `docker: DockerPanelController`
   - 理由：ratatui 50ms tick 下，trait object 的 v-table dispatch 不必要；具体类型 + 编译期穷尽 match 更安全
   - 反例：如果未来有"动态加 panel"需求（如插件系统），再考虑 trait object

2. **每个 controller 复用 v0.6.0 阶段 5 验证过的模式**：
   ```rust
   pub struct ProcessPanelController {
       pub panel: ProcessPanel,  // 原 struct 内嵌，逻辑不动
   }

   impl ProcessPanelController {
       pub fn new(...) -> Self { ... }
       pub fn handle_key(&mut self, key: KeyEvent, ctx: &PanelContext) -> PanelAction { ... }
       pub fn tick(&mut self, snapshot: &SystemSnapshot) { ... }
       pub fn panel(&self) -> &ProcessPanel { &self.panel }
       pub fn panel_mut(&mut self) -> &mut ProcessPanel { &mut self.panel }
   }
   ```
   - `ProcessPanel` 本身保留（不重命名 / 不拆字段），只是包装层
   - 测试访问路径：`app.process.cursor_index` → `app.process.panel().cursor_index`

3. **`PanelAction` 枚举统一副作用**：
   ```rust
   pub enum PanelAction {
       Noop,
       StatusMessage(String),
       Kill(KillRequest),
       RecordOp(RecordOp),
       Clipboard(String),
       Monitor(MonitorRequest),
       SwitchMode(AppMode),
       // ...
   }
   ```
   - App::handle_key 收到 PanelAction 后 dispatch 副作用
   - Controller 不直接 mutate App 全局状态（解耦）
   - **不立刻合并 InspectorAction**：先共存，验证 1 cycle 后再合并（surgical 原则）

4. **App::handle_key 简化为 dispatch**：
   - 入口处先 match AppLayer（v0.7 阶段 3 引入）→ Normal / Search / Palette
   - Normal 层 match mode（ProcessList / PortPanel / Inspector / ...）
   - 每个 mode 调对应 controller 的 `handle_key`，App 只处理全局键（1-6 切面板 / Ctrl+P / q 等）+ PanelAction 翻译

5. **保留 v0.6 公开 API 兼容性**：
   - App 字段从 `pub process_panel: ProcessPanel` → `pub process: ProcessPanelController`
   - 外部访问 `app.process_panel.cursor_index` 失败，但通过 `.panel()` 方法获取
   - **逐个测试文件跑**（不 batch 改），每修一个跑一次该文件，避免失败难定位

6. **不并行做**：5 个 controller 一个一个拆
   - ProcessPanelController 最复杂（应用分组 / 多选 / 排序 / 搜索 hot path）先做，作为模板
   - 后 4 个照葫芦画瓢
   - 每个拆完跑全量回归

## Alternatives Considered

### A. 引入 `trait PanelController` + `Box<dyn PanelController>`

**否决理由**：
- 性能：每 tick 5 次 v-table dispatch（不必要）
- 编译期类型安全：trait object 失去穷尽 match
- 5 个 controller 类型固定，未来扩展可能性低（不像插件系统）
- 适用场景：动态插件 / 运行时配置 panel 列表

### B. 不拆，App 继续持 panel 字段

**否决理由**：
- App 已经 40+ 字段 / 1715 行，新功能继续膨胀
- v0.6 阶段 5 已经定下"拆 controller"路径，TD-6 是收尾

### C. 把所有 panel 合并到一个 ViewModel

**否决理由**：
- 5 个 panel 状态正交，合并等于全局变量集合
- 更难并行开发（一个开发者改 ViewModel 影响所有人）

### D. 用 ECS（entity-component-system）架构

**否决理由**：
- ECS 适合游戏 / 大量实体场景，proc 的 5 个 panel 用不上
- 引入 beams / hecs 等库过重

### E. 立刻合并 InspectorAction + PanelAction

**否决理由**：
- surgical 原则：一次只改一件事
- 先共存验证 1 cycle（v0.7.0），v0.8.0 评估合并
- 同时改可能引入未发现的接口不一致

## Consequences

### 正面

- **App 字段数 40+ → ≤ 20**：5 个 controller + 全局状态字段
- **App::handle_key 行数减少 50%+**：从 inline 长匹配简化为 dispatch
- **可并行开发**：新功能加 panel 时只改对应 controller，不动 App
- **测试访问更明确**：`app.process.panel()` 显式获取，比裸字段更明确边界
- **未来扩展容易**：新加 panel = 新加 controller 字段，App 字段数线性而非指数膨胀

### 负面

- **测试访问路径变**：100+ 处 `app.xxx_panel.field` 改为 `app.xxx_panel.panel().field`
- **`tests/test_inspector.rs`**：v0.6.0 阶段 5 已经历类似迁移（40+ 处改），有经验
- **PanelAction 增加间接层**：副作用要走 enum + match dispatch
- **App::handle_key 仍需全局键 match**：1-6 切面板 / Ctrl+P / q 等不能下沉到 controller

### 缓解

- v0.6 阶段 5 InspectorController 拆分已验证可行（tests/test_inspector.rs 40+ 处改完成）
- controller 拆分一个跑一次全量回归，确保每步可验证
- PanelAction 与 InspectorAction 共存，不立刻合并，留 v0.8 评估

## Implementation Notes

- 入口：`src/app.rs::App` 字段重构（5 个 controller）
- Controller 实现：`src/view_models/{process,port,usb,monitor,docker}/controller.rs`
- PanelAction 枚举：`src/app_panel.rs::PanelAction`
- 测试：`tests/test_panel_controllers.rs`（5 个 controller 各 1 个 case）
- 兼容性：旧 `tests/test_inspector.rs` 40+ 处访问路径在 v0.6.0 已迁移完成，本阶段不动 inspector

## Verification Metrics

| 指标 | v0.6 baseline | v0.7 目标 |
|---|---|---|
| `wc -l src/app.rs` | 1715 | ≤ 1000 |
| `grep -c "process_panel\|port_panel\|usb_panel\|monitor_panel\|docker_panel" src/app.rs` | ~40+ | ≤ 20 |
| App 字段数 | 40+ | ≤ 20 |
| App::handle_key 行数 | ~150 | ≤ 75 |

## References

- proc v0.6.0 `docs/stages/stage-5.md`（InspectorController 拆分参考）
- proc v0.6.0 `src/inspect/controller.rs`（同款 event-based 模式）
- proc v0.6.0 `docs/tech-debt.md` TD-6（本 ADR 落地的源）
- ECS 架构对比（否定项）：https://github.com/bevyengine/bevy
