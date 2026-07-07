//! v0.16.0 阶段 2 集成测试 — MCP record.rs 子 module 业务逻辑（3 tool）：
//! proc_replay_info（双路径 v3 + VT100）+ proc_replay_search（FilterExpr + limit）
//! + proc_eject_status（4 档 suggestion）。
//!
//! 与 `test_mcp_v0_15.rs` 同款策略：直接调 `handler::record::make_*` helper 验证
//! thin wrapper 把业务模块 JSON-ify 没漏字段。stdio 端到端测试留 manual：
//! `npx mcp-inspector proc mcp serve`。
//!
//! 录屏 fixture 走 tempfile + Recorder（v3）/ 手写 VtHeader+VtFrame（VT100），
//! 不依赖外部 .prec 文件。

use proc::mcp::handler::record;
use proc::record::Player;
use proc::record::frame::{
    FrameAnomaly, FrameConnectionDiff, FrameNav, FrameProcess, RecordingHeader, UiFrame,
};
use proc::record::writer::Recorder;

// ===========================================================================
// 测试 fixture helpers
// ===========================================================================

/// 构造一个可定制的 UiFrame（cpu / mem / 进程名 / anomaly severity）。
fn make_frame(timestamp: u64, cpu: f32, mem: u64, names: &[&str], severity: &str) -> UiFrame {
    let processes: Vec<FrameProcess> = names
        .iter()
        .map(|n| FrameProcess {
            pid: 1,
            name: (*n).to_string(),
            cpu: 0.0,
            memory: 0,
            disk_read: 0,
            disk_write: 0,
        })
        .collect();
    let anomalies = if severity.is_empty() {
        Vec::new()
    } else {
        vec![FrameAnomaly {
            rule_id: "test".to_string(),
            severity: severity.to_string(),
            title: "test anomaly".to_string(),
            detail: String::new(),
            affected_pid: None,
            affected_ip: None,
        }]
    };
    UiFrame {
        timestamp,
        mode: "ProcessList".to_string(),
        status_message: None,
        cpu_usage: cpu,
        memory_used: mem,
        memory_total: 16 * 1024 * 1024 * 1024,
        net_down: 0,
        net_up: 0,
        cpu_history: Vec::new(),
        mem_history: Vec::new(),
        processes,
        search_query: String::new(),
        sort_field: "Cpu".to_string(),
        process_view_mode: 0,
        tree_nodes: Vec::new(),
        port_entries: Vec::new(),
        port_view_mode: 0,
        port_process_groups: Vec::new(),
        port_remote_groups: Vec::new(),
        connection_diff: FrameConnectionDiff::default(),
        anomalies,
        usb_devices: Vec::new(),
        usb_locks: Vec::new(),
        monitors: Vec::new(),
        docker_containers: Vec::new(),
        docker_events: Vec::new(),
        ops: Vec::new(),
        nav: FrameNav::default(),
    }
}

/// 写 v3 录屏文件（多帧），返回 Player 验证帧数。
fn write_v3_recording(path: &std::path::Path, frames: Vec<UiFrame>) {
    let recorder = Recorder::start(path.to_path_buf()).unwrap();
    for frame in frames {
        recorder.submit_frame(frame);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    recorder.stop().unwrap();
}

/// 写 VT100 录屏文件（手动序列化 VtHeader + N×VtFrame）。
fn write_vt100_recording(path: &std::path::Path, frame_count: u64, width: u16, height: u16) {
    use proc::record::vt100::{VT100_MAGIC, VT100_VERSION, VtFrame, VtHeader};
    use std::fs::File;
    use std::io::{BufWriter, Write};

    let header = VtHeader {
        magic: *VT100_MAGIC,
        version: VT100_VERSION,
        start_time: 1_700_000_000,
        width,
        height,
    };
    let header_bytes = bincode::serialize(&header).unwrap();

    let mut file = BufWriter::new(File::create(path).unwrap());
    file.write_all(&(header_bytes.len() as u64).to_le_bytes())
        .unwrap();
    file.write_all(&header_bytes).unwrap();

    for i in 0..frame_count {
        let frame = VtFrame {
            timestamp_ms: i * 1000,
            width,
            height,
            rle: Vec::new(),
        };
        let frame_bytes = bincode::serialize(&frame).unwrap();
        file.write_all(&(frame_bytes.len() as u64).to_le_bytes())
            .unwrap();
        file.write_all(&frame_bytes).unwrap();
    }
    file.flush().unwrap();
}

// ===========================================================================
// replay_info 测试（5 case）
// ===========================================================================

#[test]
fn test_replay_info_v3_recording_returns_uiframe_format() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v3.prec");
    write_v3_recording(
        &path,
        vec![
            make_frame(1_000, 10.0, 100, &["a"], ""),
            make_frame(1_001, 20.0, 200, &["b"], ""),
            make_frame(1_002, 30.0, 300, &["c"], "warning"),
        ],
    );

    let out = record::make_replay_info_json(path.to_str().unwrap());
    assert_eq!(out["ok"], serde_json::json!(true), "out: {out}");
    assert_eq!(out["format"], serde_json::json!("uiframe"));
    assert_eq!(out["version"], serde_json::json!(3));
    assert_eq!(out["frame_count"], serde_json::json!(3));
    // v3 路径含 hostname + anomaly_count + max_cpu + max_mem 字段
    assert!(out["hostname"].is_string(), "hostname missing: {out}");
    assert_eq!(out["anomaly_count"], serde_json::json!(1));
    assert_eq!(out["max_mem"], serde_json::json!(300));
    assert!(out["size_bytes"].as_u64().is_some(), "size_bytes missing");
    // Player::open 读到的 RecordingHeader 与默认 hostname 字段一致
    let player = Player::open(path.clone()).unwrap();
    let header: RecordingHeader = player.header().clone();
    assert_eq!(out["hostname"], serde_json::json!(header.hostname));
}

