//! Chia sẻ file/thư mục: giữa các user (quyền xem/sửa) + link công khai (mật khẩu, hạn).
//! File trong Bí mật nằm dưới `.nextkloud` nên `user_path` đã chặn, không chia sẻ được.
use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
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
    /// người tạo share (None = chủ file), Some = chia sẻ lại
    granted_by: Option<Uuid>,
    #[allow(dead_code)]
    parent_id: Option<Uuid>,
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
    match grant(db, me, o, &p).await? {
        Some(r) if !need_edit || r.permission == "edit" => Ok(o),
        Some(_) => Err(AppError::Forbidden("bạn chỉ được xem, không được sửa".into())),
        None => Err(AppError::NotFound("không có quyền truy cập".into())),
    }
}

/// Share còn hạn cho `me` phủ path `p` (đã clean) của owner; ưu tiên share có quyền sửa.
async fn grant(db: &PgPool, me: Uuid, o: Uuid, p: &str) -> Result<Option<ShareRow>, AppError> {
    // vùng hệ thống (.nextkloud: Bí mật, thumbnail cache...) không bao giờ lộ cho người khác
    if ops::is_reserved(p) {
        return Ok(None);
    }
    let rows: Vec<ShareRow> = sqlx::query_as(
        "SELECT id, owner, path, is_dir, target_user, token, pass_hash, permission, expires_at, created_at, granted_by, parent_id
         FROM shares WHERE owner = $1 AND target_user = $2 AND (expires_at IS NULL OR expires_at > $3)",
    )
    .bind(o)
    .bind(me)
    .bind(now())
    .fetch_all(db)
    .await?;
    let mut hit: Option<ShareRow> = None;
    for r in rows {
        if p == r.path || p.starts_with(&format!("{}/", r.path)) {
            if r.permission == "edit" {
                return Ok(Some(r));
            }
            hit = hit.or(Some(r));
        }
    }
    Ok(hit)
}

// ---------- owner: quản lý chia sẻ ----------

#[derive(Debug, Serialize)]
pub struct ShareOut {
    pub id: Uuid,
    pub path: String,
    pub is_dir: bool,
    pub target: Option<String>,
    /// username người chia sẻ lại (None = chủ file tạo)
    pub granted_by: Option<String>,
    pub token: Option<String>,
    pub has_password: bool,
    pub permission: String,
    pub expires_at: Option<i64>,
    pub created_at: i64,
}

async fn username(db: &PgPool, u: Option<Uuid>) -> Result<Option<String>, AppError> {
    let Some(u) = u else { return Ok(None) };
    let row: Option<(String,)> = sqlx::query_as("SELECT username FROM users WHERE id = $1").bind(u).fetch_optional(db).await?;
    Ok(row.map(|(n,)| n))
}

