//! v0.17 主题 F VT100 replay 增强子模块 — VT100 字节流转码 UiFrame。
//!
//! v0.17 阶段 5 Slice 实装（ADR-0028 / brainstorm 决策 6 方案 a 临时转码）。
//!
//! ## 实装澄清（vs ADR-0028 §1+§3 描述）
//!
//! ADR-0028 描述 `feed_bytes(bytes)` + `snapshot_frame()` 假设输入是 VT100 字节流，
//! 需扩 VT500 序列解析器（CSI / SGR / cursor move / clear 全套反序列化）。但
//! v0.6 落地的 `VtRecorder::try_capture(buffer: &Buffer, area: Rect)`（`src/record/vt100.rs`）
//! 在录制时已经把 ratatui `Buffer` 状态序列化为 `VtFrame.rle: Vec<(u16, CellDump)>`。
//! 回放时 `VtPlayer::open` 反序列化得到 `Vec<VtFrame>`，每个 VtFrame 已经是结构化
//! 数据（不需要 VT500 序列反序列化）。
//!
//! **结论**：VT100 文件存的是 VtFrame 流（已解析 Buffer cells），不是原始 VT100
//! 字节流。stage 5 API 改为 `convert_frame(&VtFrame) -> UiFrame` 直接 1:1 映射，
//! VtRecorder 5 FPS 切片节奏不变（不 30 FPS 重切片）。`vt100` crate（已声明依赖）
//! 仅用于 `docker exec` 交互式终端路径（`App::container_exec_vt`），与录屏 / 回放
//! 路径无关。
//!
//! ## 临时转码路径（ADR-0028 决策 6 方案 a）
//!
//! - `proc replay <file>` 检测 VT100 文件 → 临时转码到 `<file>.tmp.v3`
//! - 走 v3 Player 路径 → 退出时 [`TranscodedTempFile`] Drop 自动清理
//! - 不破坏原 VT100 文件，转码失败可回退 [`crate::record::vt100::VtPlayer`] 正向 replay
//!
//! ## 与 v0.6 VtPlayer 路径并行
//!
//! VtPlayer 不做转码，仅正向 replay VT100 字节流。本转换器把 VtFrame 流增量
//! 转换为 [`UiFrame`]，让 VT100 录屏享受 v0.14 落地的 search / 倒放 / 书签全部能力。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::encoding::options_for_version;
use super::frame::{FrameConnectionDiff, UiFrame};
use super::vt100::{
    VT100_MAGIC, VT100_VERSION, VtFrame, VtHeader, VtPlayer, extract_process_names_from_rle,
};
use super::writer::Recorder;

/// VT100 → UiFrame 转换器（v0.17 stage 5 实装）。
///
/// 把 VtFrame 流 1:1 转换为 UiFrame 流。VtRecorder 已按 5 FPS 切片节奏录制，
/// 不做 30 FPS 重切片（VtFrame 之间无中间帧数据）。
///
/// **跨帧累积**：converter 记录所有出现过的进程名到 `seen_process_names`，
/// [`Self::stats`] 返 unique 进程数（VT100 路径独有的 metadata，v3 路径没有）。
///
/// # Example
///
/// ```no_run
/// use proc::record::Vt100ToUiFrameConverter;
/// use proc::record::vt100::VtPlayer;
///
/// # fn main() -> anyhow::Result<()> {
/// let player = VtPlayer::open("rec.prec".into())?;
/// let header = player.header();
/// let mut converter = Vt100ToUiFrameConverter::new(
///     header.start_time,
///     "my-host".to_string(),
/// );
/// for i in 0..player.total_frames() {
///     if let Some(vt) = player.frame_at(i) {
///         let ui: proc::record::UiFrame = converter.convert_frame(vt);
///         // ... 提交到 Recorder / 写入 v3 文件
///     }
/// }
/// let stats = converter.stats();
/// println!("转换 {} 帧，涉及 {} 个 unique 进程", stats.frame_count, stats.unique_process_count);
/// # Ok(())
/// # }
/// ```
pub struct Vt100ToUiFrameConverter {
    /// anchor：VtHeader.start_time（unix epoch secs）。VtFrame.timestamp_ms 是相对
    /// 录制起始的毫秒，转换 UiFrame.timestamp 时加上 start_time * 1000 让它与
    /// v3 footer 的 start_time / end_time 同 epoch。
    start_time: u64,
    /// 写入 RecordingHeader.hostname 字段（与环境变量构造逻辑对齐）。
    hostname: String,
    /// 已转换的帧数（完工时返 stats）。
    frame_count: u64,
    /// 跨帧累积的进程名（lowercase 归一 dedup），完工时返 unique count。
    seen_process_names: HashSet<String>,
}

