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

async fn reindex(db: &PgPool, store: &Store, username: Option<&str>) {
    let users: Vec<(Uuid, String)> = sqlx::query_as("SELECT id, username FROM users WHERE $1::text IS NULL OR username = $1")
        .bind(username)
        .fetch_all(db)
        .await
        .expect("load users");
    if users.is_empty() {
        eprintln!("no such user");
        std::process::exit(1);
    }
    for (id, name) in users {
        let t = std::time::Instant::now();
        match nextkloud_backend::index::rebuild(db, store, id).await {
            Ok((n, b)) => println!("{name}: {n} files, {b} bytes ({:.1}s)", t.elapsed().as_secs_f32()),
            Err(e) => {
                eprintln!("{name}: {e}");
                std::process::exit(1);
            }
        }
    }
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
    // `nextkloud reindex [username]`: dựng lại chỉ mục từ store (sau migrate thủ công).
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("reindex") {
        reindex(&db, &store, args.get(2).map(String::as_str)).await;
        return;
    }
    // User chưa có chỉ mục (lần đầu nâng cấp) -> dựng nền, không chặn khởi động.
    {
        let (db, store) = (db.clone(), store.clone());
        tokio::spawn(async move {
            let ids: Vec<(Uuid,)> = sqlx::query_as(
                "SELECT id FROM users u WHERE NOT EXISTS (SELECT 1 FROM files f WHERE f.user_id = u.id)",
            )
            .fetch_all(&db)
            .await
            .unwrap_or_default();
            for (id,) in ids {
                match nextkloud_backend::index::rebuild(&db, &store, id).await {
                    Ok((n, b)) if n > 0 => tracing::info!("indexed user {id}: {n} files, {b} bytes"),
                    Ok(_) => {}
                    Err(e) => tracing::error!("index user {id}: {e}"),
                }
            }
        });
    }
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
