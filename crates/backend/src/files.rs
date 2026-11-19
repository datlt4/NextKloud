//! Filesystem object store (STORAGE_DRIVER=fs). Pure sync logic, unit-tested
//! with temp dirs — no DB needed. S3 driver (M1b) implements the same shape.
use std::path::{Component, PathBuf};
use uuid::Uuid;

use crate::quota::{Quota, QuotaError};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("unsafe path")]
    UnsafePath,
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("quota: {0}")]
    Quota(#[from] QuotaError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct FsStore {
    pub root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    /// Unix seconds.
    pub modified: i64,
}

impl FsStore {
    pub fn user_root(&self, user: Uuid) -> PathBuf {
        self.root.join(user.to_string())
    }

    /// Join a user-supplied relative path. Rejects absolute paths and `..`.
    pub fn safe_join(&self, user: Uuid, req: &str) -> Result<PathBuf, StoreError> {
        if req.starts_with('/') {
            return Err(StoreError::UnsafePath);
        }
        let mut out = self.user_root(user);
        let mut any = false;
        for comp in req.split('/') {
            if comp.is_empty() || comp == "." {
                continue;
            }
            if comp == ".." {
                return Err(StoreError::UnsafePath);
            }
            out.push(Component::Normal(comp.as_ref()));
            any = true;
        }
        if !any {
            return Err(StoreError::UnsafePath);
        }
        Ok(out)
    }

    pub fn list(&self, user: Uuid, prefix: &str) -> Result<Vec<Entry>, StoreError> {
        let dir = if prefix.is_empty() { self.user_root(user) } else { self.safe_join(user, prefix)? };
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return Ok(vec![]);
        };
        let mut out = vec![];
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && name != "." {
                // skip staging dirs
                if name == ".staging" {
                    continue;
                }
            }
            let ft = e.file_type()?;
            let meta = e.metadata()?;
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs() as i64);
            out.push(Entry {
                name,
                is_dir: ft.is_dir(),
                size: if ft.is_file() { meta.len() } else { 0 },
                modified,
            });
        }
        out.sort_by(|a, b| (b.is_dir, &a.name).cmp(&(a.is_dir, &b.name)));
        Ok(out)
    }

    pub fn stage_path(&self, user: Uuid, id: &str) -> PathBuf {
        self.user_root(user).join(".staging").join(format!("{id}.part"))
    }

    pub fn write_chunk(&self, user: Uuid, id: &str, offset: u64, data: &[u8]) -> Result<u64, StoreError> {
        let staging = self.user_root(user).join(".staging");
        std::fs::create_dir_all(&staging)?;
        let tmp = staging.join(format!("{id}.part"));
        let cur = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
        if cur != offset {
            return Err(StoreError::Conflict(format!("offset mismatch: have {cur}, got {offset}")));
        }
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&tmp)?;
        f.write_all(data)?;
        Ok(cur + data.len() as u64)
    }

    /// Finish an upload: size must match, quota charged, file moved into place.
    pub fn complete(&self, user: Uuid, id: &str, path: &str, size: u64, quota: &mut Quota) -> Result<(), StoreError> {
        let tmp = self.stage_path(user, id);
        let staged = std::fs::metadata(&tmp).map(|m| m.len()).map_err(|_| StoreError::NotFound)?;
        if staged != size {
            return Err(StoreError::Conflict(format!("incomplete: staged {staged}, want {size}")));
        }
        quota.add(size)?;
        let dest = self.safe_join(user, path)?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Overwrite counts as delete+add: release old size first.
        if let Ok(old) = std::fs::metadata(&dest) {
            quota.release(old.len());
            quota.add(size)?;
        }
        std::fs::rename(&tmp, &dest)?;
        Ok(())
    }

    /// Delete a file, refunding quota. Directories are refused (M1).
    pub fn delete(&self, user: Uuid, path: &str, quota: &mut Quota) -> Result<u64, StoreError> {
        let dest = self.safe_join(user, path)?;
        let meta = std::fs::symlink_metadata(&dest).map_err(|_| StoreError::NotFound)?;
        if !meta.is_file() {
            return Err(StoreError::Conflict("refusing to delete non-file".into()));
        }
        let size = meta.len();
        std::fs::remove_file(&dest)?;
        quota.release(size);
        // Prune newly-empty parents (best effort, never above user root).
        let mut cur = dest.parent().map(PathBuf::from);
        let root = self.user_root(user);
        while let Some(dir) = cur {
            if dir == root || !dir.starts_with(&root) {
                break;
            }
            if std::fs::remove_dir(&dir).is_err() {
                break;
            }
            cur = dir.parent().map(PathBuf::from);
        }
        Ok(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store(tag: &str) -> (FsStore, Uuid) {
        let root = std::env::temp_dir().join(format!("nk-fs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        (FsStore { root }, Uuid::new_v4())
    }

    #[test]
    fn traversal_rejected() {
        let (s, u) = tmp_store("trav");
        assert!(s.safe_join(u, "../x").is_err());
        assert!(s.safe_join(u, "a/../../b").is_err());
        assert!(s.safe_join(u, "/abs").is_err());
        assert!(s.safe_join(u, "").is_err());
        assert!(s.safe_join(u, "a/b/c.txt").is_ok());
    }

    #[test]
    fn chunk_offset_enforced_and_complete_charges_quota() {
        let (s, u) = tmp_store("up");
        let mut q = Quota::new(100, 0);
        s.write_chunk(u, "id1", 0, b"hello ").unwrap();
        assert!(s.write_chunk(u, "id1", 0, b"nope").is_err()); // must be 6
        s.write_chunk(u, "id1", 6, b"world").unwrap();
        assert!(s.complete(u, "id1", "a/b.txt", 12, &mut q).is_err()); // staged 11 vs 12
        s.complete(u, "id1", "a/b.txt", 11, &mut q).unwrap();
        assert_eq!(q.used_bytes, 11);
        let got = s.list(u, "a").unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "b.txt");
        assert!(got[0].modified > 0);
    }

    #[test]
    fn quota_blocks_oversize_complete() {
        let (s, u) = tmp_store("q");
        let mut q = Quota::new(10, 0);
        s.write_chunk(u, "id9", 0, b"0123456789ABCDEF").unwrap();
        assert!(s.complete(u, "id9", "big.bin", 16, &mut q).is_err());
        assert_eq!(q.used_bytes, 0);
    }

    #[test]
    fn delete_refunds_and_prunes() {
        let (s, u) = tmp_store("del");
        let mut q = Quota::new(100, 0);
        s.write_chunk(u, "d1", 0, b"12345").unwrap();
        s.complete(u, "d1", "x/y/f.txt", 5, &mut q).unwrap();
        assert_eq!(q.used_bytes, 5);
        let freed = s.delete(u, "x/y/f.txt", &mut q).unwrap();
        assert_eq!(freed, 5);
        assert_eq!(q.used_bytes, 0);
        assert!(!s.user_root(u).join("x").exists()); // pruned
        assert!(s.delete(u, "nope.txt", &mut q).is_err());
        assert!(s.delete(u, ".", &mut q).is_err());
    }
}
