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
        CreateDXGIFactory1, IDXGIAdapter1, IDXGIAdapter3, IDXGIFactory6,
        DXGI_ADAPTER_DESC1, DXGI_ADAPTER_FLAG_SOFTWARE,
        DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO,
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
            &desc.Description.iter().take_while(|&&c| c != 0).copied().collect::<Vec<u16>>()
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

#[cfg(not(target_os = "windows"))]
fn collect_dxgi_adapters() -> Vec<DxgiAdapter> {
    Vec::new()
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
                    utilization_pct: util as u32,
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
            let path: Vec<u16> = "\\GPU Engine(*)\\Utilization Percentage\0".encode_utf16().collect();
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

            Some(max_util.min(100.0).max(0.0) as u32)
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

#[cfg(not(target_os = "windows"))]
struct PdhState;

#[cfg(not(target_os = "windows"))]
impl PdhState {
    fn new() -> Option<Self> {
        None
    }
}

// ---------- GpuCollector ----------

pub struct GpuCollector {
    #[allow(dead_code)]
    nvml: Option<NvmlState>,
    pdh: Option<PdhState>,
}

impl GpuCollector {
    pub fn new() -> Self {
        let nvml = NvmlState::new();
        let pdh = PdhState::new();
        Self { nvml, pdh }
    }

    pub fn refresh(&mut self) -> Vec<GpuInfo> {
        let mut dxgi_adapters = collect_dxgi_adapters();
        if dxgi_adapters.is_empty() {
            return Vec::new();
        }

        // 按专用显存降序排列，独显（大VRAM）排前面
        dxgi_adapters.sort_by(|a, b| b.vram_total.cmp(&a.vram_total));

        let pdh_util = self.pdh.as_mut().and_then(|p| p.collect_utilization());

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
                let (nvml_util, nvml_temp, nvml_power, nvml_vram_used, nvml_vram_total) =
                    (None::<u32>, None::<f32>, None::<f64>, None::<u64>, None::<u64>);

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
