//! OnlyOffice: mở/sửa file office trực tiếp.
//! DocumentServer đã có sẵn trên mini PC (:8027), JWT enforced.
//!
//! - POST /api/office/config {path} -> config + token ký bằng ONLYOFFICE_JWT_SECRET
//! - GET  /api/office/file?t=<dl_jwt>  (DocumentServer tải file; không gửi được Bearer)
//! - POST /api/office/callback (DocumentServer gọi khi lưu; verify JWT của nó)
use std::collections::HashMap;

use axum::{
    Json,
    body::Body,
    extract::{Query, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    files::{clean_path, mtime_of},
    index,
    quota::Quota,
    state::{AppError, AppState, AuthUser, load_user},
};

pub(crate) fn oo_secret() -> Result<String, AppError> {
    std::env::var("ONLYOFFICE_JWT_SECRET")
        .map_err(|_| AppError::Internal("thiếu ONLYOFFICE_JWT_SECRET trên server".into()))
}

/// docx -> (fileType, documentType). Hết danh sách OnlyOffice hỗ trợ chỉnh sửa.
pub fn doc_type(path: &str) -> Option<(&'static str, &'static str)> {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "doc" | "docx" | "odt" | "rtf" => Some((ext_leak(&ext), "word")),
        "xls" | "xlsx" | "ods" | "csv" => Some((ext_leak(&ext), "cell")),
        "ppt" | "pptx" | "odp" => Some((ext_leak(&ext), "slide")),
        _ => None,
    }
}
fn ext_leak(s: &str) -> &'static str {
    match s {
        "doc" => "doc",
        "docx" => "docx",
        "odt" => "odt",
        "rtf" => "rtf",
        "xls" => "xls",
        "xlsx" => "xlsx",
        "ods" => "ods",
        "csv" => "csv",
        "ppt" => "ppt",
        "pptx" => "pptx",
        "odp" => "odp",
        _ => "docx",
    }
}

/// Key tài liệu duy nhất theo nội dung (đổi file -> key mới, khỏi kẹt cache).
pub fn doc_key(user: &uuid::Uuid, path: &str, mtime: i64, size: u64) -> String {
    let mut h = Sha256::new();
    h.update(user.as_bytes());
    h.update(path.as_bytes());
    h.update(mtime.to_be_bytes());
    h.update(size.to_be_bytes());
    hex_of(&h.finalize())
}

fn hex_of(bytes: &[u8]) -> String {
    const C: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(C[(b >> 4) as usize] as char);
        s.push(C[(b & 15) as usize] as char);
    }
    s
}

pub(crate) fn public_base(headers: &HeaderMap) -> String {
    if let Ok(v) = std::env::var("PUBLIC_URL") {
        if !v.trim().is_empty() {
            return v.trim_end_matches('/').to_string();
        }
    }
    // Fallback: host mà browser đang dùng (cùng LAN với DocumentServer).
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("192.168.0.186:8011");
    format!("http://{host}")
}

#[derive(Deserialize, Serialize)]
pub struct DlClaims {
    pub sub: uuid::Uuid,
    pub path: String,
    pub exp: usize,
}

fn dl_token(user: uuid::Uuid, path: &str, enc: &EncodingKey) -> Result<String, AppError> {
    let exp = (chrono::Utc::now() + chrono::Duration::minutes(30)).timestamp() as usize;
    encode(&Header::default(), &DlClaims { sub: user, path: path.to_string(), exp }, enc)
        .map_err(|e| AppError::Internal(format!("ký dl token: {e}")))
}

#[derive(Deserialize)]
pub struct ConfigBody {
    pub path: String,
}

/// Validation cho token của DocumentServer: nó ký cả payload JSON, KHÔNG có exp.
fn oo_validation() -> Validation {
    let mut v = Validation::default();
    v.required_spec_claims.clear();
    v.validate_exp = false;
    v
}

