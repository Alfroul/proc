//! `proc smart [device]` — SMART 磁盘健康（v0.5.0 阶段 5 B3）。

use colored::Colorize;

pub fn run_smart(device: Option<&str>) {
    match device {
        Some(dev) => run_smart_detail(dev),
        None => run_smart_list(),
    }
}

fn run_smart_list() {
    let disks = crate::smart::list_disks();
    if disks.is_empty() {
        println!(
            "{}",
            "未发现可查询的磁盘(Linux 看 /sys/block,Windows 走 WMI Win32_DiskDrive)".yellow()
        );
        return;
    }
    let mut table = comfy_table::Table::new();
    table.set_header(vec!["设备", "型号", "序列号", "健康", "温度", "属性数"]);
    let mut any_data = false;
    for dev in &disks {
        match crate::smart::read_smart(dev) {
            Ok(data) => {
                any_data = true;
                let temp = data
                    .temperature
                    .map(|t| format!("{:.1}\u{00B0}C", t))
                    .unwrap_or_else(|| "-".to_string());
                table.add_row(vec![
                    data.device.clone(),
                    data.model.clone(),
                    data.serial.clone(),
                    format!("{} {:?}", data.health.badge(), data.health),
                    temp,
                    data.attributes.len().to_string(),
                ]);
            }
            Err(e) => {
                table.add_row(vec![
                    dev.clone(),
                    "-".to_string(),
                    "-".to_string(),
                    "无数据".to_string(),
                    "-".to_string(),
                    format!("（{}）", e),
                ]);
            }
        }
    }
    println!("{table}");
    if !any_data {
        println!(
            "{}",
            "提示: 多数 Linux 装包带 smartmontools,Windows 装上 smartmontools 后 JSON 解析更完整"
                .yellow()
        );
    }
}

fn run_smart_detail(device: &str) {
    let data = match crate::smart::read_smart(device) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{} 读取 {} SMART 数据失败: {}", "错误:".red(), device, e);
            std::process::exit(1);
        }
    };
    println!("{}", format!("磁盘: {}", data.device).cyan());
    println!("型号: {}", data.model);
    println!("序列号: {}", data.serial);
    println!(
        "温度: {}",
        data.temperature
            .map(|t| format!("{:.1}\u{00B0}C", t))
            .unwrap_or_else(|| "未知".to_string())
    );
    println!("健康: {} {:?}", data.health.badge(), data.health);
    if data.attributes.is_empty() {
        println!(
            "{}",
            "（无详细 SMART 属性 —— Windows 走 WMI 降级时常见,装 smartmontools 可拿完整表）"
                .yellow()
        );
        return;
    }
    println!();
    let mut table = comfy_table::Table::new();
    table.set_header(vec!["ID", "名称", "当前值", "阈值", "原始值", "失败"]);
    for attr in &data.attributes {
        table.add_row(vec![
            format!("{:3}", attr.id),
            attr.name.clone(),
            attr.value.to_string(),
            attr.threshold.to_string(),
            attr.raw_value.to_string(),
            if attr.failing { "✗" } else { "-" }.to_string(),
        ]);
    }
    println!("{table}");
}
