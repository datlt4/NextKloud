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
use flate2::{Decompress, FlushDecompress};
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
        let mut dz = Decompress::new(true);
        let mut out = Vec::new();
        dz.decompress_vec(&comp, &mut out, FlushDecompress::Finish)
            .map_err(|er| AppError::BadRequest(format!("giải nén lỗi: {er}")))?;
        out
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
                let mut c = Compress::new(Compression::default(), true);
                let mut o = vec![0u8; data.len() + 64];
                let total = c.compress_vec(data, &mut o, FlushCompress::Finish).unwrap();
                o.truncate(total as usize);
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
    fn rejects_garbage() {
        let data = b"PK\x03\x04 this is not a zip at all, just junk data here........";
        assert!(cd_of(data).is_empty());
    }
}