pub async fn config(
    State(s): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    Json(b): Json<ConfigBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let path = clean_path(&b.path).map_err(AppError::from)?;
    let (file_type, document_type) =
        doc_type(&path).ok_or_else(|| AppError::BadRequest("loại office này chưa hỗ trợ".into()))?;
    let meta = s.store.stat(user.id, &path).await?;
    if meta.is_dir() {
        return Err(AppError::BadRequest("not a file".into()));
    }
    let mtime = mtime_of(&meta);
    let size = meta.content_length();
    let key = doc_key(&user.id, &path, mtime, size);
    // DocumentServer tải file + gọi callback: ưu tiên địa chỉ nội bộ (NextKloud public có thể nằm sau lớp đăng nhập).
    let base = std::env::var("ONLYOFFICE_BACKEND_URL").ok().map(|v| v.trim_end_matches('/').to_string()).filter(|v| !v.is_empty()).unwrap_or_else(|| public_base(&headers));
    let dl = dl_token(user.id, &path, &s.enc)?;

    sqlx::query(
        "INSERT INTO office_docs (key, user_id, path, created_at) VALUES ($1, $2, $3, $4)
         ON CONFLICT (key) DO UPDATE SET path = EXCLUDED.path",
    )
    .bind(&key)
    .bind(user.id)
    .bind(&path)
    .bind(chrono::Utc::now().timestamp())
    .execute(&s.db)
    .await?;

    let row = load_user(&s.db, user.id).await?;
    let name = path.rsplit('/').next().unwrap_or(&path);
    let display = row.display_name.filter(|d| !d.is_empty()).unwrap_or(row.username);
    let cfg = serde_json::json!({
        "document": {
            "fileType": file_type,
            "key": key,
            "title": name,
            "url": format!("{base}/api/office/file?t={dl}"),
        },
        "documentType": document_type,
        "editorConfig": {
            "callbackUrl": format!("{base}/api/office/callback"),
            "user": { "id": user.id.to_string(), "name": display },
            "mode": "edit",
            "lang": "vi",
        },
    });
    let secret = oo_secret()?;
    let token = encode(&Header::default(), &cfg, &EncodingKey::from_secret(secret.as_bytes()))
        .map_err(|e| AppError::Internal(format!("ký token office: {e}")))?;
    Ok(Json(serde_json::json!({ "config": cfg, "token": token })))
}

/// DocumentServer tải file (không gửi được Bearer -> dùng ?t= ký bằng JWT của mình).
pub async fn file_dl(
    State(s): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let tok = q.get("t").ok_or_else(|| AppError::Auth("missing token".into()))?;
    let claims = decode::<DlClaims>(tok, &s.dec, &Validation::default())
        .map(|d| d.claims)
        .map_err(|_| AppError::Auth("bad token".into()))?;
    let path = clean_path(&claims.path).map_err(AppError::from)?;
    let meta = s.store.stat(claims.sub, &path).await?;
    let size = meta.content_length();
    let key = s.store.key(claims.sub, &path)?;
    let stream = s
        .store
        .op
        .reader_with(&key)
        .chunk(4 * 1024 * 1024)
        .await
        .map_err(crate::files::StoreError::from)?
        .into_bytes_stream(0..size)
        .await
        .map_err(crate::files::StoreError::from)?;
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, crate::routes::content_type(&path))
        .header(header::CONTENT_LENGTH, size)
        .body(Body::from_stream(stream))
        .unwrap())
}

#[derive(Deserialize, Debug)]
pub struct CallbackBody {
    pub key: String,
    pub status: i32,
    pub url: Option<String>,
}

