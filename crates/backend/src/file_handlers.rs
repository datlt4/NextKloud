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

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

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
    /// xem thư mục của user khác (đòi hỏi được chia sẻ)
    #[serde(default)]
    pub owner: Option<Uuid>,
}

/// 1 trang của thư mục, đọc từ chỉ mục Postgres (không hỏi S3).
pub async fn list_files(
    State(s): State<AppState>,
    user: AuthUser,
    Query(q): Query<ListQuery>,
) -> Result<Json<index::Page>, AppError> {
    let parent = clean_path(&q.prefix)?;
    let oid = crate::shares::resolve(&s.db, user.id, q.owner, &parent, false).await?;
    let limit = q.limit.unwrap_or(200).clamp(0, 1000);
    Ok(Json(index::list(&s.db, oid, &parent, q.sort, q.desc, q.offset.max(0), limit).await?))
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub limit: Option<i64>,
}

/// Tìm theo nội dung file .md/.txt, kèm đoạn trích quanh chỗ khớp.
pub async fn search_content(
    State(s): State<AppState>,
    user: AuthUser,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Vec<crate::fulltext::Hit>>, AppError> {
    let term = q.q.trim();
    if term.chars().count() < 2 {
        return Ok(Json(vec![]));
    }
    Ok(Json(crate::fulltext::search(&s.db, user.id, term, q.limit.unwrap_or(100).clamp(1, 500)).await?))
}

#[derive(Deserialize)]
pub struct PathQuery {
    pub path: String,
}

pub async fn vault_outline(
    State(s): State<AppState>,
    user: AuthUser,
    Query(q): Query<PathQuery>,
) -> Result<Json<Option<crate::fulltext::Outline>>, AppError> {
    Ok(Json(crate::fulltext::outline(&s.db, user.id, &clean_path(&q.path)?).await?))
}

/// File nào nhắc tới file này qua [[wikilink]].
pub async fn vault_backlinks(
    State(s): State<AppState>,
    user: AuthUser,
    Query(q): Query<PathQuery>,
) -> Result<Json<Vec<crate::fulltext::Backlink>>, AppError> {
    Ok(Json(crate::fulltext::backlinks(&s.db, user.id, &clean_path(&q.path)?).await?))
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
    // đọc file của user khác: đòi hỏi được chia sẻ (xem). Token dl đã mang sẵn owner.
    let oid = crate::shares::resolve(&s.db, user.id, q.get("owner").and_then(|v| v.parse().ok()), &path, false).await?;
    let meta = s.store.stat(oid, &path).await?;
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
    let key = s.store.key(oid, &path)?;
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
    /// tải file của user khác (đòi hỏi được chia sẻ)
    #[serde(default)]
    pub owner: Option<Uuid>,
}

/// Cấp token tải 1 file (10 phút) để trình duyệt tự stream xuống đĩa, không qua RAM của tab.
pub async fn dl_token(State(s): State<AppState>, user: AuthUser, Json(b): Json<DlBody>) -> Result<Json<serde_json::Value>, AppError> {
    let p = clean_path(&b.path)?;
    let oid = crate::shares::resolve(&s.db, user.id, b.owner, &p, false).await?;
    if s.store.stat(oid, &p).await?.is_dir() {
        return Err(AppError::BadRequest("not a file".into()));
    }
    let t = crate::auth::create_dl_token(oid, &p, &s.enc).map_err(AppError::Internal)?;
    Ok(Json(serde_json::json!({ "token": t })))
}

/// Xoá = chuyển vào thùng rác (file hoặc cả thư mục). Xoá trong vùng được chia sẻ cần quyền sửa, vào thùng rác của chủ file.
pub async fn delete_file(
    State(s): State<AppState>,
    user: AuthUser,
    Path(path): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<ops::TrashItem>, AppError> {
    let p = ops::user_path(&path)?;
    let oid = crate::shares::resolve(&s.db, user.id, q.get("owner").and_then(|v| v.parse().ok()), &p, true).await?;
    Ok(Json(ops::trash(&s.db, &s.store, oid, &p).await?))
}

#[derive(Deserialize)]
pub struct MkdirBody {
    pub path: String,
    #[serde(default)]
    pub owner: Option<Uuid>,
}

pub async fn mkdir(State(s): State<AppState>, user: AuthUser, Json(b): Json<MkdirBody>) -> Result<StatusCode, AppError> {
    let p = ops::user_path(&b.path)?;
    let oid = crate::shares::resolve(&s.db, user.id, b.owner, &p, true).await?;
    ops::mkdir(&s.db, oid, &p).await?;
    Ok(StatusCode::CREATED)
}

#[derive(Deserialize)]
pub struct MoveBody {
    pub from: String,
    pub to: String,
    /// di chuyển trong vùng được chia sẻ (from/to cùng 1 chủ, cần quyền sửa)
    #[serde(default)]
    pub owner: Option<Uuid>,
}

/// Đổi tên + di chuyển. Trả về số file đã chuyển.
pub async fn move_entry(
    State(s): State<AppState>,
    user: AuthUser,
    Json(b): Json<MoveBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let (from, to) = (ops::user_path(&b.from)?, ops::user_path(&b.to)?);
    let oid = crate::shares::resolve(&s.db, user.id, b.owner, &from, true).await?;
    crate::shares::resolve(&s.db, user.id, b.owner, &to, true).await?;
    let n = ops::move_path(&s.db, &s.store, oid, &from, &to).await?;
    Ok(Json(serde_json::json!({ "files": n })))
}

/// Sao chép file/thư mục. Trả về số file đã chép.
pub async fn copy_entry(
    State(s): State<AppState>,
    user: AuthUser,
    Json(b): Json<MoveBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let (from, to) = (ops::user_path(&b.from)?, ops::user_path(&b.to)?);
    let oid = crate::shares::resolve(&s.db, user.id, b.owner, &from, true).await?;
    crate::shares::resolve(&s.db, user.id, b.owner, &to, true).await?;
    let n = ops::copy_path(&s.db, &s.store, oid, &from, &to).await?;
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
    /// upload vào thư mục được chia sẻ (cần quyền sửa, quota tính cho chủ file)
    #[serde(default)]
    pub owner: Option<Uuid>,
}

#[derive(Serialize)]
pub struct InitOut {
    pub id: String,
}

/// Start a resumable upload. Quota pre-checked; bytes flow via PUT chunk.
/// Upload dở lưu ở Postgres (bảng uploads) + file .part trên đĩa -> rớt mạng,
/// F5 hay restart server đều resume được (7 ngày). Gửi lại cùng path+size là chạy tiếp.
pub async fn init_upload(
    State(s): State<AppState>,
    user: AuthUser,
    Json(b): Json<InitBody>,
) -> Result<Json<InitOut>, AppError> {
    if b.size > 100 * 1024 * 1024 * 1024 {
        return Err(AppError::BadRequest("file too large (max 100GiB)".into()));
    }
    let row = load_user(&s.db, user.id).await?;
    ops::upload_path(&b.path)?; // validate early, chặn ghi vào .nextkloud (trừ file cài đặt client)
    let oid = crate::shares::resolve(&s.db, user.id, b.owner, &b.path, true).await?;
    // quota tính cho chủ vùng (chủ file khi upload vào thư mục được chia sẻ)
    let orow = if oid == user.id { row } else { load_user(&s.db, oid).await? };
    let oq = Quota::new(orow.quota_limit.max(0) as u64, orow.quota_used.max(0) as u64);
    oq.can_add(b.size).map_err(|e| AppError::Conflict(format!("quota exceeded: {e}")))?;
    // dọn upload dở quá 7 ngày của user này
    let old: Vec<(String, Uuid)> = sqlx::query_as("SELECT id, owner FROM uploads WHERE user_id = $1 AND created_at < $2")
        .bind(user.id)
        .bind(now() - 7 * 86_400)
        .fetch_all(&s.db)
        .await?;
    for (id, ow) in old {
        let _ = std::fs::remove_file(s.store.stage_path(ow, &id));
    }
    sqlx::query("DELETE FROM uploads WHERE user_id = $1 AND created_at < $2").bind(user.id).bind(now() - 7 * 86_400).execute(&s.db).await?;
    let pc = clean_path(&b.path)?;
    // còn dở cùng file (path+size+mtime) và file .part còn nguyên -> trả id cũ để chạy tiếp.
    // Không có mtime (Blob, saveText) thì không resume: khác nội dung cùng size sẽ bị ghép lẫn.
    let resume: Option<(String, i64)> = sqlx::query_as(
        "SELECT id, received FROM uploads WHERE user_id = $1 AND owner = $2 AND path = $3 AND size = $4 AND mtime = $5 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(user.id)
    .bind(oid)
    .bind(&pc)
    .bind(b.size as i64)
    .bind(b.mtime)
    .fetch_optional(&s.db)
    .await?;
    if let Some((id, received)) = resume {
        let part = s.store.stage_path(oid, &id);
        let len = std::fs::metadata(&part).map(|m| m.len() as i64).unwrap_or(-1);
        if len == received {
            s.uploads.lock().await.insert(
                id.clone(),
                PendingUpload { user_id: user.id, owner: oid, path: b.path, size: b.size, mtime: b.mtime },
            );
            return Ok(Json(InitOut { id }));
        }
        let _ = std::fs::remove_file(part);
        sqlx::query("DELETE FROM uploads WHERE id = $1").bind(&id).execute(&s.db).await?;
    }
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO uploads (id, user_id, owner, path, size, mtime, received, created_at) VALUES ($1,$2,$3,$4,$5,$6,0,$7)")
        .bind(&id)
        .bind(user.id)
        .bind(oid)
        .bind(&pc)
        .bind(b.size as i64)
        .bind(b.mtime)
        .bind(now())
        .execute(&s.db)
        .await?;
    s.uploads.lock().await.insert(
        id.clone(),
        PendingUpload { user_id: user.id, owner: oid, path: b.path, size: b.size, mtime: b.mtime },
    );
    Ok(Json(InitOut { id }))
}

/// Tìm upload đang dở của chính mình (để resume sau F5): cùng vùng, path và size.
pub async fn upload_active(
    State(s): State<AppState>,
    user: AuthUser,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let path = q.get("path").map(String::as_str).unwrap_or("");
    let owner: Option<Uuid> = q.get("owner").and_then(|v| v.parse().ok());
    let oid = crate::shares::resolve(&s.db, user.id, owner, path, true).await?;
    let pc = clean_path(path)?;
    let size: i64 = q.get("size").and_then(|v| v.parse().ok()).unwrap_or(-1);
    let mtime: Option<i64> = q.get("mtime").and_then(|v| v.parse().ok());
    let row: Option<(String, i64, i64)> = sqlx::query_as(
        "SELECT id, received, size FROM uploads WHERE user_id = $1 AND owner = $2 AND path = $3 AND ($4 < 0 OR size = $4) AND mtime = $5 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(user.id)
    .bind(oid)
    .bind(&pc)
    .bind(size)
    .bind(mtime)
    .fetch_optional(&s.db)
    .await?;
    match row {
        Some((id, received, size)) => Ok(Json(serde_json::json!({ "id": id, "received": received, "size": size }))),
        None => Ok(Json(serde_json::Value::Null)),
    }
}

/// Số byte đã nhận của 1 upload (client lệch offset thì hỏi lại rồi chạy tiếp từ đây).
pub async fn upload_status(
    State(s): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let row: Option<(Uuid, Uuid, String, i64, i64)> =
        sqlx::query_as("SELECT user_id, owner, path, size, received FROM uploads WHERE id = $1").bind(&id).fetch_optional(&s.db).await?;
    match row {
        Some((uid, oid, path, size, received)) if uid == user.id => {
            Ok(Json(serde_json::json!({ "id": id, "owner": oid, "path": path, "size": size, "received": received })))
        }
        _ => Err(AppError::NotFound("upload gone".into())),
    }
}

/// Lấy PendingUpload từ RAM, rớt thì dựng lại từ Postgres (sau restart server).
async fn pending(s: &AppState, user: Uuid, id: &str) -> Result<PendingUpload, AppError> {
    if let Some(p) = s.uploads.lock().await.get(id).cloned() {
        if p.user_id != user {
            return Err(AppError::Forbidden("not your upload".into()));
        }
        return Ok(p);
    }
    let row: Option<(Uuid, Uuid, String, i64, Option<i64>)> =
        sqlx::query_as("SELECT user_id, owner, path, size, mtime FROM uploads WHERE id = $1").bind(id).fetch_optional(&s.db).await?;
    match row {
        Some((uid, owner, path, size, mtime)) if uid == user => {
            let p = PendingUpload { user_id: uid, owner, path, size: size as u64, mtime };
            s.uploads.lock().await.insert(id.to_string(), p.clone());
            Ok(p)
        }
        _ => Err(AppError::NotFound("upload gone".into())),
    }
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
    let pend = pending(&s, user.id, &id).await?;
    let received = s.store.write_chunk(pend.owner, &id, q.offset, &body)?;
    sqlx::query("UPDATE uploads SET received = $2 WHERE id = $1").bind(&id).bind(received as i64).execute(&s.db).await?;
    let mut map = HashMap::new();
    map.insert("received".to_string(), received);
    Ok(Json(map))
}

pub async fn complete_upload(
    State(s): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let pend = pending(&s, user.id, &id).await?;
    let row = load_user(&s.db, pend.owner).await?;
    let mut q = Quota::new(row.quota_limit.max(0) as u64, row.quota_used.max(0) as u64);
    let r = s.store.complete(pend.owner, &id, &pend.path, pend.size, pend.mtime, &mut q).await;
    // Clean staging on failure so retries start fresh.
    if r.is_err() {
        let _ = std::fs::remove_file(s.store.stage_path(pend.owner, &id));
    }
    let mtime = r?;
    // Cộng phần chênh (file mới, hoặc chênh khi ghi đè), không ghi đè giá trị: upload song song không lệch.
    sqlx::query("UPDATE users SET quota_used = GREATEST(0, quota_used + $1) WHERE id = $2")
        .bind(q.used_bytes as i64 - row.quota_used.max(0))
        .bind(pend.owner)
        .execute(&s.db)
        .await?;
    let p = clean_path(&pend.path)?;
    index::add_file(&s.db, pend.owner, &p, pend.size, mtime).await?;
    crate::fulltext::put_from_store(&s.db, &s.store, pend.owner, &p, pend.size).await;
    sqlx::query("DELETE FROM uploads WHERE id = $1").bind(&id).execute(&s.db).await?;
    s.uploads.lock().await.remove(&id);
    Ok(StatusCode::CREATED)
}

#[cfg(test)]
mod tests {
    use super::parse_range;
    use super::{pending, upload_active, upload_status, ChunkQuery, InitBody};
    use crate::state::{AppState, AuthUser};
    use axum::extract::{Path, Query, State};
    use std::collections::HashMap;

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

    async fn up_state() -> Option<(AppState, uuid::Uuid)> {
        let Some((db, u)) = crate::kanban::tests::db_user().await else { return None };
        let (store, _) = crate::files::tests::tmp_store("upresume");
        Some((
            AppState {
                db,
                store,
                enc: jsonwebtoken::EncodingKey::from_secret(b"t"),
                dec: jsonwebtoken::DecodingKey::from_secret(b"t"),
                uploads: std::sync::Arc::new(tokio::sync::Mutex::new(HashMap::new())),
                http: reqwest::Client::new(),
                tts_url: String::new(),
                vaults: Default::default(),
            },
            u,
        ))
    }

    #[tokio::test]
    async fn upload_resume_across_restart() {
        let Some((s, u)) = up_state().await else { return };
        let auth = AuthUser { id: u, admin: false };
        let data = vec![7u8; 100];
        // init -> gửi nửa đầu
        let id = super::init_upload(State(s.clone()), auth.clone(), axum::Json(InitBody { path: "r.bin".into(), size: 100, mtime: Some(1000), owner: None }))
            .await
            .unwrap()
            .0
            .id;
        super::put_chunk(State(s.clone()), auth.clone(), Path(id.clone()), Query(ChunkQuery { offset: 0 }), axum::body::Bytes::from(data[..40].to_vec()))
            .await
            .unwrap();
        // "restart server": RAM mất, Postgres + .part còn
        s.uploads.lock().await.clear();
        let st = upload_status(State(s.clone()), auth.clone(), Path(id.clone())).await.unwrap();
        assert_eq!(st.0["received"], 40);
        // active lookup thấy upload dở
        let mut q = HashMap::new();
        q.insert("path".to_string(), "r.bin".to_string());
        q.insert("size".to_string(), "100".to_string());
        q.insert("mtime".to_string(), "999".to_string());
        // cùng path+size nhưng khác mtime (file khác): không được ghép vào upload dở
        let a = upload_active(State(s.clone()), auth.clone(), Query(q.clone())).await.unwrap();
        assert!(a.0.is_null());
        q.insert("mtime".to_string(), "1000".to_string());
        let a = upload_active(State(s.clone()), auth.clone(), Query(q)).await.unwrap();
        assert_eq!(a.0["id"], id);
        // gửi tiếp từ received -> xong
        super::put_chunk(State(s.clone()), auth.clone(), Path(id.clone()), Query(ChunkQuery { offset: 40 }), axum::body::Bytes::from(data[40..].to_vec()))
            .await
            .unwrap();
        super::complete_upload(State(s.clone()), auth.clone(), Path(id.clone())).await.unwrap();
        let st2: Option<(i64,)> = sqlx::query_as("SELECT size FROM files WHERE user_id = $1 AND path = 'r.bin'").bind(u).fetch_optional(&s.db).await.unwrap();
        assert_eq!(st2, Some((100,)));
        // xong thì active không còn
        let mut q2 = HashMap::new();
        q2.insert("path".to_string(), "r.bin".to_string());
        assert!(upload_active(State(s.clone()), auth, Query(q2)).await.unwrap().0.is_null());
        let _ = pending(&s, u, &id).await;
    }
}
