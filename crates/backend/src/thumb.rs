//! Thumbnail webp render phía server, cache vào S3.
//! GET /api/thumb/*path?s=256 -> JPEG (ảnh đục) hoặc WebP lossless (ảnh có trong suốt), ETag + Cache-Control immutable.
//! Hỗ trợ ảnh (decode bằng crate image) và video (cắt 1 frame bằng ffmpeg).
use std::collections::HashMap;
use std::io::Cursor;

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};

use crate::{
    files::StoreError,
    state::{AppError, AppState, AuthUser},
};

const IMG_EXT: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff"];
const VID_EXT: &[&str] = &["mp4", "m4v", "mov", "mkv", "webm", "avi", "3gp", "wmv", "flv", "mpg", "mpeg"];

/// true = ảnh, false = video. None = không hỗ trợ thumbnail.
pub fn thumb_kind(path: &str) -> Option<bool> {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if IMG_EXT.contains(&ext.as_str()) {
        Some(true)
    } else if VID_EXT.contains(&ext.as_str()) {
        Some(false)
    } else {
        None
    }
}

/// Cache key ổn định qua các lần restart (khác DefaultHasher).
pub fn thumb_key(user: &uuid::Uuid, path: &str, mtime: i64, size: u64, s: u32) -> String {
    let mut h = Sha256::new();
    h.update(user.as_bytes());
    h.update(path.as_bytes());
    h.update(mtime.to_be_bytes());
    h.update(size.to_be_bytes());
    h.update(s.to_be_bytes());
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

/// Key cache thumbnail dùng chung (kể cả link công khai): tránh render trùng.
pub(crate) fn thumb_object(hash: &str) -> String {
    format!(".nextkloud/thumbs/{hash}.webp")
}

/// Content-Type theo magic bytes (cache cũ là webp, mới có thể là jpeg).
pub fn mime_of(b: &[u8]) -> &'static str {
    if b.starts_with(b"RIFF") { "image/webp" } else { "image/jpeg" }
}

/// Decode ảnh, thu nhỏ vừa khung s×s (không phóng to). Ảnh có điểm trong suốt -> webp lossless, còn lại JPEG q85 (nhẹ hơn nhiều).
pub fn render_image(data: &[u8], s: u32) -> Result<Vec<u8>, String> {
    let img = image::ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| format!("không đọc được ảnh: {e}"))?
        .decode()
        .map_err(|e| format!("không decode được ảnh: {e}"))?;
    let t = if img.width() <= s && img.height() <= s { img } else { img.thumbnail(s, s) };
    let (w, h) = (t.width(), t.height());
    let mut out = Vec::new();
    let transparent = t.color().has_alpha() && t.to_rgba8().pixels().any(|p| p.0[3] < 255);
    if transparent {
        image::codecs::webp::WebPEncoder::new_lossless(&mut out)
            .encode(&t.to_rgba8().into_raw(), w, h, image::ExtendedColorType::Rgba8)
            .map_err(|e| format!("không encode được webp: {e}"))?;
    } else {
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85)
            .encode(&t.to_rgb8().into_raw(), w, h, image::ExtendedColorType::Rgb8)
            .map_err(|e| format!("không encode được jpeg: {e}"))?;
    }
    Ok(out)
}

/// Cắt 1 frame video (giây thứ 1) bằng ffmpeg, trả về PNG bytes.
pub async fn video_frame_png(user: uuid::Uuid, path: &str, store: &crate::files::Store) -> Result<Vec<u8>, String> {
    let key = store.key(user, path).map_err(|e| e.to_string())?;
    let meta = store.op.stat(&key).await.map_err(|e| e.to_string())?;
    let size = meta.content_length();
    let s = store.op.reader_with(&key).chunk(4 * 1024 * 1024).await.map_err(|e| e.to_string())?;
    let s = s.into_bytes_stream(0..size).await.map_err(|e| e.to_string())?;
    ffmpeg_frame(s).await
}

