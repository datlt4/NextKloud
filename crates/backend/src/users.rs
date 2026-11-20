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

/// Admin xoá user: dữ liệu trên store + chỉ mục (cascade). Không tự xoá chính mình.
pub async fn delete_user(
    State(s): State<AppState>,
    admin: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    if !admin.admin {
        return Err(AppError::Forbidden("admin only".into()));
    }
    if admin.id == id {
        return Err(AppError::Conflict("cannot delete yourself".into()));
    }
    load_user(&s.db, id).await.map_err(|_| AppError::NotFound("user not found".into()))?;
    s.store.op.delete_with(&format!("{id}/")).recursive(true).await.map_err(crate::files::StoreError::from)?;
    let _ = tokio::fs::remove_dir_all(s.store.staging.join(id.to_string())).await;
    sqlx::query("DELETE FROM users WHERE id = $1").bind(id).execute(&s.db).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use axum::{body::Body, http::Request};
    use jsonwebtoken::{DecodingKey, EncodingKey};
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use super::*;

    /// Chạy với Postgres thật: NK_TEST_DB=postgres://... cargo test
    async fn setup() -> Option<(axum::Router, AppState, String, Uuid)> {
        let db = sqlx::PgPool::connect(&std::env::var("NK_TEST_DB").ok()?).await.unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        let (store, admin) = crate::files::tests::tmp_store("users");
        sqlx::query("INSERT INTO users (id, username, pass_hash, is_admin, must_reset) VALUES ($1, $2, 'x', TRUE, FALSE)")
            .bind(admin)
            .bind(format!("adm-{admin}"))
            .execute(&db)
            .await
            .unwrap();
        let state = AppState {
            db,
            store,
            enc: EncodingKey::from_secret(b"t"),
            dec: DecodingKey::from_secret(b"t"),
            uploads: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            http: reqwest::Client::new(),
            tts_url: String::new(),
        };
        let tok = create_jwt(admin, true, &state.enc).unwrap();
        Some((crate::routes::api_router(state.clone()), state, tok, admin))
    }

    async fn call(app: &axum::Router, m: &str, uri: &str, tok: &str, body: Option<Value>) -> (StatusCode, Value) {
        let req = Request::builder()
            .method(m)
            .uri(uri)
            .header("authorization", format!("Bearer {tok}"))
            .header("content-type", "application/json")
            .body(body.map_or(Body::empty(), |b| Body::from(b.to_string())))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        let st = res.status();
        let b = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        (st, serde_json::from_slice(&b).unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn admin_manages_users() {
        let Some((app, s, tok, admin)) = setup().await else { return };
        let name = format!("u-{}", Uuid::new_v4());
        let body = json!({"username": name, "password": "default123", "quota_limit": 1000});
        let (st, u) = call(&app, "POST", "/api/users", &tok, Some(body.clone())).await;
        assert_eq!(st, StatusCode::CREATED);
        assert_eq!(u["must_reset"], true);
        assert!(u.get("pass_hash").is_none());
        let id: Uuid = u["id"].as_str().unwrap().parse().unwrap();
        assert_eq!(call(&app, "POST", "/api/users", &tok, Some(body)).await.0, StatusCode::CONFLICT);
        let short = json!({"username": "x-short", "password": "123"});
        assert_eq!(call(&app, "POST", "/api/users", &tok, Some(short)).await.0, StatusCode::BAD_REQUEST);

        // user thường không được gọi API admin
        let utok = create_jwt(id, false, &s.enc).unwrap();
        assert_eq!(call(&app, "GET", "/api/users", &utok, None).await.0, StatusCode::FORBIDDEN);
        assert_eq!(call(&app, "DELETE", &format!("/api/users/{admin}"), &utok, None).await.0, StatusCode::FORBIDDEN);

        let (st, u) = call(&app, "POST", &format!("/api/users/{id}/quota"), &tok, Some(json!({"quota_limit": 5000}))).await;
        assert_eq!((st, u["quota_limit"].as_i64()), (StatusCode::OK, Some(5000)));
        let st = call(&app, "POST", &format!("/api/users/{id}/password"), &tok, Some(json!({"password": "another123"}))).await.0;
        assert_eq!(st, StatusCode::NO_CONTENT);

        // xoá: dữ liệu trên store + chỉ mục biến mất
        let mut q = Quota::new(0, 0);
        s.store.write_chunk(id, "up", 0, b"hi").unwrap();
        let m = s.store.complete(id, "up", "a/b.txt", 2, None, &mut q).await.unwrap();
        crate::index::add_file(&s.db, id, "a/b.txt", 2, m).await.unwrap();
        assert_eq!(call(&app, "DELETE", &format!("/api/users/{admin}"), &tok, None).await.0, StatusCode::CONFLICT);
        assert_eq!(call(&app, "DELETE", &format!("/api/users/{id}"), &tok, None).await.0, StatusCode::NO_CONTENT);
        assert!(s.store.scan(id).await.unwrap().is_empty());
        let n: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM files WHERE user_id = $1").bind(id).fetch_one(&s.db).await.unwrap();
        assert_eq!(n.0, 0);
        assert_eq!(call(&app, "DELETE", &format!("/api/users/{id}"), &tok, None).await.0, StatusCode::NOT_FOUND);
    }
}