#[test]
fn test_replay_info_vt100_recording_returns_vt100_format() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vt100.prec");
    write_vt100_recording(&path, 5, 200, 50);

    let out = record::make_replay_info_json(path.to_str().unwrap());
    assert_eq!(out["ok"], serde_json::json!(true), "out: {out}");
    assert_eq!(out["format"], serde_json::json!("vt100"));
    assert_eq!(out["version"], serde_json::json!(2));
    assert_eq!(out["frame_count"], serde_json::json!(5));
    assert_eq!(out["width"], serde_json::json!(200));
    assert_eq!(out["height"], serde_json::json!(50));
    // VT100 路径 start_ms / end_ms 字段在（time_range_ms）
    assert!(out["start_ms"].as_u64().is_some(), "start_ms missing");
    assert!(out["end_ms"].as_u64().is_some(), "end_ms missing");
    // VT100 无 footer → 无 anomaly_count / max_cpu 字段
    assert!(
        out.get("anomaly_count").is_none(),
        "VT100 should not have anomaly_count: {out}"
    );
}

#[test]
fn test_replay_info_missing_file_returns_ok_false() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.prec");

    let out = record::make_replay_info_json(path.to_str().unwrap());
    assert_eq!(out["ok"], serde_json::json!(false), "out: {out}");
    let error = out["error"].as_str().expect("error string");
    assert!(
        error.contains("不存在") || error.contains("No such file"),
        "error should mention missing: {error}"
    );
}

#[test]
fn test_replay_info_invalid_file_returns_ok_false() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("garbage.prec");
    // 写随机字节（既非 v3 也非 VT100）—— is_vt100_file 返 false 后 Player::open 失败
    std::fs::write(&path, b"not a recording file").unwrap();

    let out = record::make_replay_info_json(path.to_str().unwrap());
    assert_eq!(out["ok"], serde_json::json!(false), "out: {out}");
    assert!(out["error"].as_str().is_some(), "error missing");
}

#[test]
fn test_replay_info_has_bookmarks_sidecar_reflects_file_existence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("with_sidecar.prec");
    write_v3_recording(&path, vec![make_frame(1_000, 10.0, 100, &["a"], "")]);

    // 无 sidecar → has_bookmarks_sidecar=false
    let out_no_sidecar = record::make_replay_info_json(path.to_str().unwrap());
    assert_eq!(
        out_no_sidecar["has_bookmarks_sidecar"],
        serde_json::json!(false)
    );

    // 创建 sidecar 文件（<recording>.bookmarks.json）→ has_bookmarks_sidecar=true
    let sidecar_path = format!("{}.bookmarks.json", path.display());
    std::fs::write(&sidecar_path, b"{}").unwrap();
    let out_with_sidecar = record::make_replay_info_json(path.to_str().unwrap());
    assert_eq!(
        out_with_sidecar["has_bookmarks_sidecar"],
        serde_json::json!(true)
    );
}

// ===========================================================================
// replay_search 测试（7 case）
// ===========================================================================

