//! `.prec.idx` sidecar 缓存：让 v1/v2 老文件（无 footer）也能享受 v3 按需加载。
//!
//! 老文件首次 open 时 reader 走 fallback 全量加载路径，加载完构造等价
//! [`RecordingFooter`]（含 `frame_offsets`）写出到 `recording.prec.idx`。
//! 后续 open 时如果 sidecar 存在 + 新鲜（mtime + size 匹配），直接走快路径。
//!
//! 设计原则：
//! - **v1/v2 老文件永不重写**（只读，sidecar 单独管理）
//! - **sidecar 损坏静默降级**（fallback 到全量加载，重新生成 sidecar）
//! - **sidecar 格式不兼容未来 footer 演进**靠 `version` 字段 bump（老 sidecar 失效重生成）

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::frame::{RecordingFooter, RecordingHeader};

const SIDECAR_MAGIC: [u8; 8] = *b"PRECIDX\x01";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdxSidecar {
    pub magic: [u8; 8],
    pub version: u16,
    pub source_path: String,
    pub source_size: u64,
    pub source_mtime: u64,
    pub header: RecordingHeader,
    pub footer: RecordingFooter,
}

impl IdxSidecar {
    /// `recording.prec` → `recording.prec.idx`。
    #[must_use]
    pub fn sidecar_path(prec_path: &Path) -> PathBuf {
        let mut s = prec_path.as_os_str().to_os_string();
        s.push(".idx");
        PathBuf::from(s)
    }

    /// 尝试加载 sidecar：文件不存在 / 损坏 / 过期 都返 `None`（让上层 fallback）。
    pub fn try_load(prec_path: &Path) -> Option<Self> {
        let sidecar_path = Self::sidecar_path(prec_path);
        let bytes = std::fs::read(&sidecar_path).ok()?;
        let sidecar: IdxSidecar = bincode::deserialize(&bytes).ok()?;
        if sidecar.magic != SIDECAR_MAGIC {
            return None;
        }
        // 新鲜性检查：source_size + source_mtime 必须匹配当前 prec 文件
        let meta = std::fs::metadata(prec_path).ok()?;
        let size = meta.len();
        let mtime = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        if sidecar.source_size != size || sidecar.source_mtime != mtime {
            return None;
        }
        Some(sidecar)
    }

    /// 把 sidecar 写到 `<prec_path>.idx`。失败静默（用户目录只读不致命）。
    pub fn write(&self, prec_path: &Path) {
        let sidecar_path = Self::sidecar_path(prec_path);
        let Ok(bytes) = bincode::serialize(self) else {
            return;
        };
        let _ = std::fs::write(sidecar_path, bytes);
    }

    /// 构造 sidecar（用于 v1/v2 老文件 fallback 路径完成后写盘）。
    #[must_use]
    pub fn from_legacy(prec_path: &Path, header: RecordingHeader, footer: RecordingFooter) -> Self {
        let (source_size, source_mtime) = std::fs::metadata(prec_path)
            .map(|m| {
                let size = m.len();
                let mtime = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                (size, mtime)
            })
            .unwrap_or((0, 0));
        Self {
            magic: SIDECAR_MAGIC,
            version: 1,
            source_path: prec_path.to_string_lossy().into_owned(),
            source_size,
            source_mtime,
            header,
            footer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_footer() -> RecordingFooter {
        RecordingFooter {
            version: 1,
            header_version: 2,
            start_time: 1000,
            end_time: 2000,
            frame_count: 10,
            anomaly_count: 3,
            event_count: 5,
            max_cpu: 99.0,
            max_mem: 4096,
            frame_offsets: (0..10).map(|i| 100 + i as u64 * 200).collect(),
        }
    }

    fn dummy_header() -> RecordingHeader {
        RecordingHeader {
            magic: *b"PREC",
            version: 2,
            start_time: 1000,
            hostname: "test".to_string(),
        }
    }

    #[test]
    fn sidecar_path_appends_idx_suffix() {
        let p = Path::new("/tmp/recording.prec");
        assert_eq!(
            IdxSidecar::sidecar_path(p),
            Path::new("/tmp/recording.prec.idx")
        );
    }

    #[test]
    fn try_load_missing_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let prec = dir.path().join("none.prec");
        std::fs::write(&prec, b"x").unwrap();
        assert!(IdxSidecar::try_load(&prec).is_none());
    }

    #[test]
    fn sidecar_roundtrip_preserves_footer() {
        let dir = tempfile::tempdir().unwrap();
        let prec = dir.path().join("rec.prec");
        std::fs::write(&prec, b"hello").unwrap();
        let original = IdxSidecar::from_legacy(&prec, dummy_header(), dummy_footer());
        original.write(&prec);
        let loaded = IdxSidecar::try_load(&prec).expect("sidecar should load");
        assert_eq!(loaded.footer.frame_count, 10);
        assert_eq!(loaded.footer.anomaly_count, 3);
        assert_eq!(loaded.footer.frame_offsets.len(), 10);
        assert_eq!(loaded.header.hostname, "test");
    }

    #[test]
    fn try_load_corrupt_sidecar_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let prec = dir.path().join("rec.prec");
        std::fs::write(&prec, b"hello").unwrap();
        let sidecar_path = IdxSidecar::sidecar_path(&prec);
        std::fs::write(&sidecar_path, b"garbage bytes not bincode").unwrap();
        assert!(IdxSidecar::try_load(&prec).is_none());
    }

    #[test]
    fn try_load_wrong_magic_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let prec = dir.path().join("rec.prec");
        std::fs::write(&prec, b"hello").unwrap();
        // 构造一个 magic 错误的 sidecar
        let mut s = IdxSidecar::from_legacy(&prec, dummy_header(), dummy_footer());
        s.magic = *b"BADMAGIC";
        let bytes = bincode::serialize(&s).unwrap();
        let sidecar_path = IdxSidecar::sidecar_path(&prec);
        std::fs::write(&sidecar_path, bytes).unwrap();
        assert!(IdxSidecar::try_load(&prec).is_none());
    }

    #[test]
    fn try_load_stale_sidecar_returns_none_when_source_size_differs() {
        let dir = tempfile::tempdir().unwrap();
        let prec = dir.path().join("rec.prec");
        std::fs::write(&prec, b"hello").unwrap();
        let original = IdxSidecar::from_legacy(&prec, dummy_header(), dummy_footer());
        original.write(&prec);
        // 改源文件大小
        std::fs::write(&prec, b"hello world!!!!").unwrap();
        assert!(IdxSidecar::try_load(&prec).is_none());
    }
}
