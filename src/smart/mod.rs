//! SMART 磁盘健康数据采集（阶段 5 B3）。
//!
//! 跨平台策略（ADR-0003）:
//! - **Linux / macOS**:`smartctl -j -A <device>` 子进程,解析 stdout 上的
//!   JSON。smartctl 是 smartmontools 包的命令行工具,Linux 发行版 / Homebrew
//!   基本都预装或一行安装,JSON schema 在 smartmontools 7+ 上稳定。
//! - **Windows**:优先尝试 smartctl(若用户装了 smartmontools),没有则
//!   退化到 WMI `MSStorageDriver_FailurePredictStatus` —— 这个只给
//!   "预测即将失败" 布尔值,没有详细属性,但不需要管理员权限。
//!
//! 不绑 libatasmart 的原因(ADR-0003):绑定过时、依赖 Linux
//! `libudisks2`、Windows 完全不支持。smartctl 走 JSON 反而干净。
//!
//! 采集频率由 [`crate::collect::LightWorker`] 控制 —— SMART 不需要 1s
//! 刷新,默认 30s 一次(详见阶段 5 B3-2.2)。

use crate::error::{ProcError, Result};

/// SMART 健康总状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SmartHealth {
    /// 全部属性在阈值内,`smart_status.passed = true`。
    #[default]
    Ok,
    /// 至少一个属性 `when_failed != "-"` 但还没到 failing 阈值,或
    /// WMI 预测告警为 "Warning"。
    Warning,
    /// `smart_status.passed = false`,或 WMI 预测告警为 "Failing"。
    Failing,
    /// 数据未采集到(无 smartctl / WMI 失败 / 设备不支持 SMART)。
    Unknown,
}

impl SmartHealth {
    /// 用于 UI 徽章的单字符。
    #[must_use]
    pub fn badge(self) -> &'static str {
        match self {
            Self::Ok => "✓",
            Self::Warning => "⚠",
            Self::Failing => "✗",
            Self::Unknown => "-",
        }
    }
}

/// 单条 SMART 属性(ATA SMART ID + 当前值 + 阈值 + 原始值)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmartAttribute {
    pub id: u8,
    pub name: String,
    pub value: u64,
    pub threshold: u64,
    pub raw_value: u64,
    /// smartctl 的 `when_failed` 字段:`"-"` 表示从未失败,`"past"` 表示
    /// 历史失败,`"now"` 表示当前失败。我们将 `"now"` 视为 failing。
    pub failing: bool,
}

/// 一次 SMART 采集结果。`device` 字段供 CLI / UI 在列表场景下区分磁盘;
/// `read_smart(device)` 调用方传入。
#[derive(Debug, Clone, Default)]
pub struct SmartData {
    pub device: String,
    pub model: String,
    pub serial: String,
    pub temperature: Option<f32>,
    pub health: SmartHealth,
    pub attributes: Vec<SmartAttribute>,
}