impl Vt100ToUiFrameConverter {
    /// 创建新转换器。
    ///
    /// # Parameters
    ///
    /// - `start_time`：VT100 文件的 `VtHeader.start_time`（unix epoch secs）
    /// - `hostname`：写入目标 v3 文件的 `RecordingHeader.hostname` 字段
    #[must_use]
    pub fn new(start_time: u64, hostname: String) -> Self {
        Self {
            start_time,
            hostname,
            frame_count: 0,
            seen_process_names: HashSet::new(),
        }
    }

    /// 把一个 VtFrame 转换为 UiFrame（1:1 映射）。
    ///
    /// VtRecorder 已按 5 FPS 切片节奏录制，每个 VtFrame 转一个 UiFrame（不合并 /
    /// 不拆分）。UiFrame 字段填充策略详见 ADR-0028 §4 + stage 5 决策 3 表：
    ///
    /// - `timestamp = start_time * 1000 + vt.timestamp_ms`（unix epoch ms）
    /// - `mode = "VT100"`（让 search `mode =~ /VT100/` 能识别 VT100 转码帧）
    /// - `cpu_usage / memory_*` = 0（VT100 路径无系统指标）
    /// - `processes` = `extract_process_names_from_rle` 启发式提取（pid=0 占位）
    /// - `anomalies` = `vec![]`（VT100 路径无 anomaly 标记）
    /// - 其他字段 = 默认值
    pub fn convert_frame(&mut self, vt: &VtFrame) -> UiFrame {
        let processes = extract_process_names_from_rle(&vt.rle, vt.width);
        // 累积 unique 进程名（lowercase 归一）
        for p in &processes {
            self.seen_process_names.insert(p.name.to_lowercase());
        }

        // timestamp：VtFrame 是相对录制起始的 ms，加上 start_time * 1000 让它落到 unix epoch ms
        let timestamp_ms = self
            .start_time
            .saturating_mul(1000)
            .saturating_add(vt.timestamp_ms);

        self.frame_count += 1;

        UiFrame {
            timestamp: timestamp_ms,
            mode: "VT100".to_string(),
            status_message: None,
            cpu_usage: 0.0,
            memory_used: 0,
            memory_total: 0,
            net_down: 0,
            net_up: 0,
            cpu_history: Vec::new(),
            mem_history: Vec::new(),
            processes,
            search_query: String::new(),
            sort_field: "Name".to_string(),
            process_view_mode: 0,
            tree_nodes: Vec::new(),
            port_entries: Vec::new(),
            port_view_mode: 0,
            port_process_groups: Vec::new(),
            port_remote_groups: Vec::new(),
            connection_diff: FrameConnectionDiff::default(),
            anomalies: Vec::new(),
            usb_devices: Vec::new(),
            usb_locks: Vec::new(),
            monitors: Vec::new(),
            docker_containers: Vec::new(),
            docker_events: Vec::new(),
            ops: Vec::new(),
            nav: Default::default(),
        }
    }

    /// 转换统计（完工后调用）。
    #[must_use]
    pub fn stats(&self) -> Vt100TranscodeStats {
        Vt100TranscodeStats {
            frame_count: self.frame_count,
            unique_process_count: self.seen_process_names.len(),
            hostname: self.hostname.clone(),
        }
    }
}

impl Default for Vt100ToUiFrameConverter {
    fn default() -> Self {
        Self::new(0, String::new())
    }
}

/// `convert_vt100_to_v3_file` 完工统计（v0.17 stage 5 新增）。
///
/// `frame_count` 与目标 v3 文件 `RecordingFooter.frame_count` 一致；
/// `unique_process_count` 是 converter 跨帧累积的 unique 进程名数（VT100 路径独有
/// 的 metadata，v3 路径没有）；`hostname` 从环境变量构造写入 v3 文件 header。
#[derive(Debug, Clone, Default)]
pub struct Vt100TranscodeStats {
    /// 已转换的帧数（= 目标 v3 文件 footer.frame_count）
    pub frame_count: u64,
    /// 跨帧累积的 unique 进程名数（lowercase 归一 dedup）
    pub unique_process_count: usize,
    /// 写入目标 v3 文件的 RecordingHeader.hostname 字段值
    pub hostname: String,
}

