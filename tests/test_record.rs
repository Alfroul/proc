use proc::record::frame::{
    FOOTER_MAGIC, FrameProcess, LegacySystemFrame, RECORDING_MAGIC, RECORDING_VERSION,
    RecordingFooter, RecordingHeader, UiFrame,
};
use proc::record::reader::{Player, frame_process_to_process_info};
use proc::record::sidecar::IdxSidecar;
use proc::record::writer::Recorder;

fn make_test_frame(timestamp: u64, cpu: f32) -> UiFrame {
    UiFrame {
        timestamp,
        mode: "ProcessList".to_string(),
        status_message: None,
        cpu_usage: cpu,
        memory_used: 8 * 1024 * 1024 * 1024,
        memory_total: 16 * 1024 * 1024 * 1024,
        net_down: 1000,
        net_up: 500,
        cpu_history: vec![],
        mem_history: vec![],
        processes: vec![
            FrameProcess {
                pid: 1,
                name: "proc1".to_string(),
                cpu: 10.0,
                memory: 1024 * 1024,
                disk_read: 100,
                disk_write: 200,
            },
            FrameProcess {
                pid: 2,
                name: "proc2".to_string(),
                cpu: 5.0,
                memory: 2 * 1024 * 1024,
                disk_read: 300,
                disk_write: 400,
            },
        ],
        search_query: String::new(),
        sort_field: "Cpu".to_string(),
        process_view_mode: 0,
        tree_nodes: vec![],
        port_entries: vec![],
        port_view_mode: 0,
        port_process_groups: vec![],
        port_remote_groups: vec![],
        connection_diff: Default::default(),
        anomalies: vec![],
        usb_devices: vec![],
        usb_locks: vec![],
        monitors: vec![],
        docker_containers: vec![],
        docker_events: vec![],
        ops: vec![],
        nav: Default::default(),
    }
}

#[test]
fn test_frame_roundtrip() {
    let frame = make_test_frame(1000, 45.5);
    let bytes = bincode::serialize(&frame).unwrap();
    let decoded: UiFrame = bincode::deserialize(&bytes).unwrap();
    assert_eq!(decoded.timestamp, 1000);
    assert!((decoded.cpu_usage - 45.5).abs() < 0.01);
    assert_eq!(decoded.memory_used, 8 * 1024 * 1024 * 1024);
    assert_eq!(decoded.net_down, 1000);
    assert_eq!(decoded.net_up, 500);
    assert_eq!(decoded.processes.len(), 2);
    assert_eq!(decoded.processes[0].name, "proc1");
    assert_eq!(decoded.processes[1].pid, 2);
}

#[test]
fn test_recording_header() {
    let header = RecordingHeader::default();
    assert_eq!(&header.magic, RECORDING_MAGIC);
    assert_eq!(header.version, RECORDING_VERSION);
    assert!(header.start_time > 0);
    assert!(!header.hostname.is_empty());

    let bytes = bincode::serialize(&header).unwrap();
    let decoded: RecordingHeader = bincode::deserialize(&bytes).unwrap();
    assert_eq!(decoded.magic, *RECORDING_MAGIC);
    assert_eq!(decoded.version, RECORDING_VERSION);
    assert_eq!(decoded.start_time, header.start_time);
}

#[test]
fn test_recorder_write_and_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.prec");

    let recorder = Recorder::start(path.clone()).unwrap();
    for i in 0..10u64 {
        let frame = make_test_frame(1000 + i, 10.0 + i as f32);
        recorder.submit_frame(frame);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    recorder.stop().unwrap();

    let player = Player::open(path).unwrap();
    assert_eq!(player.total_frames(), 10);

    let first = player.frame_at(0).unwrap();
    assert_eq!(first.timestamp, 1000);

    let last = player.frame_at(9).unwrap();
    assert_eq!(last.timestamp, 1009);
    assert!((last.cpu_usage - 19.0).abs() < 0.01);

    assert_eq!(player.header().magic, *RECORDING_MAGIC);
}

