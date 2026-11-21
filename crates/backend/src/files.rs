//! Object store qua OpenDAL: STORAGE_DRIVER=s3 (RustFS) hoặc fs (dev/test).
//! Key = "<user_uuid>/<path>". Chunk upload stage ở disk local rồi stream lên store.
use std::path::{Path, PathBuf};

use opendal::{ErrorKind, Operator};
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
    #[error("storage: {0}")]
    Backend(opendal::Error),
}

impl From<opendal::Error> for StoreError {
    fn from(e: opendal::Error) -> Self {
        if e.kind() == ErrorKind::NotFound { Self::NotFound } else { Self::Backend(e) }
    }
}

#[derive(Clone)]
pub struct Store {
    pub op: Operator,
    /// Local dir cho chunk đang upload (nhanh, append được — S3 thì không).
    pub staging: PathBuf,
}

/// Chuẩn hoá path user gửi: bỏ "", ".", chặn ".." và path tuyệt đối.
pub fn clean_path(req: &str) -> Result<String, StoreError> {
    if req.starts_with('/') || req.contains('\\') {
        return Err(StoreError::UnsafePath);
    }
    let mut parts = vec![];
    for comp in req.split('/') {
        match comp {
            "" | "." => continue,
            ".." => return Err(StoreError::UnsafePath),
            c => parts.push(c),
        }
    }
    Ok(parts.join("/"))
}

/// Giây hoặc RFC3339 (rclone lưu mtime gốc vào user metadata "mtime").
pub fn parse_mtime(s: &str) -> Option<i64> {
    if let Ok(f) = s.trim().parse::<f64>() {
        return Some(f as i64);
    }
    chrono::DateTime::parse_from_rfc3339(s.trim()).ok().map(|d| d.timestamp())
}

/// mtime gốc nếu có, không thì thời điểm ghi lên store.
pub fn mtime_of(m: &opendal::Metadata) -> i64 {
    m.user_metadata()
        .and_then(|u| u.get("mtime").and_then(parse_mtime))
        .unwrap_or_else(|| m.last_modified().map_or(0, |t| t.into_inner().as_second()))
}

impl Store {
    pub fn s3(endpoint: &str, bucket: &str, key: &str, secret: &str, staging: PathBuf) -> Result<Self, StoreError> {
        opendal::install_default(); // HTTP transport (idempotent)
        let b = opendal::services::S3::default()
            .endpoint(endpoint)
            .bucket(bucket)
            .region("us-east-1")
            .access_key_id(key)
            .secret_access_key(secret)
            .disable_config_load()
            .disable_ec2_metadata();
        Ok(Self { op: Operator::new(b)?, staging })
    }

    pub fn fs(root: &Path, staging: PathBuf) -> Result<Self, StoreError> {
        let b = opendal::services::Fs::default().root(&root.to_string_lossy());
        Ok(Self { op: Operator::new(b)?, staging })
    }

    /// Object key cho 1 file của user. Path rỗng bị từ chối.
    pub fn key(&self, user: Uuid, req: &str) -> Result<String, StoreError> {
        let p = clean_path(req)?;
        if p.is_empty() {
            return Err(StoreError::UnsafePath);
        }
        Ok(format!("{user}/{p}"))
    }