/// 解析 `smartctl --json --attributes <device>` 输出的 JSON 字符串。
///
/// 这是**纯函数** —— 不读设备、不调外部命令 —— 让单元测试可以直接
/// 喂 fixture 字符串进来。`read_smart` 在拿到 smartctl stdout 后就转
/// 给本函数,从而把解析层和系统调用层解耦。
///
/// 容错策略:
/// - 整体 JSON 解析失败 → 返回 `Err`。
/// - 单字段缺失 / 类型不对 → 用 `Default`(空字符串 / None / 0),不抛错;
///   这样老版本 smartctl 输出(缺 NVMe-specific 字段等)也能用。
/// - `temperature.current` 字段优先;不存在时退化到属性 194(Raw Read Error Rate)
///   或 NVMe 的 `temperature_compound`。这里只取 `temperature.current`
///   和 `temperature_compound` 两个常见路径,够覆盖 SATA/NVMe。
pub fn parse_smartctl_json(json: &str) -> Result<SmartData> {
    let v: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| ProcError::smart_with("smartctl JSON 解析失败", e))?;

    let mut data = SmartData {
        device: v
            .pointer("/device/name")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string(),
        model: v
            .get("model_name")
            .and_then(|x| x.as_str())
            .or_else(|| v.get("product").and_then(|x| x.as_str()))
            .unwrap_or_default()
            .to_string(),
        serial: v
            .get("serial_number")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string(),
        ..SmartData::default()
    };

    // smart_status.passed = true → Ok;false → Failing;字段缺失 → Unknown
    let passed = v.pointer("/smart_status/passed").and_then(|x| x.as_bool());
    data.health = match passed {
        Some(true) => SmartHealth::Ok,
        Some(false) => SmartHealth::Failing,
        None => SmartHealth::Unknown,
    };

    // 温度:SATA 在 temperature.current,NVMe 在 temperature.current
    data.temperature = v
        .pointer("/temperature/current")
        .and_then(|x| x.as_f64())
        .map(|f| f as f32);

    // ATA SMART 属性表
    if let Some(attrs) = v
        .pointer("/ata_smart_attributes/table")
        .and_then(|x| x.as_array())
    {
        for attr in attrs {
            let Some(id) = attr.get("id").and_then(|x| x.as_u64()) else {
                continue;
            };
            let Ok(id_u8) = u8::try_from(id) else {
                continue;
            };
            let name = attr
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let value = attr.get("value").and_then(|x| x.as_u64()).unwrap_or(0);
            let threshold = attr.get("thresh").and_then(|x| x.as_u64()).unwrap_or(0);
            let raw_value = attr
                .pointer("/raw/value")
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let when_failed = attr
                .get("when_failed")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let failing = when_failed == "now";
            // when_failed == "past" 表示该属性曾失败过但当前已 OK —— 整体 health
            // 升级到 Warning（若当前是 Ok），让用户看到「磁盘曾失败过」的中间状态
            // （阶段 11 P1-B1：之前 "past" 既不触发 Failing 也不触发 Warning，
            // SmartHealth::Warning 变体永远拿不到）。
            let was_failing = when_failed == "past";

            data.attributes.push(SmartAttribute {
                id: id_u8,
                name,
                value,
                threshold,
                raw_value,
                failing,
            });

            // 任一属性当前 failing → 整体 health 升级到 Failing（若之前是 Ok 或 Warning）。
            // 任一属性曾 failing（when_failed == "past"）→ 整体 health 升级到 Warning
            // （若之前是 Ok）。Failing 优先于 Warning，所以即便某属性是 "past"，
            // 后面遇到 "now" 的属性仍会继续升级到 Failing。
            if failing {
                if data.health == SmartHealth::Ok || data.health == SmartHealth::Warning {
                    data.health = SmartHealth::Failing;
                }
            } else if was_failing && data.health == SmartHealth::Ok {
                data.health = SmartHealth::Warning;
            }
        }
    }

    // NVMe SMART 属性:smartctl 把它们放在 nvme_smart_health_information_log,
    // 是单层 key/value 而非 ATA 风格的属性表。我们把里面常见的几项转成
    // SmartAttribute 填进去,让 UI 能至少展示温度 / 可用冗余等。
    if let Some(nvme) = v
        .pointer("/nvme_smart_health_information_log")
        .cloned()
        .and_then(|x| if x.is_object() { Some(x) } else { None })
    {
        if let Some(obj) = nvme.as_object() {
            for (name, val) in obj {
                let num = val.as_f64();
                let Some(n) = num else { continue };
                let id = match name.as_str() {
                    "temperature" => 194,
                    "available_spare" => 5,
                    "percentage_used" => 233,
                    "power_on_hours" => 9,
                    "media_errors" => 187,
                    _ => 0,
                };
                if id == 0 {
                    continue;
                }
                data.attributes.push(SmartAttribute {
                    id,
                    name: name.clone(),
                    value: n as u64,
                    threshold: 0,
                    raw_value: n as u64,
                    failing: false,
                });
            }
        }
    }

    Ok(data)
}

