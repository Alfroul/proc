//! Stage 6（B1）：GpuProvider trait + parse_nvtop_json 集成测试。
//!
//! 单元层覆盖在 `src/gpu.rs` 内嵌 tests（trait 行为 + 多 vendor 解析 + 缺字段
//! 退化）。这里加一层集成测试，跑真实 fixture 文件 + 暴露在 lib crate 公开
//! API 上的入口（`detect_providers` / `GpuCollector`），确保 sidebar / worker
//! 看到的契约稳定。

#![cfg(feature = "nvtop")]

use proc::gpu::{GpuCollector, GpuVendor, detect_providers, parse_nvtop_json};
use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push(name);
    p
}

#[test]
fn parses_real_nvtop_fixture_returns_three_vendors() {
    let path = fixture_path("nvtop_sample.json");
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read fixture {:?}: {:?}", path, e));
    let out = parse_nvtop_json(&content);
    assert_eq!(out.len(), 3, "fixture 应有 NVIDIA + AMD + Intel 各 1 条");
    let vendors: Vec<_> = out.iter().map(|g| g.vendor).collect();
    assert!(vendors.contains(&GpuVendor::Nvidia));
    assert!(vendors.contains(&GpuVendor::Amd));
    assert!(vendors.contains(&GpuVendor::Intel));
}

#[test]
fn parses_real_nvtop_fixture_populates_all_fields() {
    let content = std::fs::read_to_string(fixture_path("nvtop_sample.json")).unwrap();
    let nvidia = parse_nvtop_json(&content)
        .into_iter()
        .find(|g| g.vendor == GpuVendor::Nvidia)
        .expect("fixture 必须含 NVIDIA 样本");
    assert_eq!(nvidia.name, "NVIDIA GeForce RTX 4070");
    assert_eq!(nvidia.utilization_pct, 78);
    assert_eq!(nvidia.vram_used, 2048);
    assert_eq!(nvidia.vram_total, 12288);
    assert_eq!(nvidia.vram_budget, 12288, "vram_budget 缺省回退到 total");
    assert_eq!(nvidia.temperature, Some(62.5));
    assert_eq!(nvidia.power_watts, Some(175.0));
}

#[test]
fn parse_nvtop_json_returns_empty_for_malformed_file() {
    // 故意构造一个非法 JSON，确保解析器返回空 Vec 而不 panic。
    let out = parse_nvtop_json("<<<not json>>>");
    assert!(out.is_empty(), "garbage 输入必须返回空 Vec");
}

#[test]
fn parse_nvtop_json_handles_empty_array() {
    assert!(parse_nvtop_json("[]").is_empty());
}

#[test]
fn detect_providers_does_not_panic_and_returns_consistent_names() {
    // 不论平台（CI Linux / 开发机 Windows）和 nvtop 是否在 PATH，detect_providers
    // 都不应 panic。返回的 provider_name 必须是非空 &str。
    let providers = detect_providers();
    for p in &providers {
        let name = p.provider_name();
        assert!(!name.is_empty(), "provider_name 不能为空字符串");
        // trait 约束：name 是 'static —— 校验常见已知来源。
        assert!(
            matches!(name, "nvml+dxgi" | "nvtop" | "nvml-unavailable"),
            "unexpected provider name: {name}"
        );
    }
}

#[test]
fn gpu_collector_refresh_returns_vec_without_panicking() {
    // LightWorker 的调用契约：new() + refresh() 返回 Vec<GpuInfo>。
    let mut collector = GpuCollector::new();
    let info = collector.refresh();
    // CI 上可能没有 GPU，info 为空也接受；重点是不 panic。
    let _ = info.len();
    let names = collector.provider_names();
    for name in &names {
        assert!(!name.is_empty());
    }
}