#[test]
fn test_player_seek() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("seek.prec");

    let recorder = Recorder::start(path.clone()).unwrap();
    for i in 0..20u64 {
        let frame = make_test_frame(2000 + i, i as f32 * 2.5);
        recorder.submit_frame(frame);
    }
    recorder.stop().unwrap();

    let player = Player::open(path).unwrap();
    assert_eq!(player.total_frames(), 20);

    for idx in [0, 5, 10, 15, 19] {
        let frame = player.frame_at(idx).unwrap();
        assert_eq!(frame.timestamp, 2000 + idx as u64);
        assert!((frame.cpu_usage - idx as f32 * 2.5).abs() < 0.01);
    }

    assert!(player.frame_at(20).is_none());
}

#[test]
fn test_player_timestamp_search() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ts.prec");

    let recorder = Recorder::start(path.clone()).unwrap();
    for i in 0..10u64 {
        let frame = make_test_frame(5000 + i * 10, i as f32);
        recorder.submit_frame(frame);
    }
    recorder.stop().unwrap();

    let player = Player::open(path).unwrap();

    let idx = player.frame_near_timestamp(5030);
    assert_eq!(idx, 3);

    let idx = player.frame_near_timestamp(5034);
    assert!(idx == 3 || idx == 4);

    let idx = player.frame_near_timestamp(4000);
    assert_eq!(idx, 0);

    let idx = player.frame_near_timestamp(9999);
    assert_eq!(idx, 9);
}

#[test]
fn test_frame_process_conversion() {
    let fp = FrameProcess {
        pid: 42,
        name: "test.exe".to_string(),
        cpu: 15.5,
        memory: 4 * 1024 * 1024,
        disk_read: 1024,
        disk_write: 2048,
    };

    let pi = frame_process_to_process_info(&fp);
    assert_eq!(pi.pid, 42);
    assert_eq!(pi.name.as_ref(), "test.exe");
    assert!((pi.cpu_usage - 15.5).abs() < 0.01);
    assert_eq!(pi.memory, 4 * 1024 * 1024);
    assert_eq!(pi.disk_usage, (1024, 2048));

    assert_eq!(pi.exe, None);
    assert!(pi.cmd.is_empty());
    assert_eq!(pi.parent_pid, None);
    assert_eq!(pi.virtual_memory, 0);
    assert_eq!(pi.start_time, 0);
}

/// Header length sanity cap (P1.22): a malformed/hostile recording that
/// claims a multi-MB header must be rejected before the allocation.
#[test]
fn test_player_rejects_oversized_header() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hostile.prec");

    // Hand-craft: 8-byte len claiming 1 MB header, then a tiny body that
    // would normally be read into a 1 MB buffer.
    let fake_len: u64 = 1024 * 1024; // 1 MB, well above the 64 KB cap
    let mut bytes = fake_len.to_le_bytes().to_vec();
    bytes.extend_from_slice(&[0u8; 32]); // bogus "header" body

    std::fs::write(&path, &bytes).unwrap();

    let err = Player::open(path)
        .err()
        .expect("oversized header must error");
    let msg = format!("{err}");
    assert!(
        msg.contains("header 异常大"),
        "expected Chinese cap message, got: {msg}"
    );
}

// ─────────────────────────────────────────────────────────────────────
// v0.14 stage 1: v3 文件格式（按需加载 + footer + sidecar）
// ─────────────────────────────────────────────────────────────────────

fn make_test_frame_with_anomalies(timestamp: u64, cpu: f32, anomaly_n: usize) -> UiFrame {
    let mut frame = make_test_frame(timestamp, cpu);
    frame.cpu_usage = cpu;
    frame.anomalies = (0..anomaly_n)
        .map(|i| proc::record::frame::FrameAnomaly {
            rule_id: format!("r{i}"),
            severity: "Warning".to_string(),
            title: format!("t{i}"),
            detail: format!("d{i}"),
            affected_pid: None,
            affected_ip: None,
        })
        .collect();
    frame.memory_used = (cpu as u64) * 1024 * 1024;
    frame
}

