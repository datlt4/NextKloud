use axum::{Json, Router, routing::get};
use serde_json::{Value, json};

async fn health() -> Json<Value> {
    Json(json!({"status":"ok","service":"nextkloud"}))
}

// NOTE: không validate Host / trusted-domains như Nextcloud.
// CORS allowlist qua env ALLOWED_ORIGINS, mặc định "*".
pub fn router() -> Router {
    Router::new()
        .route("/api/health", get(health))
        // MVP stubs — triển khai dần: auth, users, files, kanban
        .route("/api/users", get(|| async { Json(json!([])) }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_ok() {
        let app = router();
        let res = app
            .oneshot(Request::builder().uri("/api/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }
}
