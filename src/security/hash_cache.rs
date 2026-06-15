use std::collections::HashMap;

use sha2::{Digest, Sha256};

use super::score::{RiskCategory, RiskFactor};
use super::signature::SignatureStatus;

const CACHE_FILE_NAME: &str = "sig_cache.json";
const MAX_ENTRIES: usize = 2000;

/// Content-hash-based signature status cache entry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HashEntry {
    pub sig_status: Option<SignatureStatus>,
    pub first_seen_epoch: u64,
    /// SHA-256 hex of the file content when cached.
    pub content_hash: String,
    /// Paths that resolved to this hash (for diagnostics / eviction).
    pub paths: Vec<String>,
}

/// Lightweight local file reputation cache.
/// Uses file-content SHA-256 as primary key, with a path→hash secondary index
/// for fast lookup. Persisted to disk, survives restarts.
pub struct HashReputation {
    /// SHA-256 hex → cache entry
    verified: HashMap<String, HashEntry>,
    /// exe path → SHA-256 hex (secondary index)
    path_index: HashMap<String, String>,
    dirty: bool,
}

impl Default for HashReputation {
    fn default() -> Self {
        Self::new()
    }
}

impl HashReputation {
    pub fn new() -> Self {
        let mut rep = Self {
            verified: HashMap::new(),
            path_index: HashMap::new(),
            dirty: false,
        };
        rep.load_from_file();
        rep
    }

    fn cache_path() -> Option<std::path::PathBuf> {
        let base = std::env::var("APPDATA")
            .or_else(|_| std::env::var("LOCALAPPDATA"))
            .ok()?;
        let dir = std::path::PathBuf::from(base).join("proc");
        Some(dir.join(CACHE_FILE_NAME))
    }

    fn load_from_file(&mut self) {
        let path = match Self::cache_path() {
            Some(p) => p,
            None => return,
        };
        if !path.exists() {
            return;
        }
        if let Ok(data) = std::fs::read_to_string(&path)
            && let Ok(map) = serde_json::from_str::<HashMap<String, HashEntry>>(&data)
        {
            // Rebuild path index from loaded entries
            for (hash, entry) in &map {
                for p in &entry.paths {
                    self.path_index.insert(p.clone(), hash.clone());
                }
            }
            self.verified = map;
        }
    }

    fn save_to_file(&self) {
        let path = match Self::cache_path() {
            Some(p) => p,
            None => return,
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string(&self.verified) {
            let _ = std::fs::write(&path, json);
        }
    }

    fn is_system_path(exe_path: &str) -> bool {
        let lower = exe_path.to_lowercase();
        lower.starts_with("c:\\windows\\system32\\")
            || lower.starts_with("c:\\windows\\syswow64\\")
            || lower.starts_with("c:\\windows\\winsxs\\")
    }

    /// Compute SHA-256 of file contents. Returns None on any I/O error.
    fn hash_file(exe_path: &str) -> Option<String> {
        let data = std::fs::read(exe_path).ok()?;
        // Large files (>128 MB): hash only first 64 MB to keep latency bounded
        let slice = if data.len() > 128 * 1024 * 1024 {
            &data[..64 * 1024 * 1024]
        } else {
            &data
        };
        let mut hasher = Sha256::new();
        hasher.update(slice);
        Some(format!("{:x}", hasher.finalize()))
    }

    pub fn check_hash(&mut self, exe_path: &str) -> Option<RiskFactor> {
        if Self::is_system_path(exe_path) {
            return None;
        }

        let hash = match self.path_index.get(exe_path) {
            Some(h) => h.clone(),
            None => return None,
        };
        let entry = self.verified.get(&hash)?;

        match entry.sig_status {
            Some(SignatureStatus::Trusted) | Some(SignatureStatus::Signed) => None,
            Some(SignatureStatus::Unsigned) => Some(RiskFactor {
                category: RiskCategory::Signature,
                name: "hash_known_unsigned".to_string(),
                weight: 15,
                description: "已知无签名程序".to_string(),
            }),
            _ => None,
        }
    }

    pub fn record(&mut self, exe_path: &str, sig_status: SignatureStatus) {
        let content_hash = match Self::hash_file(exe_path) {
            Some(h) => h,
            None => return, // Can't read file — skip caching
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        if let Some(existing) = self.verified.get_mut(&content_hash) {
            // Same content seen before — add path if new
            if !existing
                .paths
                .iter()
                .any(|p| p.eq_ignore_ascii_case(exe_path))
            {
                existing.paths.push(exe_path.to_string());
            }
            existing.sig_status = Some(sig_status);
        } else {
            self.verified.insert(
                content_hash.clone(),
                HashEntry {
                    sig_status: Some(sig_status),
                    first_seen_epoch: now,
                    content_hash: content_hash.clone(),
                    paths: vec![exe_path.to_string()],
                },
            );
        }
        self.path_index.insert(exe_path.to_string(), content_hash);
        self.dirty = true;

        // Evict oldest entries if over limit
        if self.verified.len() > MAX_ENTRIES {
            let mut entries: Vec<_> = self.verified.iter().collect();
            entries.sort_by_key(|(_, e)| e.first_seen_epoch);
            let to_remove: Vec<String> = entries
                .iter()
                .take(self.verified.len() - MAX_ENTRIES)
                .map(|(k, _)| (*k).clone())
                .collect();
            for hash_key in &to_remove {
                if let Some(entry) = self.verified.get(hash_key) {
                    for p in &entry.paths {
                        self.path_index.remove(p);
                    }
                }
                self.verified.remove(hash_key);
            }
        }
    }

    /// Return cached signature if the file content hasn't changed since caching.
    pub fn get_cached_sig(&mut self, exe_path: &str) -> Option<SignatureStatus> {
        let old_hash = self.path_index.get(exe_path)?.clone();
        let current_hash = Self::hash_file(exe_path)?;

        if current_hash != old_hash {
            // File content changed — invalidate
            self.invalidate_path(exe_path, &old_hash);
            return None;
        }

        self.verified.get(&current_hash).and_then(|e| e.sig_status)
    }

    fn invalidate_path(&mut self, exe_path: &str, old_hash: &str) {
        if let Some(entry) = self.verified.get_mut(old_hash) {
            entry.paths.retain(|p| !p.eq_ignore_ascii_case(exe_path));
            if entry.paths.is_empty() {
                self.verified.remove(old_hash);
            }
        }
        self.path_index.remove(exe_path);
        self.dirty = true;
    }

    /// Persist dirty cache to disk. Call periodically (e.g. at end of each scoring pass).
    pub fn flush(&mut self) {
        if self.dirty {
            self.save_to_file();
            self.dirty = false;
        }
    }

    pub fn is_whitelisted(exe_path: &str) -> bool {
        Self::is_system_path(exe_path)
    }
}
