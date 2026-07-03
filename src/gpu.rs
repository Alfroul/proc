//! GPU monitoring via 3-layer data collection: DXGI + NVML + PDH.

#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub name: String,
    pub vendor: GpuVendor,
    pub utilization_pct: u32,
    pub vram_used: u64,
    pub vram_total: u64,
    pub vram_budget: u64,
    pub temperature: Option<f32>,
    pub power_watts: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
    Unknown,
}

impl GpuVendor {
    fn from_vendor_id(id: u32) -> Self {
        match id {
            0x10DE => Self::Nvidia,
            0x1002 => Self::Amd,
            0x8086 => Self::Intel,
            _ => Self::Unknown,
        }
    }
}

struct DxgiAdapter {
    #[allow(dead_code)]
    index: u32,
    name: String,
    vendor: GpuVendor,
    vram_total: u64,
    vram_used: u64,
    vram_budget: u64,
}

#[cfg(target_os = "windows")]
fn collect_dxgi_adapters() -> Vec<DxgiAdapter> {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, DXGI_ADAPTER_DESC1, DXGI_ADAPTER_FLAG_SOFTWARE,
        DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO, IDXGIAdapter1,
        IDXGIAdapter3, IDXGIFactory6,
    };
    use windows::core::Interface;

    let mut adapters = Vec::new();

    let factory: IDXGIFactory6 = match unsafe { CreateDXGIFactory1() } {
        Ok(f) => f,
        Err(e) => {
            tracing::debug!("CreateDXGIFactory1 failed: {:?}", e);
            return adapters;
        }
    };

    for i in 0u32.. {
        let adapter: IDXGIAdapter1 = match unsafe { factory.EnumAdapters1(i) } {
            Ok(a) => a,
            Err(_) => break,
        };

        let mut desc: DXGI_ADAPTER_DESC1 = unsafe { std::mem::zeroed() };
        if unsafe { adapter.GetDesc1(&mut desc) }.is_err() {
            continue;
        }

        if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }

        let name = String::from_utf16_lossy(
            &desc
                .Description
                .iter()
                .take_while(|&&c| c != 0)
                .copied()
                .collect::<Vec<u16>>(),
        );
        let vendor = GpuVendor::from_vendor_id(desc.VendorId);
        let vram_total = desc.DedicatedVideoMemory as u64;

        let (vram_used, vram_budget) = match adapter.cast::<IDXGIAdapter3>() {
            Ok(a3) => {
                let mut mem_info: DXGI_QUERY_VIDEO_MEMORY_INFO = unsafe { std::mem::zeroed() };
                match unsafe {
                    a3.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut mem_info)
                } {
                    Ok(()) => (mem_info.CurrentUsage, mem_info.Budget),
                    Err(_) => (0, vram_total),
                }
            }
            Err(_) => (0, vram_total),
        };

        adapters.push(DxgiAdapter {
            index: i,
            name,
            vendor,
            vram_total,
            vram_used,
            vram_budget,
        });
    }

    adapters
}

// ---------- NVML layer (optional) ----------

#[cfg(feature = "nvidia")]
struct NvmlState {
    nvml: nvml_wrapper::Nvml,
}

#[cfg(feature = "nvidia")]
struct NvmlGpuInfo {
    utilization_pct: u32,
    temperature: Option<f32>,
    power_watts: Option<f64>,
    vram_used: Option<u64>,
    vram_total: Option<u64>,
}

#[cfg(feature = "nvidia")]
impl NvmlState {
    fn new() -> Option<Self> {
        match nvml_wrapper::Nvml::init() {
            Ok(nvml) => Some(Self { nvml }),
            Err(_) => None,
        }
    }

    fn get_info(&self, adapter_name: &str) -> Option<NvmlGpuInfo> {
        let count = self.nvml.device_count().ok()?;
        for i in 0..count {
            let device = match self.nvml.device_by_index(i).ok() {
                Some(d) => d,
                None => continue,
            };
            let name = device.name().unwrap_or_default();
            if adapter_name.contains(&name) || name.contains(adapter_name) {
                let util = device.utilization_rates().map(|u| u.gpu).unwrap_or(0);
                let temp = device
                    .temperature(nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu)
                    .ok();
                let power = device.power_usage().ok().map(|p| p as f64 / 1000.0);
                let mem = device.memory_info().ok();
                let vram_used = mem.as_ref().map(|m| m.used);
                let vram_total = mem.as_ref().map(|m| m.total);

                return Some(NvmlGpuInfo {
                    utilization_pct: util,
                    temperature: temp.map(|t| t as f32),
                    power_watts: power,
                    vram_used,
                    vram_total,
                });
            }
        }
        None
    }
}

#[cfg(not(feature = "nvidia"))]
struct NvmlState;

#[cfg(not(feature = "nvidia"))]
impl NvmlState {
    fn new() -> Option<Self> {
        None
    }
}

// ---------- PDH layer ----------

#[cfg(target_os = "windows")]
struct PdhState {
    query: isize,
    counter: isize,
    first_sample: bool,
}

