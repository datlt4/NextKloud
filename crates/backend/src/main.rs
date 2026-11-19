use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use jsonwebtoken::{DecodingKey, EncodingKey};
use nextkloud_backend::{auth::hash_password_unchecked, files::Store, routes::api_router, state::AppState};
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
    let hash = hash_password_unchecked(&password).expect("hash admin");
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

    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.into());
    let staging = PathBuf::from(env("STAGING_DIR", "/data/staging"));
    let store = match env("STORAGE_DRIVER", "fs").as_str() {
        "s3" => Store::s3(
            &env("S3_ENDPOINT", "http://rustfs:9000"),
            &env("S3_BUCKET", "nextkloud"),
            &std::env::var("S3_ACCESS_KEY").expect("S3_ACCESS_KEY required"),
            &std::env::var("S3_SECRET_KEY").expect("S3_SECRET_KEY required"),
            staging,
        ),
        _ => Store::fs(&PathBuf::from(env("FILE_DIR", "/data/files")), staging),
    }
    .expect("storage init");
    // Fail sớm nếu bucket chưa có / sai key, thay vì lỗi lúc upload.
    store.op.check().await.expect("storage check (bucket tồn tại? key đúng?)");
    let secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| {
        tracing::warn!("JWT_SECRET unset — using dev default, set it in production");
        "dev-insecure-change-me".into()
    });
    let state = AppState {
        db,
        store,
        enc: EncodingKey::from_secret(secret.as_bytes()),
        dec: DecodingKey::from_secret(secret.as_bytes()),
        uploads: Arc::new(Mutex::new(HashMap::new())),
        http: reqwest::Client::builder().timeout(std::time::Duration::from_secs(60)).build().expect("http client"),
        tts_url: std::env::var("TTS_URL").unwrap_or_else(|_| "http://localhost:8016".into()),
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