/// Bơm stream video vào ffmpeg (không nạp cả file vào RAM), lấy 1 frame PNG.
pub async fn ffmpeg_frame<S, E>(mut s: S) -> Result<Vec<u8>, String>
where
    S: futures::Stream<Item = Result<axum::body::Bytes, E>> + Send + Unpin + 'static,
    E: Send + 'static,
{
    use tokio::io::AsyncWriteExt;
    let mut child = tokio::process::Command::new("ffmpeg")
        .args(["-v", "error", "-ss", "1", "-i", "pipe:0", "-frames:v", "1", "-vf", "scale=512:-1", "-f", "image2pipe", "-vcodec", "png", "pipe:1"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true) // quá hạn -> child bị drop -> ffmpeg bị kill, không đọc S3 tiếp
        .spawn()
        .map_err(|e| format!("không chạy được ffmpeg: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("không mở được stdin ffmpeg")?;
    let pump = tokio::spawn(async move {
        use futures::StreamExt as _;
        while let Some(Ok(b)) = s.next().await {
            if stdin.write_all(&b).await.is_err() {
                break; // ffmpeg thoát sớm sau khi đủ frame -> broken pipe, bình thường
            }
        }
    });
    let out = match tokio::time::timeout(std::time::Duration::from_secs(60), child.wait_with_output()).await {
        Ok(r) => r.map_err(|e| format!("ffmpeg lỗi: {e}"))?,
        Err(_) => {
            pump.abort();
            return Err("ffmpeg quá 60 giây".into());
        }
    };
    let _ = pump.await;
    if !out.status.success() || out.stdout.is_empty() {
        return Err("ffmpeg không cắt được frame (cần image có ffmpeg)".into());
    }
    Ok(out.stdout)
}

pub async fn thumb(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(path): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    // Ảnh <img> không gửi được Bearer -> chấp nhận token qua ?t=.
    let user = auth_opt(&headers, &q, &s).await.ok_or_else(|| AppError::Auth("missing token".into()))?;
    let is_img = thumb_kind(&path).ok_or_else(|| AppError::BadRequest("loại file này chưa có thumbnail".into()))?;
    let size_px: u32 = q.get("s").and_then(|v| v.parse().ok()).unwrap_or(256).clamp(64, 2048);
    // file người khác chia sẻ: đọc + cache trong cây của chủ file, key gắn đúng path đã được cấp quyền
    let oid = crate::shares::resolve(&s.db, user.id, q.get("owner").and_then(|v| v.parse().ok()), &path, false).await?;
    let meta = s.store.stat(oid, &path).await?;
    if meta.is_dir() {
        return Err(AppError::BadRequest("not a file".into()));
    }
    let mtime = crate::files::mtime_of(&meta);
    let size = meta.content_length();
    let hash = thumb_key(&oid, &path, mtime, size, size_px);
    let tkey = s.store.key(oid, &thumb_object(&hash))?;
    let etag = format!("\"{hash}\"");
    if let Some(inm) = headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) {
        if inm.split(',').any(|t| t.trim() == etag) {
            return Ok(StatusCode::NOT_MODIFIED.into_response());
        }
    }
    // Cache hit?
    if let Ok(cached) = s.store.op.read(&tkey).await {
        return Ok(Response::builder()
            .header(header::CONTENT_TYPE, mime_of(&cached.to_vec()))
            .header(header::ETAG, &etag)
            .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
            .body(Body::from(cached.to_vec()))
            .unwrap());
    }
    let src = if is_img {
        // Ảnh phải nạp cả vào RAM để decode: chặn > 100MB. Video thì stream vào ffmpeg, không giới hạn.
        if size > 100 * 1024 * 1024 {
            return Err(AppError::BadRequest("ảnh quá lớn để tạo thumbnail".into()));
        }
        s.store.op.read(&s.store.key(oid, &path)?).await.map_err(StoreError::from)?.to_vec()
    } else {
        video_frame_png(oid, &path, &s.store).await.map_err(AppError::Internal)?
    };
    // Decode/resize tốn CPU: chạy ngoài worker async.
    let out = tokio::task::spawn_blocking(move || render_image(&src, size_px))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .map_err(AppError::Internal)?;
    // Lưu cache, best-effort (lỗi thì vẫn trả ảnh vừa render).
    let _ = s.store.op.write(&tkey, out.clone()).await;
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, mime_of(&out))
        .header(header::ETAG, &etag)
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .body(Body::from(out))
        .unwrap())
}

/// Cho phép token qua query (?t=) cho <img>/<video> không gửi được header.
pub async fn auth_opt(
    headers: &axum::http::HeaderMap,
    q: &HashMap<String, String>,
    s: &AppState,
) -> Option<AuthUser> {
    use axum::http::header;
    if let Some(h) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        if let Some(tok) = h.strip_prefix("Bearer ") {
            if let Ok(c) = crate::auth::verify_jwt(tok, &s.dec) {
                return Some(AuthUser { id: c.sub, admin: c.adm });
            }
        }
    }
    if let Some(tok) = q.get("t") {
        if let Ok(c) = crate::auth::verify_jwt(tok, &s.dec) {
            return Some(AuthUser { id: c.sub, admin: c.adm });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_detection() {
        assert_eq!(thumb_kind("a.JPG"), Some(true));
        assert_eq!(thumb_kind("v/clip.MP4"), Some(false));
        assert_eq!(thumb_kind("doc.pdf"), None);
        assert_eq!(thumb_kind("src/main.ts"), None); // TypeScript, không phải MPEG-TS
        assert_eq!(thumb_kind("noext"), None);
    }

    #[test]
    fn key_stable_and_sensitive() {
        let u = uuid::Uuid::nil();
        let a = thumb_key(&u, "a.png", 1, 2, 256);
        assert_eq!(a, thumb_key(&u, "a.png", 1, 2, 256));
        assert_ne!(a, thumb_key(&u, "a.png", 1, 3, 256));
        assert_ne!(a, thumb_key(&u, "a.png", 1, 2, 512));
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn render_png_thumbnail() {
        use image::ImageEncoder as _;
        // PNG 600x300: đục -> JPEG, có trong suốt -> webp; thu về khung 256, ảnh nhỏ giữ nguyên cỡ.
        let png = |a: u8| {
            let mut img = image::RgbaImage::new(600, 300);
            for p in img.pixels_mut() {
                *p = image::Rgba([255, 0, 0, a]);
            }
            let mut src = Vec::new();
            image::codecs::png::PngEncoder::new(&mut src)
                .write_image(&img, 600, 300, image::ExtendedColorType::Rgba8.into())
                .unwrap();
            src
        };
        let dims = |b: &[u8]| {
            let i = image::ImageReader::new(Cursor::new(b)).with_guessed_format().unwrap().decode().unwrap();
            (i.width(), i.height())
        };
        let j = render_image(&png(255), 256).unwrap();
        assert_eq!((mime_of(&j), dims(&j)), ("image/jpeg", (256, 128)));
        let w = render_image(&png(100), 256).unwrap();
        assert_eq!((mime_of(&w), dims(&w)), ("image/webp", (256, 128)));
        assert_eq!(dims(&render_image(&png(255), 2048).unwrap()), (600, 300));
    }

    #[test]
    fn render_rejects_non_image() {
        assert!(render_image(b"not an image at all", 256).is_err());
    }
}