#[test]
fn v3_round_trip_player_reads_what_writer_wrote() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v3.prec");

    let recorder = Recorder::start(path.clone()).unwrap();
    for i in 0..10u64 {
        recorder.submit_frame(make_test_frame(1000 + i, 10.0 + i as f32));
    }
    recorder.stop().unwrap();

    let player = Player::open(path).unwrap();
    assert_eq!(player.total_frames(), 10);
    assert_eq!(player.header().version, RECORDING_VERSION);

    // random seek
    for idx in [0, 3, 7, 9] {
        let frame = player.frame_at(idx).unwrap();
        assert_eq!(frame.timestamp, 1000 + idx as u64);
        assert!((frame.cpu_usage - (10.0 + idx as f32)).abs() < 0.01);
    }
    assert!(player.frame_at(10).is_none());
}

#[test]
fn v3_footer_metadata_matches_recorded_frames() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("meta.prec");

    let recorder = Recorder::start(path.clone()).unwrap();
    // 5 frames with known cpu/mem/anomaly profiles
    recorder.submit_frame(make_test_frame_with_anomalies(100, 10.0, 0));
    recorder.submit_frame(make_test_frame_with_anomalies(200, 90.0, 2));
    recorder.submit_frame(make_test_frame_with_anomalies(300, 50.0, 1));
    recorder.submit_frame(make_test_frame_with_anomalies(400, 33.0, 0));
    recorder.submit_frame(make_test_frame_with_anomalies(500, 99.0, 3));
    recorder.stop().unwrap();

    let player = Player::open(path).unwrap();
    let meta = player.meta();
    assert_eq!(meta.frame_count, 5);
    assert_eq!(meta.start_time, 100);
    assert_eq!(meta.end_time, 500);
    // frames: 0/2/1/0/3 anomalies → 总和 6
    assert_eq!(meta.anomaly_count, 6);
    assert!((meta.max_cpu - 99.0).abs() < 0.01);
    assert_eq!(meta.max_mem, 99 * 1024 * 1024);
    assert_eq!(meta.frame_offsets.len(), 5);
}

#[test]
fn v3_frame_at_cache_doesnt_break_concurrent_seek() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cache.prec");
    let recorder = Recorder::start(path.clone()).unwrap();
    for i in 0..30u64 {
        recorder.submit_frame(make_test_frame(7000 + i, i as f32));
    }
    recorder.stop().unwrap();

    let player = Player::open(path).unwrap();
    // 连续访问同一 idx（命中 cache）
    for _ in 0..3 {
        let f = player.frame_at(15).unwrap();
        assert_eq!(f.timestamp, 7015);
    }
    // 跳到别的 idx（cache 失效）
    let f = player.frame_at(0).unwrap();
    assert_eq!(f.timestamp, 7000);
    // 跳回（cache 已被覆盖，重新读）
    let f = player.frame_at(15).unwrap();
    assert_eq!(f.timestamp, 7015);
}

#[test]
fn v3_footer_trailer_persisted_at_file_end() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trailer.prec");

    let recorder = Recorder::start(path.clone()).unwrap();
    for i in 0..3u64 {
        recorder.submit_frame(make_test_frame(9000 + i, 1.0));
    }
    recorder.stop().unwrap();

    let bytes = std::fs::read(&path).unwrap();
    let n = bytes.len();
    assert!(n >= 16);
    assert_eq!(&bytes[n - 8..n], &FOOTER_MAGIC);
    let mut len_buf = [0u8; 8];
    len_buf.copy_from_slice(&bytes[n - 16..n - 8]);
    let footer_len = u64::from_le_bytes(len_buf) as usize;
    let footer_start = n - 16 - footer_len;
    let footer: RecordingFooter =
        bincode::deserialize(&bytes[footer_start..footer_start + footer_len]).unwrap();
    assert_eq!(footer.frame_count, 3);
}

