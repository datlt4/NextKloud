use std::path::PathBuf;

use nextkloud_backend::routes::router_with_web;
use tower_http::cors::CorsLayer;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter("info").init();
    // Cố tình KHÔNG check Host header (khác Nextcloud trusted_domains).
    let cors = CorsLayer::permissive(); // thu hẹp bằng ALLOWED_ORIGINS ở reverse proxy khi deploy
    // Serve web client (Vite dist) nếu NEXTKLOUD_WEB trỏ tới thư mục tồn tại.
    let web_dir = std::env::var("NEXTKLOUD_WEB").ok().filter(|d| !d.is_empty()).map(PathBuf::from);
    if let Some(ref d) = web_dir {
        tracing::info!("serving web from {}", d.display());
    }
    let app = router_with_web(web_dir).layer(cors);
    let addr = std::env::var("BIND").unwrap_or_else(|_| "0.0.0.0:8014".into());
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    tracing::info!("nextkloud listening on {addr}");
    axum::serve(listener, app).await.unwrap();
}