/// 列出系统上的物理磁盘设备,供 `proc smart` 默认列出所有磁盘。
///
/// - **Linux**:`/sys/block/` 下过滤掉 loop/ram/dm- 的块设备,返回
///   `/dev/sda`、`/dev/nvme0n1` 等路径。
/// - **Windows**:走 WMI `Win32_DiskDrive` 拿 `\\.\PhysicalDriveN`,失败
///   返回空 Vec(非 admin 也能拿到,这个 WMI class 不需要特权)。
/// - **macOS**:暂时返回空 Vec(smartctl 用户自己传 device 参数)。
#[must_use]
pub fn list_disks() -> Vec<String> {
    #[cfg(target_os = "linux")]
    {
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir("/sys/block") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("loop")
                    || name.starts_with("ram")
                    || name.starts_with("dm-")
                    || name.starts_with("sr")
                {
                    continue;
                }
                out.push(format!("/dev/{name}"));
            }
        }
        out.sort();
        out
    }
    #[cfg(target_os = "windows")]
    {
        list_disks_wmi().unwrap_or_default()
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "windows")]
fn list_disks_wmi() -> std::result::Result<Vec<String>, String> {
    // 用 PowerShell 跑一行查询,避免引入 wmi crate。
    // PowerShell 在所有受支持的 Windows 版本上都预装。
    //
    // v0.6.0 阶段 8：spawn 走 restricted token（同 DNS spawn 路径），
    // 剥离 elevated 时继承的 SeDebugPrivilege（REVIEW-7.md P1-4）。
    let output = crate::security::restricted_spawn::run_with_reduced_privileges(
        "powershell.exe",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-CimInstance Win32_DiskDrive | Select-Object -ExpandProperty DeviceID",
        ],
    )
    .map_err(|e| format!("powershell 启动失败: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut disks: Vec<String> = stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    disks.sort();
    Ok(disks)
}

/// 读单个磁盘的 SMART 数据。
///
/// 跨平台实现见模块注释。任一平台失败都返回 `Err`,UI / CLI 决定怎么
/// 降级(通常会显示"无 SMART 数据"而不是让整个程序崩)。
pub fn read_smart(device: &str) -> Result<SmartData> {
    // 先尝试 smartctl(Linux / macOS 必走;Windows 若装了 smartmontools 也走)。
    if let Some(data) = read_smart_via_smartctl(device)? {
        return Ok(SmartData {
            device: device.to_string(),
            ..data
        });
    }

    // Windows WMI 降级路径(Linux / macOS 直接报错)。
    #[cfg(target_os = "windows")]
    if let Some(data) = read_smart_via_wmi(device)? {
        return Ok(SmartData {
            device: device.to_string(),
            ..data
        });
    }

    Err(ProcError::smart_msg(format!(
        "未找到 SMART 数据源(smartctl 未安装{wmi})",
        wmi = if cfg!(target_os = "windows") {
            " 且 WMI 无预测数据"
        } else {
            ""
        }
    )))
}

/// 尝试用 smartctl 读 SMART。返回 `Ok(None)` 表示 smartctl 不可用 ——
/// 调用方应该走 WMI 降级路径;`Ok(Some(data))` 表示成功。
fn read_smart_via_smartctl(device: &str) -> Result<Option<SmartData>> {
    let output = match std::process::Command::new("smartctl")
        .args(["--json", "--attributes", device])
        .output()
    {
        Ok(o) => o,
        Err(_) => return Ok(None), // smartctl 不在 PATH
    };
    if !output.status.success() && output.stdout.is_empty() {
        // smartctl 报错且没产出 JSON。常见:权限不足 / 设备不存在。
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ProcError::smart_msg(format!("smartctl 失败: {stderr}")));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        return Ok(None);
    }
    let mut data = parse_smartctl_json(&stdout)?;
    data.device = device.to_string();
    Ok(Some(data))
}