/// 手工构造 v2 文件（无 footer）：模仿 v0.13 老用户的录屏文件。
fn write_legacy_v2_file(path: &std::path::Path, frames: &[UiFrame]) {
    use std::io::Write;
    let header = RecordingHeader {
        magic: *RECORDING_MAGIC,
        version: 2,
        start_time: frames.first().map(|f| f.timestamp).unwrap_or(0),
        hostname: "legacy".to_string(),
    };
    let header_bytes = bincode::serialize(&header).unwrap();

    let mut file = std::fs::File::create(path).unwrap();
    file.write_all(&(header_bytes.len() as u64).to_le_bytes())
        .unwrap();
    file.write_all(&header_bytes).unwrap();
    for f in frames {
        let bytes = bincode::serialize(f).unwrap();
        file.write_all(&(bytes.len() as u64).to_le_bytes()).unwrap();
        file.write_all(&bytes).unwrap();
    }
}

/// 手工构造 v1 文件（LegacySystemFrame）：模仿 v0.5 老老用户。
fn write_legacy_v1_file(path: &std::path::Path, frames: &[LegacySystemFrame]) {
    use std::io::Write;
    let header = RecordingHeader {
        magic: *RECORDING_MAGIC,
        version: 1,
        start_time: frames.first().map(|f| f.timestamp).unwrap_or(0),
        hostname: "legacy1".to_string(),
    };
    let header_bytes = bincode::serialize(&header).unwrap();

    let mut file = std::fs::File::create(path).unwrap();
    file.write_all(&(header_bytes.len() as u64).to_le_bytes())
        .unwrap();
    file.write_all(&header_bytes).unwrap();
    for f in frames {
        let bytes = bincode::serialize(f).unwrap();
        file.write_all(&(bytes.len() as u64).to_le_bytes()).unwrap();
        file.write_all(&bytes).unwrap();
    }
}

fn legacy_v1_frame(ts: u64, cpu: f32) -> LegacySystemFrame {
    LegacySystemFrame {
        timestamp: ts,
        cpu_usage: cpu,
        memory_used: 1024,
        memory_total: 4096,
        net_down: 0,
        net_up: 0,
        processes: vec![],
    }
}

#[test]
fn v2_legacy_file_loads_via_sidecar_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy_v2.prec");

    // 写 5 帧 v2 文件，没有 footer
    let frames: Vec<UiFrame> = (0..5u64).map(|i| make_test_frame(2000 + i, 5.0)).collect();
    write_legacy_v2_file(&path, &frames);

    // 首次 open：fallback 全量加载，写 sidecar
    let player = Player::open(path.clone()).unwrap();
    assert_eq!(player.total_frames(), 5);
    assert_eq!(player.header().version, 2);
    let meta = player.meta();
    assert_eq!(meta.start_time, 2000);
    assert_eq!(meta.end_time, 2004);
    assert_eq!(meta.frame_offsets.len(), 5);

    // random seek 仍可用
    let f = player.frame_at(2).unwrap();
    assert_eq!(f.timestamp, 2002);

    // sidecar 应已生成
    let sidecar_path = IdxSidecar::sidecar_path(&path);
    assert!(sidecar_path.exists(), "sidecar should be written");
}

#[test]
fn v1_legacy_file_loads_via_sidecar_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy_v1.prec");

    let frames = vec![
        legacy_v1_frame(3000, 10.0),
        legacy_v1_frame(3010, 20.0),
        legacy_v1_frame(3020, 30.0),
    ];
    write_legacy_v1_file(&path, &frames);

    let player = Player::open(path.clone()).unwrap();
    assert_eq!(player.total_frames(), 3);
    assert_eq!(player.header().version, 1);
    let f0 = player.frame_at(0).unwrap();
    assert_eq!(f0.timestamp, 3000);
    // v1 frame 升级到 v2 后 memory_used 应等于 LegacySystemFrame.memory_used
    assert_eq!(f0.memory_used, 1024);
    let f2 = player.frame_at(2).unwrap();
    assert_eq!(f2.timestamp, 3020);
}