async fn share_out(db: &PgPool, r: ShareRow) -> Result<ShareOut, AppError> {
    let target = username(db, r.target_user).await?;
    let granted_by = username(db, r.granted_by).await?;
    Ok(ShareOut {
        id: r.id,
        path: r.path,
        is_dir: r.is_dir,
        target,
        granted_by,
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
    /// mục được chia sẻ: chỉ liệt kê các share lại do chính tôi tạo
    #[serde(default)]
    pub owner: Option<Uuid>,
}

/// Các chia sẻ/link của chính mình (lọc theo path nếu có). Chủ file thấy cả share lại của người nhận.
pub async fn list_shares(State(s): State<AppState>, user: AuthUser, Query(q): Query<SharesQuery>) -> Result<Json<Vec<ShareOut>>, AppError> {
    let rows: Vec<ShareRow> = if let Some(o) = q.owner.filter(|o| *o != user.id) {
        let p = ops::user_path(&q.path)?;
        sqlx::query_as(
            "SELECT id, owner, path, is_dir, target_user, token, pass_hash, permission, expires_at, created_at, granted_by, parent_id
             FROM shares WHERE owner = $1 AND path = $2 AND granted_by = $3 ORDER BY created_at DESC",
        )
        .bind(o)
        .bind(p)
        .bind(user.id)
        .fetch_all(&s.db)
        .await?
    } else if q.path.is_empty() {
        sqlx::query_as(
            "SELECT id, owner, path, is_dir, target_user, token, pass_hash, permission, expires_at, created_at, granted_by, parent_id
             FROM shares WHERE owner = $1 ORDER BY created_at DESC LIMIT 200",
        )
        .bind(user.id)
        .fetch_all(&s.db)
        .await?
    } else {
        let p = ops::user_path(&q.path)?;
        sqlx::query_as(
            "SELECT id, owner, path, is_dir, target_user, token, pass_hash, permission, expires_at, created_at, granted_by, parent_id
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
    /// chia sẻ lại mục người khác chia sẻ cho tôi
    #[serde(default)]
    pub owner: Option<Uuid>,
}
fn view_perm() -> String {
    "view".into()
}

/// Chia sẻ cho 1 user. owner = người khác -> chia sẻ lại (quyền ≤ quyền mình, hạn theo share gốc).
pub async fn share_with(State(s): State<AppState>, user: AuthUser, Json(b): Json<ShareWith>) -> Result<(StatusCode, Json<ShareOut>), AppError> {
    if b.permission != "view" && b.permission != "edit" {
        return Err(AppError::BadRequest("permission phải là view hoặc edit".into()));
    }
    let p = ops::user_path(&b.path)?;
    let o = b.owner.unwrap_or(user.id);
    let parent = if o == user.id {
        None
    } else {
        let g = grant(&s.db, user.id, o, &p).await?.ok_or_else(|| AppError::NotFound("không có quyền truy cập".into()))?;
        if b.permission == "edit" && g.permission != "edit" {
            return Err(AppError::Forbidden("bạn chỉ được xem nên chỉ chia sẻ lại được quyền xem".into()));
        }
        Some(g)
    };
    let is_dir = ops::kind(&s.db, o, &p).await?.ok_or_else(|| AppError::NotFound("not found".into()))?;
    let target: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE username = $1").bind(b.username.trim()).fetch_optional(&s.db).await?;
    let (tid,) = target.ok_or_else(|| AppError::NotFound("không có user này".into()))?;
    if tid == user.id {
        return Err(AppError::BadRequest("không thể chia sẻ cho chính mình".into()));
    }
    if tid == o {
        return Err(AppError::BadRequest("người này là chủ file".into()));
    }
    let dup: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM shares WHERE owner = $1 AND path = $2 AND target_user = $3")
        .bind(o)
        .bind(&p)
        .bind(tid)
        .fetch_optional(&s.db)
        .await?;
    if dup.is_some() {
        return Err(AppError::Conflict("đã chia sẻ cho user này".into()));
    }
    let r: ShareRow = sqlx::query_as(
        "INSERT INTO shares (id, owner, path, is_dir, target_user, permission, created_at, granted_by, parent_id, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         RETURNING id, owner, path, is_dir, target_user, token, pass_hash, permission, expires_at, created_at, granted_by, parent_id",
    )
    .bind(Uuid::new_v4())
    .bind(o)
    .bind(&p)
    .bind(is_dir)
    .bind(tid)
    .bind(b.permission)
    .bind(now())
    .bind(parent.as_ref().map(|_| user.id))
    .bind(parent.as_ref().map(|g| g.id))
    .bind(parent.as_ref().and_then(|g| g.expires_at))
    .fetch_one(&s.db)
    .await?;
    let sharer = crate::state::load_user(&s.db, user.id)
        .await
        .map(|u| u.display_name.unwrap_or(u.username))
        .unwrap_or_default();
    crate::events::emit(tid, crate::events::Ev::SharedWithYou { path: p.clone(), from: sharer });
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
/// Chỉ chủ file: người nhận không tạo link được (path luôn tính trong cây của chính mình).
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
         RETURNING id, owner, path, is_dir, target_user, token, pass_hash, permission, expires_at, created_at, granted_by, parent_id",
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

/// Xoá chia sẻ/link (chủ file, hoặc người đã chia sẻ lại). Người từng được chia sẻ mất quyền ngay,
/// các share lại tạo từ share này cũng mất theo (FK parent_id ON DELETE CASCADE).
pub async fn unshare(State(s): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> Result<StatusCode, AppError> {
    let n = sqlx::query("DELETE FROM shares WHERE id = $1 AND (owner = $2 OR granted_by = $2)").bind(id).bind(user.id).execute(&s.db).await?.rows_affected();
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
    /// username người chia sẻ lại cho tôi (None = chủ file)
    pub via: Option<String>,
}

/// Tất cả file/thư mục người khác chia sẻ cho tôi (còn hạn).
pub async fn shared_with_me(State(s): State<AppState>, user: AuthUser) -> Result<Json<Vec<SharedItem>>, AppError> {
    Ok(Json(
        sqlx::query_as(
            "SELECT s.id, s.owner AS owner_id, u.username AS owner_name, s.path, s.is_dir, s.permission, s.expires_at, g.username AS via
             FROM shares s JOIN users u ON u.id = s.owner LEFT JOIN users g ON g.id = s.granted_by
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
pub struct SharedSearch {
    pub owner: Uuid,
    /// thư mục gốc được chia sẻ (hoặc thư mục con của nó)
    pub root: String,
    pub q: String,
    pub limit: Option<i64>,
}

/// Tìm theo tên trong cây được chia sẻ; chỉ trả path nằm dưới root.
pub async fn shared_search(State(s): State<AppState>, user: AuthUser, Query(q): Query<SharedSearch>) -> Result<Json<Vec<index::Row>>, AppError> {
    let root = ops::user_path(&q.root)?;
    let o = resolve(&s.db, user.id, Some(q.owner), &root, false).await?;
    let term = q.q.trim();
    if term.is_empty() {
        return Ok(Json(vec![]));
    }
    Ok(Json(index::search_under(&s.db, o, &root, term, q.limit.unwrap_or(200).clamp(1, 1000)).await?))
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
    #[serde(default)]
    pub sort: index::SortKey,
    #[serde(default)]
    pub desc: bool,
    #[serde(default)]
    pub offset: i64,
    pub limit: Option<i64>,
}

async fn public_share(db: &PgPool, token: &str, pw: &str) -> Result<ShareRow, AppError> {
    let r: Option<ShareRow> = sqlx::query_as(
        "SELECT id, owner, path, is_dir, target_user, token, pass_hash, permission, expires_at, created_at, granted_by, parent_id FROM shares WHERE token = $1",
    )
    .bind(token)
    .fetch_optional(db)
    .await?;
    let r = r.ok_or_else(|| AppError::NotFound("link không tồn tại".into()))?;
    if expired(&r) {
        return Err(AppError::NotFound("link đã hết hạn".into()));
    }
    if let Some(h) = &r.pass_hash {
        if pw.is_empty() {
            return Err(AppError::Auth("sai mật khẩu link".into()));
        }
        // chống dò mật khẩu: sai 5 lần thì khoá link 5 phút (cũng đỡ tốn CPU hash)
        let now = chrono::Utc::now().timestamp();
        let locked = PW_FAILS.lock().unwrap().get(token).is_some_and(|&(n, at)| n >= 5 && now - at < 300);
        if locked {
            return Err(AppError::Forbidden("sai mật khẩu quá nhiều lần, thử lại sau 5 phút".into()));
        }
        if !crate::auth::verify_password(pw, h) {
            let mut m = PW_FAILS.lock().unwrap();
            m.retain(|_, &mut (_, at)| now - at < 300);
            let e = m.entry(token.to_string()).or_insert((0, now));
            *e = (e.0 + 1, now);
            return Err(AppError::Auth("sai mật khẩu link".into()));
        }
        PW_FAILS.lock().unwrap().remove(token);
    }
    Ok(r)
}

/// token link -> (số lần sai mật khẩu, lần sai gần nhất)
static PW_FAILS: std::sync::LazyLock<std::sync::Mutex<HashMap<String, (u32, i64)>>> = std::sync::LazyLock::new(Default::default);

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

/// Đường dẫn trả cho khách: tương đối so với gốc link (không lộ cây thư mục của chủ).
fn rel_of(r: &ShareRow, full: &str) -> String {
    if r.is_dir { full.strip_prefix(&format!("{}/", r.path)).unwrap_or(full).to_string() } else { full.rsplit('/').next().unwrap_or(full).to_string() }
}

/// Liệt kê trong thư mục được chia sẻ qua link (path trả về tương đối so với gốc link).
/// Link trỏ tới 1 file: trả đúng 1 dòng là file đó.
pub async fn link_list(State(s): State<AppState>, Path(token): Path<String>, Query(q): Query<LinkQuery>) -> Result<Json<index::Page>, AppError> {
    let r = public_share(&s.db, &token, &q.pw).await?;
    if !r.is_dir {
        let row: Option<index::Row> = sqlx::query_as("SELECT path, name, is_dir, size, modified FROM files WHERE user_id = $1 AND path = $2")
            .bind(r.owner)
            .bind(&r.path)
            .fetch_optional(&s.db)
            .await?;
        let row = row.ok_or_else(|| AppError::NotFound("file không còn tồn tại".into()))?;
        let bytes = row.size;
        let entries = if q.offset > 0 { vec![] } else { vec![index::Row { path: rel_of(&r, &row.path), ..row }] };
        return Ok(Json(index::Page { entries, dirs: 0, files: 1, bytes }));
    }
    let prefix = scope_path(&r, &q.prefix)?;
    let limit = q.limit.unwrap_or(200).clamp(0, 1000);
    let mut page = index::list(&s.db, r.owner, &prefix, q.sort, q.desc, q.offset.max(0), limit).await?;
    for e in &mut page.entries {
        e.path = rel_of(&r, &e.path);
    }
    Ok(Json(page))
}

#[derive(Deserialize)]
pub struct LinkSearch {
    #[serde(default)]
    pub pw: String,
    pub q: String,
}

/// Tìm theo tên trong phạm vi link.
pub async fn link_search(State(s): State<AppState>, Path(token): Path<String>, Query(q): Query<LinkSearch>) -> Result<Json<Vec<index::Row>>, AppError> {
    let r = public_share(&s.db, &token, &q.pw).await?;
    let term = q.q.trim();
    if term.is_empty() || !r.is_dir {
        return Ok(Json(vec![]));
    }
    let mut v = index::search_under(&s.db, r.owner, &r.path, term, 300).await?;
    for e in &mut v {
        e.path = rel_of(&r, &e.path);
    }
    Ok(Json(v))
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
    let full = scope_path(&r, &path)?;
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
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename*=UTF-8''{}", crate::zip::pct(&name)));
    if status == StatusCode::PARTIAL_CONTENT {
        res = res.header(header::CONTENT_RANGE, format!("bytes {start}-{}/{size}", end - 1));
    }
    Ok(res.body(Body::from_stream(stream)).unwrap())
}

/// Ràng buộc path nằm trong phạm vi link (gốc share hoặc bên dưới).
fn scope_path(r: &ShareRow, rel: &str) -> Result<String, AppError> {
    let rel = clean_path(rel).map_err(|_| AppError::BadRequest("path không hợp lệ".into()))?;
    // link trỏ tới 1 file: chỉ cho phép rỗng / đúng file / tên file
    if !r.is_dir {
        let name = r.path.rsplit('/').next().unwrap_or(&r.path);
        if rel.is_empty() || rel == r.path || rel == name {
            return Ok(r.path.clone());
        }
        return Err(AppError::Forbidden("ngoài phạm vi link".into()));
    }
    let full = if rel.is_empty() { r.path.clone() } else { format!("{}/{}", r.path, rel) };
    if !(full == r.path || full.starts_with(&format!("{}/", r.path))) {
        return Err(AppError::Forbidden("ngoài phạm vi link".into()));
    }
    Ok(full)
}

/// Đọc text (md/txt/drawio/excalidraw...) qua link công khai, tối đa 5MB.
pub async fn link_text(
    State(s): State<AppState>,
    Path(token): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let pw = q.get("pw").map(String::as_str).unwrap_or("");
    let r = public_share(&s.db, &token, pw).await?;
    let full = scope_path(&r, q.get("path").cloned().unwrap_or_default().as_str())?;
    let meta = s.store.stat(r.owner, &full).await.map_err(AppError::from)?;
    if meta.is_dir() {
        return Err(AppError::BadRequest("đây là thư mục".into()));
    }
    if meta.content_length() > 5 * 1024 * 1024 {
        return Err(AppError::BadRequest("file quá lớn để xem trực tiếp (>5MB), hãy tải về".into()));
    }
    let key = s.store.key(r.owner, &full)?;
    let data = s.store.op.read(&key).await.map_err(crate::files::StoreError::from)?;
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(data.to_vec()))
        .unwrap())
}

/// Thumbnail qua link công khai (cache chung với chủ file).
pub async fn link_thumb(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let pw = q.get("pw").map(String::as_str).unwrap_or("");
    let r = public_share(&s.db, &token, pw).await?;
    let full = scope_path(&r, q.get("path").cloned().unwrap_or_default().as_str())?;
    let is_img = crate::thumb::thumb_kind(&full).ok_or_else(|| AppError::BadRequest("loại file này chưa có thumbnail".into()))?;
    let size_px: u32 = q.get("s").and_then(|v| v.parse().ok()).unwrap_or(256).clamp(64, 2048);
    let meta = s.store.stat(r.owner, &full).await.map_err(AppError::from)?;
    if meta.is_dir() {
        return Err(AppError::BadRequest("not a file".into()));
    }
    let mtime = crate::files::mtime_of(&meta);
    let size = meta.content_length();
    let hash = crate::thumb::thumb_key(&r.owner, &full, mtime, size, size_px);
    let tkey = s.store.key(r.owner, &crate::thumb::thumb_object(&hash))?;
    let etag = format!("\"{hash}\"");
    if let Some(inm) = headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) {
        if inm.split(',').any(|t| t.trim() == etag) {
            return Ok(StatusCode::NOT_MODIFIED.into_response());
        }
    }
    if let Ok(cached) = s.store.op.read(&tkey).await {
        return Ok(Response::builder()
            .header(header::CONTENT_TYPE, crate::thumb::mime_of(&cached.to_vec()))
            .header(header::ETAG, &etag)
            .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
            .body(Body::from(cached.to_vec()))
            .unwrap());
    }
    let src = if is_img {
        if size > 100 * 1024 * 1024 {
            return Err(AppError::BadRequest("ảnh quá lớn để tạo thumbnail".into()));
        }
        s.store.op.read(&s.store.key(r.owner, &full)?).await.map_err(crate::files::StoreError::from)?.to_vec()
    } else {
        crate::thumb::video_frame_png(r.owner, &full, &s.store).await.map_err(AppError::Internal)?
    };
    let out = tokio::task::spawn_blocking(move || crate::thumb::render_image(&src, size_px))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .map_err(AppError::Internal)?;
    let _ = s.store.op.write(&tkey, out.clone()).await;
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, crate::thumb::mime_of(&out))
        .header(header::ETAG, &etag)
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .body(Body::from(out))
        .unwrap())
}

/// Tải zip qua link công khai: cả thư mục `prefix` (mặc định gốc link) hoặc các mục `p` (lặp) nằm trong `prefix`.
pub async fn link_zip(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
    Query(all): Query<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    let get = |k: &str| all.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str()).unwrap_or("");
    let r = public_share(&s.db, &token, get("pw")).await?;
    let base = scope_path(&r, get("prefix"))?;
    let picks = all.iter().filter(|(k, _)| k == "p").map(|(_, v)| scope_path(&r, v)).collect::<Result<Vec<_>, _>>()?;
    let files = crate::zip::collect(&s.db, &s.store, r.owner, &base, &picks).await?;
    crate::zip::respond_zip(&s, &headers, r.owner, files, base).await
}

/// Cấu hình OnlyOffice xem trước qua link công khai (luôn chỉ xem, không lưu).
#[derive(Deserialize)]
pub struct LinkOfficeBody {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub pw: String,
}

pub async fn link_office_config(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
    Json(b): Json<LinkOfficeBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let r = public_share(&s.db, &token, &b.pw).await?;
    let full = scope_path(&r, &b.path)?;
    let (file_type, document_type) = crate::office::doc_type(&full).ok_or_else(|| AppError::BadRequest("loại office này chưa hỗ trợ".into()))?;
    let meta = s.store.stat(r.owner, &full).await.map_err(AppError::from)?;
    if meta.is_dir() {
        return Err(AppError::BadRequest("not a file".into()));
    }
    let mtime = crate::files::mtime_of(&meta);
    let size = meta.content_length();
    let key = crate::office::doc_key(&r.owner, &full, mtime, size);
    let base = std::env::var("ONLYOFFICE_BACKEND_URL").ok().filter(|v| !v.trim().is_empty()).map(|v| v.trim_end_matches('/').to_string()).unwrap_or_else(|| crate::office::public_base(&headers));
    let dl = crate::office::dl_token(r.owner, &full, &s.enc)?;
    let name = full.rsplit('/').next().unwrap_or(&full);
    let cfg = serde_json::json!({
        "document": {
            "fileType": file_type,
            "key": key,
            "title": name,
            "url": format!("{base}/api/office/file?t={dl}"),
            "permissions": { "edit": false },
        },
        "documentType": document_type,
        "editorConfig": {
            "user": { "id": "guest", "name": "Khách" },
            "mode": "view",
            "lang": "vi",
        },
    });
    let secret = crate::office::oo_secret()?;
    let signed = jsonwebtoken::encode(&jsonwebtoken::Header::default(), &cfg, &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()))
        .map_err(|e| AppError::Internal(format!("ký token office: {e}")))?;
    Ok(Json(serde_json::json!({ "config": cfg, "token": signed })))
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
        share_with(State(s.clone()), auth(a), Json(ShareWith { path: "doc".into(), username: format!("sh-{b}"), permission: "view".into(), owner: None }))
            .await
            .unwrap();
        // B đọc được file bên trong, không sửa được
        assert_eq!(resolve(&s.db, b, Some(a), "doc/a.txt", false).await.unwrap(), a);
        assert!(resolve(&s.db, b, Some(a), "doc/a.txt", true).await.is_err());
        // B không trèo ra ngoài
        assert!(resolve(&s.db, b, Some(a), "other/x".into(), false).await.is_err());
        // A thu hồi: B mất quyền
        let list = list_shares(State(s.clone()), auth(a), Query(SharesQuery { path: "doc".into(), owner: None })).await.unwrap();
        unshare(State(s.clone()), auth(a), Path(list.0[0].id)).await.unwrap();
        assert!(resolve(&s.db, b, Some(a), "doc/a.txt", false).await.is_err());
    }

    #[tokio::test]
    async fn link_password_and_expiry() {
        let Some((s, a, _)) = setup().await else { return };
        let r = create_link(
            State(s.clone()),
            auth(a),
            Json(NewLink { path: "doc/a.txt".into(), password: "bi-mat-123".into(), expires_in_days: None }),
        )
        .await
        .unwrap()
        .1;
        let tok = r.0.token.clone().unwrap();
        // sai/không mật khẩu đều lỗi
        assert!(public_share(&s.db, &tok, "").await.is_err());
        assert!(public_share(&s.db, &tok, "sai-mat-khau").await.is_err());
        assert!(public_share(&s.db, &tok, "bi-mat-123").await.is_ok());
        // hết hạn
        sqlx::query("UPDATE shares SET expires_at = $1 WHERE token = $2").bind(now() - 1).bind(&tok).execute(&s.db).await.unwrap();
        assert!(public_share(&s.db, &tok, "bi-mat-123").await.is_err());
    }

    async fn new_user(db: &PgPool) -> Uuid {
        let u = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, username, pass_hash) VALUES ($1, $2, 'x')").bind(u).bind(format!("sh-{u}")).execute(db).await.unwrap();
        u
    }

    async fn give(s: &AppState, from: Uuid, owner: Option<Uuid>, path: &str, to: Uuid, perm: &str) -> Result<ShareOut, AppError> {
        let b = ShareWith { path: path.into(), username: format!("sh-{to}"), permission: perm.into(), owner };
        share_with(State(s.clone()), auth(from), Json(b)).await.map(|r| r.1 .0)
    }

    async fn add_file(db: &PgPool, u: Uuid, path: &str) {
        crate::index::add_file(db, u, path, 3, 0).await.unwrap();
    }

    #[tokio::test]
    async fn resolve_stays_in_subtree() {
        let Some((s, a, b)) = setup().await else { return };
        give(&s, a, None, "doc", b, "view").await.unwrap();
        assert!(resolve(&s.db, b, Some(a), "doc", false).await.is_ok());
        // "doc2" chỉ trùng tiền tố chuỗi, không nằm trong "doc"
        assert!(resolve(&s.db, b, Some(a), "doc2/x", false).await.is_err());
        assert!(resolve(&s.db, b, Some(a), "doc/../other/x", false).await.is_err());
        assert!(resolve(&s.db, b, Some(a), "", false).await.is_err());
        // kể cả có dòng share trỏ vào .nextkloud (không tạo được qua API), vùng hệ thống vẫn bị chặn
        sqlx::query("INSERT INTO shares (id, owner, path, is_dir, target_user, permission, created_at) VALUES ($1, $2, '.nextkloud', TRUE, $3, 'edit', 0)")
            .bind(Uuid::new_v4())
            .bind(a)
            .bind(b)
            .execute(&s.db)
            .await
            .unwrap();
        assert!(resolve(&s.db, b, Some(a), ".nextkloud/thumbs/x.webp", false).await.is_err());
        assert!(resolve(&s.db, b, Some(a), ".nextkloud/secret/k", false).await.is_err());
    }

    #[tokio::test]
    async fn reshare_rules_and_cascade() {
        let Some((s, a, b)) = setup().await else { return };
        let (c, d) = (new_user(&s.db).await, new_user(&s.db).await);
        // chưa được chia sẻ: không chia sẻ lại được
        assert!(give(&s, b, Some(a), "doc", c, "view").await.is_err());
        let ab = give(&s, a, None, "doc", b, "view").await.unwrap();
        let exp = now() + 3600;
        sqlx::query("UPDATE shares SET expires_at = $1 WHERE id = $2").bind(exp).bind(ab.id).execute(&s.db).await.unwrap();
        // quyền xem không nâng lên sửa được
        assert!(matches!(give(&s, b, Some(a), "doc/a.txt", c, "edit").await, Err(AppError::Forbidden(_))));
        // không chia sẻ ngoài vùng được cấp, không chia sẻ lại cho chủ file
        add_file(&s.db, a, "other/x.txt").await;
        assert!(give(&s, b, Some(a), "other/x.txt", c, "view").await.is_err());
        assert!(give(&s, b, Some(a), "doc", a, "view").await.is_err());
        // chia sẻ lại quyền xem: C đọc được, không sửa được, hạn theo share gốc
        let bc = give(&s, b, Some(a), "doc", c, "view").await.unwrap();
        assert_eq!(bc.granted_by, Some(format!("sh-{b}")));
        assert_eq!(bc.expires_at, Some(exp));
        assert_eq!(resolve(&s.db, c, Some(a), "doc/a.txt", false).await.unwrap(), a);
        assert!(resolve(&s.db, c, Some(a), "doc/a.txt", true).await.is_err());
        // C chia tiếp cho D (chuỗi)
        give(&s, c, Some(a), "doc/a.txt", d, "view").await.unwrap();
        assert!(resolve(&s.db, d, Some(a), "doc/a.txt", false).await.is_ok());
        // chủ file thấy mọi share của mục; B chỉ thấy share do B tạo
        let all = list_shares(State(s.clone()), auth(a), Query(SharesQuery { path: "doc".into(), owner: None })).await.unwrap().0;
        assert_eq!(all.len(), 2);
        let mine = list_shares(State(s.clone()), auth(b), Query(SharesQuery { path: "doc".into(), owner: Some(a) })).await.unwrap().0;
        assert_eq!(mine.iter().map(|r| r.id).collect::<Vec<_>>(), vec![bc.id]);
        // C không thu hồi được share của B; người nhận không tạo link công khai được
        assert!(unshare(State(s.clone()), auth(c), Path(ab.id)).await.is_err());
        assert!(create_link(State(s.clone()), auth(b), Json(NewLink { path: "doc".into(), password: String::new(), expires_in_days: None })).await.is_err());
        // chủ thu hồi share của B -> C, D mất quyền theo
        unshare(State(s.clone()), auth(a), Path(ab.id)).await.unwrap();
        assert!(resolve(&s.db, b, Some(a), "doc/a.txt", false).await.is_err());
        assert!(resolve(&s.db, c, Some(a), "doc/a.txt", false).await.is_err());
        assert!(resolve(&s.db, d, Some(a), "doc/a.txt", false).await.is_err());
        let left: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM shares WHERE owner = $1").bind(a).fetch_one(&s.db).await.unwrap();
        assert_eq!(left.0, 0);
    }

    #[tokio::test]
    async fn reshare_edit_and_self_revoke() {
        let Some((s, a, b)) = setup().await else { return };
        let c = new_user(&s.db).await;
        give(&s, a, None, "doc", b, "edit").await.unwrap();
        let bc = give(&s, b, Some(a), "doc", c, "edit").await.unwrap();
        assert!(resolve(&s.db, c, Some(a), "doc/a.txt", true).await.is_ok());
        // B tự thu hồi share mình đã tạo
        unshare(State(s.clone()), auth(b), Path(bc.id)).await.unwrap();
        assert!(resolve(&s.db, c, Some(a), "doc/a.txt", false).await.is_err());
        assert!(resolve(&s.db, b, Some(a), "doc/a.txt", true).await.is_ok());
    }

    #[tokio::test]
    async fn shared_search_scoped() {
        let Some((s, a, b)) = setup().await else { return };
        for p in ["doc/report-1.txt", "doc/sub/report-2.txt", "doc2/report.txt", "other/report.txt", "doc/.h/report.txt"] {
            add_file(&s.db, a, p).await;
        }
        let q = |root: &str| Query(SharedSearch { owner: a, root: root.into(), q: "report".into(), limit: None });
        assert!(shared_search(State(s.clone()), auth(b), q("doc")).await.is_err());
        give(&s, a, None, "doc", b, "view").await.unwrap();
        let mut got: Vec<String> = shared_search(State(s.clone()), auth(b), q("doc")).await.unwrap().0.into_iter().map(|r| r.path).collect();
        got.sort();
        assert_eq!(got, vec!["doc/report-1.txt", "doc/sub/report-2.txt"]);
        let sub: Vec<String> = shared_search(State(s.clone()), auth(b), q("doc/sub")).await.unwrap().0.into_iter().map(|r| r.path).collect();
        assert_eq!(sub, vec!["doc/sub/report-2.txt"]);
        assert!(shared_search(State(s.clone()), auth(b), q("doc2")).await.is_err());
        assert!(shared_search(State(s.clone()), auth(b), q("doc/..")).await.is_err());
        assert!(shared_search(State(s.clone()), auth(b), q("")).await.is_err());
    }

    #[tokio::test]
    async fn office_access_follows_permission() {
        let Some((s, a, b)) = setup().await else { return };
        assert!(crate::office::access(&s.db, b, Some(a), "doc/a.docx").await.is_err());
        let ab = give(&s, a, None, "doc", b, "view").await.unwrap();
        assert_eq!(crate::office::access(&s.db, b, Some(a), "doc/a.docx").await.unwrap(), (a, false));
        unshare(State(s.clone()), auth(a), Path(ab.id)).await.unwrap();
        give(&s, a, None, "doc", b, "edit").await.unwrap();
        assert_eq!(crate::office::access(&s.db, b, Some(a), "doc/a.docx").await.unwrap(), (a, true));
        assert_eq!(crate::office::access(&s.db, a, None, "doc/a.docx").await.unwrap(), (a, true));
    }

    #[tokio::test]
    async fn thumb_and_zip_need_share() {
        use image::ImageEncoder as _;
        let Some((mut s, a, b)) = setup().await else { return };
        s.store = crate::files::tests::tmp_store(&format!("sh-{a}")).0;
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png).write_image(&[255u8, 0, 0, 255].repeat(16), 4, 4, image::ExtendedColorType::Rgba8).unwrap();
        s.store.op.write(&format!("{a}/doc/p.png"), png).await.unwrap();
        s.store.op.write(&format!("{a}/doc/a.txt"), b"abc".to_vec()).await.unwrap();
        s.store.op.write(&format!("{a}/other/q.png"), b"x".to_vec()).await.unwrap();
        add_file(&s.db, a, "other/q.png").await;
        let tok = crate::auth::create_jwt(b, false, &s.enc).unwrap();
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, format!("Bearer {tok}").parse().unwrap());
        let q = |pairs: &[(&str, String)]| Query(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect::<HashMap<_, _>>());
        let thumb = |p: &str| crate::thumb::thumb(State(s.clone()), h.clone(), Path(p.to_string()), q(&[("owner", a.to_string())]));
        let zip = |p: &str| crate::zip::download_dir(State(s.clone()), h.clone(), Query(vec![("owner".into(), a.to_string()), ("path".into(), p.to_string())]));
        // chưa chia sẻ: bị chặn
        assert!(thumb("doc/p.png").await.is_err());
        assert!(zip("doc").await.is_err());
        give(&s, a, None, "doc", b, "view").await.unwrap();
        assert_eq!(thumb("doc/p.png").await.unwrap().status(), StatusCode::OK);
        // thumbnail đã cache trong cây của A, nhưng B không đọc thẳng được cache / file ngoài vùng
        let hash = {
            let m = s.store.stat(a, "doc/p.png").await.unwrap();
            crate::thumb::thumb_key(&a, "doc/p.png", crate::files::mtime_of(&m), m.content_length(), 256)
        };
        assert!(s.store.op.stat(&format!("{a}/.nextkloud/thumbs/{hash}.webp")).await.is_ok());
        assert!(thumb(&format!(".nextkloud/thumbs/{hash}.webp")).await.is_err());
        assert!(thumb("other/q.png").await.is_err());
        // zip: chỉ đúng thư mục được chia sẻ
        let res = zip("doc").await.unwrap();
        let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        assert!(body.windows(5).any(|w| w == b"a.txt") && body.windows(3).any(|w| w == b"abc"));
        assert!(zip("other").await.is_err());
        assert!(zip("").await.is_err());
    }
}