    /// Toàn bộ file của user (đệ quy): (path tương đối, size, mtime). Dùng để dựng lại chỉ mục.
    pub async fn scan(&self, user: Uuid) -> Result<Vec<(String, u64, i64)>, StoreError> {
        use futures::{StreamExt, TryStreamExt};
        let root = format!("{user}/");
        let keys: Vec<String> = match self.op.lister_with(&root).recursive(true).await {
            Ok(l) => l
                .try_filter(|e| futures::future::ready(!e.metadata().is_dir()))
                .map_ok(|e| e.path().to_string())
                .try_collect()
                .await?,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e.into()),
        };
        // List S3 không kèm user metadata (mtime gốc) -> HEAD từng object, song song.
        // Bỏ qua cache thumbnail (.nextkloud/thumbs/): file hệ thống, không tính quota.
        // Mọi thứ khác dưới .nextkloud (avatar, cài đặt, thùng rác) vẫn quét bình thường.
        let skip = root.len();
        futures::stream::iter(keys)
            .filter(|k: &String| {
                let rel = k.as_str().get(skip..).unwrap_or("");
                futures::future::ready(!rel.starts_with(".nextkloud/thumbs/"))
            })
            .map(|k| async move {
                let m = self.op.stat(&k).await?;
                Ok((k[skip..].to_string(), m.content_length(), mtime_of(&m)))
            })
            .buffer_unordered(32)
            .try_collect()
            .await
    }

    pub async fn stat(&self, user: Uuid, path: &str) -> Result<opendal::Metadata, StoreError> {
        Ok(self.op.stat(&self.key(user, path)?).await?)
    }

    pub fn stage_path(&self, user: Uuid, id: &str) -> PathBuf {
        self.staging.join(user.to_string()).join(format!("{id}.part"))
    }

    pub fn write_chunk(&self, user: Uuid, id: &str, offset: u64, data: &[u8]) -> Result<u64, StoreError> {
        let tmp = self.stage_path(user, id);
        std::fs::create_dir_all(tmp.parent().unwrap())?;
        let cur = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
        if cur != offset {
            return Err(StoreError::Conflict(format!("offset mismatch: have {cur}, got {offset}")));
        }
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&tmp)?;
        f.write_all(data)?;
        Ok(cur + data.len() as u64)
    }

    /// Finish an upload: size must match, quota charged, staged file streamed to store.
    /// Trả về mtime đã lưu (mtime client gửi, mặc định là bây giờ).
    pub async fn complete(
        &self,
        user: Uuid,
        id: &str,
        path: &str,
        size: u64,
        mtime: Option<i64>,
        quota: &mut Quota,
    ) -> Result<i64, StoreError> {
        let tmp = self.stage_path(user, id);
        let staged = std::fs::metadata(&tmp).map(|m| m.len()).map_err(|_| StoreError::NotFound)?;
        if staged != size {
            return Err(StoreError::Conflict(format!("incomplete: staged {staged}, want {size}")));
        }
        let mtime = self.put_local(user, path, &tmp, mtime, quota).await?;
        let _ = tokio::fs::remove_file(&tmp).await;
        Ok(mtime)
    }

    /// Ghi 1 file từ đĩa local lên store (ghi đè được), tính quota. Trả về mtime đã lưu.
    pub async fn put_local(&self, user: Uuid, path: &str, src: &Path, mtime: Option<i64>, quota: &mut Quota) -> Result<i64, StoreError> {
        let size = tokio::fs::metadata(src).await?.len();
        let key = self.key(user, path)?;
        // Overwrite = delete+add: tính trên bản sao, chỉ commit khi ghi xong.
        let mut q = *quota;
        if let Ok(old) = self.op.stat(&key).await {
            q.release(old.content_length());
        }
        q.add(size)?;
        // Multipart 16MB x4 song song: file trăm GB không nạp vào RAM.
        let mtime = mtime.unwrap_or_else(|| chrono::Utc::now().timestamp());
        let mut w = self.op.writer_with(&key).chunk(16 * 1024 * 1024).concurrent(4);
        if self.op.info().capability().write_with_user_metadata {
            w = w.user_metadata([("mtime".to_string(), mtime.to_string())]);
        }
        let mut w = w.await?;
        let mut f = tokio::fs::File::open(src).await?;
        let mut buf = vec![0u8; (size as usize).clamp(1, 8 * 1024 * 1024)]; // file nhỏ (vault md) khỏi cấp 8MB
        use tokio::io::AsyncReadExt;
        loop {
            let n = f.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            w.write(buf[..n].to_vec()).await?;
        }
        w.close().await?;
        *quota = q;
        Ok(mtime)
    }

    /// Delete a file, refunding quota. Directories are refused (M1).
    pub async fn delete(&self, user: Uuid, path: &str, quota: &mut Quota) -> Result<u64, StoreError> {
        let key = self.key(user, path)?;
        let meta = self.op.stat(&key).await?;
        if meta.is_dir() {
            return Err(StoreError::Conflict("refusing to delete non-file".into()));
        }
        let size = meta.content_length();
        self.op.delete(&key).await?;
        quota.release(size);
        Ok(size)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn tmp_store(tag: &str) -> (Store, Uuid) {
        let root = std::env::temp_dir().join(format!("nk-fs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        (Store::fs(&root.join("data"), root.join("staging")).unwrap(), Uuid::new_v4())
    }

    /// Chạy thêm với RustFS thật: NK_TEST_S3=endpoint,bucket,key,secret cargo test
    pub fn s3_store() -> Option<(Store, Uuid)> {
        let v = std::env::var("NK_TEST_S3").ok()?;
        let p: Vec<&str> = v.split(',').collect();
        let staging = std::env::temp_dir().join(format!("nk-s3-stage-{}", std::process::id()));
        Some((Store::s3(p[0], p[1], p[2], p[3], staging).unwrap(), Uuid::new_v4()))
    }

    #[test]
    fn mtime_formats() {
        assert_eq!(parse_mtime("1728372187.123456789"), Some(1728372187));
        assert_eq!(parse_mtime("2024-10-08T07:23:07Z"), Some(1728372187));
        assert_eq!(parse_mtime("garbage"), None);
    }

    #[test]
    fn traversal_rejected() {
        let (s, u) = tmp_store("trav");
        assert!(s.key(u, "../x").is_err());
        assert!(s.key(u, "a/../../b").is_err());
        assert!(s.key(u, "/abs").is_err());
        assert!(s.key(u, "").is_err());
        assert!(s.key(u, "a\\..\\b").is_err());
        assert_eq!(s.key(u, "a//./b/c.txt").unwrap(), format!("{u}/a/b/c.txt"));
    }

    async fn upload_roundtrip(s: Store, u: Uuid) {
        let mut q = Quota::new(100, 0);
        s.write_chunk(u, "id1", 0, b"hello ").unwrap();
        assert!(s.write_chunk(u, "id1", 0, b"nope").is_err()); // must be 6
        s.write_chunk(u, "id1", 6, b"world").unwrap();
        assert!(s.complete(u, "id1", "a/b.txt", 12, None, &mut q).await.is_err()); // staged 11 vs 12
        s.complete(u, "id1", "a/b.txt", 11, Some(1_500_000_000), &mut q).await.unwrap();
        assert_eq!(q.used_bytes, 11);
        assert!(!s.stage_path(u, "id1").exists()); // staging dọn sạch

        let all = s.scan(u).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!((all[0].0.as_str(), all[0].1), ("a/b.txt", 11));
        if s.op.info().capability().write_with_user_metadata {
            assert_eq!(all[0].2, 1_500_000_000); // mtime gốc giữ qua user metadata
        } else {
            assert!(all[0].2 > 0);
        }
        let body = s.op.read(&s.key(u, "a/b.txt").unwrap()).await.unwrap().to_vec();
        assert_eq!(body, b"hello world");

        // overwrite: quota tính lại theo size mới
        s.write_chunk(u, "id2", 0, b"hey").unwrap();
        s.complete(u, "id2", "a/b.txt", 3, None, &mut q).await.unwrap();
        assert_eq!(q.used_bytes, 3);

        let freed = s.delete(u, "a/b.txt", &mut q).await.unwrap();
        assert_eq!(freed, 3);
        assert_eq!(q.used_bytes, 0);
        assert!(s.delete(u, "nope.txt", &mut q).await.is_err());
        assert!(s.scan(Uuid::new_v4()).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn fs_upload_list_delete() {
        let (s, u) = tmp_store("up");
        upload_roundtrip(s, u).await;
    }

    #[tokio::test]
    async fn s3_upload_list_delete() {
        let Some((s, u)) = s3_store() else { return };
        upload_roundtrip(s, u).await;
    }

    #[tokio::test]
    async fn quota_blocks_oversize_complete() {
        let (s, u) = tmp_store("q");
        let mut q = Quota::new(10, 0);
        s.write_chunk(u, "id9", 0, b"0123456789ABCDEF").unwrap();
        assert!(s.complete(u, "id9", "big.bin", 16, None, &mut q).await.is_err());
        assert_eq!(q.used_bytes, 0);
        assert!(s.stat(u, "big.bin").await.is_err()); // không ghi gì
    }

    #[tokio::test]
    async fn overwrite_over_quota_keeps_old() {
        let (s, u) = tmp_store("ow");
        let mut q = Quota::new(10, 0);
        s.write_chunk(u, "a", 0, b"12345").unwrap();
        s.complete(u, "a", "f.bin", 5, None, &mut q).await.unwrap();
        s.write_chunk(u, "b", 0, b"0123456789AB").unwrap();
        assert!(s.complete(u, "b", "f.bin", 12, None, &mut q).await.is_err());
        assert_eq!(q.used_bytes, 5);
        assert_eq!(s.stat(u, "f.bin").await.unwrap().content_length(), 5);
    }
}
