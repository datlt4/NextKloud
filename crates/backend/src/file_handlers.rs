//! File HTTP handlers. Auth + quota enforced here, bytes via FsStore.
use std::collections::HashMap;

use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{StatusCode, header},
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
    let entries: Vec<Entry> = s.store.list(user.id, &q.prefix)?;
    Ok(Json(entries.into_iter().map(|e| EntryOut { name: e.name, is_dir: e.is_dir, size: e.size, modified: e.modified }).collect()))
}

pub async fn download(
    State(s): State<AppState>,
    user: AuthUser,
    Path(path): Path<String>,
) -> Result<Response, AppError> {
    let full = s.store.safe_join(user.id, &path)?;
    let meta = tokio::fs::metadata(&full).await.map_err(|_| AppError::NotFound("not found".into()))?;
    if !meta.is_file() {
        return Err(AppError::BadRequest("not a file (use ?prefix= to list)".into()));
    }
    let bytes = tokio::fs::read(&full).await.map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, crate::routes::content_type(&path))
        .header(header::ACCEPT_RANGES, "bytes")
        .body(Body::from(bytes))
        .unwrap())
}

pub async fn delete_file(
    State(s): State<AppState>,
    user: AuthUser,
    Path(path): Path<String>,
) -> Result<StatusCode, AppError> {
    let mut row = load_user(&s.db, user.id).await?;
    let mut q = Quota::new(row.quota_limit.max(0) as u64, row.quota_used.max(0) as u64);
    let _freed = s.store.delete(user.id, &path, &mut q)?;
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
    s.store.safe_join(user.id, &b.path)?; // validate early
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
    let r = s.store.complete(user.id, &id, &pend.path, pend.size, &mut q);
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
