//! 录屏书签系统：录制时按 `b` 添加书签，回放时按 `B` 打开列表跳转 / 编辑 / 删除。
//!
//! 持久化到 `.prec.bookmarks.json` sidecar（与录屏同名 `.prec.bookmarks.json`），
//! 与录屏本体解耦（不污染录屏文件，可单独分享 / 删除）。
//!
//! 设计原则（同 `.prec.idx` sidecar）：
//! - 录屏文件**永不重写**（书签变动只动 sidecar）
//! - sidecar 损坏静默降级（fallback 到空书签列表）
//! - sidecar schema 演进靠 `version` 字段 bump
//!
//! v0.14 stage 2 落地。详见 `docs/stages/v0.14-stage-2.md`。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const BOOKMARK_MAGIC: [u8; 8] = *b"PRECBMK\x01";

/// 单个书签条目（一帧对应一书签）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    /// 自增 ID（在单个录屏范围内唯一）。
    pub id: u64,
    /// 录屏帧索引（positional，0-based；跳转时调 `player.frame_at(idx)`）。
    pub frame_idx: usize,
    /// 该帧的绝对 unix epoch 秒（用于显示 + 跨录屏排序）。
    pub timestamp_secs: u64,
    /// 用户输入 / 默认生成的 label（如「书签 #N」）。
    pub label: String,
    /// 书签创建时间（unix epoch 秒）。
    pub created_at: u64,
}

/// 书签 sidecar 文件（与录屏同名 `.prec.bookmarks.json`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkFile {
    pub magic: [u8; 8],
    pub version: u16,
    /// 关联录屏路径（debug 用，不参与新鲜性校验）。
    pub source_path: String,
    /// 录屏文件大小（字节）— 新鲜性校验用。
    pub source_size: u64,
    /// 录屏文件 mtime（unix epoch 秒）— 新鲜性校验用。
    pub source_mtime: u64,
    /// 全部书签（建议按 frame_idx 升序；`add` 不维持顺序，调 `sort_by_frame`）。
    pub bookmarks: Vec<Bookmark>,
}

impl BookmarkFile {
    /// `recording.prec` → `recording.prec.bookmarks.json`。
    #[must_use]
    pub fn sidecar_path(prec_path: &Path) -> PathBuf {
        let mut s = prec_path.as_os_str().to_os_string();
        s.push(".bookmarks.json");
        PathBuf::from(s)
    }

    /// 尝试加载 sidecar：文件不存在 / 损坏 / magic 不匹配 / 录屏已变化 都返 `None`。
    /// 调用方 fallback 到空书签列表（不致命）。
    pub fn try_load(prec_path: &Path) -> Option<Self> {
        let sidecar_path = Self::sidecar_path(prec_path);
        let text = std::fs::read_to_string(&sidecar_path).ok()?;
        let file: BookmarkFile = serde_json::from_str(&text).ok()?;
        if file.magic != BOOKMARK_MAGIC {
            return None;
        }
        // 新鲜性：录屏文件 size + mtime 必须匹配
        let meta = std::fs::metadata(prec_path).ok()?;
        let size = meta.len();
        let mtime = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        if file.source_size != size || file.source_mtime != mtime {
            return None;
        }
        Some(file)
    }

    /// 写 sidecar（JSON pretty）。失败静默（用户目录只读不致命）。
    pub fn write(&self, prec_path: &Path) {
        let sidecar_path = Self::sidecar_path(prec_path);
        let Ok(text) = serde_json::to_string_pretty(self) else {
            return;
        };
        let _ = std::fs::write(sidecar_path, text);
    }

    /// 构造空 sidecar（首次 open 录屏 / sidecar 不存在时用）。
    #[must_use]
    pub fn empty_for(prec_path: &Path) -> Self {
        let (size, mtime) = std::fs::metadata(prec_path)
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
            magic: BOOKMARK_MAGIC,
            version: 1,
            source_path: prec_path.to_string_lossy().into_owned(),
            source_size: size,
            source_mtime: mtime,
            bookmarks: Vec::new(),
        }
    }

    /// 加 sidecar 如不存在；存在则 try_load；损坏 / 过期返 empty。
    #[must_use]
    pub fn load_or_empty(prec_path: &Path) -> Self {
        Self::try_load(prec_path).unwrap_or_else(|| Self::empty_for(prec_path))
    }

    /// 添加书签（id 自增；frame_idx 不要求单调，调用方决定）。
    /// `now` 是当前 unix epoch 秒（让测试可注入固定时间）。
    pub fn add(
        &mut self,
        frame_idx: usize,
        timestamp_secs: u64,
        label: String,
        now: u64,
    ) -> &Bookmark {
        let next_id = self.bookmarks.iter().map(|b| b.id).max().unwrap_or(0) + 1;
        let bm = Bookmark {
            id: next_id,
            frame_idx,
            timestamp_secs,
            label,
            created_at: now,
        };
        self.bookmarks.push(bm);
        self.bookmarks.last().unwrap()
    }

    /// 按 id 删除书签，返回是否删除成功。
    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.bookmarks.len();
        self.bookmarks.retain(|b| b.id != id);
        self.bookmarks.len() != before
    }

    /// 按 id 编辑书签 label。
    pub fn edit_label(&mut self, id: u64, new_label: String) -> bool {
        if let Some(b) = self.bookmarks.iter_mut().find(|b| b.id == id) {
            b.label = new_label;
            true
        } else {
            false
        }
    }

    /// 按 frame_idx 升序排序（UI 展示用）。
    pub fn sort_by_frame(&mut self) {
        self.bookmarks.sort_by_key(|b| b.frame_idx);
    }
}