/// 一次性把 VT100 `.prec` 文件转码为 v3 `.prec` 临时文件（v0.17 stage 5 新增）。
///
/// 流程：
/// 1. `VtPlayer::open(src)` 读 header + 全部 VtFrame
/// 2. 构造 `RecordingHeader`（沿用 VtHeader.start_time + 调用方提供 hostname）
/// 3. `Recorder::start(dst)` 启动 v3 writer（写 header）
/// 4. for each VtFrame: `converter.convert_frame(vt)` → `recorder.submit_frame(ui)`
/// 5. `recorder.stop()` 写 footer + close
/// 6. 返 [`Vt100TranscodeStats`]（frame_count / unique_process_count / hostname）
///
/// # Parameters
///
/// - `src`：源 VT100 `.prec` 文件路径（VT10 magic）
/// - `dst`：目标 v3 `.prec` 文件路径（自动创建父目录）
///
/// # Returns
///
/// `Ok(Vt100TranscodeStats)` 转换成功；`Err(String)` 含失败原因：
/// - `"源文件不是 VT100 格式..."`：src 不存在 / 不是 VT100 magic / 损坏
/// - `"目标文件创建失败: ..."`：dst 路径不可写 / 磁盘满
/// - `"VtPlayer 打开失败: ..."`：反序列化 header 失败
/// - `"Recorder 启动失败: ..."`：v3 writer spawn 失败
pub fn convert_vt100_to_v3_file(src: &Path, dst: &Path) -> Result<Vt100TranscodeStats, String> {
    // 1. 读 VT100 文件 header（不依赖 VtPlayer::open，避免全帧加载到内存）
    let header = read_vt100_header(src)?;
    let hostname = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".to_string());

    // 2. VtPlayer::open 读全部 VtFrame（VtPlayer 设计就是 open 时全量加载，
    //    与 v3 Player 按需加载不同，但 VT100 文件通常较小，可接受）
    let player =
        VtPlayer::open(src.to_path_buf()).map_err(|e| format!("VtPlayer 打开失败: {e}"))?;

    // 3. 启动 v3 Recorder（写 header）
    let recorder =
        Recorder::start(dst.to_path_buf()).map_err(|e| format!("Recorder 启动失败: {e}"))?;

    // 4. 转换 + 提交
    let mut converter = Vt100ToUiFrameConverter::new(header.start_time, hostname);
    for i in 0..player.total_frames() {
        if let Some(vt) = player.frame_at(i) {
            let ui = converter.convert_frame(vt);
            recorder.submit_frame(ui);
        }
    }

    // 5. stop 写 footer
    recorder
        .stop()
        .map_err(|e| format!("Recorder stop 失败: {e}"))?;

    // 6. 返统计
    Ok(converter.stats())
}

/// 读 VT100 文件 header（不加载全帧），用于转码前快速校验 + 拿 start_time。
fn read_vt100_header(path: &Path) -> Result<VtHeader, String> {
    use bincode::Options;
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| format!("源文件打开失败: {e}"))?;
    let mut len_buf = [0u8; 8];
    file.read_exact(&mut len_buf)
        .map_err(|e| format!("读 header len 失败: {e}"))?;
    let header_len = u64::from_le_bytes(len_buf) as usize;
    if header_len > 1024 {
        return Err(format!("header 异常大: {header_len} bytes"));
    }
    let mut header_buf = vec![0u8; header_len];
    file.read_exact(&mut header_buf)
        .map_err(|e| format!("读 header 失败: {e}"))?;
    let header: VtHeader = options_for_version(VT100_VERSION)
        .deserialize(&header_buf)
        .map_err(|e| format!("header 反序列化失败: {e}"))?;
    if &header.magic != VT100_MAGIC {
        return Err(format!(
            "源文件不是 VT100 格式 (magic={:?}, expected={:?})",
            header.magic, VT100_MAGIC
        ));
    }
    Ok(header)
}

/// 临时 v3 文件 RAII wrapper（v0.17 stage 5 新增）。
///
/// Drop 时自动删除文件，与 `tempfile::TempPath` 同款语义。让 CLI / MCP 路径用
/// `let _tmp = TranscodedTempFile::new(path)?;` 持有，退出作用域时自动清理
/// （即使 panic 也清理）。
///
/// # Example
///
/// ```no_run
/// use proc::record::{TranscodedTempFile, convert_vt100_to_v3_file};
/// use std::path::Path;
///
/// # fn main() -> Result<(), String> {
/// let src = Path::new("rec.prec");
/// let tmp = src.with_extension("prec.tmp.v3");
/// let _stats = convert_vt100_to_v3_file(src, &tmp)?;
/// let _cleanup = TranscodedTempFile::new(tmp.clone());
/// // ... 用 tmp 做 replay / search ...
/// // _cleanup drop 时自动删 tmp
/// # Ok(())
/// # }
/// ```
pub struct TranscodedTempFile {
    path: PathBuf,
    keep: bool,
}