#[test]
fn sidecar_speeds_up_second_open_of_legacy_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy_cached.prec");

    let frames: Vec<UiFrame> = (0..8u64).map(|i| make_test_frame(4000 + i, 1.0)).collect();
    write_legacy_v2_file(&path, &frames);

    // 首次：fallback 路径 + sidecar 生成
    let p1 = Player::open(path.clone()).unwrap();
    assert_eq!(p1.total_frames(), 8);
    drop(p1);

    let sidecar_path = IdxSidecar::sidecar_path(&path);
    assert!(sidecar_path.exists());

    // 二次：sidecar 命中（不再全量 deserialize）
    let p2 = Player::open(path.clone()).unwrap();
    assert_eq!(p2.total_frames(), 8);
    let f = p2.frame_at(5).unwrap();
    assert_eq!(f.timestamp, 4005);
}

#[test]
fn sidecar_invalidated_when_source_size_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy_changed.prec");

    let frames: Vec<UiFrame> = (0..5u64).map(|i| make_test_frame(5000 + i, 1.0)).collect();
    write_legacy_v2_file(&path, &frames);

    let p1 = Player::open(path.clone()).unwrap();
    drop(p1);
    let sidecar_path = IdxSidecar::sidecar_path(&path);
    let original_sidecar = std::fs::read(&sidecar_path).unwrap();

    // 改源文件（追加更多帧，size + mtime 都变）
    let mut more_frames = frames.clone();
    more_frames.push(make_test_frame(6000, 9.0));
    more_frames.push(make_test_frame(6010, 99.0));
    write_legacy_v2_file(&path, &more_frames);

    // 此时 sidecar 应失效 → 第二次 open 触发重新 fallback
    let p2 = Player::open(path.clone()).unwrap();
    assert_eq!(p2.total_frames(), 7); // 新的 7 帧

    // sidecar 应被刷新
    let new_sidecar = std::fs::read(&sidecar_path).unwrap();
    assert_ne!(original_sidecar, new_sidecar);
}

#[test]
fn sidecar_corruption_falls_back_silently() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy_corrupt_sidecar.prec");

    let frames: Vec<UiFrame> = (0..4u64).map(|i| make_test_frame(7000 + i, 1.0)).collect();
    write_legacy_v2_file(&path, &frames);

    // 写损坏的 sidecar
    let sidecar_path = IdxSidecar::sidecar_path(&path);
    std::fs::write(&sidecar_path, b"not valid bincode garbage").unwrap();

    // 应静默降级到全量加载，不 panic
    let player = Player::open(path.clone()).unwrap();
    assert_eq!(player.total_frames(), 4);
}

#[test]
fn v3_file_does_not_write_sidecar() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v3_no_sidecar.prec");

    let recorder = Recorder::start(path.clone()).unwrap();
    for i in 0..5u64 {
        recorder.submit_frame(make_test_frame(8000 + i, 1.0));
    }
    recorder.stop().unwrap();

    let _player = Player::open(path.clone()).unwrap();
    let sidecar_path = IdxSidecar::sidecar_path(&path);
    // v3 文件自带 footer，不需要 sidecar
    assert!(
        !sidecar_path.exists(),
        "v3 file should not generate sidecar"
    );
}

#[test]
fn v3_player_time_range_uses_footer_meta() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("range.prec");

    let recorder = Recorder::start(path.clone()).unwrap();
    recorder.submit_frame(make_test_frame(11_000, 1.0));
    recorder.submit_frame(make_test_frame(11_500, 2.0));
    recorder.submit_frame(make_test_frame(12_000, 3.0));
    recorder.stop().unwrap();

    let player = Player::open(path).unwrap();
    let (start, end) = player.time_range();
    assert_eq!(start, 11_000);
    assert_eq!(end, 12_000);
}

#[test]
fn v3_player_frame_near_timestamp_uses_meta_range() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("near.prec");

    let recorder = Recorder::start(path.clone()).unwrap();
    for i in 0..10u64 {
        recorder.submit_frame(make_test_frame(20_000 + i * 100, i as f32));
    }
    recorder.stop().unwrap();

    let player = Player::open(path).unwrap();
    // 中间帧
    let idx = player.frame_near_timestamp(20_450);
    assert!(idx == 4 || idx == 5);
    // 早于 start
    assert_eq!(player.frame_near_timestamp(1000), 0);
    // 晚于 end
    assert_eq!(player.frame_near_timestamp(100_000), 9);
}