/// 书签面板状态（录制 / 回放两侧共用 — VT100 与 UiFrame 路径同款状态机）。
#[derive(Debug, Clone, Default)]
pub struct BookmarkPanelState {
    /// 在过滤后列表中的 cursor（0-based）。
    pub cursor: usize,
    /// 子串过滤（空 = 显示全部）。
    pub search_query: String,
    /// `e` 键激活的 inline 编辑状态：None=未激活 / Some=激活中。
    pub editing_label: Option<String>,
    /// 正在编辑的 bookmark id（与 editing_label 同步设置 / 清空）。
    pub editing_id: Option<u64>,
}

impl BookmarkPanelState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 编辑模式激活。
    pub fn start_edit(&mut self, id: u64, current_label: &str) {
        self.editing_label = Some(current_label.to_string());
        self.editing_id = Some(id);
    }

    /// 编辑模式退出（提交 / 取消都用此）。
    pub fn end_edit(&mut self) -> Option<(u64, String)> {
        let label = self.editing_label.take()?;
        let id = self.editing_id.take()?;
        Some((id, label))
    }

    #[must_use]
    pub fn is_editing(&self) -> bool {
        self.editing_label.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_bookmark(id: u64, frame_idx: usize, label: &str) -> Bookmark {
        Bookmark {
            id,
            frame_idx,
            timestamp_secs: 1000 + frame_idx as u64,
            label: label.to_string(),
            created_at: 5000,
        }
    }

    #[test]
    fn bookmark_serde_roundtrip() {
        let bm = dummy_bookmark(7, 42, "CPU 飙升");
        let json = serde_json::to_string(&bm).unwrap();
        let back: Bookmark = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, 7);
        assert_eq!(back.frame_idx, 42);
        assert_eq!(back.label, "CPU 飙升");
        assert_eq!(back.timestamp_secs, 1042);
    }

    #[test]
    fn bookmark_file_serde_roundtrip() {
        let mut file = BookmarkFile {
            magic: BOOKMARK_MAGIC,
            version: 1,
            source_path: "/tmp/x.prec".to_string(),
            source_size: 100,
            source_mtime: 9_999,
            bookmarks: Vec::new(),
        };
        file.bookmarks.push(dummy_bookmark(1, 5, "a"));
        file.bookmarks.push(dummy_bookmark(2, 10, "b"));
        let json = serde_json::to_string_pretty(&file).unwrap();
        let back: BookmarkFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.magic, BOOKMARK_MAGIC);
        assert_eq!(back.bookmarks.len(), 2);
        assert_eq!(back.bookmarks[0].label, "a");
    }

    #[test]
    fn sidecar_path_appends_bookmarks_json_suffix() {
        let p = Path::new("/tmp/recording.prec");
        assert_eq!(
            BookmarkFile::sidecar_path(p),
            Path::new("/tmp/recording.prec.bookmarks.json")
        );
    }

    #[test]
    fn try_load_missing_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let prec = dir.path().join("none.prec");
        std::fs::write(&prec, b"x").unwrap();
        assert!(BookmarkFile::try_load(&prec).is_none());
    }

    #[test]
    fn try_load_corrupt_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let prec = dir.path().join("rec.prec");
        std::fs::write(&prec, b"hello").unwrap();
        let sidecar = BookmarkFile::sidecar_path(&prec);
        std::fs::write(sidecar, b"not json at all").unwrap();
        assert!(BookmarkFile::try_load(&prec).is_none());
    }

    #[test]
    fn try_load_wrong_magic_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let prec = dir.path().join("rec.prec");
        std::fs::write(&prec, b"hello").unwrap();
        let mut file = BookmarkFile::empty_for(&prec);
        file.magic = *b"BADMAGIC";
        let json = serde_json::to_string(&file).unwrap();
        std::fs::write(BookmarkFile::sidecar_path(&prec), json).unwrap();
        assert!(BookmarkFile::try_load(&prec).is_none());
    }

    #[test]
    fn try_load_stale_returns_none_when_source_size_differs() {
        let dir = tempfile::tempdir().unwrap();
        let prec = dir.path().join("rec.prec");
        std::fs::write(&prec, b"hello").unwrap();
        let file = BookmarkFile::empty_for(&prec);
        file.write(&prec);
        // 改源文件大小
        std::fs::write(&prec, b"hello world!!! longer content").unwrap();
        assert!(BookmarkFile::try_load(&prec).is_none());
    }

    #[test]
    fn try_load_stale_returns_none_when_source_mtime_differs() {
        let dir = tempfile::tempdir().unwrap();
        let prec = dir.path().join("rec.prec");
        std::fs::write(&prec, b"hello").unwrap();
        let file = BookmarkFile::empty_for(&prec);
        file.write(&prec);
        // 改源文件 mtime（保持 size 不变 → file_size 检查过 → mtime 检查 fail）
        let now = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
        // filetime::set_file_mtime 需要 unix; 改用更稳的写法 — 写同长度内容会更新 mtime
        std::fs::write(&prec, b"world").unwrap();
        // 等 1 秒确保 mtime 真的变（Windows 文件系统精度有时是 1s）
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&prec, b"hello").unwrap();
        let _ = now;
        assert!(BookmarkFile::try_load(&prec).is_none());
    }

    #[test]
    fn add_assigns_increasing_ids() {
        let dir = tempfile::tempdir().unwrap();
        let prec = dir.path().join("rec.prec");
        std::fs::write(&prec, b"hello").unwrap();
        let mut file = BookmarkFile::empty_for(&prec);
        file.add(1, 100, "first".to_string(), 1000);
        file.add(2, 200, "second".to_string(), 1001);
        file.add(3, 300, "third".to_string(), 1002);
        assert_eq!(file.bookmarks.len(), 3);
        assert_eq!(file.bookmarks[0].id, 1);
        assert_eq!(file.bookmarks[1].id, 2);
        assert_eq!(file.bookmarks[2].id, 3);
    }

    #[test]
    fn remove_by_id_works() {
        let dir = tempfile::tempdir().unwrap();
        let prec = dir.path().join("rec.prec");
        std::fs::write(&prec, b"hello").unwrap();
        let mut file = BookmarkFile::empty_for(&prec);
        file.add(1, 100, "first".to_string(), 1000);
        file.add(2, 200, "second".to_string(), 1001);
        assert!(file.remove(1));
        assert_eq!(file.bookmarks.len(), 1);
        assert_eq!(file.bookmarks[0].id, 2);
        assert!(!file.remove(999));
    }

    #[test]
    fn edit_label_by_id_works() {
        let dir = tempfile::tempdir().unwrap();
        let prec = dir.path().join("rec.prec");
        std::fs::write(&prec, b"hello").unwrap();
        let mut file = BookmarkFile::empty_for(&prec);
        file.add(1, 100, "old".to_string(), 1000);
        assert!(file.edit_label(1, "new label".to_string()));
        assert_eq!(file.bookmarks[0].label, "new label");
        assert!(!file.edit_label(999, "x".to_string()));
    }

    #[test]
    fn load_or_empty_creates_empty_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let prec = dir.path().join("rec.prec");
        std::fs::write(&prec, b"hello").unwrap();
        let file = BookmarkFile::load_or_empty(&prec);
        assert_eq!(file.bookmarks.len(), 0);
        assert_eq!(file.magic, BOOKMARK_MAGIC);
    }

    #[test]
    fn write_then_load_preserves_bookmarks() {
        let dir = tempfile::tempdir().unwrap();
        let prec = dir.path().join("rec.prec");
        std::fs::write(&prec, b"hello").unwrap();
        let mut file = BookmarkFile::empty_for(&prec);
        file.add(1, 100, "first".to_string(), 1000);
        file.add(2, 200, "second".to_string(), 1001);
        file.write(&prec);
        let loaded = BookmarkFile::try_load(&prec).expect("sidecar should load");
        assert_eq!(loaded.bookmarks.len(), 2);
        assert_eq!(loaded.bookmarks[0].label, "first");
        assert_eq!(loaded.bookmarks[1].label, "second");
    }

    #[test]
    fn sort_by_frame_arranges_ascending() {
        let dir = tempfile::tempdir().unwrap();
        let prec = dir.path().join("rec.prec");
        std::fs::write(&prec, b"hello").unwrap();
        let mut file = BookmarkFile::empty_for(&prec);
        // 故意乱序 add（第一个参数是 frame_idx，第二个是 timestamp_secs）
        file.add(50, 0, "fifty".to_string(), 1000);
        file.add(10, 0, "ten".to_string(), 1001);
        file.add(30, 0, "thirty".to_string(), 1002);
        file.sort_by_frame();
        assert_eq!(file.bookmarks[0].frame_idx, 10);
        assert_eq!(file.bookmarks[1].frame_idx, 30);
        assert_eq!(file.bookmarks[2].frame_idx, 50);
    }

    #[test]
    fn panel_state_start_end_edit_round_trip() {
        let mut panel = BookmarkPanelState::new();
        assert!(!panel.is_editing());
        panel.start_edit(7, "current");
        assert!(panel.is_editing());
        assert_eq!(panel.editing_id, Some(7));
        assert_eq!(panel.editing_label.as_deref(), Some("current"));
        let (id, label) = panel.end_edit().unwrap();
        assert_eq!(id, 7);
        assert_eq!(label, "current");
        assert!(!panel.is_editing());
    }

    #[test]
    fn panel_state_end_edit_when_not_active_returns_none() {
        let mut panel = BookmarkPanelState::new();
        assert!(panel.end_edit().is_none());
    }
}
