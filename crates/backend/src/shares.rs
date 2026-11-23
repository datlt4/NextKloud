//! Chia sẻ file/thư mục: giữa các user (quyền xem/sửa) + link công khai (mật khẩu, hạn).
//! File trong Bí mật nằm dưới `.nextkloud` nên `user_path` đã chặn, không chia sẻ được.
use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::Response,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

use crate::{
    files::clean_path,
    index,
    ops::{self, check_parents},
    state::{AppError, AppState, AuthUser},
};

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[derive(Debug, sqlx::FromRow)]
struct ShareRow {
    id: Uuid,
    owner: Uuid,
    path: String,
    is_dir: bool,
    target_user: Option<Uuid>,
    token: Option<String>,
    pass_hash: Option<String>,
    permission: String,
    expires_at: Option<i64>,
    #[allow(dead_code)]
    created_at: i64,
}

fn expired(r: &ShareRow) -> bool {
    r.expires_at.is_some_and(|e| e <= now())
}

/// user có quyền trên path của owner không (path == gốc share hoặc nằm bên dưới).
/// need_edit=true đòi quyền sửa.
pub async fn resolve(db: &PgPool, me: Uuid, owner: Option<Uuid>, path: &str, need_edit: bool) -> Result<Uuid, AppError> {
    let o = owner.unwrap_or(me);
    if o == me {
        return Ok(me);
    }
    if me.is_nil() {
        return Err(AppError::Forbidden("cần đăng nhập".into()));
    }
    let p = clean_path(path).map_err(|_| AppError::BadRequest("path không hợp lệ".into()))?;
    if p.is_empty() {
        return Err(AppError::BadRequest("cần path cụ thể trong thư mục được chia sẻ".into()));
    }
    let rows: Vec<ShareRow> = sqlx::query_as(
        "SELECT id, owner, path, is_dir, target_user, token, pass_hash, permission, expires_at, created_at
         FROM shares WHERE owner = $1 AND target_user = $2 AND (expires_at IS NULL OR expires_at > $3)",
    )
    .bind(o)
    .bind(me)
    .bind(now())
    .fetch_all(db)
    .await?;
    let mut seen_view = false;
    for r in &rows {
        if p == r.path || p.starts_with(&format!("{}/", r.path)) {
            if !need_edit || r.permission == "edit" {
                return Ok(o);
            }
            seen_view = true;
        }
    }
    if seen_view {
        return Err(AppError::Forbidden("bạn chỉ được xem, không được sửa".into()));
    }
    Err(AppError::NotFound("không có quyền truy cập".into()))
}

// ---------- owner: quản lý chia sẻ ----------

#[derive(Debug, Serialize)]
pub struct ShareOut {
    pub id: Uuid,
    pub path: String,
    pub is_dir: bool,
    pub target: Option<String>,
    pub token: Option<String>,
    pub has_password: bool,
    pub permission: String,
    pub expires_at: Option<i64>,
    pub created_at: i64,
}

async fn share_out(db: &PgPool, r: ShareRow) -> Result<ShareOut, AppError> {
    let target = match r.target_user {
        Some(u) => {
            let row: Option<(String,)> = sqlx::query_as("SELECT username FROM users WHERE id = $1").bind(u).fetch_optional(db).await?;
            row.map(|(n,)| n)
        }
        None => None,
    };
    Ok(ShareOut {
        id: r.id,
        path: r.path,
        is_dir: r.is_dir,
        target,
        token: r.token,
        has_password: r.pass_hash.is_some(),
        permission: r.permission,
        expires_at: r.expires_at,
        created_at: r.created_at,
    })
}

#[derive(Deserialize)]
pub struct SharesQuery {
    #[serde(default)]
    pub path: String,
}