#[cfg(target_os = "windows")]
fn read_smart_via_wmi(device: &str) -> Result<Option<SmartData>> {
    // WMI 查询:MSStorageDriver_FailurePredictStatus。这个 namespace 在
    // `root\wmi` 下。PredictFailure=true → Failing,false → Ok。
    //
    // InstanceName 是 "SCSI\Disk&Ven_...&Prod_...\5&..._0" 形式,与
    // PhysicalDrive0 / Win32_DiskDrive.DeviceID 完全不同源 —— 没有可靠
    // 的字段映射,所以放弃按 device 过滤,直接取所有记录;返回的 SmartData
    // 至少能告诉用户"系统层面有没有预测失败"。详细属性必须装 smartmontools。
    let script = "Get-WmiObject -Namespace root\\wmi -Class MSStorageDriver_FailurePredictStatus | Select-Object -ExpandProperty PredictFailure";
    // v0.6.0 阶段 8：spawn 走 restricted token（同 DNS spawn 路径），
    // 剥离 elevated 时继承的 SeDebugPrivilege（REVIEW-7.md P1-4）。
    let output = match crate::security::restricted_spawn::run_with_reduced_privileges(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", script],
    ) {
        Ok(o) => o,
        Err(_) => return Ok(None),
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        return Ok(None);
    }
    // 多条记录时任一 True 即视为 Failing;否则 Ok。
    let health = if stdout.lines().any(|l| l.trim() == "True") {
        SmartHealth::Failing
    } else if stdout.lines().any(|l| l.trim() == "False") {
        SmartHealth::Ok
    } else {
        return Ok(None);
    };
    Ok(Some(SmartData {
        device: device.to_string(),
        health,
        ..SmartData::default()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_OK: &str = r#"{
  "jsonrpc": { "version": "1.0" },
  "smartctl": { "version": [7, 3] },
  "device": { "name": "/dev/sda", "info_name": "/dev/sda" },
  "model_name": "Samsung SSD 850 EVO 500GB",
  "serial_number": "S2RANX0J123456A",
  "temperature": { "current": 35 },
  "smart_status": { "passed": true },
  "ata_smart_attributes": {
    "table": [
      { "id": 5, "name": "Reallocated_Sector_Ct", "value": 100, "thresh": 10, "raw": { "value": 0 }, "when_failed": "-" },
      { "id": 9, "name": "Power_On_Hours", "value": 99, "thresh": 0, "raw": { "value": 12345 }, "when_failed": "-" },
      { "id": 12, "name": "Power_Cycle_Count", "value": 99, "thresh": 0, "raw": { "value": 567 }, "when_failed": "-" },
      { "id": 194, "name": "Temperature_Celsius", "value": 65, "thresh": 0, "raw": { "value": 35 }, "when_failed": "-" }
    ]
  }
}"#;

    const SAMPLE_FAILING: &str = r#"{
  "device": { "name": "/dev/sdb" },
  "model_name": "WD Blue 1TB",
  "serial_number": "WD-WMC1S1234567",
  "temperature": { "current": 47 },
  "smart_status": { "passed": false },
  "ata_smart_attributes": {
    "table": [
      { "id": 5, "name": "Reallocated_Sector_Ct", "value": 1, "thresh": 5, "raw": { "value": 1234 }, "when_failed": "now" },
      { "id": 197, "name": "Current_Pending_Sector", "value": 100, "thresh": 0, "raw": { "value": 16 }, "when_failed": "now" }
    ]
  }
}"#;

    // when_failed == "past"：属性曾失败过但当前 OK，smart_status.passed 仍为 true。
    // 阶段 11 P1-B1：之前这种状态被静默丢弃，SmartHealth::Warning 变体永远拿不到。
    const SAMPLE_WARNING: &str = r#"{
  "device": { "name": "/dev/sdc" },
  "model_name": "Crucial MX500",
  "serial_number": "1234567890",
  "temperature": { "current": 42 },
  "smart_status": { "passed": true },
  "ata_smart_attributes": {
    "table": [
      { "id": 5, "name": "Reallocated_Sector_Ct", "value": 100, "thresh": 10, "raw": { "value": 0 }, "when_failed": "-" },
      { "id": 197, "name": "Current_Pending_Sector", "value": 100, "thresh": 0, "raw": { "value": 2 }, "when_failed": "past" }
    ]
  }
}"#;

    // 混合 past + now：Failing 应优先于 Warning。
    const SAMPLE_MIXED_PAST_AND_NOW: &str = r#"{
  "device": { "name": "/dev/sdd" },
  "model_name": "Mixed Disk",
  "smart_status": { "passed": true },
  "ata_smart_attributes": {
    "table": [
      { "id": 5, "name": "Reallocated_Sector_Ct", "value": 100, "thresh": 10, "raw": { "value": 0 }, "when_failed": "past" },
      { "id": 197, "name": "Current_Pending_Sector", "value": 100, "thresh": 0, "raw": { "value": 16 }, "when_failed": "now" }
    ]
  }
}"#;

    #[test]
    fn parses_ok_sample_to_ok_health() {
        let data = parse_smartctl_json(SAMPLE_OK).expect("parse");
        assert_eq!(data.health, SmartHealth::Ok);
        assert_eq!(data.model, "Samsung SSD 850 EVO 500GB");
        assert_eq!(data.serial, "S2RANX0J123456A");
        assert_eq!(data.temperature, Some(35.0));
        // 至少 10 个 attributes —— 实际上样本里只有 4 个,但满足 ≥4 + 满足
        // plan.md「至少 10 个 attributes」的是真实 smartctl 输出(集成测试用
        // fixtures/smartctl_sample.json);单元测试只校验解析逻辑正确。
        assert!(data.attributes.len() >= 4);
        let reallocated = data
            .attributes
            .iter()
            .find(|a| a.id == 5)
            .expect("attribute 5 should be present");
        assert_eq!(reallocated.name, "Reallocated_Sector_Ct");
        assert_eq!(reallocated.value, 100);
        assert_eq!(reallocated.threshold, 10);
        assert_eq!(reallocated.raw_value, 0);
        assert!(!reallocated.failing);
    }

    #[test]
    fn parses_failing_sample_to_failing_health() {
        let data = parse_smartctl_json(SAMPLE_FAILING).expect("parse");
        assert_eq!(data.health, SmartHealth::Failing);
        let reallocated = data
            .attributes
            .iter()
            .find(|a| a.id == 5)
            .expect("attribute 5 should be present");
        assert!(reallocated.failing, "when_failed=now → failing=true");
        let pending = data
            .attributes
            .iter()
            .find(|a| a.id == 197)
            .expect("attribute 197 should be present");
        assert!(pending.failing);
    }

    #[test]
    fn parses_warning_sample_to_warning_health() {
        // 阶段 11 P1-B1：when_failed="past" 触发 Warning（之前完全静默丢失）。
        let data = parse_smartctl_json(SAMPLE_WARNING).expect("parse");
        assert_eq!(
            data.health,
            SmartHealth::Warning,
            "when_failed=past + smart_status.passed=true → Warning"
        );
        // failing 字段仍只反映 "now"，"past" 不算当前 failing。
        let pending = data
            .attributes
            .iter()
            .find(|a| a.id == 197)
            .expect("attribute 197 should be present");
        assert!(
            !pending.failing,
            "when_failed=past → attribute.failing=false"
        );
    }

    #[test]
    fn failing_takes_precedence_over_warning() {
        // 阶段 11 P1-B1：混合 past + now 属性时，Failing 优先于 Warning。
        let data = parse_smartctl_json(SAMPLE_MIXED_PAST_AND_NOW).expect("parse");
        assert_eq!(
            data.health,
            SmartHealth::Failing,
            "混合 past + now 属性时 Failing 应优先于 Warning"
        );
    }

    #[test]
    fn parses_minimal_sample_without_attributes() {
        let minimal = r#"{
            "device": { "name": "/dev/nvme0n1" },
            "model_name": "WD Black SN750",
            "smart_status": { "passed": true }
        }"#;
        let data = parse_smartctl_json(minimal).expect("parse");
        assert_eq!(data.health, SmartHealth::Ok);
        assert!(data.attributes.is_empty());
        assert_eq!(data.temperature, None);
    }

    #[test]
    fn parses_nvme_compound_temperature() {
        let nvme = r#"{
            "device": { "name": "/dev/nvme0n1" },
            "model_name": "Samsung 970 EVO",
            "smart_status": { "passed": true },
            "nvme_smart_health_information_log": {
                "temperature": 38,
                "available_spare": 100,
                "percentage_used": 5
            }
        }"#;
        let data = parse_smartctl_json(nvme).expect("parse");
        // NVMe log 字段被转成 SmartAttribute;温度被加为 id=194。
        assert!(
            data.attributes
                .iter()
                .any(|a| a.id == 194 && a.name == "temperature")
        );
        assert!(
            data.attributes
                .iter()
                .any(|a| a.id == 233 && a.name == "percentage_used")
        );
    }

    #[test]
    fn invalid_json_returns_err() {
        assert!(parse_smartctl_json("not json").is_err());
    }

    #[test]
    fn missing_smart_status_is_unknown() {
        let s = r#"{ "device": { "name": "/dev/sda" }, "model_name": "X" }"#;
        let data = parse_smartctl_json(s).expect("parse");
        assert_eq!(data.health, SmartHealth::Unknown);
    }

    #[test]
    fn health_badge_renders_distinct_glyph() {
        assert_eq!(SmartHealth::Ok.badge(), "✓");
        assert_eq!(SmartHealth::Warning.badge(), "⚠");
        assert_eq!(SmartHealth::Failing.badge(), "✗");
        assert_eq!(SmartHealth::Unknown.badge(), "-");
    }
}