/// DocumentServer gọi khi user lưu (status 2). Verify JWT của nó ở header Authorization.
pub async fn callback(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(b): Json<CallbackBody>,
) -> Result<impl IntoResponse, AppError> {
    let secret = oo_secret()?;
    let tok = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(|| AppError::Auth("thiếu Authorization từ DocumentServer".into()))?;
    decode::<serde_json::Value>(tok, &DecodingKey::from_secret(secret.as_bytes()), &oo_validation())
        .map_err(|e| AppError::Auth(format!("token DocumentServer không hợp lệ: {e}")))?;
    if b.status != 2 {
        // 0/1/3/4/6/7: đang sửa, đóng không lưu, lỗi... -> không có gì để ghi.
        return Ok(Json(serde_json::json!({ "error": 0 })));
    }
    let url = b.url.ok_or_else(|| AppError::BadRequest("thiếu file url".into()))?;
    let row: Option<(uuid::Uuid, String)> =
        sqlx::query_as("SELECT user_id, path FROM office_docs WHERE key = $1")
            .bind(&b.key)
            .fetch_optional(&s.db)
            .await?;
    let (user_id, path) = row.ok_or_else(|| AppError::NotFound("phiên office hết hạn".into()))?;

    // Tải bản đã lưu về staging rồi ghi đè qua cùng đường quota như upload.
    let mut get = s.http.get(crate::ooproxy::internal_url(&url));
    for (k, v) in crate::ooproxy::extra_headers() {
        get = get.header(k, v);
    }
    let bytes = get
        .timeout(std::time::Duration::from_secs(300))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("không tải được file đã lưu: {e}")))?
        .error_for_status()
        .map_err(|e| AppError::Internal(format!("DocumentServer báo lỗi: {e}")))?
        .bytes()
        .await
        .map_err(|e| AppError::Internal(format!("đọc file đã lưu: {e}")))?;
    let tmp = s.store.staging.join(format!("office-{}.bin", uuid::Uuid::new_v4()));
    tokio::fs::write(&tmp, &bytes).await.map_err(|e| AppError::Internal(e.to_string()))?;

    let urow = load_user(&s.db, user_id).await?;
    let mut quota = Quota::new(urow.quota_limit.max(0) as u64, urow.quota_used.max(0) as u64);
    let mtime = s.store.put_local(user_id, &path, &tmp, Some(chrono::Utc::now().timestamp()), &mut quota).await;
    let _ = tokio::fs::remove_file(&tmp).await;
    let mtime = mtime?;
    // Cộng phần chênh, không ghi đè: upload/dọn rác chạy song song không bị mất.
    sqlx::query("UPDATE users SET quota_used = GREATEST(0, quota_used + $1) WHERE id = $2")
        .bind(quota.used_bytes as i64 - urow.quota_used.max(0))
        .bind(user_id)
        .execute(&s.db)
        .await?;
    index::add_file(&s.db, user_id, &clean_path(&path)?, bytes.len() as u64, mtime).await?;
    sqlx::query("DELETE FROM office_docs WHERE key = $1").bind(&b.key).execute(&s.db).await?;
    tracing::info!("office saved {path} ({} bytes)", bytes.len());
    Ok(Json(serde_json::json!({ "error": 0 })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_type_mapping() {
        assert_eq!(doc_type("a.docx"), Some(("docx", "word")));
        assert_eq!(doc_type("A.XLSX"), Some(("xlsx", "cell")));
        assert_eq!(doc_type("p.PPT"), Some(("ppt", "slide")));
        assert_eq!(doc_type("a.pdf"), None);
        assert_eq!(doc_type("a.txt"), None);
        assert_eq!(doc_type("noext"), None);
    }

    #[test]
    fn doc_key_changes_with_content() {
        let u = uuid::Uuid::nil();
        let a = doc_key(&u, "a.docx", 1, 2);
        assert_eq!(a.len(), 64);
        assert_eq!(a, doc_key(&u, "a.docx", 1, 2));
        assert_ne!(a, doc_key(&u, "a.docx", 2, 2));
        assert_ne!(a, doc_key(&u, "b.docx", 1, 2));
    }

    #[test]
    fn dl_token_roundtrip() {
        let enc = EncodingKey::from_secret(b"test-secret-that-is-long-enough");
        let dec = DecodingKey::from_secret(b"test-secret-that-is-long-enough");
        let t = dl_token(uuid::Uuid::new_v4(), "a/b.docx", &enc).unwrap();
        let c = decode::<DlClaims>(&t, &dec, &Validation::default()).unwrap().claims;
        assert_eq!(c.path, "a/b.docx");
    }

    #[test]
    fn office_token_sign_verify_with_docserver_secret() {
        // Mô phỏng DocumentServer: ký config bằng secret của nó, backend verify.
        let secret = b"fake-docserver-secret-long-enough";
        let cfg = serde_json::json!({"document": {"key": "abc"}});
        let tok = encode(&Header::default(), &cfg, &EncodingKey::from_secret(secret)).unwrap();
        let back = decode::<serde_json::Value>(&tok, &DecodingKey::from_secret(secret), &oo_validation()).unwrap();
        assert_eq!(back.claims["document"]["key"], "abc");
        assert!(decode::<serde_json::Value>(&tok, &DecodingKey::from_secret(b"wrong-secret-123456789012345"), &oo_validation()).is_err());
    }
}
