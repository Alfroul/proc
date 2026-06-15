use proc::record::frame::{
    FrameProcess, RECORDING_MAGIC, RECORDING_VERSION, RecordingHeader, UiFrame,
};
use proc::record::reader::{Player, frame_process_to_process_info};
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
    assert_eq!(pi.name, "test.exe");
    assert!((pi.cpu_usage - 15.5).abs() < 0.01);
    assert_eq!(pi.memory, 4 * 1024 * 1024);
    assert_eq!(pi.disk_usage, (1024, 2048));

    assert_eq!(pi.exe, None);
    assert!(pi.cmd.is_empty());
    assert_eq!(pi.parent_pid, None);
    assert_eq!(pi.virtual_memory, 0);
    assert_eq!(pi.start_time, 0);
}
