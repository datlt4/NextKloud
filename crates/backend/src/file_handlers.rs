//! File HTTP handlers. Auth + quota enforced here, bytes via FsStore.
use std::collections::HashMap;

use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::Response,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    files::Entry,
    quota::Quota,
    state::{AppError, AppState, AuthUser, PendingUpload, load_user},
};

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub prefix: String,
}

#[derive(Serialize)]
pub struct EntryOut {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: i64,
}

pub async fn list_files(
    State(s): State<AppState>,
    user: AuthUser,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<EntryOut>>, AppError> {
    let entries: Vec<Entry> = s.store.list(user.id, &q.prefix).await?;
    Ok(Json(entries.into_iter().map(|e| EntryOut { name: e.name, is_dir: e.is_dir, size: e.size, modified: e.modified }).collect()))
}

/// Parse "bytes=a-b" | "bytes=a-" | "bytes=-n" -> [start, end). None = không hợp lệ/không có.
pub fn parse_range(h: &str, size: u64) -> Option<(u64, u64)> {
    let r = h.strip_prefix("bytes=")?;
    if r.contains(',') || size == 0 {
        return None;
    }
    let (a, b) = r.split_once('-')?;
    let (start, end) = match (a.trim(), b.trim()) {
        ("", n) => {
            let n: u64 = n.parse().ok()?;
            (size.saturating_sub(n), size)
        }
        (a, "") => (a.parse().ok()?, size),
        (a, b) => (a.parse().ok()?, b.parse::<u64>().ok()?.saturating_add(1).min(size)),
    };
    (start < end).then_some((start, end))
}

/// Stream từ store, không nạp cả file vào RAM. Hỗ trợ Range (tua video, tải tiếp).
pub async fn download(
    State(s): State<AppState>,
    user: AuthUser,
    Path(path): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let meta = s.store.stat(user.id, &path).await?;
    if meta.is_dir() {
        return Err(AppError::BadRequest("not a file (use ?prefix= to list)".into()));
    }
    let size = meta.content_length();
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok()).map(|h| parse_range(h, size));
    let (start, end, status) = match range {
        Some(Some((a, b))) => (a, b, StatusCode::PARTIAL_CONTENT),
        Some(None) => {
            return Ok(Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{size}"))
                .body(Body::empty())
                .unwrap());
        }
        None => (0, size, StatusCode::OK),
    };
    let key = s.store.key(user.id, &path)?;
    let stream = s
        .store
        .op
        .reader_with(&key)
        .chunk(4 * 1024 * 1024)
        .await
        .map_err(crate::files::StoreError::from)?
        .into_bytes_stream(start..end)
        .await
        .map_err(crate::files::StoreError::from)?;
    let mut res = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, crate::routes::content_type(&path))
        .header(header::CONTENT_LENGTH, end - start)
        .header(header::ACCEPT_RANGES, "bytes");
    if status == StatusCode::PARTIAL_CONTENT {
        res = res.header(header::CONTENT_RANGE, format!("bytes {start}-{}/{size}", end - 1));
    }
    Ok(res.body(Body::from_stream(stream)).unwrap())
}

pub async fn delete_file(
    State(s): State<AppState>,
    user: AuthUser,
    Path(path): Path<String>,
) -> Result<StatusCode, AppError> {
    let mut row = load_user(&s.db, user.id).await?;
    let mut q = Quota::new(row.quota_limit.max(0) as u64, row.quota_used.max(0) as u64);
    let _freed = s.store.delete(user.id, &path, &mut q).await?;
    row.quota_used = q.used_bytes as i64;
    sqlx::query("UPDATE users SET quota_used = $1 WHERE id = $2")
        .bind(row.quota_used)
        .bind(user.id)
        .execute(&s.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct InitBody {
    pub path: String,
    pub size: u64,
}

#[derive(Serialize)]
pub struct InitOut {
    pub id: String,
}

/// Start a resumable upload. Quota pre-checked; bytes flow via PUT chunk.
pub async fn init_upload(
    State(s): State<AppState>,
    user: AuthUser,
    Json(b): Json<InitBody>,
) -> Result<Json<InitOut>, AppError> {
    if b.size > 100 * 1024 * 1024 * 1024 {
        return Err(AppError::BadRequest("file too large (max 100GiB)".into()));
    }
    let row = load_user(&s.db, user.id).await?;
    let q = Quota::new(row.quota_limit.max(0) as u64, row.quota_used.max(0) as u64);
    q.can_add(b.size).map_err(|e| AppError::Conflict(format!("quota exceeded: {e}")))?;
    s.store.key(user.id, &b.path)?; // validate early
    let id = Uuid::new_v4().to_string();
    s.uploads.lock().await.insert(
        id.clone(),
        PendingUpload { user_id: user.id, path: b.path, size: b.size },
    );
    Ok(Json(InitOut { id }))
}

#[derive(Deserialize)]
pub struct ChunkQuery {
    pub offset: u64,
}

/// Append bytes at exactly `offset` (client retries safely). Max 64MiB per chunk.
pub async fn put_chunk(
    State(s): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Query(q): Query<ChunkQuery>,
    body: axum::body::Bytes,
) -> Result<Json<HashMap<String, u64>>, AppError> {
    if body.len() > 64 * 1024 * 1024 {
        return Err(AppError::BadRequest("chunk too large (max 64MiB)".into()));
    }
    let pend = s.uploads.lock().await.get(&id).cloned().ok_or_else(|| AppError::NotFound("upload gone".into()))?;
    if pend.user_id != user.id {
        return Err(AppError::Forbidden("not your upload".into()));
    }
    let mut map = HashMap::new();
    map.insert("received".to_string(), s.store.write_chunk(user.id, &id, q.offset, &body)?);
    Ok(Json(map))
}

pub async fn complete_upload(
    State(s): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let pend = s
        .uploads
        .lock()
        .await
        .remove(&id)
        .ok_or_else(|| AppError::NotFound("upload gone".into()))?;
    if pend.user_id != user.id {
        return Err(AppError::Forbidden("not your upload".into()));
    }
    let row = load_user(&s.db, user.id).await?;
    let mut q = Quota::new(row.quota_limit.max(0) as u64, row.quota_used.max(0) as u64);
    let r = s.store.complete(user.id, &id, &pend.path, pend.size, &mut q).await;
    // Clean staging on failure so retries start fresh.
    if r.is_err() {
        let _ = std::fs::remove_file(s.store.stage_path(user.id, &id));
    }
    r?;
    sqlx::query("UPDATE users SET quota_used = $1 WHERE id = $2")
        .bind(q.used_bytes as i64)
        .bind(user.id)
        .execute(&s.db)
        .await?;
    Ok(StatusCode::CREATED)
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn range_parsing() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 100)));
        assert_eq!(parse_range("bytes=900-", 1000), Some((900, 1000)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 1000)));
        assert_eq!(parse_range("bytes=0-5000", 1000), Some((0, 1000))); // clamp
        assert_eq!(parse_range("bytes=1000-", 1000), None);
        assert_eq!(parse_range("bytes=5-2", 1000), None);
        assert_eq!(parse_range("bytes=0-1,5-9", 1000), None);
        assert_eq!(parse_range("items=0-1", 1000), None);
    }
}
