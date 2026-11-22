//! Thumbnail webp render phía server, cache vào S3.
//! GET /api/thumb/*path?s=256 -> image/webp, ETag + Cache-Control immutable.
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

fn thumb_object(hash: &str) -> String {
    format!(".nextkloud/thumbs/{hash}.webp")
}

/// Decode ảnh rồi resize vừa khung s×s, encode webp lossy.
pub fn render_image(data: &[u8], s: u32) -> Result<Vec<u8>, String> {
    let img = image::ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| format!("không đọc được ảnh: {e}"))?
        .decode()
        .map_err(|e| format!("không decode được ảnh: {e}"))?;
    let t = img.thumbnail(s, s);
    let rgba = t.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    let mut out = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut out)
        .encode(&rgba.into_raw(), w, h, image::ExtendedColorType::Rgba8)
        .map_err(|e| format!("không encode được webp: {e}"))?;
    Ok(out)
}

/// Cắt 1 frame video (giây thứ 1) bằng ffmpeg, trả về PNG bytes.
pub async fn video_frame_png(user: uuid::Uuid, path: &str, store: &crate::files::Store) -> Result<Vec<u8>, String> {
    use tokio::io::AsyncWriteExt;
    let key = store.key(user, path).map_err(|e| e.to_string())?;
    let meta = store.op.stat(&key).await.map_err(|e| e.to_string())?;
    let size = meta.content_length();
    let mut child = tokio::process::Command::new("ffmpeg")
        .args(["-v", "error", "-ss", "1", "-i", "pipe:0", "-frames:v", "1", "-vf", "scale=512:-1", "-f", "image2pipe", "-vcodec", "png", "pipe:1"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true) // quá hạn -> child bị drop -> ffmpeg bị kill, không đọc S3 tiếp
        .spawn()
        .map_err(|e| format!("không chạy được ffmpeg: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("không mở được stdin ffmpeg")?;
    let op = store.op.clone();
    let key2 = key.clone();
    // Bơm stream từng chunk từ S3 vào ffmpeg (không nạp cả file video vào RAM).
    let pump = tokio::spawn(async move {
        let mut s = match op.reader_with(&key2).chunk(4 * 1024 * 1024).await {
            Ok(r) => match r.into_bytes_stream(0..size).await {
                Ok(s) => s,
                Err(_) => return,
            },
            Err(_) => return,
        };
        use futures::TryStreamExt as _;
        while let Ok(Some(b)) = s.try_next().await {
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
    let _ = size;
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
    let size_px: u32 = q.get("s").and_then(|v| v.parse().ok()).unwrap_or(256).clamp(64, 1024);
    let meta = s.store.stat(user.id, &path).await?;
    if meta.is_dir() {
        return Err(AppError::BadRequest("not a file".into()));
    }
    let mtime = crate::files::mtime_of(&meta);
    let size = meta.content_length();
    let hash = thumb_key(&user.id, &path, mtime, size, size_px);
    let tkey = s.store.key(user.id, &thumb_object(&hash))?;
    let etag = format!("\"{hash}\"");
    if let Some(inm) = headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) {
        if inm.split(',').any(|t| t.trim() == etag) {
            return Ok(StatusCode::NOT_MODIFIED.into_response());
        }
    }
    // Cache hit?
    if let Ok(cached) = s.store.op.read(&tkey).await {
        return Ok(Response::builder()
            .header(header::CONTENT_TYPE, "image/webp")
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
        s.store.op.read(&s.store.key(user.id, &path)?).await.map_err(StoreError::from)?.to_vec()
    } else {
        video_frame_png(user.id, &path, &s.store).await.map_err(AppError::Internal)?
    };
    // Decode/resize tốn CPU: chạy ngoài worker async.
    let webp = tokio::task::spawn_blocking(move || render_image(&src, size_px))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .map_err(AppError::Internal)?;
    // Lưu cache, best-effort (lỗi thì vẫn trả ảnh vừa render).
    let _ = s.store.op.write(&tkey, webp.clone()).await;
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "image/webp")
        .header(header::ETAG, &etag)
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .body(Body::from(webp))
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
        // PNG 8x4 đỏ -> thumb webp, vẫn mở được.
        let mut img = image::RgbaImage::new(8, 4);
        for p in img.pixels_mut() {
            *p = image::Rgba([255, 0, 0, 255]);
        }
        let mut src = Vec::new();
        image::codecs::png::PngEncoder::new(&mut src)
            .write_image(&img, 8, 4, image::ExtendedColorType::Rgba8.into())
            .unwrap();
        let webp = render_image(&src, 256).unwrap();
        assert!(webp.starts_with(b"RIFF"));
        let back = image::ImageReader::new(Cursor::new(&webp))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap();
        assert!(back.width() <= 256 && back.height() <= 256);
    }

    #[test]
    fn render_rejects_non_image() {
        assert!(render_image(b"not an image at all", 256).is_err());
    }
}
