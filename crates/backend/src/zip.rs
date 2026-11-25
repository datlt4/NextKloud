//! Đọc central directory của zip bằng Range (không tải cả file),
//! và giải nén 1 file con để tải về (giới hạn: nén ≤ 256MB, bung ≤ 2GB).
//! GET /api/zip/list?path=... -> {entries, total, truncated}
//! GET /api/zip/file?path=...&entry=... -> file con (attachment)
use std::collections::HashMap;

use axum::{
    Json,
    body::Body,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::Response,
};
use crc32fast::Hasher;
use serde::Serialize;

use crate::{
    files::Store,
    state::{AppError, AppState},
    thumb::auth_opt,
};

const MAX_ZIP: u64 = 20 * 1024 * 1024 * 1024;
const MAX_CD: u64 = 256 * 1024 * 1024;
const MAX_ENTRY_COMP: u64 = 256 * 1024 * 1024;
const MAX_ENTRY_OUT: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone)]
struct CdEntry {
    name: String,
    method: u16,
    crc: u32,
    comp_size: u64,
    uncomp_size: u64,
    local_offset: u64,
    is_dir: bool,
    encrypted: bool,
}

fn u16le(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}
fn u32le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}
fn u64le(b: &[u8]) -> u64 {
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

async fn fetch(op: &opendal::Operator, key: &str, start: u64, end: u64) -> Result<Vec<u8>, AppError> {
    use futures::TryStreamExt as _;
    if start >= end {
        return Ok(Vec::new());
    }
    let mut s = op
        .reader_with(key)
        .chunk(4 * 1024 * 1024)
        .await
        .map_err(|e| AppError::Internal(format!("đọc zip: {e}")))?
        .into_bytes_stream(start..end)
        .await
        .map_err(|e| AppError::Internal(format!("đọc zip: {e}")))?;
    let mut out = Vec::new();
    while let Some(b) = s.try_next().await.map_err(|e| AppError::Internal(format!("đọc zip: {e}")))? {
        out.extend_from_slice(&b);
    }
    Ok(out)
}

/// Tìm End-Of-Central-Directory trong đuôi file, trả (cd_offset, cd_size).
/// Hỗ trợ cả Zip64.
async fn find_cd(op: &opendal::Operator, key: &str, size: u64) -> Result<(u64, u64), AppError> {
    let tail_len = size.min(70000);
    let tail = fetch(op, key, size - tail_len, size).await?;
    // Quét ngược tìm signature EOCD; file không comment thì EOCD nằm sát cuối (i = len-22).
    let mut eocd_at = None;
    for i in (0..tail.len().saturating_sub(21)).rev() {
        if i + 22 > tail.len() {
            continue;
        }
        if tail[i..].starts_with(&[0x50, 0x4b, 0x05, 0x06]) {
            eocd_at = Some(i);
            break;
        }
    }
    let i = eocd_at.ok_or_else(|| AppError::BadRequest("không phải file zip (thiếu EOCD)".into()))?;
    let e = &tail[i..];
    if e.len() < 22 {
        return Err(AppError::BadRequest("EOCD bị cắt cụt".into()));
    }
    let mut cd_size = u32le(&e[12..16]) as u64;
    let mut cd_offset = u32le(&e[16..20]) as u64;
    if cd_size == 0xFFFF_FFFF || cd_offset == 0xFFFF_FFFF || u16le(&e[8..10]) == 0xFFFF {
        if i < 20 || !tail[i - 20..].starts_with(&[0x50, 0x4b, 0x07, 0x06]) {
            return Err(AppError::BadRequest("zip64 thiếu locator".into()));
        }
        let zoff = u64le(&tail[i - 12..i - 4]);
        let z = fetch(op, key, zoff, zoff + 56).await?;
        if z.len() < 56 || !z.starts_with(&[0x50, 0x4b, 0x06, 0x06]) {
            return Err(AppError::BadRequest("zip64 EOCD hỏng".into()));
        }
        cd_size = u64le(&z[40..48]);
        cd_offset = u64le(&z[48..56]);
    }
    if cd_offset.checked_add(cd_size).is_none_or(|end| end > size) {
        return Err(AppError::BadRequest("central directory nằm ngoài file".into()));
    }
    Ok((cd_offset, cd_size))
}

/// Parse central directory thành danh sách entry (kể cả Zip64).
async fn read_cd(op: &opendal::Operator, key: &str, size: u64) -> Result<Vec<CdEntry>, AppError> {
    let (off, len) = find_cd(op, key, size).await?;
    if len > MAX_CD {
        return Err(AppError::BadRequest("zip quá nhiều file (CD > 256MB)".into()));
    }
    let cd = fetch(op, key, off, off + len).await?;
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 46 <= cd.len() {
        if !cd[i..].starts_with(&[0x50, 0x4b, 0x01, 0x02]) {
            break;
        }
        let e = &cd[i..];
        let flags = u16le(&e[8..10]);
        let method = u16le(&e[10..12]);
        let crc = u32le(&e[16..20]);
        let mut comp = u32le(&e[20..24]) as u64;
        let mut uncomp = u32le(&e[24..28]) as u64;
        let mut local_off = u32le(&e[42..46]) as u64;
        let nl = u16le(&e[28..30]) as usize;
        let el = u16le(&e[30..32]) as usize;
        let cl = u16le(&e[32..34]) as usize;
        if i + 46 + nl + el + cl > cd.len() {
            break;
        }
        let name = String::from_utf8_lossy(&cd[i + 46..i + 46 + nl]).into_owned();
        if comp == 0xFFFF_FFFF || uncomp == 0xFFFF_FFFF || local_off == 0xFFFF_FFFF {
            let mut x = i + 46 + nl;
            let xend = x + el;
            while x + 4 <= xend.min(cd.len()) {
                let tag = u16le(&cd[x..x + 2]);
                let sz = u16le(&cd[x + 2..x + 4]) as usize;
                if tag == 0x0001 {
                    let mut p = x + 4;
                    if uncomp == 0xFFFF_FFFF && p + 8 <= xend {
                        uncomp = u64le(&cd[p..p + 8]);
                        p += 8;
                    }
                    if comp == 0xFFFF_FFFF && p + 8 <= xend {
                        comp = u64le(&cd[p..p + 8]);
                        p += 8;
                    }
                    if local_off == 0xFFFF_FFFF && p + 8 <= xend {
                        local_off = u64le(&cd[p..p + 8]);
                    }
                    let _ = sz;
                    break;
                }
                x += 4 + sz;
            }
        }
        out.push(CdEntry {
            is_dir: name.ends_with('/'),
            encrypted: flags & 0x1 != 0,
            name,
            method,
            crc,
            comp_size: comp,
            uncomp_size: uncomp,
            local_offset: local_off,
        });
        i += 46 + nl + el + cl;
    }
    if out.is_empty() {
        return Err(AppError::BadRequest("không đọc được entry nào".into()));
    }
    Ok(out)
}

/// Offset bắt đầu dữ liệu nén của 1 entry (qua local header).
async fn data_start(op: &opendal::Operator, key: &str, e: &CdEntry) -> Result<u64, AppError> {
    let h = fetch(op, key, e.local_offset, e.local_offset + 30).await?;
    if h.len() < 30 || !h.starts_with(&[0x50, 0x4b, 0x03, 0x04]) {
        return Err(AppError::BadRequest("local header hỏng".into()));
    }
    let nl = u16le(&h[26..28]) as u64;
    let el = u16le(&h[28..30]) as u64;
    Ok(e.local_offset + 30 + nl + el)
}

async fn open_entries(store: &Store, user: uuid::Uuid, path: &str) -> Result<(String, Vec<CdEntry>), AppError> {
    let clean = crate::files::clean_path(path).map_err(AppError::from)?;
    if !matches!(clean.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str(), "zip" | "cbz") {
        return Err(AppError::BadRequest("không phải file zip".into()));
    }
    let meta = store.stat(user, &clean).await?;
    if meta.is_dir() {
        return Err(AppError::BadRequest("not a file".into()));
    }
    let size = meta.content_length();
    if size > MAX_ZIP {
        return Err(AppError::BadRequest("zip quá lớn (>20GB)".into()));
    }
    if size < 22 {
        return Err(AppError::BadRequest("không phải file zip".into()));
    }
    let key = store.key(user, &clean)?;
    let entries = read_cd(&store.op, &key, size).await?;
    Ok((key, entries))
}

#[derive(Serialize)]
pub struct ZipEntry {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
}

#[derive(Serialize)]
pub struct ZipList {
    pub entries: Vec<ZipEntry>,
    pub total: usize,
    pub truncated: bool,
}

pub async fn list(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<ZipList>, AppError> {
    let user = auth_opt(&headers, &q, &s).await.ok_or_else(|| AppError::Auth("missing token".into()))?;
    let path = q.get("path").ok_or_else(|| AppError::BadRequest("thiếu path".into()))?;
    let limit: usize = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(20000).clamp(100, 100_000);
    let (_, entries) = open_entries(&s.store, user.id, path).await?;
    let total = entries.len();
    let truncated = total > limit;
    let list = entries
        .into_iter()
        .take(limit)
        .map(|e| ZipEntry { name: e.name, size: e.uncomp_size, is_dir: e.is_dir })
        .collect();
    Ok(Json(ZipList { entries: list, total, truncated }))
}

/// Tải 1 file con: stored/deflate + verify CRC32.
pub async fn extract(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>
) -> Result<Response, AppError> {
    let user = auth_opt(&headers, &q, &s).await.ok_or_else(|| AppError::Auth("missing token".into()))?;
    let path = q.get("path").cloned().ok_or_else(|| AppError::BadRequest("thiếu path".into()))?;
    let want = q.get("entry").cloned().ok_or_else(|| AppError::BadRequest("thiếu entry".into()))?;
    let (key, entries) = open_entries(&s.store, user.id, &path).await?;
    let e = entries.into_iter().find(|e| e.name == want).ok_or_else(|| AppError::NotFound("không thấy file trong zip".into()))?;
    if e.is_dir {
        return Err(AppError::BadRequest("entry là thư mục".into()));
    }
    if e.encrypted {
        return Err(AppError::BadRequest("zip có mật khẩu, chưa hỗ trợ".into()));
    }
    if e.method != 0 && e.method != 8 {
        return Err(AppError::BadRequest(format!("kiểu nén {} chưa hỗ trợ (chỉ stored/deflate)", e.method)));
    }
    if e.comp_size > MAX_ENTRY_COMP || e.uncomp_size > MAX_ENTRY_OUT {
        return Err(AppError::BadRequest("file con quá lớn, hãy tải cả file zip".into()));
    }
    let data_off = data_start(&s.store.op, &key, &e).await?;
    let end = data_off.checked_add(e.comp_size).ok_or_else(|| AppError::BadRequest("zip hỏng (size tràn)".into()))?;
    let comp = fetch(&s.store.op, &key, data_off, end).await?;
    if comp.len() as u64 != e.comp_size {
        return Err(AppError::BadRequest("zip bị cắt cụt".into()));
    }
    let raw: Vec<u8> = if e.method == 0 {
        comp
    } else {
        inflate(&comp, e.uncomp_size).map_err(|er| AppError::BadRequest(format!("giải nén lỗi: {er}")))?
    };
    if raw.len() as u64 != e.uncomp_size {
        return Err(AppError::BadRequest("độ dài sau giải nén không khớp".into()));
    }
    let mut hasher = Hasher::new();
    hasher.update(&raw);
    if hasher.finalize() != e.crc {
        return Err(AppError::BadRequest("sai checksum (file hỏng hoặc cần mật khẩu)".into()));
    }
    let fname = e.name.rsplit('/').next().unwrap_or("file").to_string();
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, crate::routes::content_type(&fname))
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{fname}\""))
        .body(Body::from(raw))
        .unwrap())
}

