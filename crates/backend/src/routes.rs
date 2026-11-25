use std::path::{Component, PathBuf};

use axum::{
    Router,
    body::Body,
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde_json::{Value, json};

use crate::state::AppState;

async fn health() -> axum::Json<Value> {
    axum::Json(json!({"status":"ok","service":"nextkloud"}))
}

pub fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "epub" => "application/epub+zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "odp" => "application/vnd.oasis.opendocument.presentation",
        "csv" => "text/csv",
        "md" => "text/markdown; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

pub async fn serve_web(web_dir: PathBuf, uri: Uri) -> Response {
    // Giữ trong web_dir: chặn `..`.
    let mut rel = PathBuf::new();
    for comp in uri.path().trim_start_matches('/').split('/') {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp == ".." {
            return StatusCode::FORBIDDEN.into_response();
        }
        rel.push(Component::Normal(comp.as_ref()));
    }
    // Thư mục hoặc file không tồn tại (route SPA) -> index.html.
    let mut file = web_dir.join(&rel);
    let is_dir = tokio::fs::metadata(&file).await.map(|m| m.is_dir()).unwrap_or(false);
    let missing = !tokio::fs::try_exists(&file).await.unwrap_or(false);
    // file build (assets/*.js...) không còn (bản mới đã thay tên): 404 để app biết mà tải lại, đừng trả index.html
    if missing && rel.starts_with("assets") {
        return StatusCode::NOT_FOUND.into_response();
    }
    if rel.as_os_str().is_empty() || is_dir || missing {
        file = web_dir.join("index.html");
    }
    match tokio::fs::read(&file).await {
        Ok(bytes) => {
            let name = file.to_string_lossy().into_owned();
            Response::builder()
                .header(header::CONTENT_TYPE, content_type(&name))
                .header(header::CACHE_CONTROL, "no-cache")
                .body(Body::from(bytes))
                .unwrap()
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

// NOTE: không validate Host / trusted-domains như Nextcloud.
// CORS allowlist qua env ALLOWED_ORIGINS, mặc định "*".
pub fn router() -> Router {
    router_with_web(None)
}

pub fn api_router(state: AppState) -> Router {
    use crate::{file_handlers as f, users as u};
    Router::new()
        .route("/api/health", get(health))
        .route("/api/auth/login", post(u::login))
        .route("/api/auth/password", post(u::change_password))
        .route("/api/me", get(u::me).patch(u::update_me))
        .route("/api/me/avatar", put(u::put_avatar).delete(u::delete_avatar))
        .route("/api/users/:id/avatar", get(u::get_avatar))
        .route("/api/admin/stats", get(u::admin_stats))
        .route("/api/admin/reindex/:id", post(u::admin_reindex))
        .route("/api/users", get(u::list_users).post(u::create_user))
        .route("/api/users/:id", axum::routing::delete(u::delete_user))
        .route("/api/users/:id/quota", post(u::set_quota))
        .route("/api/users/:id/password", post(u::reset_password))
        .route("/api/files", get(f::list_files))
        // Không đặt dưới /api/files/ để khỏi đụng file tên "search".
        .route("/api/search", get(f::search_files))
        .route("/api/search/content", get(f::search_content))
        .route("/api/vault/outline", get(f::vault_outline))
        .route("/api/vault/backlinks", get(f::vault_backlinks))
        .route("/api/mkdir", post(f::mkdir))
        .route("/api/dl", post(f::dl_token))
        .route("/api/annotations", get(crate::notes::list).post(crate::notes::create))
        .route("/api/annotations/:id", axum::routing::patch(crate::notes::update).delete(crate::notes::remove))
        .route("/api/move", post(f::move_entry))
        .route("/api/copy", post(f::copy_entry))
        .route("/api/config", get(crate::config::get))
        .route("/api/versions", get(crate::versions::list))
        .route("/api/versions/file", get(crate::versions::file))
        .route("/api/versions/restore", post(crate::versions::restore))
        .route("/api/trash", get(f::list_trash).delete(f::empty_trash))
        .route("/api/trash/:id", axum::routing::delete(f::purge_trash))
        .route("/api/trash/:id/restore", post(f::restore_trash))
        .route("/api/git", get(crate::git::list).post(crate::git::create))
        .route("/api/git/:id", axum::routing::patch(crate::git::update).delete(crate::git::remove))
        .route("/api/git/:id/sync", post(crate::git::sync_now))
        .route("/api/files/uploads", post(f::init_upload))
        .route("/api/files/uploads/active", get(f::upload_active))
        .route("/api/files/uploads/:id", get(f::upload_status))
        // axum mặc định chặn body > 2MB; chunk cho phép tới 64MB.
        .route(
            "/api/files/uploads/:id",
            put(f::put_chunk).layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024 + 1024)),
        )
        .route("/api/files/uploads/:id/complete", post(f::complete_upload))
        .route("/api/files/*path", get(f::download).delete(f::delete_file))
        .route("/api/tts/voices", get(crate::tts::voices))
        .route("/api/tts/detect", get(crate::tts::detect))
        .route("/api/tts/synthesize", post(crate::tts::synthesize))
        .route("/api/zip/list", get(crate::zip::list))
        .route("/api/zip/download", get(crate::zip::download_dir))
        .route("/api/zip/file", get(crate::zip::extract))
        .route("/api/thumb/*path", get(crate::thumb::thumb))
        .route("/api/office/config", post(crate::office::config))
        .route("/api/office/file", get(crate::office::file_dl))
        .route("/api/office/callback", post(crate::office::callback))
        .route("/api/boards", get(crate::kanban::list_boards).post(crate::kanban::create_board))
        .route("/api/boards/:id", get(crate::kanban::get_board).patch(crate::kanban::update_board).delete(crate::kanban::delete_board))
        .route("/api/boards/:id/columns", post(crate::kanban::create_column))
        .route("/api/boards/:id/parents", get(crate::kanban::parent_search))
        .route("/api/linkpreview", get(crate::linkpreview::get))
        .route("/api/columns/:id", axum::routing::patch(crate::kanban::update_column).delete(crate::kanban::delete_column))
        .route("/api/columns/:id/cards", get(crate::kanban::column_cards))
        .route("/api/cards", post(crate::kanban::create_card))
        .route("/api/cards/:id", get(crate::kanban::card_by_id).patch(crate::kanban::update_card).delete(crate::kanban::delete_card))
        .route("/api/kanban/import/deck", get(crate::deck::preview).post(crate::deck::apply))
        .route("/api/cards/:id/subtasks", get(crate::kanban::subtasks))
        .route("/api/tasks", get(crate::tracker::tasks))
        .route("/api/events", get(crate::events::stream))
        .route("/api/events/ticket", post(crate::events::ticket))
        .route("/api/shares", get(crate::shares::list_shares).post(crate::shares::share_with))
        .route("/api/shares/link", post(crate::shares::create_link))
        .route("/api/shares/:id", axum::routing::delete(crate::shares::unshare))
        .route("/api/shared", get(crate::shares::shared_with_me))
        .route("/api/shared/save", post(crate::shares::save_copy))
        // link công khai: không cần đăng nhập
        .route("/api/s/:token", get(crate::shares::link_meta))
        .route("/api/s/:token/list", get(crate::shares::link_list))
        .route("/api/s/:token/file", get(crate::shares::link_file_root))
        .route("/api/s/:token/file/*path", get(crate::shares::link_file))
        .route("/api/habits", get(crate::tracker::list_habits).post(crate::tracker::create_habit))
        .route("/api/habits/:id", axum::routing::patch(crate::tracker::update_habit).delete(crate::tracker::delete_habit))
        .route("/api/habits/:id/check", post(crate::tracker::check_habit))
        .route("/api/secret", get(crate::secret::status).delete(crate::secret::destroy))
        .route("/api/secret/setup", post(crate::secret::setup))
        .route("/api/secret/unlock", post(crate::secret::unlock))
        .route("/api/secret/lock", post(crate::secret::lock))
        .route("/api/secret/pin", post(crate::secret::change_pin))
        .route("/api/secret/recovery", post(crate::secret::new_recovery))
        .route("/api/secret/list", get(crate::secret::list))
        .route("/api/secret/mkdir", post(crate::secret::mkdir))
        .route("/api/secret/move", post(crate::secret::move_entry))
        .route("/api/secret/copy", post(crate::secret::copy))
        .route("/api/secret/import", post(crate::secret::import))
        .route("/api/secret/export", post(crate::secret::export))
        .route("/api/secret/files/*path", get(crate::secret::download).delete(crate::secret::delete))
        .route("/api/secret/thumb/*path", get(crate::secret::thumb))
        .route("/api/secret/uploads", post(crate::secret::init_upload))
        .route(
            "/api/secret/uploads/:id",
            put(crate::secret::put_chunk).layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024 + 1024)),
        )
        .route("/api/secret/uploads/:id/complete", post(crate::secret::complete_upload))
        .route("/api/secret/trash", get(crate::secret::list_trash).delete(crate::secret::empty_trash))
        .route("/api/secret/trash/:id", axum::routing::delete(crate::secret::purge))
        .route("/api/secret/trash/:id/restore", post(crate::secret::restore))
        .route("/api/secret/office/config", post(crate::secret::office_config))
        .route("/api/secret/office/file", get(crate::secret::office_file))
        .route("/oo/*rest", axum::routing::any(crate::ooproxy::proxy))
        .with_state(state)
}

