use nextkloud_backend::routes::router;
use tower_http::cors::CorsLayer;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter("info").init();
    // Cố tình KHÔNG check Host header (khác Nextcloud trusted_domains).
    let cors = CorsLayer::permissive(); // thu hẹp bằng ALLOWED_ORIGINS ở reverse proxy khi deploy
    let app = router().layer(cors);
    let addr = std::env::var("BIND").unwrap_or_else(|_| "0.0.0.0:8014".into());
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    tracing::info!("nextkloud listening on {addr}");
    axum::serve(listener, app).await.unwrap();
}