#[test]
fn test_replay_search_substring_matches_process_name() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("substring.prec");
    write_v3_recording(
        &path,
        vec![
            make_frame(1_000, 10.0, 100, &["chrome.exe", "explorer.exe"], ""),
            make_frame(1_001, 20.0, 200, &["firefox.exe"], ""),
            make_frame(1_002, 30.0, 300, &["chrome.exe"], ""),
        ],
    );

    let out = record::make_replay_search_json(path.to_str().unwrap(), "chrome", None);
    assert_eq!(out["ok"], serde_json::json!(true), "out: {out}");
    assert_eq!(out["query"], serde_json::json!("chrome"));
    let match_count = out["match_count"].as_u64().expect("match_count");
    assert!(
        match_count >= 2,
        "should match 2 chrome frames: {match_count}"
    );
    // matches[] 字段结构
    let matches = out["matches"].as_array().expect("matches array");
    assert!(!matches.is_empty());
    // 第一个 match 的 matched_processes 应含 "chrome.exe"
    let first_procs = matches[0]["matched_processes"]
        .as_array()
        .expect("matched_processes array");
    let proc_names: Vec<&str> = first_procs
        .iter()
        .map(|v| v.as_str().unwrap_or(""))
        .collect();
    assert!(
        proc_names.iter().any(|n| n.contains("chrome")),
        "matched_processes should contain chrome: {proc_names:?}"
    );
}

#[test]
fn test_replay_search_filter_expr_cpu_gt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cpu_gt.prec");
    write_v3_recording(
        &path,
        vec![
            make_frame(1_000, 10.0, 100, &["a"], ""),
            make_frame(1_001, 80.0, 200, &["b"], ""),
            make_frame(1_002, 20.0, 300, &["c"], ""),
            make_frame(1_003, 90.0, 400, &["d"], ""),
        ],
    );

    let out = record::make_replay_search_json(path.to_str().unwrap(), ": cpu > 50", None);
    assert_eq!(out["ok"], serde_json::json!(true), "out: {out}");
    assert_eq!(out["match_count"], serde_json::json!(2));
    assert_eq!(out["returned"], serde_json::json!(2));
    assert_eq!(out["truncated"], serde_json::json!(false));
    // cpu>50 命中 frame 1 + 3，cpu_usage 字段在
    let matches = out["matches"].as_array().expect("matches");
    let cpus: Vec<f64> = matches
        .iter()
        .map(|m| m["cpu_usage"].as_f64().unwrap_or(0.0))
        .collect();
    assert!(
        cpus.iter().all(|c| *c > 50.0),
        "all matches should have cpu>50: {cpus:?}"
    );
}

#[test]
fn test_replay_search_filter_expr_mem_gt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mem_gt.prec");
    write_v3_recording(
        &path,
        vec![
            make_frame(1_000, 10.0, 100, &["a"], ""),
            make_frame(1_001, 20.0, 1_000_000, &["b"], ""),
        ],
    );

    let out = record::make_replay_search_json(path.to_str().unwrap(), ": mem > 5000", None);
    assert_eq!(out["ok"], serde_json::json!(true), "out: {out}");
    assert_eq!(out["match_count"], serde_json::json!(1));
    let matches = out["matches"].as_array().expect("matches");
    assert_eq!(matches[0]["memory_used"], serde_json::json!(1_000_000));
}

#[test]
fn test_replay_search_default_limit_100_truncates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("limit_default.prec");
    // 150 帧 cpu=99（全命中 cpu > 50）—— 默认 limit=100 应截断
    let frames: Vec<UiFrame> = (0..150)
        .map(|i| make_frame(1_000 + i, 99.0, 100, &["a"], ""))
        .collect();
    write_v3_recording(&path, frames);

    let out = record::make_replay_search_json(path.to_str().unwrap(), ": cpu > 50", None);
    assert_eq!(out["ok"], serde_json::json!(true), "out: {out}");
    assert_eq!(out["match_count"], serde_json::json!(150));
    assert_eq!(out["returned"], serde_json::json!(100));
    assert_eq!(out["truncated"], serde_json::json!(true));
    assert_eq!(out["limit"], serde_json::json!(100));
    let matches = out["matches"].as_array().expect("matches");
    assert_eq!(matches.len(), 100);
}

#[test]
fn test_replay_search_custom_limit_truncates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("limit_custom.prec");
    let frames: Vec<UiFrame> = (0..50)
        .map(|i| make_frame(1_000 + i, 99.0, 100, &["a"], ""))
        .collect();
    write_v3_recording(&path, frames);

    let out = record::make_replay_search_json(path.to_str().unwrap(), ": cpu > 50", Some(5));
    assert_eq!(out["ok"], serde_json::json!(true), "out: {out}");
    assert_eq!(out["match_count"], serde_json::json!(50));
    assert_eq!(out["returned"], serde_json::json!(5));
    assert_eq!(out["truncated"], serde_json::json!(true));
    assert_eq!(out["limit"], serde_json::json!(5));
}

#[test]
fn test_replay_search_vt100_returns_ok_false() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vt100_search.prec");
    write_vt100_recording(&path, 5, 200, 50);

    let out = record::make_replay_search_json(path.to_str().unwrap(), "anything", None);
    assert_eq!(out["ok"], serde_json::json!(false), "out: {out}");
    let error = out["error"].as_str().expect("error string");
    assert!(
        error.contains("VT100") || error.contains("不支持"),
        "error should mention VT100 not supported: {error}"
    );
}

