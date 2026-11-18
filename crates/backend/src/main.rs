use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use jsonwebtoken::{DecodingKey, EncodingKey};
use nextkloud_backend::{auth::hash_password, files::FsStore, routes::api_router, state::AppState};
use sqlx::PgPool;
use tokio::sync::Mutex;
use tower_http::cors::CorsLayer;
use uuid::Uuid;

async fn ensure_admin(db: &PgPool) -> Result<(), sqlx::Error> {
    let n: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users").fetch_one(db).await?;
    if n.0 > 0 {
        return Ok(());
    }
    let password = std::env::var("ADMIN_PASSWORD").unwrap_or_else(|_| "admin".into());
    if password == "admin" {
        tracing::warn!("seeded admin/admin with must_reset — change ADMIN_PASSWORD and reset on first login");
    }
    let hash = hash_password(&password).expect("admin password too short");
    sqlx::query(
        "INSERT INTO users (id, username, pass_hash, must_reset, quota_limit, is_admin)
         VALUES ($1, 'admin', $2, TRUE, 107374182400, TRUE)",
    )
    .bind(Uuid::new_v4())
    .bind(hash)
    .execute(db)
    .await?;
    Ok(())
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter("info").init();
    // Cố tình KHÔNG check Host header (khác Nextcloud trusted_domains).
    let cors = CorsLayer::permissive(); // thu hẹp ở reverse proxy khi deploy

    let db = PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL required"))
        .await
        .expect("db connect");
    sqlx::migrate!("./migrations").run(&db).await.expect("migrate");
    ensure_admin(&db).await.expect("seed admin");

    let file_dir = std::env::var("FILE_DIR").unwrap_or_else(|_| "/data/files".into());
    let secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| {
        tracing::warn!("JWT_SECRET unset — using dev default, set it in production");
        "dev-insecure-change-me".into()
    });
    let state = AppState {
        db,
        store: FsStore { root: PathBuf::from(file_dir) },
        enc: EncodingKey::from_secret(secret.as_bytes()),
        dec: DecodingKey::from_secret(secret.as_bytes()),
        uploads: Arc::new(Mutex::new(HashMap::new())),
    };

    let web_dir = std::env::var("NEXTKLOUD_WEB").ok().filter(|d| !d.is_empty()).map(PathBuf::from);
    if let Some(ref d) = web_dir {
        tracing::info!("serving web from {}", d.display());
    }
    let mut app = api_router(state).layer(cors);
    if let Some(dir) = web_dir {
        // Web client + SPA fallback; route API cụ thể vẫn ưu tiên.
        app = app.fallback(move |uri: axum::http::Uri| {
            let dir = dir.clone();
            async move { nextkloud_backend::routes::serve_web(dir, uri).await }
        });
    }

    let addr = std::env::var("BIND").unwrap_or_else(|_| "0.0.0.0:8014".into());
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    tracing::info!("nextkloud listening on {addr}");
    axum::serve(listener, app).await.unwrap();
}