impl TranscodedTempFile {
    /// 创建 wrapper（不创建文件，文件应由 [`convert_vt100_to_v3_file`] 创建）。
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path, keep: false }
    }

    /// 拿临时文件路径（传给 Player::open 等）。
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Opt-out 自动清理（debug 场景保留临时文件）。
    ///
    /// 调用后 Drop 不再删文件，让用户手动检查转码结果。
    pub fn keep(mut self) {
        self.keep = true;
    }
}

impl Drop for TranscodedTempFile {
    fn drop(&mut self) {
        if !self.keep {
            // 忽略删除错误（与 v0.6 VtRecorder::Drop 同款语义）——文件可能被
            // Player 持有 handle / 权限问题 / 已删除。残留临时文件不影响功能。
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::vt100::{CellDump, VT100_MAGIC, VT100_VERSION, VtHeader};
    use bincode::Options;

    fn make_test_vt_frame(text: &str, ts_ms: u64, width: u16) -> VtFrame {
        let cells: Vec<CellDump> = text
            .chars()
            .map(|c| CellDump {
                ch: c as u32,
                fg: 0,
                bg: 0,
                flags: 0,
            })
            .collect();
        let height = (cells.len() as u16).div_ceil(width.max(1));
        let rle = cells.into_iter().map(|c| (1, c)).collect();
        VtFrame {
            timestamp_ms: ts_ms,
            width,
            height: height.max(1),
            rle,
        }
    }

    fn write_vt100_file(path: &Path, frames: &[VtFrame]) -> std::io::Result<()> {
        use std::io::Write;
        let header = VtHeader {
            magic: *VT100_MAGIC,
            version: VT100_VERSION,
            start_time: 1_700_000_000,
            width: 80,
            height: 24,
        };
        let header_bytes = options_for_version(VT100_VERSION)
            .serialize(&header)
            .unwrap();
        let mut file = std::fs::File::create(path)?;
        let mut buf = std::io::BufWriter::new(&mut file);
        buf.write_all(&(header_bytes.len() as u64).to_le_bytes())?;
        buf.write_all(&header_bytes)?;
        for frame in frames {
            let bytes = options_for_version(VT100_VERSION).serialize(frame).unwrap();
            buf.write_all(&(bytes.len() as u64).to_le_bytes())?;
            buf.write_all(&bytes)?;
        }
        buf.flush()?;
        Ok(())
    }

    #[test]
    fn converter_new_initial_state() {
        let c = Vt100ToUiFrameConverter::new(1_700_000_000, "host".to_string());
        let stats = c.stats();
        assert_eq!(stats.frame_count, 0);
        assert_eq!(stats.unique_process_count, 0);
        assert_eq!(stats.hostname, "host");
    }

    #[test]
    fn convert_frame_basic_fields() {
        let mut c = Vt100ToUiFrameConverter::new(1_700_000_000, "host".to_string());
        let vt = make_test_vt_frame("chrome.exe", 5000, 30);
        let ui = c.convert_frame(&vt);

        // timestamp = start_time * 1000 + ts_ms = 1700000000 * 1000 + 5000
        assert_eq!(ui.timestamp, 1_700_000_005_000);
        assert_eq!(ui.mode, "VT100");
        assert_eq!(ui.cpu_usage, 0.0);
        assert_eq!(ui.memory_used, 0);
        assert_eq!(ui.memory_total, 0);
        assert!(ui.anomalies.is_empty());
        assert_eq!(ui.sort_field, "Name");
        assert!(ui.process_view_mode == 0);
    }

    #[test]
    fn convert_frame_extracts_process_names() {
        let mut c = Vt100ToUiFrameConverter::new(0, String::new());
        let vt = make_test_vt_frame("chrome.exe    code.exe", 0, 30);
        let ui = c.convert_frame(&vt);
        let names: Vec<String> = ui.processes.iter().map(|p| p.name.to_lowercase()).collect();
        assert!(names.contains(&"chrome.exe".to_string()));
        assert!(names.contains(&"code.exe".to_string()));
        // 占位字段
        for p in &ui.processes {
            assert_eq!(p.pid, 0);
            assert_eq!(p.cpu, 0.0);
            assert_eq!(p.memory, 0);
        }
    }

    #[test]
    fn convert_frame_accumulates_unique_processes() {
        let mut c = Vt100ToUiFrameConverter::new(0, String::new());
        // 帧 1：chrome + code
        c.convert_frame(&make_test_vt_frame("chrome.exe    code.exe", 0, 30));
        // 帧 2：chrome + slack（chrome 重复）
        c.convert_frame(&make_test_vt_frame("chrome.exe    slack.exe", 1000, 30));
        // 帧 3：notepad（新）
        c.convert_frame(&make_test_vt_frame("notepad.exe", 2000, 30));

        let stats = c.stats();
        assert_eq!(stats.frame_count, 3);
        // unique: chrome / code / slack / notepad = 4
        assert_eq!(stats.unique_process_count, 4);
    }

    #[test]
    fn convert_frame_dedup_within_frame() {
        let mut c = Vt100ToUiFrameConverter::new(0, String::new());
        let vt = make_test_vt_frame("chrome.exe chrome.exe chrome.exe", 0, 40);
        let ui = c.convert_frame(&vt);
        let chrome_count = ui
            .processes
            .iter()
            .filter(|p| p.name.to_lowercase() == "chrome.exe")
            .count();
        assert_eq!(chrome_count, 1);
    }

    #[test]
    fn read_vt100_header_rejects_non_vt100() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not_vt100.prec");
        // 写一个真正的 v3 .prec 文件（PREC magic，不是 VT100）。
        // bincode deserializing RecordingHeader bytes as VtHeader 会因 schema 不匹配
        // （RecordingHeader 多 hostname 字段）或 magic mismatch 失败 —— 任意错误都接受。
        let recorder = Recorder::start(path.clone()).unwrap();
        recorder.stop().unwrap();

        let result = read_vt100_header(&path);
        assert!(
            result.is_err(),
            "non-VT100 file should be rejected, got: {:?}",
            result
        );
    }

    #[test]
    fn convert_vt100_to_v3_file_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.prec");
        let dst = dir.path().join("dst.prec");

        let frames = vec![
            make_test_vt_frame("chrome.exe    code.exe", 0, 30),
            make_test_vt_frame("chrome.exe    firefox.exe", 1000, 30),
        ];
        write_vt100_file(&src, &frames).unwrap();

        let stats = convert_vt100_to_v3_file(&src, &dst).expect("transcode failed");
        assert_eq!(stats.frame_count, 2);
        // unique: chrome / code / firefox = 3
        assert_eq!(stats.unique_process_count, 3);

        // 用 v3 Player 读出验证
        let player = crate::record::Player::open(dst.clone()).unwrap();
        assert_eq!(player.total_frames(), 2);
        let frame0 = player.frame_at(0).unwrap();
        assert_eq!(frame0.mode, "VT100");
        let names: Vec<String> = frame0
            .processes
            .iter()
            .map(|p| p.name.to_lowercase())
            .collect();
        assert!(names.contains(&"chrome.exe".to_string()));
        assert!(names.contains(&"code.exe".to_string()));
    }

    #[test]
    fn convert_vt100_to_v3_file_rejects_missing_source() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("nonexistent.prec");
        let dst = dir.path().join("dst.prec");
        let result = convert_vt100_to_v3_file(&src, &dst);
        assert!(result.is_err());
    }

    #[test]
    fn transcoded_temp_file_cleanup_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tmp.prec");
        std::fs::write(&path, b"test").unwrap();
        assert!(path.exists());

        {
            let _tmp = TranscodedTempFile::new(path.clone());
        } // Drop here
        assert!(!path.exists(), "file should be cleaned up after drop");
    }

    #[test]
    fn transcoded_temp_file_keep_opt_out() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tmp.prec");
        std::fs::write(&path, b"test").unwrap();
        assert!(path.exists());

        {
            let tmp = TranscodedTempFile::new(path.clone());
            tmp.keep();
        } // Drop here but keep=true
        assert!(path.exists(), "file should be retained after keep()");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn transcoded_temp_file_path_accessor() {
        let path = PathBuf::from("/tmp/test.prec");
        let tmp = TranscodedTempFile::new(path.clone());
        assert_eq!(tmp.path(), &path);
        tmp.keep(); // disown so Drop doesn't try to remove nonexistent path
    }
}