#[cfg(target_os = "windows")]
impl PdhState {
    fn new() -> Option<Self> {
        use windows::Win32::System::Performance::*;
        use windows::core::PCWSTR;

        unsafe {
            let mut query: isize = 0;
            if PdhOpenQueryW(PCWSTR::null(), 0, &mut query) != 0 {
                return None;
            }

            let mut counter: isize = 0;
            let path: Vec<u16> = "\\GPU Engine(*)\\Utilization Percentage\0"
                .encode_utf16()
                .collect();
            if PdhAddCounterW(query, PCWSTR(path.as_ptr()), 0, &mut counter) != 0 {
                let _ = PdhCloseQuery(query);
                return None;
            }

            let _ = PdhCollectQueryData(query);

            Some(Self {
                query,
                counter,
                first_sample: true,
            })
        }
    }

    fn collect_utilization(&mut self) -> Option<u32> {
        use windows::Win32::System::Performance::*;

        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return None;
            }

            if self.first_sample {
                self.first_sample = false;
                return None;
            }

            let mut buf_size: u32 = 0;
            let mut item_count: u32 = 0;

            let status = PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_DOUBLE,
                &mut buf_size,
                &mut item_count,
                None,
            );

            if status != PDH_MORE_DATA || buf_size == 0 {
                return None;
            }

            let mut buffer: Vec<u8> = vec![0u8; buf_size as usize];
            let status = PdhGetFormattedCounterArrayW(
                self.counter,
                PDH_FMT_DOUBLE,
                &mut buf_size,
                &mut item_count,
                Some(buffer.as_mut_ptr() as *mut _),
            );

            if status != 0 {
                return None;
            }

            let items = std::slice::from_raw_parts(
                buffer.as_ptr() as *const PDH_FMT_COUNTERVALUE_ITEM_W,
                item_count as usize,
            );

            let mut max_util = 0.0f64;
            for item in items {
                let value = item.FmtValue.Anonymous.doubleValue;
                if value > max_util {
                    max_util = value;
                }
            }

            Some(max_util.clamp(0.0, 100.0) as u32)
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for PdhState {
    fn drop(&mut self) {
        use windows::Win32::System::Performance::PdhCloseQuery;
        unsafe {
            let _ = PdhCloseQuery(self.query);
        }
    }
}

// ---------- GpuProvider trait（阶段 6 B1：多厂商 GPU 抽象） ----------

/// GPU 数据源抽象。每个 impl 代表一个独立的 GPU 信息来源：
/// `NvmlProvider`（Windows DXGI + NVML + PDH 三层）、`NvtopProvider`
/// （Linux nvtop 子进程）、以及未来的 sysfs / WMI 等扩展。
///
/// `detect_providers()` 根据 feature flag + 平台 + 二进制可用性返回当前
/// 活跃的 provider 列表；`GpuCollector` 聚合所有 provider 的 `list_gpus()`
/// 结果。多 provider 并存支持混合 GPU 笔记本（Intel iGPU + NVIDIA dGPU）。
///
/// 设计约束：
/// - `list_gpus` 取 `&self`（缓存由 `refresh` 维护），让调用方在多 provider
///   场景下能并发查询而无需 `&mut`
/// - `Send + Sync` 让 provider 可跨 worker 线程传递（LightWorker 持有）
pub trait GpuProvider: Send + Sync {
    /// 返回此 provider 当前缓存的所有 GPU。空 Vec 表示无数据，不 panic。
    fn list_gpus(&self) -> Vec<GpuInfo>;
    /// 触发一次底层刷新（spawn 子进程、刷 PDH 计数器等）。耗时调用，由
    /// `GpuCollector::refresh` 统一调度，不应在 `list_gpus` 里重负载数据采集。
    fn refresh(&mut self);
    /// Provider 来源标识（`"nvml+dxgi"` / `"nvtop"`），用于日志和未来 UI 详情。
    fn provider_name(&self) -> &'static str;
}

// ---------- NvmlProvider（封装现有 Windows DXGI + NVML + PDH 路径） ----------

/// 现有 NVIDIA 路径的 provider 封装：DXGI 枚举所有适配器（含 AMD/Intel
/// iGPU），NVML 在 NVIDIA 卡上补 utilization/temp/power，PDH 兜底 utilization。
///
/// 在非 Windows 平台此 provider 不可用（`detect_providers` 不会构造它）。
#[cfg(target_os = "windows")]
pub struct NvmlProvider {
    #[allow(dead_code)]
    nvml: Option<NvmlState>,
    pdh: Option<PdhState>,
    /// refresh 把 DXGI + NVML + PDH 聚合结果缓存到这里，list_gpus 直接 clone
    /// 不再触碰 &mut self（阶段 11 P1-B3：之前每次 list_gpus 都重做完整
    /// DXGI 枚举 + NVML get_info，sidebar 每秒渲染一次就跑一次完整枚举，
    /// 注释声称缓存但实际没缓存）。
    cached: Vec<GpuInfo>,
}

#[cfg(target_os = "windows")]
impl Default for NvmlProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "windows")]
impl NvmlProvider {
    #[must_use]
    pub fn new() -> Self {
        Self {
            nvml: NvmlState::new(),
            pdh: PdhState::new(),
            cached: Vec::new(),
        }
    }