/// Các chia sẻ/link của chính mình (lọc theo path nếu có).
pub async fn list_shares(State(s): State<AppState>, user: AuthUser, Query(q): Query<SharesQuery>) -> Result<Json<Vec<ShareOut>>, AppError> {
    let rows: Vec<ShareRow> = if q.path.is_empty() {
        sqlx::query_as(
            "SELECT id, owner, path, is_dir, target_user, token, pass_hash, permission, expires_at, created_at
             FROM shares WHERE owner = $1 ORDER BY created_at DESC LIMIT 200",
        )
        .bind(user.id)
        .fetch_all(&s.db)
        .await?
    } else {
        let p = ops::user_path(&q.path)?;
        sqlx::query_as(
            "SELECT id, owner, path, is_dir, target_user, token, pass_hash, permission, expires_at, created_at
             FROM shares WHERE owner = $1 AND path = $2 ORDER BY created_at DESC",
        )
        .bind(user.id)
        .bind(p)
        .fetch_all(&s.db)
        .await?
    };
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(share_out(&s.db, r).await?);
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct ShareWith {
    pub path: String,
    pub username: String,
    #[serde(default = "view_perm")]
    pub permission: String,
}
fn view_perm() -> String {
    "view".into()
}

/// Chia sẻ cho 1 user (không chia sẻ cho chính mình, không re-share).
pub async fn share_with(State(s): State<AppState>, user: AuthUser, Json(b): Json<ShareWith>) -> Result<(StatusCode, Json<ShareOut>), AppError> {
    if b.permission != "view" && b.permission != "edit" {
        return Err(AppError::BadRequest("permission phải là view hoặc edit".into()));
    }
    let p = ops::user_path(&b.path)?;
    let is_dir = ops::kind(&s.db, user.id, &p).await?.ok_or_else(|| AppError::NotFound("not found".into()))?;
    let target: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE username = $1").bind(b.username.trim()).fetch_optional(&s.db).await?;
    let (tid,) = target.ok_or_else(|| AppError::NotFound("không có user này".into()))?;
    if tid == user.id {
        return Err(AppError::BadRequest("không thể chia sẻ cho chính mình".into()));
    }
    let dup: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM shares WHERE owner = $1 AND path = $2 AND target_user = $3")
        .bind(user.id)
        .bind(&p)
        .bind(tid)
        .fetch_optional(&s.db)
        .await?;
    if dup.is_some() {
        return Err(AppError::Conflict("đã chia sẻ cho user này".into()));
    }
    let r: ShareRow = sqlx::query_as(
        "INSERT INTO shares (id, owner, path, is_dir, target_user, permission, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id, owner, path, is_dir, target_user, token, pass_hash, permission, expires_at, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(user.id)
    .bind(p)
    .bind(is_dir)
    .bind(tid)
    .bind(b.permission)
    .bind(now())
    .fetch_one(&s.db)
    .await?;
    Ok((StatusCode::CREATED, Json(share_out(&s.db, r).await?)))
}

#[derive(Deserialize)]
pub struct NewLink {
    pub path: String,
    /// mật khẩu link (trống = không đặt)
    #[serde(default)]
    pub password: String,
    /// số ngày hiệu lực (trống = vĩnh viễn, tối đa 365)
    pub expires_in_days: Option<i64>,
}

/// Tạo link công khai (chỉ xem + tải, kể cả khi path được chia sẻ edit cho user khác).
pub async fn create_link(State(s): State<AppState>, user: AuthUser, Json(b): Json<NewLink>) -> Result<(StatusCode, Json<ShareOut>), AppError> {
    let p = ops::user_path(&b.path)?;
    let is_dir = ops::kind(&s.db, user.id, &p).await?.ok_or_else(|| AppError::NotFound("not found".into()))?;
    let old: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM shares WHERE owner = $1 AND path = $2 AND token IS NOT NULL")
        .bind(user.id)
        .bind(&p)
        .fetch_optional(&s.db)
        .await?;
    if old.is_some() {
        return Err(AppError::Conflict("path này đã có link công khai".into()));
    }
    let days = b.expires_in_days.unwrap_or(0);
    if days < 0 || days > 365 {
        return Err(AppError::BadRequest("hạn dùng 1–365 ngày, hoặc để trống".into()));
    }
    let hash = if b.password.trim().is_empty() {
        None
    } else {
        if b.password.chars().count() > 200 {
            return Err(AppError::BadRequest("mật khẩu quá dài".into()));
        }
        Some(crate::auth::hash_password(b.password.trim()).map_err(AppError::Internal)?)
    };
    let token: String = Uuid::new_v4().simple().to_string();
    let r: ShareRow = sqlx::query_as(
        "INSERT INTO shares (id, owner, path, is_dir, token, pass_hash, permission, expires_at, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, 'view', $7, $8)
         RETURNING id, owner, path, is_dir, target_user, token, pass_hash, permission, expires_at, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(user.id)
    .bind(p)
    .bind(is_dir)
    .bind(token)
    .bind(hash)
    .bind(if days > 0 { Some(now() + days * 86_400) } else { None })
    .bind(now())
    .fetch_one(&s.db)
    .await?;
    Ok((StatusCode::CREATED, Json(share_out(&s.db, r).await?)))
}

/// Xoá chia sẻ/link của chính mình. Người từng được chia sẻ mất quyền ngay.
pub async fn unshare(State(s): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> Result<StatusCode, AppError> {
    let n = sqlx::query("DELETE FROM shares WHERE id = $1 AND owner = $2").bind(id).bind(user.id).execute(&s.db).await?.rows_affected();
    if n == 0 {
        return Err(AppError::NotFound("share not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------- grantee: nội dung được chia sẻ với tôi ----------

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SharedItem {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub owner_name: String,
    pub path: String,
    pub is_dir: bool,
    pub permission: String,
    pub expires_at: Option<i64>,
}

/// Tất cả file/thư mục người khác chia sẻ cho tôi (còn hạn).
pub async fn shared_with_me(State(s): State<AppState>, user: AuthUser) -> Result<Json<Vec<SharedItem>>, AppError> {
    Ok(Json(
        sqlx::query_as(
            "SELECT s.id, s.owner AS owner_id, u.username AS owner_name, s.path, s.is_dir, s.permission, s.expires_at
             FROM shares s JOIN users u ON u.id = s.owner
             WHERE s.target_user = $1 AND (s.expires_at IS NULL OR s.expires_at > $2)
             ORDER BY u.username, s.path",
        )
        .bind(user.id)
        .bind(now())
        .fetch_all(&s.db)
        .await?,
    ))
}

#[derive(Deserialize)]
pub struct SaveCopy {
    pub owner: Uuid,
    pub path: String,
    /// đích trong cây file của tôi
    pub to: String,
}

/// Lưu bản sao từ vùng được chia sẻ về file của tôi (server copy trực tiếp, không qua RAM client).
pub async fn save_copy(State(s): State<AppState>, user: AuthUser, Json(b): Json<SaveCopy>) -> Result<Json<serde_json::Value>, AppError> {
    let from = ops::user_path(&b.path)?;
    let to = ops::user_path(&b.to)?;
    if b.owner == user.id {
        return Err(AppError::BadRequest("đây đã là file của bạn, dùng Sao chép".into()));
    }
    let o = resolve(&s.db, user.id, Some(b.owner), &from, false).await?;
    if to == from || to.starts_with(&format!("{from}/")) {
        return Err(AppError::BadRequest("đích không hợp lệ".into()));
    }
    if ops::kind(&s.db, o, &from).await?.is_none() {
        return Err(AppError::NotFound("nguồn không còn tồn tại".into()));
    }
    if ops::kind(&s.db, user.id, &to).await?.is_some() {
        return Err(AppError::Conflict("đích đã tồn tại".into()));
    }
    check_parents(&s.db, user.id, &to).await?;
    // liệt kê cây nguồn từ chỉ mục của chủ file
    let files: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT path, size, modified FROM files WHERE user_id = $1 AND NOT is_dir AND (path = $2 OR path LIKE $3)",
    )
    .bind(o)
    .bind(&from)
    .bind(format!("{}/%", crate::index::like_escape(&from)))
    .fetch_all(&s.db)
    .await?;
    if files.is_empty() {
        return Err(AppError::NotFound("nguồn trống hoặc không tồn tại".into()));
    }
    let total: i64 = files.iter().map(|f| f.1).sum();
    let row = crate::state::load_user(&s.db, user.id).await?;
    crate::quota::Quota::new(row.quota_limit.max(0) as u64, row.quota_used.max(0) as u64)
        .can_add(total.max(0) as u64)
        .map_err(|e| AppError::Conflict(format!("quota exceeded: {e}")))?;
    let dest = |p: &str| format!("{to}{}", &p[from.len()..]);
    let mut tx = s.db.begin().await?;
    // tạo thư mục cha + giữ cấu trúc thư mục con của nguồn
    let mut dirset: Vec<String> = crate::index::ancestors(&to).into_iter().map(String::from).collect();
    let subdirs: Vec<(String,)> = sqlx::query_as("SELECT path FROM files WHERE user_id = $1 AND is_dir AND path LIKE $2")
        .bind(o)
        .bind(format!("{}/%", crate::index::like_escape(&from)))
        .fetch_all(&s.db)
        .await?;
    for (d,) in &subdirs {
        dirset.push(dest(d));
    }
    let dirrefs: Vec<&str> = dirset.iter().map(String::as_str).collect();
    crate::index::ensure_dirs(&mut tx, user.id, &dirrefs, now()).await?;
    tx.commit().await?;
    let op = &s.store.op;
    let results: Vec<Result<(String, i64, i64), crate::files::StoreError>> =
        futures::stream::iter(files.iter().cloned())
            .map(|(p, size, m)| {
                let (a, b2) = (format!("{o}/{p}"), format!("{}/{d}", user.id, d = dest(&p)));
                async move { op.copy(&a, &b2).await.map(|_| (dest(&p), size, m)).map_err(crate::files::StoreError::from) }
            })
            .buffered(16)
            .collect()
            .await;
    let mut n = 0u64;
    let mut added = 0i64;
    let mut first_err = None;
    for r in results {
        match r {
            Ok((p, size, m)) => {
                crate::index::add_file(&s.db, user.id, &p, size as u64, m).await?;
                crate::fulltext::put_from_store(&s.db, &s.store, user.id, &p, size as u64).await;
                n += 1;
                added += size;
            }
            Err(e) => first_err = first_err.or(Some(e)),
        }
    }
    sqlx::query("UPDATE users SET quota_used = quota_used + $2 WHERE id = $1").bind(user.id).bind(added).execute(&s.db).await?;
    if let Some(e) = first_err {
        return Err(AppError::Internal(format!("lưu được {n}/{} file: {e}", files.len())));
    }
    Ok(Json(serde_json::json!({ "files": n })))
}

// ---------- link công khai (không cần đăng nhập) ----------

#[derive(Deserialize)]
pub struct LinkQuery {
    #[serde(default)]
    pub pw: String,
    #[serde(default)]
    pub prefix: String,
}

async fn public_share(db: &PgPool, token: &str, pw: &str) -> Result<ShareRow, AppError> {
    let r: Option<ShareRow> = sqlx::query_as(
        "SELECT id, owner, path, is_dir, target_user, token, pass_hash, permission, expires_at, created_at FROM shares WHERE token = $1",
    )
    .bind(token)
    .fetch_optional(db)
    .await?;
    let r = r.ok_or_else(|| AppError::NotFound("link không tồn tại".into()))?;
    if expired(&r) {
        return Err(AppError::NotFound("link đã hết hạn".into()));
    }
    if let Some(h) = &r.pass_hash {
        if pw.is_empty() || !crate::auth::verify_password(pw, h) {
            return Err(AppError::Auth("sai mật khẩu link".into()));
        }
    }
    Ok(r)
}

#[derive(Serialize)]
pub struct LinkMeta {
    pub title: String,
    pub is_dir: bool,
    pub locked: bool,
    pub expires_at: Option<i64>,
}

/// Thông tin link (tên file/thư mục, có cần mật khẩu không).
pub async fn link_meta(State(s): State<AppState>, Path(token): Path<String>, Query(q): Query<LinkQuery>) -> Result<Json<LinkMeta>, AppError> {
    let r = public_share(&s.db, &token, &q.pw).await;
    match r {
        Ok(r) => Ok(Json(LinkMeta {
            title: r.path.rsplit('/').next().unwrap_or(&r.path).to_string(),
            is_dir: r.is_dir,
            locked: false,
            expires_at: r.expires_at,
        })),
        Err(AppError::Auth(_)) => {
            let locked: Option<(String, bool, Option<i64>)> =
                sqlx::query_as("SELECT path, is_dir, expires_at FROM shares WHERE token = $1").bind(&token).fetch_optional(&s.db).await?;
            match locked {
                Some((p, d, e)) => Ok(Json(LinkMeta {
                    title: p.rsplit('/').next().unwrap_or(&p).to_string(),
                    is_dir: d,
                    locked: true,
                    expires_at: e,
                })),
                None => Err(AppError::NotFound("link không tồn tại".into())),
            }
        }
        Err(e) => Err(e),
    }
}

/// Liệt kê trong thư mục được chia sẻ qua link.
pub async fn link_list(State(s): State<AppState>, Path(token): Path<String>, Query(q): Query<LinkQuery>) -> Result<Json<index::Page>, AppError> {
    let r = public_share(&s.db, &token, &q.pw).await?;
    let prefix = if q.prefix.is_empty() {
        if !r.is_dir {
            return Err(AppError::BadRequest("link này trỏ tới 1 file, dùng link tải trực tiếp".into()));
        }
        r.path.clone()
    } else {
        let rel = clean_path(&q.prefix).map_err(|_| AppError::BadRequest("prefix không hợp lệ".into()))?;
        if rel.is_empty() {
            r.path.clone()
        } else {
            format!("{}/{}", r.path, rel)
        }
    };
    if !(prefix == r.path || prefix.starts_with(&format!("{}/", r.path))) {
        return Err(AppError::Forbidden("ngoài phạm vi link".into()));
    }
    Ok(Json(index::list(&s.db, r.owner, &prefix, index::SortKey::default(), false, 0, 200).await?))
}

/// Tải file qua link công khai (stream trực tiếp, hỗ trợ Range).
/// Bản không kèm path (link trỏ tới 1 file).
pub async fn link_file_root(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    link_file(State(s), headers, Path((token, String::new())), Query(q)).await
}
pub async fn link_file(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((token, path)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let pw = q.get("pw").map(String::as_str).unwrap_or("");
    let r = public_share(&s.db, &token, pw).await?;
    let full = if path.is_empty() { r.path.clone() } else { format!("{}/{}", r.path, clean_path(&path).map_err(|_| AppError::BadRequest("path không hợp lệ".into()))?) };
    if !(full == r.path || full.starts_with(&format!("{}/", r.path))) {
        return Err(AppError::Forbidden("ngoài phạm vi link".into()));
    }
    if !r.is_dir && full != r.path {
        return Err(AppError::NotFound("not found".into()));
    }
    let meta = s.store.stat(r.owner, &full).await?;
    if meta.is_dir() {
        return Err(AppError::BadRequest("đây là thư mục".into()));
    }
    let size = meta.content_length();
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok()).map(|h| crate::file_handlers::parse_range(h, size));
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
    let key = s.store.key(r.owner, &full)?;
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
    let name = full.rsplit('/').next().unwrap_or("file");
    let mut res = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, crate::routes::content_type(&full))
        .header(header::CONTENT_LENGTH, end - start)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename*=UTF-8''{name}"));
    if status == StatusCode::PARTIAL_CONTENT {
        res = res.header(header::CONTENT_RANGE, format!("bytes {start}-{}/{size}", end - 1));
    }
    Ok(res.body(Body::from_stream(stream)).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Query, State};

    async fn setup() -> Option<(AppState, Uuid, Uuid)> {
        let Some((db, a)) = crate::kanban::tests::db_user().await else { return None };
        let b = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, username, pass_hash) VALUES ($1, $2, 'x')").bind(b).bind(format!("sh-{b}")).execute(&db).await.unwrap();
        let (store, _) = crate::files::tests::tmp_store("shares");
        let s = AppState {
            db: db.clone(),
            store,
            enc: jsonwebtoken::EncodingKey::from_secret(b"t"),
            dec: jsonwebtoken::DecodingKey::from_secret(b"t"),
            uploads: std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            http: reqwest::Client::new(),
            tts_url: String::new(),
            vaults: Default::default(),
        };
        // file của A
        sqlx::query("INSERT INTO files (user_id, path, parent, name, is_dir, size, modified) VALUES ($1, 'doc/a.txt', 'doc', 'a.txt', FALSE, 3, 0)")
            .bind(a)
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO files (user_id, path, parent, name, is_dir, size, modified) VALUES ($1, 'doc', '', 'doc', TRUE, 0, 0)")
            .bind(a)
            .execute(&db)
            .await
            .unwrap();
        Some((s, a, b))
    }

    fn auth(id: Uuid) -> AuthUser {
        AuthUser { id, admin: false }
    }

    #[tokio::test]
    async fn share_grant_and_revoke() {
        let Some((s, a, b)) = setup().await else { return };
        // chưa share: B không resolve được
        assert!(resolve(&s.db, b, Some(a), "doc/a.txt", false).await.is_err());
        // A share thư mục doc cho B (xem)
        share_with(State(s.clone()), auth(a), Json(ShareWith { path: "doc".into(), username: format!("sh-{b}"), permission: "view".into() }))
            .await
            .unwrap();
        // B đọc được file bên trong, không sửa được
        assert_eq!(resolve(&s.db, b, Some(a), "doc/a.txt", false).await.unwrap(), a);
        assert!(resolve(&s.db, b, Some(a), "doc/a.txt", true).await.is_err());
        // B không trèo ra ngoài
        assert!(resolve(&s.db, b, Some(a), "other/x".into(), false).await.is_err());
        // A thu hồi: B mất quyền
        let list = list_shares(State(s.clone()), auth(a), Query(SharesQuery { path: "doc".into() })).await.unwrap();
        unshare(State(s.clone()), auth(a), Path(list.0[0].id)).await.unwrap();
        assert!(resolve(&s.db, b, Some(a), "doc/a.txt", false).await.is_err());
    }

    #[tokio::test]
    async fn link_password_and_expiry() {
        let Some((s, a, _)) = setup().await else { return };
        let r = create_link(
            State(s.clone()),
            auth(a),
            Json(NewLink { path: "doc/a.txt".into(), password: "bi-mat".into(), expires_in_days: None }),
        )
        .await
        .unwrap()
        .1;
        let tok = r.0.token.clone().unwrap();
        // sai/không mật khẩu đều lỗi
        assert!(public_share(&s.db, &tok, "").await.is_err());
        assert!(public_share(&s.db, &tok, "sai").await.is_err());
        assert!(public_share(&s.db, &tok, "bi-mat").await.is_ok());
        // hết hạn
        sqlx::query("UPDATE shares SET expires_at = $1 WHERE token = $2").bind(now() - 1).bind(&tok).execute(&s.db).await.unwrap();
        assert!(public_share(&s.db, &tok, "bi-mat").await.is_err());
    }
}
