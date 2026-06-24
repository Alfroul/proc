# ADR-0002: Inspector Tab 扩展机制（enum + match）

- **Status**: Accepted
- **Date**: 2026-06-15
- **Phase**: v0.5.0 阶段 4

## 背景

详情页要支持 6 个 Tab（Summary / Env / Network / Dlls / Handles / Memory），需要选择扩展机制。未来可能加 Threads / Services / .NET CLR 等 Tab。

## 选项

| 方案 | 优点 | 缺点 |
|---|---|---|
| A. trait object（`Box<dyn InspectorTab>`） | 运行时可增删 Tab、插件化 | 数据共享通过 trait 抽象成本高、vtable 开销、字段独立加载困难 |
| B. enum + match（`InspectionTab` 枚举） | 编译期穷尽性、字段直接挂在 App 便于按需加载、`label()` 作测试 anchor | 加新 Tab 要改枚举 + match（可接受）|

## 决策

采用方案 B（enum + match）。理由：

1. **编译期穷尽性 > 运行时灵活性**：加新 Tab 时编译器会强制把所有 match 分支补全，避免漏处理
2. **数据量小，vtable 开销无意义**：6 个 Tab 的数据结构已知，不需要 trait object 的开放扩展
3. **字段独立加载**：Handles / Memory 数据量大且采集昂贵，必须 lazy load；enum + 独立字段（`inspection_handles_data: Option<Vec<HandleInfo>>`）让 lazy 加载语义清晰
4. **`label()` 方法作测试 anchor**：每个变体的 `label()` 返回稳定字符串，集成测试断言全 Tab 循环时不依赖 `Debug` 格式

## 后果

- 正面：v0.5.0 阶段 4 加 Handles / Memory 两 Tab 只需改 enum + match，460+ 行新代码零回归
- 正面：`InspectionTab::all()` / `next()` / `prev()` / `label()` 4 个方法都有 `#[must_use]`，强制消费
- 负面：未来加 Threads / .NET CLR 等仍需改枚举（但成本可控）

## 参考

- v0.5.0 阶段 4 落地：CHANGELOG.md
- 相关代码：`src/tui/detail_view.rs::InspectionTab`