/// Tải cả thư mục thành 1 file zip (stream trực tiếp, kiểu store/không nén).
/// Không file tạm, RAM cố định bất kể thư mục to cỡ nào. Giới hạn: 200k file, tổng 100GB.
pub async fn download_dir(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let user = auth_opt(&headers, &q, &s).await.ok_or_else(|| AppError::Auth("missing token".into()))?;
    let raw = q.get("path").cloned().unwrap_or_default();
    let prefix = if raw.trim().is_empty() { String::new() } else { crate::ops::user_path(&raw)? };
    let like = format!("{}/%", crate::index::like_escape(&prefix));
    let files: Vec<(String, i64, i64)> = if prefix.is_empty() {
        sqlx::query_as("SELECT path, size, modified FROM files WHERE user_id = $1 AND NOT is_dir AND path NOT LIKE '.nextkloud/%' ORDER BY path LIMIT 200001")
            .bind(user.id)
            .fetch_all(&s.db)
            .await?
    } else {
        sqlx::query_as(
            "SELECT path, size, modified FROM files WHERE user_id = $1 AND NOT is_dir AND path NOT LIKE '.nextkloud/%'
             AND (path = $2 OR path LIKE $3) ORDER BY path LIMIT 200001",
        )
        .bind(user.id)
        .bind(&prefix)
        .bind(&like)
        .fetch_all(&s.db)
        .await?
    };
    if files.len() > 200_000 {
        return Err(AppError::BadRequest("thư mục quá nhiều file (>200.000), chia nhỏ ra".into()));
    }
    let total: i64 = files.iter().map(|f| f.1).sum();
    if total > 100 * 1024 * 1024 * 1024 {
        return Err(AppError::BadRequest("thư mục quá lớn (>100GB), chia nhỏ ra".into()));
    }
    if files.is_empty() {
        // phân biệt thư mục rỗng (zip chỉ có entry thư mục) với path không tồn tại
        if prefix.is_empty() || crate::ops::kind(&s.db, user.id, &prefix).await?.is_none() {
            return Err(AppError::NotFound("thư mục trống hoặc không tồn tại".into()));
        }
    }
    // tên trong zip: đường dẫn tương đối so với thư mục được tải
    // (path là file lẻ -> zip chứa đúng 1 file đó)
    let rel = |p: &str| {
        if prefix.is_empty() {
            p.to_string()
        } else if p == prefix {
            p.rsplit('/').next().unwrap_or(p).to_string()
        } else {
            p[prefix.len() + 1..].to_string()
        }
    };
    // thư mục (kể cả rỗng): suy từ đường dẫn file
    let mut dirs = std::collections::BTreeSet::new();
    for (p, _, _) in &files {
        let r = rel(p);
        let mut d = String::new();
        for part in r.split('/').take(r.split('/').count().saturating_sub(1)) {
            if !d.is_empty() {
                d.push('/');
            }
            d.push_str(part);
            dirs.insert(format!("{d}/"));
        }
    }
    let mut entries: Vec<Ze> = dirs.into_iter().map(|name| Ze { name, full: String::new(), size: 0, modified: 0, is_dir: true }).collect();
    for (p, sz, m) in &files {
        entries.push(Ze { name: rel(p), full: p.clone(), size: (*sz).max(0) as u64, modified: *m, is_dir: false });
    }
    // thứ tự + giờ cố định -> cùng nội dung chỉ mục thì cùng byte, phục vụ Range được
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let need64 = total as u64 >= 0xFFFF_FFFF || files.iter().any(|f| f.1 as u64 >= 0xFFFF_FFFF);
    let lay = layout(&entries, need64);
    let size = lay.total;
    let etag = etag(&entries);
    // If-Range lệch (thư mục đã đổi) -> trả cả file. Đổi nội dung mà giữ nguyên size/mtime thì không phát hiện được.
    let fresh = headers.get(header::IF_RANGE).and_then(|v| v.to_str().ok()).is_none_or(|v| v == etag);
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok()).filter(|_| fresh).map(|h| crate::file_handlers::parse_range(h, size));
    let (a, b) = match range {
        Some(Some(r)) => r,
        Some(None) => {
            return Ok(Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{size}"))
                .body(Body::empty())
                .unwrap());
        }
        None => (0, size),
    };

    let (tx, rx) = futures::channel::mpsc::channel::<Result<Vec<u8>, std::io::Error>>(8);
    let op = s.store.op.clone();
    let uid = user.id;
    tokio::spawn(async move {
        if let Err(e) = zip_range(&op, uid, &entries, &lay, a, b, tx.clone()).await {
            let _ = tx.clone().try_send(Err(std::io::Error::new(std::io::ErrorKind::Other, format!("{e:?}"))));
        }
    });
    let fname = if prefix.is_empty() { "files".to_string() } else { prefix.rsplit('/').next().unwrap_or("files").to_string() };
    let mut res = Response::builder()
        .status(if range.is_some() { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK })
        .header(header::CONTENT_TYPE, "application/zip")
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename*=UTF-8''{}.zip", pct(&fname)))
        .header(header::CONTENT_LENGTH, b - a)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::ETAG, etag);
    if range.is_some() {
        res = res.header(header::CONTENT_RANGE, format!("bytes {a}-{}/{size}", b - 1));
    }
    Ok(res.body(Body::from_stream(rx)).unwrap())
}

