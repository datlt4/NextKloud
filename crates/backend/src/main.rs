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
    // ALLOWED_ORIGINS: "*" (mặc định) hoặc danh sách cách nhau dấu phẩy; app desktop Tauri luôn được phép
    let cors = match std::env::var("ALLOWED_ORIGINS").unwrap_or_default().trim() {
        "" | "*" => CorsLayer::permissive(),
        list => {
            let mut origins: Vec<axum::http::HeaderValue> = list.split(',').filter_map(|o| o.trim().parse().ok()).collect();
            origins.extend(["tauri://localhost", "http://tauri.localhost", "https://tauri.localhost"].map(axum::http::HeaderValue::from_static));
            CorsLayer::permissive().allow_origin(tower_http::cors::AllowOrigin::list(origins))
        }
    };

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
            // Bù chỉ mục nội dung (.md/.txt) còn thiếu cho mọi user.
            let all: Vec<(Uuid,)> = sqlx::query_as("SELECT id FROM users").fetch_all(&db).await.unwrap_or_default();
            for (id,) in all {
                match nextkloud_backend::fulltext::backfill(&db, &store, id).await {
                    Ok(n) if n > 0 => tracing::info!("fulltext user {id}: {n} files"),
                    Ok(_) => {}
                    Err(e) => tracing::error!("fulltext user {id}: {e}"),
                }
            }
        });
    }
    // Dọn thùng rác quá TRASH_DAYS ngày (mặc định 30), mỗi giờ 1 lần.
    {
        let (db, store) = (db.clone(), store.clone());
        let days = nextkloud_backend::ops::trash_days();
        tokio::spawn(async move {
            loop {
                match nextkloud_backend::ops::purge_expired(&db, &store, days).await {
                    Ok(n) if n > 0 => tracing::info!("trash: purged {n} items"),
                    Ok(_) => {}
                    Err(e) => tracing::error!("trash purge: {e:?}"),
                }
                if let Err(e) = nextkloud_backend::secret::purge_expired(&db, &store, days).await {
                    tracing::error!("secret trash purge: {e:?}");
                }
                let n = nextkloud_backend::versions::purge_expired(&db, &store).await;
                if n > 0 {
                    tracing::info!("versions: purged {n} old");
                }
                // upload bỏ dở > 7 ngày: xoá file staging + dòng resume
                let n = store.sweep_staging(std::time::Duration::from_secs(7 * 86400));
                let cutoff = chrono::Utc::now().timestamp() - 7 * 86400;
                let _ = sqlx::query("DELETE FROM uploads WHERE created_at < $1").bind(cutoff).execute(&db).await;
                if n > 0 {
                    tracing::info!("staging: removed {n} stale files");
                }
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            }
        });
    }
    // Mirror git: kiểm tra mỗi 5 phút repo nào tới hạn (mặc định 8 giờ/lần), chạy lần lượt.
    {
        let (db, store) = (db.clone(), store.clone());
        tokio::spawn(async move {
            use nextkloud_backend::git;
            if let Err(e) = git::reset_stuck(&db).await {
                tracing::error!("git reset: {e}");
            }
            if let Err(e) = git::migrate_vaults_folder(&db, &store).await {
                tracing::error!("git migrate Vaults: {e:?}");
            }
            loop {
                for id in git::due(&db).await.unwrap_or_default() {
                    if let Ok(r) = git::begin(&db, id, None).await {
                        git::finish(&db, &store, r).await;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            }
        });
    }
    // Nhắc task đến hạn qua SSE: quét mỗi phút (đánh dấu reminded_at để không spam).
    {
        let db = db.clone();
        tokio::spawn(async move {
            loop {
                let n = nextkloud_backend::tracker::check_due_once(&db).await;
                if n > 0 {
                    tracing::info!("reminders sent: {n}");
                }
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
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
        vaults: Default::default(),
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
