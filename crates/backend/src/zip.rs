//! Đọc central directory của zip bằng Range (không tải cả file),
//! và giải nén 1 file con để tải về (giới hạn: nén ≤ 256MB, bung ≤ 2GB).
//! GET /api/zip/list?path=... -> {entries, total, truncated}
//! GET /api/zip/file?path=...&entry=... -> file con (attachment)
use std::collections::HashMap;

use axum::{
    Json,
    body::Body,
    extract::{Query, State},
    http::{HeaderMap, header},
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
    let files: Vec<(String, i64)> = if prefix.is_empty() {
        sqlx::query_as("SELECT path, size FROM files WHERE user_id = $1 AND NOT is_dir AND path NOT LIKE '.nextkloud/%' ORDER BY path LIMIT 200001")
            .bind(user.id)
            .fetch_all(&s.db)
            .await?
    } else {
        sqlx::query_as(
            "SELECT path, size FROM files WHERE user_id = $1 AND NOT is_dir AND path NOT LIKE '.nextkloud/%'
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
    for (p, _) in &files {
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
    let mut entries: Vec<Ze> = dirs.into_iter().map(|name| Ze { name, full: String::new(), size: 0, is_dir: true }).collect();
    for (p, sz) in &files {
        entries.push(Ze { name: rel(p), full: p.clone(), size: (*sz).max(0) as u64, is_dir: false });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let need64 = total as u64 >= 0xFFFF_FFFF || files.iter().any(|f| f.1 as u64 >= 0xFFFF_FFFF);

    let (tx, rx) = futures::channel::mpsc::channel::<Result<Vec<u8>, std::io::Error>>(8);
    let op = s.store.op.clone();
    let uid = user.id;
    tokio::spawn(async move {
        if let Err(e) = zip_stream(&op, uid, &entries, need64, tx.clone()).await {
            let _ = tx.clone().try_send(Err(std::io::Error::new(std::io::ErrorKind::Other, format!("{e:?}"))));
        }
    });
    let fname = if prefix.is_empty() { "files".to_string() } else { prefix.rsplit('/').next().unwrap_or("files").to_string() };
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "application/zip")
        .header(header::CONTENT_DISPOSITION, format!("attachment; filename*=UTF-8''{}.zip", pct(&fname)))
        .body(Body::from_stream(rx))
        .unwrap())
}

#[derive(Clone)]
struct Ze {
    /// tên trong zip (tương đối)
    name: String,
    /// path đầy đủ trong cây user (để đọc bytes)
    full: String,
    size: u64,
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

/// Ghi zip store-method qua channel: local header + data + descriptor từng file, rồi central directory.
async fn zip_stream(
    op: &opendal::Operator,
    user: uuid::Uuid,
    entries: &[Ze],
    need64: bool,
    mut tx: futures::channel::mpsc::Sender<Result<Vec<u8>, std::io::Error>>,
) -> Result<(), AppError> {
    use futures::{SinkExt as _, TryStreamExt as _};
    // date cố định 2020-01-01 (tránh phụ thuộc múi giờ server)
    const TIME: u16 = 0;
    const DATE: u16 = ((2020 - 1980) << 9) | (1 << 5) | 1;
    // send chờ (không try_send): channel đầy thì producer dừng theo, không mất chunk
    macro_rules! emit {
        ($b:expr) => {
            // client đã huỷ tải -> dừng, khỏi đọc tiếp cả thư mục từ S3
            if tx.send(Ok($b)).await.is_err() {
                return Ok(());
            }
        };
    }
    let mut central: Vec<u8> = Vec::new();
    let mut off: u64 = 0;
    for e in entries {
        let name = e.name.as_bytes();
        let z64 = need64 || e.size >= 0xFFFF_FFFF || off >= 0xFFFF_FFFF;
        let ver: u16 = if z64 { 45 } else { 20 };
        // --- local header (bit 3: CRC/size nằm ở descriptor sau data)
        let mut h = Vec::with_capacity(30 + 28 + name.len());
        w32(0x04034b50, &mut h);
        w16(ver, &mut h);
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
        let hlen = h.len() as u64;
        emit!(h);
        off += hlen;
        // --- data (vừa stream vừa tính CRC)
        let mut hasher = Hasher::new();
        if !e.is_dir && e.size > 0 {
            let key = format!("{user}/{}", e.full);
            let mut s = op
                .reader_with(&key)
                .chunk(4 * 1024 * 1024)
                .await
                .map_err(|er| AppError::Internal(format!("đọc {key}: {er}")))?
                .into_bytes_stream(0..e.size)
                .await
                .map_err(|er| AppError::Internal(format!("đọc {key}: {er}")))?;
            let mut got = 0u64;
            while let Some(b) = s.try_next().await.map_err(|er| AppError::Internal(format!("đọc {key}: {er}")))? {
                hasher.update(&b);
                got += b.len() as u64;
                emit!(b.to_vec());
            }
            if got != e.size {
                return Err(AppError::Internal(format!("file đổi giữa chừng: {}", e.full)));
            }
        }
        // --- descriptor
        let crc = hasher.finalize();
        let mut d = Vec::with_capacity(if z64 { 24 } else { 16 });
        w32(0x08074b50, &mut d);
        w32(crc, &mut d);
        if z64 {
            w64(e.size, &mut d);
            w64(e.size, &mut d);
        } else {
            w32(e.size as u32, &mut d);
            w32(e.size as u32, &mut d);
        }
        let dlen = d.len() as u64;
        emit!(d);
        // --- central
        let local_off = off - hlen;
        w32(0x02014b50, &mut central);
        w16(63 << 8 | ver, &mut central);
        w16(ver, &mut central);
        w16(0x0808, &mut central);
        w16(0, &mut central);
        w16(TIME, &mut central);
        w16(DATE, &mut central);
        w32(crc, &mut central);
        if z64 {
            w32(0xFFFF_FFFF, &mut central);
            w32(0xFFFF_FFFF, &mut central);
        } else {
            w32(e.size as u32, &mut central);
            w32(e.size as u32, &mut central);
        }
        w16(name.len() as u16, &mut central);
        let xlen: u16 = if z64 { 28 } else { 0 };
        w16(xlen, &mut central);
        w16(0, &mut central); // comment
        w16(0, &mut central); // disk
        w16(0, &mut central); // int attr
        w32(if e.is_dir { 0x10 << 16 } else { 0 }, &mut central);
        if z64 {
            w32(0xFFFF_FFFF, &mut central);
        } else {
            w32(local_off as u32, &mut central);
        }
        central.extend_from_slice(name);
        if z64 {
            w16(0x0001, &mut central);
            w16(24, &mut central);
            w64(e.size, &mut central);
            w64(e.size, &mut central);
            w64(local_off, &mut central);
        }
        off += dlen + e.size;
    }
    let cd_off = off;
    let cd_len = central.len() as u64;
    emit!(std::mem::take(&mut central));
    off += cd_len;
    let n = entries.len() as u64;
    let z64end = need64 || n >= 0xFFFF || cd_len >= 0xFFFF_FFFF || cd_off >= 0xFFFF_FFFF;
    if z64end {
        let mut z = Vec::with_capacity(56);
        w32(0x06064b50, &mut z);
        w64(44, &mut z);
        w16(45, &mut z);
        w16(45, &mut z);
        w32(0, &mut z);
        w32(0, &mut z);
        w64(n, &mut z);
        w64(n, &mut z);
        w64(cd_len, &mut z);
        w64(cd_off, &mut z);
        emit!(z);
        let mut l = Vec::with_capacity(20);
        w32(0x07064b50, &mut l);
        w32(0, &mut l);
        w64(off, &mut l);
        w32(1, &mut l);
        emit!(l);
        off += 56 + 20;
    }
    let mut e = Vec::with_capacity(22);
    w32(0x06054b50, &mut e);
    w16(0, &mut e);
    w16(0, &mut e);
    w16((n.min(0xFFFF)) as u16, &mut e);
    w16((n.min(0xFFFF)) as u16, &mut e);
    w32(cd_len.min(0xFFFF_FFFF) as u32, &mut e);
    w32(cd_off.min(0xFFFF_FFFF) as u32, &mut e);
    w16(0, &mut e);
    emit!(e);
    let _ = off;
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

    #[tokio::test]
    async fn stream_roundtrip_verifies_crc() {
        use futures::StreamExt as _;
        let (store, u) = crate::files::tests::tmp_store("zipdl");
        store.op.write(&format!("{u}/d/a.txt"), "hello".as_bytes().to_vec()).await.unwrap();
        let entries = vec![
            Ze { name: "d/".into(), full: String::new(), size: 0, is_dir: true },
            Ze { name: "d/a.txt".into(), full: "d/a.txt".into(), size: 5, is_dir: false },
        ];
        let (tx, mut rx) = futures::channel::mpsc::channel(32);
        zip_stream(&store.op, u, &entries, false, tx).await.unwrap();
        let mut out = Vec::new();
        while let Some(r) = rx.next().await {
            out.extend_from_slice(&r.unwrap());
        }
        // central directory có đủ 2 entry, CRC đúng (đi qua descriptor bit 3)
        let cd = cd_of(&out);
        assert_eq!(cd.len(), 2);
        let f = cd.iter().find(|e| e.name == "d/a.txt").unwrap();
        let mut h = Hasher::new();
        h.update(b"hello");
        assert_eq!(f.crc, h.finalize());
        assert_eq!(f.uncomp_size, 5);
        // data nằm sau local header (30 + tên, không extra vì need64=false)
        let start = f.local_offset as usize + 30 + "d/a.txt".len();
        assert_eq!(&out[start..start + 5], b"hello");
        // EOCD classic (không zip64)
        let tail = &out[out.len() - 22..];
        assert!(tail.starts_with(&[0x50, 0x4b, 0x05, 0x06]));
    }
}
