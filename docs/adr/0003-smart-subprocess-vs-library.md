# ADR-0003: SMART 采集走 smartctl 子进程而非 libatasmart

- **Status**: Accepted
- **Date**: 2026-06-16
- **Phase**: v0.5.0 阶段 5

## 背景

需要跨平台采集 SMART 磁盘健康数据（温度、属性表、预测失败状态）。Linux/macOS/Windows 三平台都要支持。

## 选项

| 方案 | 优点 | 缺点 |
|---|---|---|
| A. libatasmart（libsmi 绑定）| 无子进程开销 | 维护停滞（2013 年最后一版）、**完全不支持 NVMe**、Windows 完全不支持、bindgen 配置麻烦 |
| B. **smartctl 子进程 + JSON 解析** | smartmontools 持续维护、JSON schema 7.0+ 稳定、跨平台覆盖、依赖管理干净、30s poll 周期下子进程开销可接受 | 需要 PATH 有 smartctl、子进程 spawn 延迟 ~50ms |
| C. 直接走 OS 原生（Linux `/sys/class/block/*/device/smart` / Windows WMI） | 无外部依赖 | Linux 路径每个发行版不同；Windows WMI `MSStorageDriver_FailurePredictStatus` 只给布尔聚合，无详细属性表 |

## 决策

采用方案 B（smartctl 子进程），降级用方案 C 的 Windows WMI 路径。理由：

1. **NVMe 支持**：现代 SSD 都是 NVMe，libatasmart 完全不支持 → 一票否决
2. **跨平台一致**：smartctl 在 Linux/macOS/Windows 都装得上
3. **JSON 解析稳定**：smartmontools 7.0+ 的 JSON schema 文档化良好，缺字段优雅降级
4. **依赖管理干净**：无需 bindgen / 无 native lib 装机要求
5. **降级路径**：Windows 未装 smartctl 时退到 WMI，至少给个聚合状态

## 后果

- 正面：跨平台 30s poll 一致工作，Windows WMI 降级路径已落地
- 正面：JSON 解析是纯函数 `parse_smartctl_json(content)`，单测覆盖完整
- 负面：用户必须装 smartmontools；FAQ 已说明
- 负面：30s 周期下子进程 spawn ~50ms × N 盘，对大容量服务器开销累积

## 参考

- v0.5.0 阶段 5 落地：CHANGELOG.md
- 相关代码：`src/smart/mod.rs`
- 同类取舍：[ADR-0004](./0004-gpu-via-nvtop-subprocess.md)（nvtop）、[ADR-0005](./0005-netflow-windows-iphelper-not-etw.md)（IP Helper）