#[derive(Clone)]
struct Ze {
    /// tên trong zip (tương đối)
    name: String,
    /// path đầy đủ trong cây user (để đọc bytes)
    full: String,
    size: u64,
    /// chỉ để làm khoá cache CRC / ETag
    modified: i64,
    is_dir: bool,
}

fn w16(v: u16, o: &mut Vec<u8>) {
    o.extend_from_slice(&v.to_le_bytes());
}
fn w32(v: u32, o: &mut Vec<u8>) {
    o.extend_from_slice(&v.to_le_bytes());
}
fn w64(v: u64, o: &mut Vec<u8>) {
    o.extend_from_slice(&v.to_le_bytes());
}
/// Percent-encode tên file zip (tiếng Việt/khoảng trắng).
pub(crate) fn pct(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

// date cố định 2020-01-01 (tránh phụ thuộc múi giờ server, giữ zip giống hệt giữa các request)
const TIME: u16 = 0;
const DATE: u16 = ((2020 - 1980) << 9) | (1 << 5) | 1;

/// Vị trí từng phần trong zip: tính trước được vì kiểu store + size lấy từ chỉ mục.
struct Layout {
    /// offset local header từng entry
    off: Vec<u64>,
    z64: Vec<bool>,
    cd_off: u64,
    cd_len: u64,
    z64end: bool,
    total: u64,
}

fn ver(z64: bool) -> u16 {
    if z64 { 45 } else { 20 }
}
fn hdr_len(e: &Ze, z64: bool) -> u64 {
    30 + e.name.len() as u64 + if z64 { 28 } else { 0 }
}
fn desc_len(z64: bool) -> u64 {
    if z64 { 24 } else { 16 }
}
fn rec_len(e: &Ze, z64: bool) -> u64 {
    46 + e.name.len() as u64 + if z64 { 28 } else { 0 }
}

fn layout(entries: &[Ze], need64: bool) -> Layout {
    let (mut off, mut offs, mut z64s, mut cd_len) = (0u64, vec![], vec![], 0u64);
    for e in entries {
        let z = need64 || e.size >= 0xFFFF_FFFF || off >= 0xFFFF_FFFF;
        offs.push(off);
        z64s.push(z);
        off += hdr_len(e, z) + e.size + desc_len(z);
        cd_len += rec_len(e, z);
    }
    let z64end = need64 || entries.len() >= 0xFFFF || cd_len >= 0xFFFF_FFFF || off >= 0xFFFF_FFFF;
    let total = off + cd_len + if z64end { 56 + 20 } else { 0 } + 22;
    Layout { off: offs, z64: z64s, cd_off: off, cd_len, z64end, total }
}

/// Local header (bit 3: CRC/size nằm ở descriptor sau data).
fn local_header(e: &Ze, off: u64, z64: bool) -> Vec<u8> {
    let name = e.name.as_bytes();
    let mut h = Vec::with_capacity(hdr_len(e, z64) as usize);
    w32(0x04034b50, &mut h);
    w16(ver(z64), &mut h);
    w16(0x0808, &mut h); // bit 3 descriptor + bit 11 UTF-8
    w16(0, &mut h); // store
    w16(TIME, &mut h);
    w16(DATE, &mut h);
    w32(0, &mut h); // crc (xem descriptor)
    if z64 {
        w32(0xFFFF_FFFF, &mut h);
        w32(0xFFFF_FFFF, &mut h);
    } else {
        w32(e.size as u32, &mut h);
        w32(e.size as u32, &mut h);
    }
    w16(name.len() as u16, &mut h);
    w16(if z64 { 28 } else { 0 }, &mut h);
    h.extend_from_slice(name);
    if z64 {
        w16(0x0001, &mut h);
        w16(24, &mut h);
        w64(e.size, &mut h);
        w64(e.size, &mut h);
        w64(off, &mut h);
    }
    h
}

fn descriptor(e: &Ze, crc: u32, z64: bool) -> Vec<u8> {
    let mut d = Vec::with_capacity(desc_len(z64) as usize);
    w32(0x08074b50, &mut d);
    w32(crc, &mut d);
    if z64 {
        w64(e.size, &mut d);
        w64(e.size, &mut d);
    } else {
        w32(e.size as u32, &mut d);
        w32(e.size as u32, &mut d);
    }
    d
}

fn central(e: &Ze, crc: u32, local_off: u64, z64: bool) -> Vec<u8> {
    let name = e.name.as_bytes();
    let mut c = Vec::with_capacity(rec_len(e, z64) as usize);
    w32(0x02014b50, &mut c);
    w16(63 << 8 | ver(z64), &mut c);
    w16(ver(z64), &mut c);
    w16(0x0808, &mut c);
    w16(0, &mut c);
    w16(TIME, &mut c);
    w16(DATE, &mut c);
    w32(crc, &mut c);
    if z64 {
        w32(0xFFFF_FFFF, &mut c);
        w32(0xFFFF_FFFF, &mut c);
    } else {
        w32(e.size as u32, &mut c);
        w32(e.size as u32, &mut c);
    }
    w16(name.len() as u16, &mut c);
    w16(if z64 { 28 } else { 0 }, &mut c);
    w16(0, &mut c); // comment
    w16(0, &mut c); // disk
    w16(0, &mut c); // int attr
    w32(if e.is_dir { 0x10 << 16 } else { 0 }, &mut c);
    w32(if z64 { 0xFFFF_FFFF } else { local_off as u32 }, &mut c);
    c.extend_from_slice(name);
    if z64 {
        w16(0x0001, &mut c);
        w16(24, &mut c);
        w64(e.size, &mut c);
        w64(e.size, &mut c);
        w64(local_off, &mut c);
    }
    c
}

/// Zip64 EOCD + locator (nếu cần) + EOCD.
fn tail(n: u64, lay: &Layout) -> Vec<u8> {
    let (cd_off, cd_len) = (lay.cd_off, lay.cd_len);
    let mut t = Vec::with_capacity(98);
    if lay.z64end {
        w32(0x06064b50, &mut t);
        w64(44, &mut t);
        w16(45, &mut t);
        w16(45, &mut t);
        w32(0, &mut t);
        w32(0, &mut t);
        w64(n, &mut t);
        w64(n, &mut t);
        w64(cd_len, &mut t);
        w64(cd_off, &mut t);
        w32(0x07064b50, &mut t);
        w32(0, &mut t);
        w64(cd_off + cd_len, &mut t);
        w32(1, &mut t);
    }
    w32(0x06054b50, &mut t);
    w16(0, &mut t);
    w16(0, &mut t);
    w16(n.min(0xFFFF) as u16, &mut t);
    w16(n.min(0xFFFF) as u16, &mut t);
    w32(cd_len.min(0xFFFF_FFFF) as u32, &mut t);
    w32(cd_off.min(0xFFFF_FFFF) as u32, &mut t);
    w16(0, &mut t);
    t
}

/// ETag theo danh sách entry (tên, size, mtime): thư mục đổi thì tải tiếp phải bắt đầu lại.
fn etag(entries: &[Ze]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for e in entries {
        h.update(e.name.as_bytes());
        h.update([0]);
        h.update(e.size.to_le_bytes());
        h.update(e.modified.to_le_bytes());
    }
    let d = h.finalize();
    format!("\"z{}\"", d[..12].iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// CRC các file đã đọc qua khi tải zip: request Range sau (tải tiếp) khỏi đọc lại từ đầu.
/// Khoá (user, path, size, mtime): nội dung đổi mà giữ nguyên size/mtime thì CRC cũ sẽ sai.
type CrcKey = (uuid::Uuid, String, u64, i64);
static CRCS: std::sync::LazyLock<std::sync::Mutex<HashMap<CrcKey, u32>>> = std::sync::LazyLock::new(Default::default);
const CRC_CACHE_MAX: usize = 500_000;

fn crc_key(user: uuid::Uuid, e: &Ze) -> CrcKey {
    (user, e.full.clone(), e.size, e.modified)
}

/// Gom các mảnh nằm trong [a, b) rồi gửi qua channel.
struct Sink {
    tx: futures::channel::mpsc::Sender<Result<Vec<u8>, std::io::Error>>,
    a: u64,
    b: u64,
    buf: Vec<u8>,
}

impl Sink {
    fn hits(&self, s: u64, e: u64) -> bool {
        s < e && s < self.b && e > self.a
    }
    /// false = client đã huỷ tải
    async fn put(&mut self, start: u64, bytes: &[u8]) -> bool {
        let end = start + bytes.len() as u64;
        if !self.hits(start, end) {
            return true;
        }
        let (i, j) = (self.a.max(start) - start, self.b.min(end) - start);
        self.buf.extend_from_slice(&bytes[i as usize..j as usize]);
        self.buf.len() < 1 << 20 || self.flush().await
    }
    async fn flush(&mut self) -> bool {
        use futures::SinkExt as _;
        // send chờ (không try_send): channel đầy thì producer dừng theo, không mất chunk
        self.buf.is_empty() || self.tx.send(Ok(std::mem::take(&mut self.buf))).await.is_ok()
    }
}

/// Ghi byte [a, b) của zip store-method. CRC lấy từ cache; thiếu mà descriptor/central record
/// nằm trong range thì đọc cả file để tính (tải tiếp lúc server vừa khởi động lại sẽ chậm).
async fn zip_range(
    op: &opendal::Operator,
    user: uuid::Uuid,
    entries: &[Ze],
    lay: &Layout,
    a: u64,
    b: u64,
    tx: futures::channel::mpsc::Sender<Result<Vec<u8>, std::io::Error>>,
) -> Result<(), AppError> {
    use futures::TryStreamExt as _;
    let mut s = Sink { tx, a, b, buf: Vec::new() };
    macro_rules! put {
        ($at:expr, $b:expr) => {
            // client đã huỷ tải -> dừng, khỏi đọc tiếp cả thư mục từ S3
            if !s.put($at, $b).await {
                return Ok(());
            }
        };
    }
    let mut crcs: Vec<Option<u32>> = {
        let c = CRCS.lock().unwrap();
        entries.iter().map(|e| if e.size == 0 { Some(0) } else { c.get(&crc_key(user, e)).copied() }).collect()
    };
    // entry nào cần CRC: descriptor hoặc central record của nó nằm trong range
    let mut need = Vec::with_capacity(entries.len());
    let mut rec = lay.cd_off;
    for (i, e) in entries.iter().enumerate() {
        let z = lay.z64[i];
        let d = lay.off[i] + hdr_len(e, z) + e.size;
        let rl = rec_len(e, z);
        need.push(s.hits(d, d + desc_len(z)) || s.hits(rec, rec + rl));
        rec += rl;
    }
    for (i, e) in entries.iter().enumerate() {
        let (off, z) = (lay.off[i], lay.z64[i]);
        if off >= b {
            break;
        }
        put!(off, &local_header(e, off, z));
        let d0 = off + hdr_len(e, z);
        let d1 = d0 + e.size;
        // cần CRC, hoặc cả file nằm trong range (đọc hết thì tính CRC luôn)
        let full = crcs[i].is_none() && (need[i] || (d0 >= a && d1 <= b));
        if full || s.hits(d0, d1) {
            let (r0, r1) = if full { (0, e.size) } else { (a.max(d0) - d0, b.min(d1) - d0) };
            let key = format!("{user}/{}", e.full);
            let mut st = op
                .reader_with(&key)
                .chunk(4 * 1024 * 1024)
                .await
                .map_err(|er| AppError::Internal(format!("đọc {key}: {er}")))?
                .into_bytes_stream(r0..r1)
                .await
                .map_err(|er| AppError::Internal(format!("đọc {key}: {er}")))?;
            let mut hasher = Hasher::new();
            let mut pos = r0;
            while let Some(c) = st.try_next().await.map_err(|er| AppError::Internal(format!("đọc {key}: {er}")))? {
                if full {
                    hasher.update(&c);
                }
                put!(d0 + pos, &c);
                pos += c.len() as u64;
            }
            if pos != r1 {
                return Err(AppError::Internal(format!("file đổi giữa chừng: {}", e.full)));
            }
            if full {
                let crc = hasher.finalize();
                crcs[i] = Some(crc);
                let mut c = CRCS.lock().unwrap();
                if c.len() >= CRC_CACHE_MAX {
                    c.clear();
                }
                c.insert(crc_key(user, e), crc);
            }
        }
        if s.hits(d1, d1 + desc_len(z)) {
            let crc = crcs[i].ok_or_else(|| AppError::Internal("thiếu CRC".into()))?;
            put!(d1, &descriptor(e, crc, z));
        }
    }
    if b > lay.cd_off {
        let mut rec = lay.cd_off;
        for (i, e) in entries.iter().enumerate() {
            let rl = rec_len(e, lay.z64[i]);
            if s.hits(rec, rec + rl) {
                let crc = crcs[i].ok_or_else(|| AppError::Internal("thiếu CRC".into()))?;
                put!(rec, &central(e, crc, lay.off[i], lay.z64[i]));
            }
            rec += rl;
        }
        put!(rec, &tail(entries.len() as u64, lay));
    }
    s.flush().await;
    Ok(())
}
fn inflate(comp: &[u8], want: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::with_capacity(want as usize);
    flate2::read::DeflateDecoder::new(comp).take(want + 1).read_to_end(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, FlushCompress, Compress};

    /// Dựng 1 file zip hợp lệ trong RAM: entries (name, data, method).
    fn make_zip(entries: &[(&str, &[u8], u16)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut cd = Vec::new();
        for (name, data, method) in entries {
            let local_off = out.len() as u32;
            let comp: Vec<u8> = if *method == 8 {
                let mut c = Compress::new(Compression::default(), false); // zip = raw deflate
                let mut o = vec![0u8; data.len() + 64];
                o.clear();
                c.compress_vec(data, &mut o, FlushCompress::Finish).unwrap();
                o
            } else {
                data.to_vec()
            };
            let mut h = Hasher::new();
            h.update(data);
            let crc = h.finalize();
            let push16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_le_bytes());
            let push32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
            out.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04]);
            push16(&mut out, 20);
            push16(&mut out, 0);
            push16(&mut out, *method);
            push16(&mut out, 0);
            push16(&mut out, 0);
            push32(&mut out, crc);
            push32(&mut out, comp.len() as u32);
            push32(&mut out, data.len() as u32);
            push16(&mut out, name.len() as u16);
            push16(&mut out, 0);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&comp);
            // central directory
            cd.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02]);
            push16(&mut cd, 20);
            push16(&mut cd, 20);
            push16(&mut cd, 0);
            push16(&mut cd, *method);
            push16(&mut cd, 0);
            push16(&mut cd, 0);
            push32(&mut cd, crc);
            push32(&mut cd, comp.len() as u32);
            push32(&mut cd, data.len() as u32);
            push16(&mut cd, name.len() as u16);
            push16(&mut cd, 0);
            push16(&mut cd, 0);
            push16(&mut cd, 0);
            push16(&mut cd, 0);
            push32(&mut cd, 0);
            push32(&mut cd, local_off);
            cd.extend_from_slice(name.as_bytes());
        }
        let cd_off = out.len() as u32;
        let cd_len = cd.len() as u32;
        out.extend_from_slice(&cd);
        out.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06]);
        out.extend_from_slice(&[0u8; 4]); // disk numbers
        let mut tmp = Vec::new();
        let push16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_le_bytes());
        let push32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
        let n = entries.len();
        push16(&mut tmp, n as u16);
        push16(&mut tmp, n as u16);
        push32(&mut tmp, cd_len);
        push32(&mut tmp, cd_off);
        push16(&mut tmp, 0);
        out.extend_from_slice(&tmp);
        out
    }

    fn cd_of(data: &[u8]) -> Vec<CdEntry> {
        // Parse trực tiếp từ buffer RAM bằng cùng logic (qua OpReader giả lập).
        let mut out = Vec::new();
        // Tìm EOCD (file không comment thì EOCD nằm sát cuối).
        let mut at = None;
        for i in (0..data.len().saturating_sub(21)).rev() {
            if i + 22 > data.len() {
                continue;
            }
            if data[i..].starts_with(&[0x50, 0x4b, 0x05, 0x06]) {
                at = Some(i);
                break;
            }
        }
        let Some(i) = at else { return out };
        let off = u32le(&data[i + 16..i + 20]) as usize;
        let len = u32le(&data[i + 12..i + 16]) as usize;
        let cd = &data[off..off + len];
        let mut p = 0usize;
        while p + 46 <= cd.len() {
            let nl = u16le(&cd[p + 28..p + 30]) as usize;
            let el = u16le(&cd[p + 30..p + 32]) as usize;
            let cl = u16le(&cd[p + 32..p + 34]) as usize;
            let name = String::from_utf8_lossy(&cd[p + 46..p + 46 + nl]).into_owned();
            out.push(CdEntry {
                name,
                method: u16le(&cd[p + 10..p + 12]),
                crc: u32le(&cd[p + 16..p + 20]),
                comp_size: u32le(&cd[p + 20..p + 24]) as u64,
                uncomp_size: u32le(&cd[p + 24..p + 28]) as u64,
                local_offset: u32le(&cd[p + 42..p + 46]) as u64,
                is_dir: false,
                encrypted: false,
            });
            p += 46 + nl + el + cl;
        }
        out
    }

    #[test]
    fn list_stored_and_deflated() {
        let data = make_zip(&[("a.txt", b"hello zip", 0), ("d/b.bin", &[9u8; 500], 8)]);
        let entries = cd_of(&data);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "a.txt");
        assert_eq!(entries[0].uncomp_size, 9);
        assert_eq!(entries[1].method, 8);
        // Verify CRC bằng tay.
        let mut h = Hasher::new();
        h.update(b"hello zip");
        assert_eq!(entries[0].crc, h.finalize());
    }

    #[test]
    fn inflate_real_zip_entry() {
        // Entry deflate (method 8) như zip của công cụ thường.
        let src = "hello hello hello\n".repeat(20);
        let data = make_zip(&[("a.txt", src.as_bytes(), 8)]);
        let e = &cd_of(&data)[0];
        let start = 30 + e.name.len();
        let comp = &data[start..start + e.comp_size as usize];
        assert_eq!(inflate(comp, e.uncomp_size).unwrap(), src.as_bytes());
        // dữ liệu bung ra dài hơn khai báo -> đọc thừa 1 byte, phía gọi sẽ báo lệch
        assert_eq!(inflate(comp, 5).unwrap().len(), 6);
        assert!(inflate(b"\xff\xff\xff", 10).is_err());
    }

    #[test]
    fn rejects_garbage() {
        let data = b"PK\x03\x04 this is not a zip at all, just junk data here........";
        assert!(cd_of(data).is_empty());
    }

    async fn run(op: &opendal::Operator, u: uuid::Uuid, entries: &[Ze], lay: &Layout, a: u64, b: u64) -> Vec<u8> {
        use futures::StreamExt as _;
        let (tx, mut rx) = futures::channel::mpsc::channel(32);
        zip_range(op, u, entries, lay, a, b, tx).await.unwrap();
        let mut out = Vec::new();
        while let Some(r) = rx.next().await {
            out.extend_from_slice(&r.unwrap());
        }
        out
    }

    fn forget_crcs(u: uuid::Uuid) {
        CRCS.lock().unwrap().retain(|k, _| k.0 != u);
    }

    async fn sample(tag: &str) -> (Store, uuid::Uuid, Vec<Ze>) {
        let (store, u) = crate::files::tests::tmp_store(tag);
        let big: Vec<u8> = (0..300_000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
        let files: Vec<(&str, Vec<u8>)> =
            vec![("d/a.txt", b"hello".to_vec()), ("d/empty.txt", vec![]), ("d/big.bin", big), ("ư/tên.md", "xin chào".as_bytes().to_vec())];
        let mut entries = vec![
            Ze { name: "d/".into(), full: String::new(), size: 0, modified: 0, is_dir: true },
            Ze { name: "ư/".into(), full: String::new(), size: 0, modified: 0, is_dir: true },
        ];
        for (p, b) in files {
            store.op.write(&format!("{u}/{p}"), b.clone()).await.unwrap();
            entries.push(Ze { name: p.into(), full: p.into(), size: b.len() as u64, modified: 7, is_dir: false });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        (store, u, entries)
    }

    #[tokio::test]
    async fn stream_roundtrip_verifies_crc() {
        let (store, u, entries) = sample("zipdl").await;
        let lay = layout(&entries, false);
        let out = run(&store.op, u, &entries, &lay, 0, lay.total).await;
        assert_eq!(out.len() as u64, lay.total);
        // central directory đủ entry, CRC đúng (đi qua descriptor bit 3), data nằm sau local header
        let cd = cd_of(&out);
        assert_eq!(cd.len(), entries.len());
        for e in entries.iter().filter(|e| !e.is_dir) {
            let c = cd.iter().find(|c| c.name == e.name).unwrap();
            let body = store.op.read(&format!("{u}/{}", e.full)).await.unwrap().to_vec();
            let mut h = Hasher::new();
            h.update(&body);
            assert_eq!((c.crc, c.uncomp_size), (h.finalize(), e.size));
            let start = c.local_offset as usize + 30 + e.name.len();
            assert_eq!(&out[start..start + body.len()], &body[..]);
        }
        // EOCD classic (không zip64)
        assert!(out[out.len() - 22..].starts_with(&[0x50, 0x4b, 0x05, 0x06]));
        // lần 2 giống hệt từng byte
        assert_eq!(run(&store.op, u, &entries, &lay, 0, lay.total).await, out);
    }

    #[tokio::test]
    async fn ranges_equal_slices_of_full() {
        let (store, u, entries) = sample("ziprange").await;
        for need64 in [false, true] {
            let lay = layout(&entries, need64);
            forget_crcs(u);
            let full = run(&store.op, u, &entries, &lay, 0, lay.total).await;
            assert_eq!(full.len() as u64, lay.total);
            let t = lay.total;
            let big = entries.iter().position(|e| e.name == "d/big.bin").unwrap();
            let d0 = lay.off[big] + hdr_len(&entries[big], lay.z64[big]);
            let mut ranges = vec![(0, 1), (t - 1, t), (0, t), (t - 22, t), (lay.cd_off, t), (lay.cd_off + 7, lay.cd_off + 60)];
            ranges.push((lay.cd_off - 10, lay.cd_off + 10)); // descriptor cuối -> central directory
            ranges.push((d0 + 1000, d0 + 5000)); // giữa data
            ranges.push((d0 + 299_990, d0 + 300_030)); // đuôi data -> descriptor -> header kế
            ranges.push((d0 + 150_000, t)); // tải tiếp tới hết
            for i in 1..entries.len() {
                ranges.push((lay.off[i] - 5, lay.off[i] + 5)); // vắt qua ranh giới entry
            }
            for (k, &(a, b)) in ranges.iter().enumerate() {
                // nửa số lần xoá cache: phải tự đọc lại file để tính CRC
                if k % 2 == 0 {
                    forget_crcs(u);
                }
                let got = run(&store.op, u, &entries, &lay, a, b).await;
                assert_eq!(got, &full[a as usize..b as usize], "range {a}-{b} need64={need64}");
            }
        }
    }

    #[tokio::test]
    async fn download_dir_http_range() {
        use axum::http::HeaderValue;
        let Some((db, u)) = crate::kanban::tests::db_user().await else { return };
        let (store, _) = crate::files::tests::tmp_store("ziphttp");
        for (p, b) in [("z/a.txt", &b"hello"[..]), ("z/s/b.bin", &[3u8; 5000][..])] {
            store.op.write(&format!("{u}/{p}"), b.to_vec()).await.unwrap();
            crate::index::add_file(&db, u, p, b.len() as u64, 1).await.unwrap();
        }
        let s = AppState {
            db,
            store,
            enc: jsonwebtoken::EncodingKey::from_secret(b"t"),
            dec: jsonwebtoken::DecodingKey::from_secret(b"t"),
            uploads: Default::default(),
            http: reqwest::Client::new(),
            tts_url: String::new(),
            vaults: Default::default(),
        };
        let tok = crate::auth::create_jwt(u, false, &s.enc).unwrap();
        let get = |h: Vec<(header::HeaderName, String)>| {
            let s = s.clone();
            let tok = tok.clone();
            async move {
                let mut hm = HeaderMap::new();
                hm.insert(header::AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {tok}")).unwrap());
                for (k, v) in h {
                    hm.insert(k, HeaderValue::from_str(&v).unwrap());
                }
                let q = HashMap::from([("path".to_string(), "z".to_string())]);
                let r = download_dir(State(s), hm, Query(q)).await.unwrap();
                let (st, hd) = (r.status(), r.headers().clone());
                let body = axum::body::to_bytes(r.into_body(), 1 << 24).await.unwrap().to_vec();
                (st, hd, body)
            }
        };
        let (st, hd, full) = get(vec![]).await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(hd[header::CONTENT_LENGTH].to_str().unwrap(), full.len().to_string());
        assert_eq!(hd[header::ACCEPT_RANGES], "bytes");
        let tag = hd[header::ETAG].to_str().unwrap().to_string();
        let n = full.len();
        let (st, hd, part) = get(vec![(header::RANGE, "bytes=40-5100".into())]).await;
        assert_eq!(st, StatusCode::PARTIAL_CONTENT);
        assert_eq!(hd[header::CONTENT_RANGE].to_str().unwrap(), format!("bytes 40-5100/{n}"));
        assert_eq!(part, &full[40..5101]);
        let (_, _, part) = get(vec![(header::RANGE, "bytes=-30".into()), (header::IF_RANGE, tag.clone())]).await;
        assert_eq!(part, &full[n - 30..]);
        // If-Range lệch -> trả cả file
        let (st, _, all) = get(vec![(header::RANGE, "bytes=10-20".into()), (header::IF_RANGE, "\"khac\"".into())]).await;
        assert_eq!((st, all.len()), (StatusCode::OK, n));
        let (st, hd, _) = get(vec![(header::RANGE, format!("bytes={n}-"))]).await;
        assert_eq!(st, StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(hd[header::CONTENT_RANGE].to_str().unwrap(), format!("bytes */{n}"));
    }
}
