# ADR-0004: Linux GPU 采集走 nvtop 子进程

- **Status**: Accepted
- **Date**: 2026-06-16
- **Phase**: v0.5.0 阶段 6

## 背景

v0.5.0 之前 Windows 上 GPU 监控仅 NVIDIA（NVML），Linux/macOS 完全无 GPU 监控。需要覆盖 Linux 上 AMD / Intel / NVIDIA 全厂商。

## 选项

| 方案 | 优点 | 缺点 |
|---|---|---|
| A. 直接绑定 libdrm / libpci | 性能最优 | 每个厂商独立 API（NVIDIA NVML / AMD ADL / Intel Intel-GPU-Tools），3 套 bindgen 配置 |
| B. **nvtop 子进程 + JSON 解析** | 一套解析器覆盖三厂商、依赖管理干净 | 需要 PATH 有 nvtop、子进程开销 |
| C. Linux sysfs `/sys/class/drm` | 无外部依赖 | 只能给基本 utilization，温度 / 功率 / 显存多数 GPU 不暴露 |
| D. Windows AMD/Intel 走 WMI | 原生 | 仅 VRAM（DXGI 已有），无 utilization / temp / power；留 0.6.0+ |

## 决策

采用方案 B（Linux nvtop 子进程）+ 方案 A 保留（Windows NvmlProvider 继续走 NVML）。理由：

1. **跨厂商一次覆盖**：AMD+Intel+NVIDIA 一套解析器，开发成本 1/3
2. **依赖管理干净**：与 [ADR-0003](./0003-smart-subprocess-vs-library.md) smartctl 同类取舍
3. **失败优雅降级**：`is_available()` 通过 `nvtop --version` 探测，未装时 detect_providers 跳过
4. **多 provider 并存**：`detect_providers() -> Vec<Box<dyn GpuProvider>>`，支持 Intel iGPU + NVIDIA dGPU 混合笔记本
5. Windows AMD/Intel 列入 0.6.0+ 路线图（方案 D WMI 后续）

## 后果

- 正面：Linux 三厂商 GPU 监控一次落地，sidebar 多 GPU 循环已有支持（surgical 原则）
- 正面：`GpuProvider` trait 抽象让未来 Windows AMD/Intel 是 additive 新 impl
- 负面：Linux 用户必须装 nvtop（已在 FAQ 说明）
- 已知限制：Windows AMD/Intel 仍无 utilization / temp / power（DXGI 仅 VRAM）

## 参考

- v0.5.0 阶段 6 落地：CHANGELOG.md
- 相关代码：`src/gpu.rs::GpuProvider / NvtopProvider / detect_providers`
- 同类取舍：[ADR-0003](./0003-smart-subprocess-vs-library.md)
