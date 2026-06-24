# ADR-0005: Windows per-process 网络流量走 IP Helper 而非 ETW

- **Status**: Accepted
- **Date**: 2026-06-17
- **Phase**: v0.5.0 阶段 7

## 背景

需要 per-process 网络流量（bytes/sec sent + recv）。Windows 上有两个候选数据源。

## 选项

| 方案 | 优点 | 缺点 |
|---|---|---|
| A. ETW（Event Tracing for Windows）实时 session | 全协议覆盖（TCP/UDP/IPv4/IPv6）、kernel 级精度 | ~500 行 unsafe FFI 脚手架、独立消费者线程、`ProcessTrace` 阻塞调用、ROI 不匹配 |
| B. **IP Helper（GetTcpTable2 + GetPerTcpConnectionEStats + netstat2 PID join）** | 复用 [src/estats] 已测的同款 Win32 调用、1s poll 下 CPU < 1%、非管理员通常仍可工作 | 仅覆盖 IPv4 TCP；UDP 无 per-PID 字节速率概念；IPv6 路径留后续 |

## 决策

采用方案 B（IP Helper）。理由：

1. **ROI**：500 行 unsafe FFI vs 150 行复用现有代码
2. **复用 estats 同款调用**：`SetPerTcpConnectionEStats` / `GetPerTcpConnectionEStats` 已在 estats 模块验证过
3. **非管理员友好**：`SetPerTcpConnectionEStats` 在非管理员下通常仍可工作
4. **trait 抽象保留迁移空间**：`NetFlowCollector` trait 让未来切 ETW 是 additive（新增 impl + detect_collector 分支）

## 后果

- 正面：1s poll 下 worker CPU < 1%（实测）
- 正面：trait 抽象让 v0.6.0+ IPv6 路径（`GetPerTcp6ConnectionEStats`）和 ETW 全协议覆盖都是 additive
- 已知限制：仅 IPv4 TCP；UDP 无 per-PID 字节速率；非管理员下部分其它进程的连接可能拿不到字节（显示 0B/s）
- 已知限制：PID 复用检测（当前累计 < 上次累计 → 视为新进程，速率按 0 计）

## 参考

- v0.5.0 阶段 7 落地：CHANGELOG.md
- 相关代码：`src/net_flow/{mod.rs,windows.rs,worker.rs}`
- 同类取舍：[ADR-0006](./0006-dns-subprocess-not-etw-dbus.md)（同样放弃 ETW）