#[test]
fn test_replay_search_invalid_filter_expr_returns_ok_false() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invalid_expr.prec");
    write_v3_recording(&path, vec![make_frame(1_000, 10.0, 100, &["a"], "")]);

    // 非法 FilterExpr —— `: cpu >>>` 解析失败
    let out = record::make_replay_search_json(path.to_str().unwrap(), ": cpu >>>", None);
    assert_eq!(out["ok"], serde_json::json!(false), "out: {out}");
    assert!(out["error"].as_str().is_some(), "error missing");
}

// ===========================================================================
// eject_status 测试（6 case）
// ===========================================================================

#[test]
fn test_eject_status_empty_drive_returns_unknown_drive() {
    let out = record::make_eject_status_json("");
    assert_eq!(out["ok"], serde_json::json!(true), "out: {out}");
    assert_eq!(out["suggestion"], serde_json::json!("unknown_drive"));
    assert!(out["device"].is_null(), "device should be null");
    assert_eq!(out["lock_count"], serde_json::json!(0));
    assert_eq!(out["ejectable"], serde_json::json!(false));
}

#[test]
fn test_eject_status_non_alpha_returns_unknown_drive() {
    // "123" 无字母字符 → normalize 后 cleaned="" → unknown_drive
    let out = record::make_eject_status_json("123");
    assert_eq!(out["ok"], serde_json::json!(true), "out: {out}");
    assert_eq!(out["suggestion"], serde_json::json!("unknown_drive"));
    assert!(out["device"].is_null());
}

#[test]
fn test_eject_status_invalid_letter_returns_unknown_drive() {
    // Z 盘几乎肯定不是 removable device（Windows 通常 C 为系统盘 + D/E/F 为数据 / USB）
    // 即使 Z 是 removable 也接受（test 容忍 eject_now / kill_locks / unknown_drive 任一）
    let out = record::make_eject_status_json("Z");
    assert_eq!(out["ok"], serde_json::json!(true), "out: {out}");
    let suggestion = out["suggestion"].as_str().expect("suggestion");
    assert!(
        matches!(
            suggestion,
            "unknown_drive" | "eject_now" | "kill_locks" | "unavailable"
        ),
        "suggestion must be valid enum value: {suggestion}"
    );
    // drive 字段必须 normalize 成 "Z:"（不返原始 "Z" 也不带反斜杠）
    let drive = out["drive"].as_str().expect("drive");
    assert!(drive.starts_with('Z'), "drive should start with Z: {drive}");
}

#[test]
fn test_eject_status_normalizes_letter_case_and_colon() {
    // 不同写法 "e:" / "E:\\" / "e" 都应 normalize 成 drive="E:"
    let inputs = ["e:", "E:\\", "e", " E "];
    let mut normalized = Vec::new();
    for input in inputs {
        let out = record::make_eject_status_json(input);
        assert_eq!(
            out["ok"],
            serde_json::json!(true),
            "input={input}, out: {out}"
        );
        let drive = out["drive"].as_str().expect("drive");
        normalized.push(drive.to_string());
    }
    // 所有 input normalize 后都应得到 "E:"（或更大集合）
    assert!(
        normalized.iter().all(|d| d == "E:"),
        "all inputs should normalize to 'E:': {normalized:?}"
    );
}

#[test]
fn test_eject_status_returns_expected_json_shape() {
    // 任意 drive 字符都应返完整字段集
    let out = record::make_eject_status_json("E");
    assert_eq!(out["ok"], serde_json::json!(true), "out: {out}");
    // 必须含所有声明字段（device / ejectable / lock_count / locks / suggestion / drive）
    for key in &[
        "drive",
        "device",
        "ejectable",
        "lock_count",
        "locks",
        "suggestion",
    ] {
        assert!(
            out.get(*key).is_some(),
            "missing required field '{key}': {out}"
        );
    }
    // locks 必须是数组（即便空）
    assert!(out["locks"].is_array(), "locks must be array: {out}");
    // ejectable 必须是 bool
    assert!(
        out["ejectable"].is_boolean(),
        "ejectable must be bool: {out}"
    );
}

#[test]
fn test_eject_status_suggestion_is_one_of_four_values() {
    // 测试多个 drive 字符，每个 suggestion 都应在 4 档枚举内
    for drive in &["C", "D", "E", "F", "G", "Q", "Y", "Z"] {
        let out = record::make_eject_status_json(drive);
        assert_eq!(out["ok"], serde_json::json!(true), "drive={drive}: {out}");
        let suggestion = out["suggestion"].as_str().expect("suggestion string");
        assert!(
            matches!(
                suggestion,
                "eject_now" | "kill_locks" | "unknown_drive" | "unavailable"
            ),
            "drive={drive}: suggestion must be valid enum value, got: {suggestion}"
        );
    }
}