pub fn router_with_web(web_dir: Option<PathBuf>) -> Router {
    match web_dir {
        Some(dir) => Router::new()
            .route("/api/health", get(health))
            .fallback(move |uri: Uri| {
                let dir = dir.clone();
                async move { serve_web(dir, uri).await }
            }),
        None => Router::new().route("/api/health", get(health)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn body_text(res: axum::response::Response) -> String {
        let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024).await.unwrap();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[tokio::test]
    async fn health_ok() {
        let app = router();
        let res = app
            .oneshot(Request::builder().uri("/api/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn web_serves_index_and_spa_fallback() {
        let dir = std::env::temp_dir().join("nk-web-test");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("index.html"), "<h1>NextKloud</h1>").await.unwrap();
        let app = router_with_web(Some(dir));
        // / -> index.html
        let res = app
            .clone()
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(body_text(res).await.contains("NextKloud"));
        // route SPA không tồn tại -> index.html
        let res = app
            .clone()
            .oneshot(Request::builder().uri("/files/abc").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        // file build cũ không còn -> 404 (không trả index.html)
        let res = app
            .clone()
            .oneshot(Request::builder().uri("/assets/ImageView-old.js").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        // API vẫn ưu tiên
        let res = app
            .oneshot(Request::builder().uri("/api/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn web_blocks_path_traversal() {
        let dir = std::env::temp_dir().join("nk-web-test2");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let app = router_with_web(Some(dir));
        let res = app
            .oneshot(Request::builder().uri("/..%2f..%2fetc/passwd").body(Body::empty()).unwrap())
            .await
            .unwrap();
        // %2f không decode ở path segment này hoặc bị chặn -> 403/404 đều an toàn
        assert!(res.status() == StatusCode::FORBIDDEN || res.status() == StatusCode::NOT_FOUND);
    }
}
