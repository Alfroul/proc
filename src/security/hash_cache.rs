use std::collections::HashMap;
use std::io::{BufReader, Read};

use sha2::{Digest, Sha256};

use super::score::{RiskCategory, RiskFactor};
use super::signature::SignatureStatus;

const CACHE_FILE_NAME: &str = "sig_cache.json";
const MAX_ENTRIES: usize = 2000;

/// Hard cap on how many bytes of a file we'll hash. Past this point we stop
/// reading and finalize the digest over what we have. Keeps latency bounded
/// on multi-GB installers and (more importantly) means a hostile file can't
/// make us OOM by being arbitrarily large.
const MAX_HASH_BYTES: u64 = 64 * 1024 * 1024;

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
    #[must_use]
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

    /// Compute SHA-256 of file contents via streaming reader. Caps reading at
    /// `MAX_HASH_BYTES` (64 MB) so a hostile file can't trigger OOM and so
    /// multi-GB installers stay bounded. Returns None on any I/O error.
    fn hash_file(exe_path: &str) -> Option<String> {
        let file = std::fs::File::open(exe_path).ok()?;
        let mut reader = BufReader::new(file);

        let mut hasher = Sha256::new();
        // 1 MB chunks — small enough to keep peak memory flat regardless of
        // input size, large enough that read syscall overhead is negligible.
        let mut buf = vec![0u8; 1024 * 1024];
        let mut consumed: u64 = 0;

        loop {
            if consumed >= MAX_HASH_BYTES {
                break;
            }
            let remaining = MAX_HASH_BYTES - consumed;
            let to_read = std::cmp::min(buf.len() as u64, remaining) as usize;
            if to_read == 0 {
                break;
            }
            let n = match reader.read(&mut buf[..to_read]) {
                Ok(0) => break, // EOF
                Ok(n) => n,
                Err(_) => return None,
            };
            hasher.update(&buf[..n]);
            consumed += n as u64;
        }

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

    #[must_use]
    pub fn is_whitelisted(exe_path: &str) -> bool {
        Self::is_system_path(exe_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Streaming hash completes on a file larger than `MAX_HASH_BYTES`.
    /// Before P1.23 this would have called `std::fs::read` on the whole file
    /// (100 MB → 100 MB resident). The streaming version reads in 1 MB chunks
    /// and stops at the cap.
    #[test]
    fn hash_file_handles_oversized_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");

        // Write a file bigger than MAX_HASH_BYTES (64 MB → write 80 MB).
        // Use a repeating pattern so the bytes are non-zero; otherwise the
        // OS may give us a sparse file that doesn't really exercise the read path.
        let chunk = vec![0xA5u8; 4 * 1024 * 1024]; // 4 MB
        let mut f = std::fs::File::create(&path).unwrap();
        for _ in 0..20 {
            f.write_all(&chunk).unwrap();
        }
        f.flush().unwrap();
        drop(f);

        // Must not panic / OOM / hang.
        let hash = HashReputation::hash_file(path.to_str().unwrap());
        assert!(hash.is_some(), "hash_file on 80 MB file must succeed");
        let hash = hash.unwrap();
        assert_eq!(hash.len(), 64, "SHA-256 hex length");
    }

    /// Files with identical first `MAX_HASH_BYTES` bytes hash to the same digest.
    /// This is the observable contract the cap introduces — and the test
    /// fails fast if someone bumps MAX_HASH_BYTES without also reconsidering
    /// collision risk.
    #[test]
    fn hash_file_caps_at_max_bytes() {
        let dir = tempfile::tempdir().unwrap();

        // Build two files whose first MAX_HASH_BYTES are identical; one is
        // exactly MAX_HASH_BYTES, the other is MAX_HASH_BYTES + extra tail.
        let cap = MAX_HASH_BYTES as usize;
        let pattern_chunk = vec![0x5Au8; 4 * 1024 * 1024];

        let capped_path = dir.path().join("capped.bin");
        let tail_path = dir.path().join("tail.bin");

        {
            let mut a = std::fs::File::create(&capped_path).unwrap();
            let mut written = 0usize;
            while written < cap {
                let take = std::cmp::min(pattern_chunk.len(), cap - written);
                a.write_all(&pattern_chunk[..take]).unwrap();
                written += take;
            }
            a.flush().unwrap();

            let mut b = std::fs::File::create(&tail_path).unwrap();
            written = 0;
            while written < cap {
                let take = std::cmp::min(pattern_chunk.len(), cap - written);
                b.write_all(&pattern_chunk[..take]).unwrap();
                written += take;
            }
            // Append a different tail beyond the cap — it should be ignored.
            b.write_all(&[0xFFu8; 1024]).unwrap();
            b.flush().unwrap();
        }

        let h1 = HashReputation::hash_file(capped_path.to_str().unwrap()).unwrap();
        let h2 = HashReputation::hash_file(tail_path.to_str().unwrap()).unwrap();
        assert_eq!(h1, h2, "bytes beyond MAX_HASH_BYTES must not affect digest");
    }
}
