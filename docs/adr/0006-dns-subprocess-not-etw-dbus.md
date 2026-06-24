# ADR-0006: DNS 查询日志走 PowerShell 子进程而非 ETW / DBus

- **Status**: Accepted
- **Date**: 2026-06-18
- **Phase**: v0.5.0 阶段 8

## 背景

需要 per-process DNS 查询日志（哪个进程查了哪个域名）。Windows 上有 ETW，Linux 上原计划走 systemd-resolved DBus。

## 选项

| 方案 | 平台 | 优点 | 缺点 |
|---|---|---|---|
| A. Windows ETW `Microsoft-Windows-DNS-Client/Operational` | Win | kernel 级精度、低开销 | ~500 行 unsafe FFI、独立消费者线程、`ProcessTrace` 阻塞、ROI 不匹配 |
| B. **Windows PowerShell `Get-WinEvent -FilterHashtable`** | Win | 150 行子进程脚本、复用 Windows 内置工具、PID 名 lookup 走 sysinfo | spawn 延迟 ~300ms；事件 3010 高频时 CPU 不可忽略 |
| C. Linux systemd-resolved DBus | Linux | 原生 | **DBus 接口不暴露 per-query 信号**（stage-8.md 原计划有误） |
| D. Linux pcap / eBPF | Linux | 全协议、低层 | 工程量大（pcap 解析 / eBPF 程序加载）超出阶段范围 |

## 决策

Windows 采用方案 B（PowerShell 子进程）；Linux 暂不支持（pcap/eBPF 留 0.6.0+）。理由：

1. **Windows ROI**：与 [ADR-0005](./0005-netflow-windows-iphelper-not-etw.md) 同样放弃 ETW
2. **复用 Windows 内置工具**：无需额外装机
3. **隐私设计**：DNS 查询含敏感信息（用户访问的域名），**永不持久化到磁盘**；仅在内存中保留最近 1000 条，状态栏 `📡DNS(仅内存)` 指示
4. **Linux 放弃 DBus**：原 stage-8.md 计划有误，DBus 接口确实不暴露 per-query 信号
5. **Linux pcap/eBPF 工程量**超出 v0.5.0 阶段范围，列入 0.6.0+ 路线图

## 后果

- 正面：Windows 路径 150 行落地（vs ETW ~500 行）
- 正面：trait 抽象（`DnsLogCollector`）让未来 Linux pcap/eBPF 是 additive
- 正面：隐私承诺明确（永不持久化），录屏路径 `record/frame.rs` 不序列化 `DnsQuery`
- 已知限制：仅覆盖 event 3010（QueryResultsEx）；PowerShell 启动延迟 ~300ms；PID 名 lookup 10s 刷新一次新进程可能显示 `?`
- 已知限制：v0.6.0 阶段 2 发现录屏会捕获**屏幕上显示的** DNS 字符（虽然 record/frame.rs 不序列化 DnsQuery 数据结构，但 VT100 字节录制照常）→ 录屏前需确认（见 [ADR-0008](./0008-self-mitigation-policy.md) 后续 / 阶段 2 录屏防护任务）

## 参考

- v0.5.0 阶段 8 落地：CHANGELOG.md
- 相关代码：`src/dns_log/{mod.rs,windows_dns.rs,worker.rs}`
- 同类取舍：[ADR-0005](./0005-netflow-windows-iphelper-not-etw.md)（同样放弃 ETW）
- v0.6.0 阶段 2 录屏 secret 防护：plan.md
