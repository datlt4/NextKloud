//! Shared state, typed errors, auth extractor.
use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use jsonwebtoken::{DecodingKey, EncodingKey};
use serde::Serialize;
use sqlx::PgPool;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{auth::verify_jwt, files::Store};

#[derive(Debug, Clone)]
pub struct PendingUpload {
    pub user_id: Uuid,
    /// chủ file (== user_id khi upload file của mình; khác khi upload vào thư mục được chia sẻ)
    pub owner: Uuid,
    pub path: String,
    pub size: u64,
    pub mtime: Option<i64>,
}

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub store: Store,
    pub enc: EncodingKey,
    pub dec: DecodingKey,
    pub uploads: Arc<Mutex<HashMap<String, PendingUpload>>>,
    pub http: reqwest::Client,
    pub tts_url: String,
    pub vaults: Arc<crate::secret::Vaults>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct UserRow {
    pub id: Uuid,
    pub username: String,
    #[serde(skip_serializing)]
    pub pass_hash: String,
    pub must_reset: bool,
    pub quota_limit: i64,
    pub quota_used: i64,
    pub is_admin: bool,
    pub display_name: Option<String>,
    pub avatar_v: i32,
}

#[derive(Debug)]
pub enum AppError {
    Auth(String),
    Forbidden(String),
    NotFound(String),
    Conflict(String),
    BadRequest(String),
    TooMany(String),
    Store(crate::files::StoreError),
    Db(sqlx::Error),
    Internal(String),
}

impl From<crate::files::StoreError> for AppError {
    fn from(e: crate::files::StoreError) -> Self {
        use crate::files::StoreError as E;
        match e {
            E::UnsafePath => Self::Forbidden("unsafe path".into()),
            E::NotFound => Self::NotFound("not found".into()),
            E::Conflict(m) => Self::Conflict(m),
            E::Quota(q) => Self::Conflict(format!("quota exceeded: {q}")),
            E::Io(e) => Self::Internal(e.to_string()),
            E::Backend(e) => Self::Internal(e.to_string()),
        }
    }
}
impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e)
    }
}

#[derive(Serialize)]
struct ErrBody {
    error: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (code, msg) = match self {
            Self::Auth(m) => (StatusCode::UNAUTHORIZED, m),
            Self::Forbidden(m) => (StatusCode::FORBIDDEN, m),
            Self::NotFound(m) => (StatusCode::NOT_FOUND, m),
            Self::Conflict(m) => (StatusCode::CONFLICT, m),
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            Self::TooMany(m) => (StatusCode::TOO_MANY_REQUESTS, m),
            Self::Store(e) => return AppError::from(e).into_response(),
            Self::Db(e) => {
                tracing::error!("db: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "database error".into())
            }
            Self::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        };
        let body = serde_json::to_string(&ErrBody { error: msg }).unwrap();
        (code, [(header::CONTENT_TYPE, "application/json")], body).into_response()
    }
}

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: Uuid,
    pub admin: bool,
}

#[async_trait]
impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let hdr = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AppError::Auth("missing bearer token".into()))?;
        let tok = hdr.strip_prefix("Bearer ").ok_or_else(|| AppError::Auth("bad auth scheme".into()))?;
        let claims = verify_jwt(tok, &state.dec).map_err(AppError::Auth)?;
        // quyền admin đọc từ DB (JWT cũ không giữ quyền sau khi bị hạ/xoá user)
        let admin: Option<bool> = sqlx::query_scalar("SELECT is_admin FROM users WHERE id = $1").bind(claims.sub).fetch_optional(&state.db).await?;
        let admin = admin.ok_or_else(|| AppError::Auth("user gone".into()))?;
        Ok(Self { id: claims.sub, admin })
    }
}

/// Load full user row (for quota/admin checks).
pub async fn load_user(db: &PgPool, id: Uuid) -> Result<UserRow, AppError> {
    sqlx::query_as::<_, UserRow>("SELECT * FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| AppError::Auth("user gone".into()))
}