    /// 聚合 DXGI + NVML + PDH 当前快照为 `Vec<GpuInfo>`。
    /// 由 `refresh` 调用，结果存 `self.cached`；`list_gpus` 直接 clone。
    fn build_snapshot(&mut self) -> Vec<GpuInfo> {
        let mut dxgi_adapters = collect_dxgi_adapters();
        if dxgi_adapters.is_empty() {
            return Vec::new();
        }

        // 按专用显存降序排列，独显（大 VRAM）排前面
        dxgi_adapters.sort_by_key(|a| std::cmp::Reverse(a.vram_total));

        // 按名称去重：DXGI 可能枚举到同一 GPU 的多个适配器实例
        let mut seen = std::collections::HashSet::new();
        dxgi_adapters.retain(|a| seen.insert(a.name.clone()));

        // PDH 单步推进（first_sample 状态机要求两次 collect 之间有时间间隔）。
        // 把结果留作本地 utilization fallback。
        let pdh_util = self.pdh.as_mut().and_then(PdhState::collect_utilization);

        dxgi_adapters
            .into_iter()
            .map(|adapter| {
                #[cfg(feature = "nvidia")]
                let (nvml_util, nvml_temp, nvml_power, nvml_vram_used, nvml_vram_total) = self
                    .nvml
                    .as_ref()
                    .and_then(|n| n.get_info(&adapter.name))
                    .map(|info| {
                        (
                            Some(info.utilization_pct),
                            info.temperature,
                            info.power_watts,
                            info.vram_used,
                            info.vram_total,
                        )
                    })
                    .unwrap_or((None, None, None, None, None));

                #[cfg(not(feature = "nvidia"))]
                let (nvml_util, nvml_temp, nvml_power, nvml_vram_used, nvml_vram_total) = (
                    None::<u32>,
                    None::<f32>,
                    None::<f64>,
                    None::<u64>,
                    None::<u64>,
                );

                let utilization_pct = nvml_util.unwrap_or(pdh_util.unwrap_or(0));
                // NVML VRAM 覆盖 DXGI（笔记本 Optimus 空闲时 DXGI 返回 0）
                let vram_used = nvml_vram_used.unwrap_or(adapter.vram_used);
                let vram_total = nvml_vram_total.unwrap_or(adapter.vram_total);

                GpuInfo {
                    name: adapter.name,
                    vendor: adapter.vendor,
                    utilization_pct,
                    vram_used,
                    vram_total,
                    vram_budget: adapter.vram_budget,
                    temperature: nvml_temp,
                    power_watts: nvml_power,
                }
            })
            .collect()
    }
}

#[cfg(target_os = "windows")]
impl GpuProvider for NvmlProvider {
    fn list_gpus(&self) -> Vec<GpuInfo> {
        // 读缓存（refresh 在 LightWorker 1s tick 里被调）。
        self.cached.clone()
    }

    fn refresh(&mut self) {
        // DXGI + NVML + PDH 一次聚合写进 cached，list_gpus 直接读 clone。
        self.cached = self.build_snapshot();
    }

    fn provider_name(&self) -> &'static str {
        "nvml+dxgi"
    }
}

/// 返回当前活跃的 provider 列表。任何初始化失败都跳过对应 provider，绝不 panic。
#[must_use]
pub fn detect_providers() -> Vec<Box<dyn GpuProvider>> {
    let mut providers: Vec<Box<dyn GpuProvider>> = Vec::new();

    #[cfg(target_os = "windows")]
    {
        providers.push(Box::new(NvmlProvider::new()));
    }

    providers
}

/// GPU 采集聚合器：跨多 provider 收集 `GpuInfo`。`LightWorker` 持有一份，
/// 每秒调一次 `refresh()` 拉所有 provider 的最新数据。
pub struct GpuCollector {
    providers: Vec<Box<dyn GpuProvider>>,
}

impl Default for GpuCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuCollector {
    #[must_use]
    pub fn new() -> Self {
        Self {
            providers: detect_providers(),
        }
    }

    /// 触发所有 provider 的刷新 + 聚合 list_gpus。原 `gpu_collector.refresh()`
    /// 调用点（`collect.rs` LightWorker）无需改动。
    pub fn refresh(&mut self) -> Vec<GpuInfo> {
        for provider in &mut self.providers {
            provider.refresh();
        }
        let mut all = Vec::new();
        for provider in &self.providers {
            all.extend(provider.list_gpus());
        }
        all
    }

    /// 调试用：枚举当前活跃的 provider 名字。
    #[must_use]
    pub fn provider_names(&self) -> Vec<&'static str> {
        self.providers.iter().map(|p| p.provider_name()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_providers_does_not_panic() {
        // detect_providers 不应 panic。
        let providers = detect_providers();
        for p in &providers {
            assert!(!p.provider_name().is_empty());
        }
    }

    #[test]
    fn gpu_collector_default_constructs_and_refreshes() {
        // Default::default() / new() / refresh() / provider_names() 全链路不 panic。
        let mut c = GpuCollector::default();
        let _info = c.refresh();
        let _names = c.provider_names();
    }
}
