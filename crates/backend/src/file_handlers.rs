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
    files::clean_path,
    index::{self, SortKey},
    ops,
    quota::Quota,
    state::{AppError, AppState, AuthUser, PendingUpload, load_user},
};

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub sort: SortKey,
    #[serde(default)]
    pub desc: bool,
    #[serde(default)]
    pub offset: i64,
    pub limit: Option<i64>,
}

/// 1 trang của thư mục, đọc từ chỉ mục Postgres (không hỏi S3).
pub async fn list_files(
    State(s): State<AppState>,
    user: AuthUser,
    Query(q): Query<ListQuery>,
) -> Result<Json<index::Page>, AppError> {
    let parent = clean_path(&q.prefix)?;
    let limit = q.limit.unwrap_or(200).clamp(0, 1000);
    Ok(Json(index::list(&s.db, user.id, &parent, q.sort, q.desc, q.offset.max(0), limit).await?))
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub limit: Option<i64>,
}

pub async fn search_files(
    State(s): State<AppState>,
    user: AuthUser,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Vec<index::Row>>, AppError> {
    let term = q.q.trim();
    if term.is_empty() {
        return Ok(Json(vec![]));
    }
    Ok(Json(index::search(&s.db, user.id, term, q.limit.unwrap_or(200).clamp(1, 1000)).await?))
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
/// <video>/<audio> không gửi được Bearer -> chấp nhận token qua ?t=.
pub async fn download(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Result<Response, AppError> {
    // ?dl=<token tải 1 file> (link tải trực tiếp) hoặc đăng nhập thường.
    let dl = match q.get("dl") {
        Some(t) => {
            let c = crate::auth::verify_dl_token(t, &s.dec).map_err(AppError::Auth)?;
            if clean_path(&c.path)? != clean_path(&path)? {
                return Err(AppError::Forbidden("token không dành cho file này".into()));
            }
            Some(c.sub)
        }
        None => None,
    };
    let user = match dl {
        Some(id) => crate::state::AuthUser { id, admin: false },
        None => crate::thumb::auth_opt(&headers, &q, &s).await.ok_or_else(|| AppError::Auth("missing token".into()))?,
    };
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
    if dl.is_some() {
        let name = path.rsplit('/').next().unwrap_or("file");
        res = res.header(header::CONTENT_DISPOSITION, format!("attachment; filename*=UTF-8''{}", pct(name)));
    }
    Ok(res.body(Body::from_stream(stream)).unwrap())
}

/// Percent-encode cho `filename*` (RFC 5987): tên tiếng Việt/khoảng trắng vẫn đúng.
fn pct(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

#[derive(Deserialize)]
pub struct DlBody {
    pub path: String,
}

/// Cấp token tải 1 file (10 phút) để trình duyệt tự stream xuống đĩa, không qua RAM của tab.
pub async fn dl_token(State(s): State<AppState>, user: AuthUser, Json(b): Json<DlBody>) -> Result<Json<serde_json::Value>, AppError> {
    let p = clean_path(&b.path)?;
    if s.store.stat(user.id, &p).await?.is_dir() {
        return Err(AppError::BadRequest("not a file".into()));
    }
    let t = crate::auth::create_dl_token(user.id, &p, &s.enc).map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "token": t })))
}

/// Xoá = chuyển vào thùng rác (file hoặc cả thư mục).
pub async fn delete_file(
    State(s): State<AppState>,
    user: AuthUser,
    Path(path): Path<String>,
) -> Result<Json<ops::TrashItem>, AppError> {
    let p = ops::user_path(&path)?;
    Ok(Json(ops::trash(&s.db, &s.store, user.id, &p).await?))
}

#[derive(Deserialize)]
pub struct MkdirBody {
    pub path: String,
}

pub async fn mkdir(State(s): State<AppState>, user: AuthUser, Json(b): Json<MkdirBody>) -> Result<StatusCode, AppError> {
    ops::mkdir(&s.db, user.id, &ops::user_path(&b.path)?).await?;
    Ok(StatusCode::CREATED)
}

#[derive(Deserialize)]
pub struct MoveBody {
    pub from: String,
    pub to: String,
}

/// Đổi tên + di chuyển. Trả về số file đã chuyển.
pub async fn move_entry(
    State(s): State<AppState>,
    user: AuthUser,
    Json(b): Json<MoveBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let (from, to) = (ops::user_path(&b.from)?, ops::user_path(&b.to)?);
    let n = ops::move_path(&s.db, &s.store, user.id, &from, &to).await?;
    Ok(Json(serde_json::json!({ "files": n })))
}

pub async fn list_trash(State(s): State<AppState>, user: AuthUser) -> Result<Json<Vec<ops::TrashItem>>, AppError> {
    Ok(Json(ops::list_trash(&s.db, user.id).await?))
}

pub async fn restore_trash(
    State(s): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<ops::TrashItem>, AppError> {
    Ok(Json(ops::restore(&s.db, &s.store, user.id, id).await?))
}

pub async fn purge_trash(State(s): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> Result<StatusCode, AppError> {
    ops::purge(&s.db, &s.store, user.id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn empty_trash(State(s): State<AppState>, user: AuthUser) -> Result<StatusCode, AppError> {
    for t in ops::list_trash(&s.db, user.id).await? {
        ops::purge(&s.db, &s.store, user.id, t.id).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct InitBody {
    pub path: String,
    pub size: u64,
    /// mtime gốc của file (unix giây), giữ lại thay vì lấy giờ upload.
    pub mtime: Option<i64>,
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
    ops::upload_path(&b.path)?; // validate early, chặn ghi vào .nextkloud (trừ file cài đặt client)
    let id = Uuid::new_v4().to_string();
    s.uploads.lock().await.insert(
        id.clone(),
        PendingUpload { user_id: user.id, path: b.path, size: b.size, mtime: b.mtime },
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
    let r = s.store.complete(user.id, &id, &pend.path, pend.size, pend.mtime, &mut q).await;
    // Clean staging on failure so retries start fresh.
    if r.is_err() {
        let _ = std::fs::remove_file(s.store.stage_path(user.id, &id));
    }
    let mtime = r?;
    // Cộng phần chênh (file mới, hoặc chênh khi ghi đè), không ghi đè giá trị: upload song song không lệch.
    sqlx::query("UPDATE users SET quota_used = GREATEST(0, quota_used + $1) WHERE id = $2")
        .bind(q.used_bytes as i64 - row.quota_used.max(0))
        .bind(user.id)
        .execute(&s.db)
        .await?;
    index::add_file(&s.db, user.id, &clean_path(&pend.path)?, pend.size, mtime).await?;
    Ok(StatusCode::CREATED)
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn pct_filename() {
        assert_eq!(super::pct("Báo cáo 1.pdf"), "B%C3%A1o%20c%C3%A1o%201.pdf");
    }

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
