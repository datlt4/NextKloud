//! User + auth HTTP handlers. DB-backed; thin wrappers over auth:: + quota::.
use axum::{Json, extract::{Path, State}, http::StatusCode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::{create_jwt, hash_password, verify_password},
    quota::Quota,
    state::{AppError, AppState, AuthUser, UserRow, load_user},
};

#[derive(Deserialize)]
pub struct LoginBody {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginOut {
    pub token: String,
    pub must_reset: bool,
    pub is_admin: bool,
}

pub async fn login(State(s): State<AppState>, Json(b): Json<LoginBody>) -> Result<Json<LoginOut>, AppError> {
    let row = sqlx::query_as::<_, UserRow>("SELECT * FROM users WHERE username = $1")
        .bind(b.username.trim())
        .fetch_optional(&s.db)
        .await?;
    let Some(row) = row else {
        return Err(AppError::Auth("bad credentials".into()));
    };
    if !verify_password(&b.password, &row.pass_hash) {
        return Err(AppError::Auth("bad credentials".into()));
    }
    let token = create_jwt(row.id, row.is_admin, &s.enc).map_err(AppError::Internal)?;
    Ok(Json(LoginOut { token, must_reset: row.must_reset, is_admin: row.is_admin }))
}

/// Current user (quota, admin flag) — survives page reload, unlike login response.
pub async fn me(State(s): State<AppState>, user: AuthUser) -> Result<Json<UserRow>, AppError> {
    Ok(Json(load_user(&s.db, user.id).await?))
}

#[derive(Deserialize)]
pub struct PasswordBody {
    pub old: String,
    pub new: String,
}

/// Self-service change. Clears must_reset.
pub async fn change_password(
    State(s): State<AppState>,
    user: AuthUser,
    Json(b): Json<PasswordBody>,
) -> Result<StatusCode, AppError> {
    let row = load_user(&s.db, user.id).await?;
    if !verify_password(&b.old, &row.pass_hash) {
        return Err(AppError::Auth("old password wrong".into()));
    }
    let hash = hash_password(&b.new).map_err(AppError::BadRequest)?;
    sqlx::query("UPDATE users SET pass_hash = $1, must_reset = FALSE WHERE id = $2")
        .bind(hash)
        .bind(user.id)
        .execute(&s.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct CreateUserBody {
    pub username: String,
    pub password: String,
    #[serde(default = "default_quota")]
    pub quota_limit: i64,
    #[serde(default)]
    pub is_admin: bool,
}
fn default_quota() -> i64 {
    10 * 1024 * 1024 * 1024
}

/// Admin creates a user with a default password; user must reset on login.
pub async fn create_user(
    State(s): State<AppState>,
    admin: AuthUser,
    Json(b): Json<CreateUserBody>,
) -> Result<(StatusCode, Json<UserRow>), AppError> {
    if !admin.admin {
        return Err(AppError::Forbidden("admin only".into()));
    }
    let username = b.username.trim().to_string();
    if username.is_empty() || username.len() > 64 || username.contains('/') {
        return Err(AppError::BadRequest("bad username".into()));
    }
    let hash = hash_password(&b.password).map_err(AppError::BadRequest)?;
    let row = sqlx::query_as::<_, UserRow>(
        "INSERT INTO users (id, username, pass_hash, must_reset, quota_limit, is_admin)
         VALUES ($1, $2, $3, TRUE, $4, $5) RETURNING *",
    )
    .bind(Uuid::new_v4())
    .bind(username)
    .bind(hash)
    .bind(b.quota_limit.max(0))
    .bind(b.is_admin)
    .fetch_one(&s.db)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(d) if d.constraint() == Some("users_username_key") => {
            AppError::Conflict("username taken".into())
        }
        e => AppError::Db(e),
    })?;
    Ok((StatusCode::CREATED, Json(row)))
}

pub async fn list_users(State(s): State<AppState>, admin: AuthUser) -> Result<Json<Vec<UserRow>>, AppError> {
    if !admin.admin {
        return Err(AppError::Forbidden("admin only".into()));
    }
    let rows = sqlx::query_as::<_, UserRow>("SELECT * FROM users ORDER BY username").fetch_all(&s.db).await?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
pub struct QuotaBody {
    pub quota_limit: i64,
}

/// Admin expand/shrink. Shrinking below usage is allowed (user read-only until freed).
pub async fn set_quota(
    State(s): State<AppState>,
    admin: AuthUser,
    Path(id): Path<Uuid>,
    Json(b): Json<QuotaBody>,
) -> Result<Json<UserRow>, AppError> {
    if !admin.admin {
        return Err(AppError::Forbidden("admin only".into()));
    }
    let mut row = load_user(&s.db, id).await?;
    let mut q = Quota::new(row.quota_limit.max(0) as u64, row.quota_used.max(0) as u64);
    q.resize(b.quota_limit.max(0) as u64);
    row = sqlx::query_as::<_, UserRow>("UPDATE users SET quota_limit = $1 WHERE id = $2 RETURNING *")
        .bind(b.quota_limit.max(0))
        .bind(id)
        .fetch_one(&s.db)
        .await?;
    let _ = q;
    Ok(Json(row))
}

#[derive(Deserialize)]
pub struct ResetBody {
    pub password: String,
}

/// Admin forces a new default password; user must reset on next login.
pub async fn reset_password(
    State(s): State<AppState>,
    admin: AuthUser,
    Path(id): Path<Uuid>,
    Json(b): Json<ResetBody>,
) -> Result<StatusCode, AppError> {
    if !admin.admin {
        return Err(AppError::Forbidden("admin only".into()));
    }
    let hash = hash_password(&b.password).map_err(AppError::BadRequest)?;
    let n = sqlx::query("UPDATE users SET pass_hash = $1, must_reset = TRUE WHERE id = $2")
        .bind(hash)
        .bind(id)
        .execute(&s.db)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound("user not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}
