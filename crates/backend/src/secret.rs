//! Secret Vault (thiết kế: docs/SECRET_VAULT.md): file mã hoá, mở bằng PIN 6 số, tự khoá sau 10 phút không dùng.
//! Object `<user>/.nextkloud/secret/<uuid>` = header + các khối 64KB AES-256-GCM (đọc giữa file được).
//! Tên file/thư mục mã hoá trong DB; tra theo HMAC của tên.
use std::collections::HashMap;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use axum::{
    Json,
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::Response,
};
use hmac::{Hmac, Mac};
use rand::{Rng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    file_handlers::{ListQuery, parse_range},
    files::{Store, StoreError, clean_path},
    index::{Page, Row, SortKey, split},
    ops::TrashItem,
    quota::Quota,
    state::{AppError, AppState, AuthUser, load_user},
};

pub const BLOCK: u64 = 64 * 1024;
const TAG: u64 = 16;
const MAGIC: &[u8; 4] = b"NKS1";
/// magic(4) + size(8) + nonce prefix(8) + DEK đã bọc (12 nonce + 32 + 16 tag).
pub const HDR: u64 = 4 + 8 + 8 + 60;
pub const IDLE: Duration = Duration::from_secs(600);
const OBJ_DIR: &str = ".nextkloud/secret";

type Key = [u8; 32];

// ---------- mật mã ----------

fn rand_key() -> Key {
    let mut k = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut k);
    k
}

fn cipher(key: &Key) -> Aes256Gcm {
    Aes256Gcm::new_from_slice(key).expect("32-byte key")
}

/// nonce(12) || ciphertext+tag
fn seal(key: &Key, pt: &[u8], aad: &[u8]) -> Vec<u8> {
    let mut n = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut n);
    let ct = cipher(key).encrypt(Nonce::from_slice(&n), Payload { msg: pt, aad }).expect("encrypt");
    [n.as_slice(), &ct].concat()
}

fn open(key: &Key, data: &[u8], aad: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 12 + TAG as usize {
        return None;
    }
    cipher(key).decrypt(Nonce::from_slice(&data[..12]), Payload { msg: &data[12..], aad }).ok()
}

fn sub(k: &[u8], salt: Option<&[u8]>, info: &[u8]) -> Key {
    let mut o = [0u8; 32];
    hkdf::Hkdf::<Sha256>::new(salt, k).expand(info, &mut o).expect("hkdf");
    o
}

/// Argon2id 64MB, 3 vòng (test dùng bản nhẹ cho nhanh).
fn argon(secret: &[u8], salt: &[u8]) -> Key {
    use argon2::{Algorithm, Argon2, Params, Version};
    let m = if cfg!(test) { 256 } else { 64 * 1024 };
    let mut o = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::new(m, 3, 1, Some(32)).unwrap())
        .hash_password_into(secret, salt, &mut o)
        .expect("argon2");
    o
}

/// Pepper nằm ở .env, không có trong DB/bản sao lưu: lấy được DB cũng không dò PIN offline được.
fn pepper() -> Result<Vec<u8>, AppError> {
    std::env::var("SECRET_PEPPER")
        .ok()
        .filter(|v| v.len() >= 16)
        .map(String::into_bytes)
        .ok_or_else(|| AppError::Internal("server chưa cấu hình SECRET_PEPPER".into()))
}

fn kek_pin(pin: &str, salt: &[u8], pepper: &[u8]) -> Key {
    sub(&argon(pin.as_bytes(), salt), Some(pepper), b"nk-vault-pin")
}

/// Mã khôi phục ~120 bit, đủ mạnh nên không cần pepper: mất .env vẫn mở được bằng mã này.
fn kek_rec(code: &str, salt: &[u8]) -> Key {
    sub(&argon(norm_code(code).as_bytes(), salt), None, b"nk-vault-rec")
}

const ALPHA: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

fn new_code() -> String {
    let mut r = rand::thread_rng();
    let raw: String = (0..24).map(|_| ALPHA[r.gen_range(0..ALPHA.len())] as char).collect();
    raw.as_bytes().chunks(4).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join("-")
}

fn norm_code(c: &str) -> String {
    c.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect()
}

fn valid_pin(p: &str) -> bool {
    p.len() == 6 && p.bytes().all(|b| b.is_ascii_digit())
}

/// Khoá con dẫn từ Vault Key.
#[derive(Clone)]
pub struct Keys {
    vk: Key,
    name: Key,
    mac: Key,
}

impl Keys {
    fn new(vk: Key) -> Self {
        Self { vk, name: sub(&vk, None, b"nk-name-enc"), mac: sub(&vk, None, b"nk-name-mac") }
    }
    fn name_mac(&self, name: &str) -> Vec<u8> {
        let mut m = <Hmac<Sha256> as Mac>::new_from_slice(&self.mac).unwrap();
        m.update(name.as_bytes());
        m.finalize().into_bytes().to_vec()
    }
    fn enc_name(&self, name: &str) -> Vec<u8> {
        seal(&self.name, name.as_bytes(), b"name")
    }
    fn dec_name(&self, b: &[u8]) -> String {
        open(&self.name, b, b"name").map(|v| String::from_utf8_lossy(&v).into_owned()).unwrap_or_else(|| "?".into())
    }
}

fn ct_len(size: u64) -> u64 {
    size + size.div_ceil(BLOCK) * TAG
}

fn block_nonce(prefix: &[u8], i: u64) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[..8].copy_from_slice(prefix);
    n[8..].copy_from_slice(&(i as u32).to_be_bytes());
    n
}

/// Header object: magic | size | nonce prefix | DEK bọc bằng VK. Phần đầu (20 byte) làm AAD cho mọi khối.
fn new_header(vk: &Key, size: u64) -> (Vec<u8>, Key) {
    let dek = rand_key();
    let mut h = Vec::with_capacity(HDR as usize);
    h.extend_from_slice(MAGIC);
    h.extend_from_slice(&size.to_be_bytes());
    let mut prefix = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut prefix);
    h.extend_from_slice(&prefix);
    let w = seal(vk, &dek, &h[..20]);
    h.extend_from_slice(&w);
    (h, dek)
}

fn parse_header(vk: &Key, h: &[u8], size: u64) -> Option<Key> {
    if h.len() != HDR as usize || &h[..4] != MAGIC || u64::from_be_bytes(h[4..12].try_into().ok()?) != size {
        return None;
    }
    open(vk, &h[20..], &h[..20])?.try_into().ok()
}

fn enc_block(dek: &Key, hdr: &[u8], i: u64, pt: &[u8]) -> Vec<u8> {
    cipher(dek).encrypt(Nonce::from_slice(&block_nonce(&hdr[12..20], i)), Payload { msg: pt, aad: &hdr[..20] }).expect("encrypt")
}

fn dec_block(dek: &Key, hdr: &[u8], i: u64, ct: &[u8]) -> Option<Vec<u8>> {
    cipher(dek).decrypt(Nonce::from_slice(&block_nonce(&hdr[12..20], i)), Payload { msg: ct, aad: &hdr[..20] }).ok()
}

// ---------- phiên mở khoá ----------

struct Session {
    user: Uuid,
    keys: Keys,
    last: Instant,
    via_rec: bool,
}

struct Up {
    user: Uuid,
    path: String,
    size: u64,
    mtime: Option<i64>,
    dek: Key,
    hdr: Vec<u8>,
    buf: Vec<u8>,
    received: u64,
    block: u64,
}

/// VK chỉ nằm trong RAM: khởi động lại server là mọi vault khoá.
/// Vé cho OnlyOffice tải 1 file bí mật (không đưa token mở khoá ra ngoài).
struct Ticket {
    user: Uuid,
    keys: Keys,
    path: String,
    at: Instant,
}

#[derive(Default)]
pub struct Vaults {
    sessions: StdMutex<HashMap<String, Session>>,
    uploads: StdMutex<HashMap<String, Up>>,
    office: StdMutex<HashMap<String, Ticket>>,
}

impl Vaults {
    fn open_session(&self, user: Uuid, keys: Keys, via_rec: bool) -> String {
        let t = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        self.sessions.lock().unwrap().insert(t.clone(), Session { user, keys, last: Instant::now(), via_rec });
        t
    }
    fn drop_user(&self, user: Uuid) {
        self.sessions.lock().unwrap().retain(|_, s| s.user != user);
        self.office.lock().unwrap().retain(|_, t| t.user != user);
    }
}

struct Ctx {
    user: Uuid,
    keys: Keys,
    via_rec: bool,
}

fn locked() -> AppError {
    AppError::Forbidden("vault locked".into())
}

fn vault_token(headers: &HeaderMap, q: &HashMap<String, String>) -> Option<String> {
    headers.get("x-vault-token").and_then(|v| v.to_str().ok()).map(str::to_string).or_else(|| q.get("vt").cloned())
}

/// Đăng nhập (Bearer hoặc ?t=) + token mở khoá (X-Vault-Token hoặc ?vt=). Mỗi lần dùng gia hạn 10 phút.
async fn ctx(s: &AppState, headers: &HeaderMap, q: &HashMap<String, String>) -> Result<Ctx, AppError> {
    let user = crate::thumb::auth_opt(headers, q, s).await.ok_or_else(|| AppError::Auth("missing token".into()))?;
    let vt = vault_token(headers, q).ok_or_else(locked)?;
    let mut m = s.vaults.sessions.lock().unwrap();
    m.retain(|_, v| v.last.elapsed() < IDLE);
    let ses = m.get_mut(&vt).filter(|v| v.user == user.id).ok_or_else(locked)?;
    ses.last = Instant::now();
    Ok(Ctx { user: user.id, keys: ses.keys.clone(), via_rec: ses.via_rec })
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

// ---------- vault: tạo / mở / khoá ----------

#[derive(sqlx::FromRow)]
struct VaultRow {
    salt: Vec<u8>,
    wrapped_vk: Vec<u8>,
    rec_salt: Vec<u8>,
    wrapped_vk_rec: Vec<u8>,
    failed: i32,
    lockouts: i32,
    locked_until: i64,
}

async fn vault_row(db: &PgPool, user: Uuid) -> Result<Option<VaultRow>, sqlx::Error> {
    sqlx::query_as("SELECT salt, wrapped_vk, rec_salt, wrapped_vk_rec, failed, lockouts, locked_until FROM secret_vaults WHERE user_id = $1")
        .bind(user)
        .fetch_optional(db)
        .await
}

#[derive(Serialize)]
pub struct StatusOut {
    exists: bool,
    unlocked: bool,
    /// Giây còn phải chờ sau khi nhập sai quá nhiều.
    wait: i64,
    idle: u64,
    /// Giây còn lại tới lúc tự khoá.
    left: u64,
}

pub async fn status(
    State(s): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<StatusOut>, AppError> {
    let row = vault_row(&s.db, user.id).await?;
    // chỉ xem, không gia hạn phiên (client hỏi định kỳ)
    let left = vault_token(&headers, &q)
        .and_then(|t| s.vaults.sessions.lock().unwrap().get(&t).filter(|v| v.user == user.id).map(|v| IDLE.saturating_sub(v.last.elapsed()).as_secs()))
        .unwrap_or(0);
    Ok(Json(StatusOut {
        exists: row.is_some(),
        unlocked: row.is_some() && left > 0,
        wait: row.map_or(0, |r| (r.locked_until - now()).max(0)),
        idle: IDLE.as_secs(),
        left,
    }))
}

#[derive(Deserialize)]
pub struct PinBody {
    pin: String,
}

pub async fn setup(State(s): State<AppState>, user: AuthUser, Json(b): Json<PinBody>) -> Result<Json<serde_json::Value>, AppError> {
    if !valid_pin(&b.pin) {
        return Err(AppError::BadRequest("PIN phải gồm 6 chữ số".into()));
    }
    let pep = pepper()?;
    if vault_row(&s.db, user.id).await?.is_some() {
        return Err(AppError::Conflict("vault đã tồn tại".into()));
    }
    let vk = rand_key();
    let (salt, rec_salt) = (rand_key().to_vec(), rand_key().to_vec());
    let code = new_code();
    let (pin, c2, s1, s2) = (b.pin.clone(), code.clone(), salt.clone(), rec_salt.clone());
    let (kp, kr) = tokio::task::spawn_blocking(move || (kek_pin(&pin, &s1, &pep), kek_rec(&c2, &s2)))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    sqlx::query("INSERT INTO secret_vaults (user_id, salt, wrapped_vk, rec_salt, wrapped_vk_rec) VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING")
        .bind(user.id)
        .bind(&salt)
        .bind(seal(&kp, &vk, b"vk"))
        .bind(&rec_salt)
        .bind(seal(&kr, &vk, b"vk"))
        .execute(&s.db)
        .await?;
    let token = s.vaults.open_session(user.id, Keys::new(vk), false);
    Ok(Json(serde_json::json!({ "token": token, "recovery": code })))
}

/// Kiểm PIN hoặc mã khôi phục, có giới hạn: sai 5 lần khoá 15 phút, mỗi lần khoá sau gấp đôi.
async fn check_secret(db: &PgPool, user: Uuid, pin: Option<&str>, code: Option<&str>) -> Result<Key, AppError> {
    let row = vault_row(db, user).await?.ok_or_else(|| AppError::NotFound("chưa tạo vault".into()))?;
    let wait = row.locked_until - now();
    if wait > 0 {
        return Err(AppError::Forbidden(format!("Sai quá nhiều lần, thử lại sau {} phút", (wait + 59) / 60)));
    }
    let (pin, code) = (pin.map(str::to_string), code.map(str::to_string));
    if pin.as_deref().is_some_and(|p| !valid_pin(p)) || (pin.is_none() && code.is_none()) {
        return Err(AppError::BadRequest("PIN phải gồm 6 chữ số".into()));
    }
    let pep = if pin.is_some() { pepper()? } else { vec![] };
    let (salt, rec_salt, w, wr) = (row.salt.clone(), row.rec_salt.clone(), row.wrapped_vk.clone(), row.wrapped_vk_rec.clone());
    let vk = tokio::task::spawn_blocking(move || match (pin, code) {
        (Some(p), _) => open(&kek_pin(&p, &salt, &pep), &w, b"vk"),
        (None, Some(c)) => open(&kek_rec(&c, &rec_salt), &wr, b"vk"),
        _ => None,
    })
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;
    match vk.and_then(|v| Key::try_from(v).ok()) {
        Some(k) => {
            sqlx::query("UPDATE secret_vaults SET failed = 0, lockouts = 0, locked_until = 0 WHERE user_id = $1").bind(user).execute(db).await?;
            Ok(k)
        }
        None => {
            let failed = row.failed + 1;
            if failed >= 5 {
                let mins = 15i64 << row.lockouts.min(10);
                sqlx::query("UPDATE secret_vaults SET failed = 0, lockouts = lockouts + 1, locked_until = $2 WHERE user_id = $1")
                    .bind(user)
                    .bind(now() + mins * 60)
                    .execute(db)
                    .await?;
                Err(AppError::Forbidden(format!("Sai quá nhiều lần, thử lại sau {mins} phút")))
            } else {
                sqlx::query("UPDATE secret_vaults SET failed = $2 WHERE user_id = $1").bind(user).bind(failed).execute(db).await?;
                Err(AppError::Forbidden(format!("Sai PIN hoặc mã khôi phục (còn {} lần thử)", 5 - failed)))
            }
        }
    }
}

#[derive(Deserialize)]
pub struct UnlockBody {
    pin: Option<String>,
    recovery: Option<String>,
}

pub async fn unlock(State(s): State<AppState>, user: AuthUser, Json(b): Json<UnlockBody>) -> Result<Json<serde_json::Value>, AppError> {
    let vk = check_secret(&s.db, user.id, b.pin.as_deref(), b.recovery.as_deref()).await?;
    let token = s.vaults.open_session(user.id, Keys::new(vk), b.pin.is_none());
    Ok(Json(serde_json::json!({ "token": token })))
}

pub async fn lock(State(s): State<AppState>, headers: HeaderMap) -> StatusCode {
    if let Some(t) = vault_token(&headers, &HashMap::new()) {
        if let Some(ses) = s.vaults.sessions.lock().unwrap().remove(&t) {
            s.vaults.office.lock().unwrap().retain(|_, x| x.user != ses.user);
        }
    }
    StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
pub struct ChangePin {
    old: Option<String>,
    new: String,
}

/// Đổi PIN: chỉ bọc lại VK. Mở bằng mã khôi phục thì không cần PIN cũ (trường hợp quên PIN).
pub async fn change_pin(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    Json(b): Json<ChangePin>,
) -> Result<StatusCode, AppError> {
    let c = ctx(&s, &headers, &q).await?;
    if !valid_pin(&b.new) {
        return Err(AppError::BadRequest("PIN phải gồm 6 chữ số".into()));
    }
    if !c.via_rec {
        check_secret(&s.db, c.user, Some(b.old.as_deref().unwrap_or("")), None).await?;
    }
    let pep = pepper()?;
    let salt = rand_key().to_vec();
    let (pin, s1) = (b.new.clone(), salt.clone());
    let kp = tokio::task::spawn_blocking(move || kek_pin(&pin, &s1, &pep)).await.map_err(|e| AppError::Internal(e.to_string()))?;
    sqlx::query("UPDATE secret_vaults SET salt = $2, wrapped_vk = $3 WHERE user_id = $1")
        .bind(c.user)
        .bind(&salt)
        .bind(seal(&kp, &c.keys.vk, b"vk"))
        .execute(&s.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Tạo mã khôi phục mới (mã cũ hết hiệu lực). Cần PIN.
pub async fn new_recovery(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    Json(b): Json<PinBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let c = ctx(&s, &headers, &q).await?;
    check_secret(&s.db, c.user, Some(&b.pin), None).await?;
    let code = new_code();
    let salt = rand_key().to_vec();
    let (c2, s1) = (code.clone(), salt.clone());
    let kr = tokio::task::spawn_blocking(move || kek_rec(&c2, &s1)).await.map_err(|e| AppError::Internal(e.to_string()))?;
    sqlx::query("UPDATE secret_vaults SET rec_salt = $2, wrapped_vk_rec = $3 WHERE user_id = $1")
        .bind(c.user)
        .bind(&salt)
        .bind(seal(&kr, &c.keys.vk, b"vk"))
        .execute(&s.db)
        .await?;
    Ok(Json(serde_json::json!({ "recovery": code })))
}

/// Xoá cả vault và mọi file trong đó (không khôi phục được). Cần PIN.
pub async fn destroy(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    Json(b): Json<PinBody>,
) -> Result<StatusCode, AppError> {
    let c = ctx(&s, &headers, &q).await?;
    check_secret(&s.db, c.user, Some(&b.pin), None).await?;
    let objs: Vec<(Option<String>, i64)> = sqlx::query_as("SELECT object_key, stored FROM secret_files WHERE user_id = $1")
        .bind(c.user)
        .fetch_all(&s.db)
        .await?;
    drop_objects(&s.db, &s.store, c.user, &objs).await?;
    sqlx::query("DELETE FROM secret_files WHERE user_id = $1").bind(c.user).execute(&s.db).await?;
    sqlx::query("DELETE FROM secret_vaults WHERE user_id = $1").bind(c.user).execute(&s.db).await?;
    s.vaults.drop_user(c.user);
    Ok(StatusCode::NO_CONTENT)
}

// ---------- cây file ----------

#[derive(sqlx::FromRow, Clone)]
struct Node {
    id: Uuid,
    is_dir: bool,
    size: i64,
    object_key: Option<String>,
    stored: i64,
}

const NODE: &str = "id, is_dir, size, object_key, stored";

async fn child(db: &PgPool, user: Uuid, parent: Option<Uuid>, mac: &[u8]) -> Result<Option<Node>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {NODE} FROM secret_files WHERE user_id = $1 AND parent_id IS NOT DISTINCT FROM $2 AND name_mac = $3 AND trashed_at IS NULL"
    ))
    .bind(user)
    .bind(parent)
    .bind(mac)
    .fetch_optional(db)
    .await
}

/// Path -> node (None nếu không có). Path rỗng không hợp lệ.
async fn resolve(db: &PgPool, user: Uuid, k: &Keys, path: &str) -> Result<Option<Node>, AppError> {
    let p = clean_path(path)?;
    if p.is_empty() {
        return Err(AppError::BadRequest("path rỗng".into()));
    }
    let mut cur: Option<Node> = None;
    for seg in p.split('/') {
        if cur.as_ref().is_some_and(|n| !n.is_dir) {
            return Ok(None);
        }
        match child(db, user, cur.as_ref().map(|n| n.id), &k.name_mac(seg)).await? {
            Some(n) => cur = Some(n),
            None => return Ok(None),
        }
    }
    Ok(cur)
}

/// Thư mục -> id (None = gốc vault).
async fn resolve_dir(db: &PgPool, user: Uuid, k: &Keys, path: &str) -> Result<Option<Uuid>, AppError> {
    if clean_path(path)?.is_empty() {
        return Ok(None);
    }
    match resolve(db, user, k, path).await? {
        Some(n) if n.is_dir => Ok(Some(n.id)),
        _ => Err(AppError::NotFound("không có thư mục này".into())),
    }
}

async fn insert(db: &PgPool, user: Uuid, k: &Keys, parent: Option<Uuid>, name: &str, is_dir: bool, size: i64, mtime: i64, obj: Option<&str>, stored: i64) -> Result<Uuid, AppError> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO secret_files (id, user_id, parent_id, name_enc, name_mac, is_dir, size, mtime, object_key, stored)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(id)
    .bind(user)
    .bind(parent)
    .bind(k.enc_name(name))
    .bind(k.name_mac(name))
    .bind(is_dir)
    .bind(size)
    .bind(mtime)
    .bind(obj)
    .bind(stored)
    .execute(db)
    .await
    .map_err(dup)?;
    Ok(id)
}

fn dup(e: sqlx::Error) -> AppError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => AppError::Conflict("đã có mục cùng tên".into()),
        _ => AppError::Db(e),
    }
}

/// mkdir -p, trả về id thư mục cuối (None = gốc).
async fn ensure_dir(db: &PgPool, user: Uuid, k: &Keys, path: &str) -> Result<Option<Uuid>, AppError> {
    let p = clean_path(path)?;
    let mut cur: Option<Uuid> = None;
    for seg in p.split('/').filter(|s| !s.is_empty()) {
        cur = Some(match child(db, user, cur, &k.name_mac(seg)).await? {
            Some(n) if n.is_dir => n.id,
            Some(_) => return Err(AppError::Conflict(format!("'{seg}' là file, không phải thư mục"))),
            None => insert(db, user, k, cur, seg, true, 0, now(), None, 0).await?,
        });
    }
    Ok(cur)
}

pub async fn list(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(raw): Query<HashMap<String, String>>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Page>, AppError> {
    let c = ctx(&s, &headers, &raw).await?;
    let prefix = clean_path(&q.prefix)?;
    let parent = resolve_dir(&s.db, c.user, &c.keys, &prefix).await?;
    let rows: Vec<(Vec<u8>, bool, i64, i64)> = sqlx::query_as(
        "SELECT name_enc, is_dir, size, mtime FROM secret_files WHERE user_id = $1 AND parent_id IS NOT DISTINCT FROM $2 AND trashed_at IS NULL",
    )
    .bind(c.user)
    .bind(parent)
    .fetch_all(&s.db)
    .await?;
    let mut all: Vec<Row> = rows
        .into_iter()
        .map(|(n, is_dir, size, mtime)| {
            let name = c.keys.dec_name(&n);
            let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            Row { path, name, is_dir, size, modified: mtime }
        })
        .collect();
    all.sort_by(|a, b| {
        let o = match q.sort {
            SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortKey::Size => a.size.cmp(&b.size),
            SortKey::Modified => a.modified.cmp(&b.modified),
        };
        b.is_dir.cmp(&a.is_dir).then(if q.desc { o.reverse() } else { o })
    });
    let dirs = all.iter().filter(|r| r.is_dir).count() as i64;
    let bytes = all.iter().filter(|r| !r.is_dir).map(|r| r.size).sum();
    let files = all.len() as i64 - dirs;
    let limit = q.limit.unwrap_or(200).clamp(0, 1000) as usize;
    let entries = all.into_iter().skip(q.offset.max(0) as usize).take(limit).collect();
    Ok(Json(Page { entries, dirs, files, bytes }))
}

#[derive(Deserialize)]
pub struct PathBody {
    path: String,
}

pub async fn mkdir(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    Json(b): Json<PathBody>,
) -> Result<StatusCode, AppError> {
    let c = ctx(&s, &headers, &q).await?;
    if resolve(&s.db, c.user, &c.keys, &b.path).await?.is_some() {
        return Err(AppError::Conflict("đã có mục cùng tên".into()));
    }
    ensure_dir(&s.db, c.user, &c.keys, &b.path).await?;
    Ok(StatusCode::CREATED)
}

#[derive(Deserialize)]
pub struct MoveBody {
    from: String,
    to: String,
}

pub async fn move_entry(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    Json(b): Json<MoveBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let c = ctx(&s, &headers, &q).await?;
    let n = resolve(&s.db, c.user, &c.keys, &b.from).await?.ok_or_else(|| AppError::NotFound("not found".into()))?;
    let to = clean_path(&b.to)?;
    let (dir, name) = split(&to);
    if name.is_empty() {
        return Err(AppError::BadRequest("tên rỗng".into()));
    }
    let parent = ensure_dir(&s.db, c.user, &c.keys, dir).await?;
    // không cho chuyển thư mục vào chính nó / con của nó
    let mut up = parent;
    while let Some(id) = up {
        if id == n.id {
            return Err(AppError::BadRequest("không thể chuyển thư mục vào trong chính nó".into()));
        }
        up = sqlx::query_scalar("SELECT parent_id FROM secret_files WHERE id = $1").bind(id).fetch_one(&s.db).await?;
    }
    sqlx::query("UPDATE secret_files SET parent_id = $2, name_enc = $3, name_mac = $4 WHERE id = $1")
        .bind(n.id)
        .bind(parent)
        .bind(c.keys.enc_name(name))
        .bind(c.keys.name_mac(name))
        .execute(&s.db)
        .await
        .map_err(dup)?;
    Ok(Json(serde_json::json!({ "files": 1 })))
}

/// Xoá = vào thùng rác riêng của vault (dữ liệu vẫn mã hoá).
pub async fn delete(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<TrashItem>, AppError> {
    let c = ctx(&s, &headers, &q).await?;
    let n = resolve(&s.db, c.user, &c.keys, &path).await?.ok_or_else(|| AppError::NotFound("not found".into()))?;
    let at = now();
    sqlx::query("UPDATE secret_files SET trashed_at = $2 WHERE id = $1").bind(n.id).bind(at).execute(&s.db).await?;
    Ok(Json(TrashItem { id: n.id, orig_path: clean_path(&path)?, is_dir: n.is_dir, size: n.size, files: 0, deleted_at: at }))
}

// ---------- đọc file ----------

/// Stream bản rõ của [start, end): chỉ đọc + giải mã các khối cần (tua video, PDF trang giữa).
fn plain_stream(
    store: Store,
    key: String,
    dek: Key,
    hdr: Vec<u8>,
    size: u64,
    start: u64,
    end: u64,
) -> impl futures::Stream<Item = Result<Bytes, std::io::Error>> + Send + Unpin + 'static {
    let last = if end == 0 { 0 } else { (end - 1) / BLOCK };
    let first = start / BLOCK;
    let total = HDR + ct_len(size);
    Box::pin(futures::stream::unfold(if start < end { first } else { u64::MAX }, move |i| {
        let (store, key, hdr) = (store.clone(), key.clone(), hdr.clone());
        async move {
            if i == u64::MAX || i > last {
                return None;
            }
            let j = (i + 15).min(last); // ~1MB mỗi lần đọc
            let (cs, ce) = (HDR + i * (BLOCK + TAG), (HDR + (j + 1) * (BLOCK + TAG)).min(total));
            let ct = match store.op.read_with(&key).range(cs..ce).await {
                Ok(b) => b.to_vec(),
                Err(e) => return Some((Err(std::io::Error::other(e.to_string())), u64::MAX)),
            };
            let mut out = Vec::with_capacity(ct.len());
            for (n, chunk) in ct.chunks((BLOCK + TAG) as usize).enumerate() {
                let b = i + n as u64;
                let Some(pt) = dec_block(&dek, &hdr, b, chunk) else {
                    return Some((Err(std::io::Error::other("khối dữ liệu hỏng")), u64::MAX));
                };
                let off = b * BLOCK;
                let a = start.saturating_sub(off).min(pt.len() as u64) as usize;
                let z = (end - off).min(pt.len() as u64) as usize;
                out.extend_from_slice(&pt[a..z]);
            }
            Some((Ok(Bytes::from(out)), j + 1))
        }
    }))
}

/// Mở file: node + DEK đã kiểm header.
async fn open_file(s: &AppState, c: &Ctx, path: &str) -> Result<(Node, String, Key, Vec<u8>), AppError> {
    let n = resolve(&s.db, c.user, &c.keys, path).await?.ok_or_else(|| AppError::NotFound("not found".into()))?;
    let key = n.object_key.clone().filter(|_| !n.is_dir).ok_or_else(|| AppError::BadRequest("not a file".into()))?;
    let key = s.store.key(c.user, &key)?;
    let hdr = s.store.op.read_with(&key).range(0..HDR).await.map_err(StoreError::from)?.to_vec();
    let dek = parse_header(&c.keys.vk, &hdr, n.size as u64).ok_or_else(|| AppError::Internal("file mã hoá bị hỏng".into()))?;
    Ok((n, key, dek, hdr))
}

pub async fn download(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let c = ctx(&s, &headers, &q).await?;
    let (n, key, dek, hdr) = open_file(&s, &c, &path).await?;
    let size = n.size as u64;
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok()).map(|h| parse_range(h, size));
    let (start, end, status) = match range {
        Some(Some((a, b))) => (a, b, StatusCode::PARTIAL_CONTENT),
        Some(None) => {
            return Ok(Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{size}"))
                .body(Body::empty())
                .unwrap());
        }
        None => (0, size, StatusCode::OK),
    };
    let mut res = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, crate::routes::content_type(&path))
        .header(header::CONTENT_LENGTH, end - start)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CACHE_CONTROL, "no-store"); // bản rõ không lưu vào cache trình duyệt
    if status == StatusCode::PARTIAL_CONTENT {
        res = res.header(header::CONTENT_RANGE, format!("bytes {start}-{}/{size}", end - 1));
    }
    if q.contains_key("download") {
        let name = path.rsplit('/').next().unwrap_or("file");
        res = res.header(header::CONTENT_DISPOSITION, format!("attachment; filename*=UTF-8''{}", urlencode(name)));
    }
    Ok(res.body(Body::from_stream(plain_stream(s.store.clone(), key, dek, hdr, size, start, end))).unwrap())
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

/// Thumbnail tạo lúc xem, không cache (cache sẽ là bản rõ).
pub async fn thumb(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(path): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let c = ctx(&s, &headers, &q).await?;
    let is_img = crate::thumb::thumb_kind(&path).ok_or_else(|| AppError::BadRequest("loại file này chưa có thumbnail".into()))?;
    let px: u32 = q.get("s").and_then(|v| v.parse().ok()).unwrap_or(256).clamp(64, 2048);
    let (n, key, dek, hdr) = open_file(&s, &c, &path).await?;
    let size = n.size as u64;
    let stream = plain_stream(s.store.clone(), key, dek, hdr, size, 0, size);
    let src = if is_img {
        if size > 100 * 1024 * 1024 {
            return Err(AppError::BadRequest("ảnh quá lớn để tạo thumbnail".into()));
        }
        use futures::TryStreamExt;
        let parts: Vec<Bytes> = stream.try_collect().await.map_err(|e| AppError::Internal(e.to_string()))?;
        parts.concat()
    } else {
        crate::thumb::ffmpeg_frame(stream).await.map_err(AppError::Internal)?
    };
    let out = tokio::task::spawn_blocking(move || crate::thumb::render_image(&src, px))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .map_err(AppError::Internal)?;
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, crate::thumb::mime_of(&out))
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(out))
        .unwrap())
}

// ---------- OnlyOffice: chỉ xem ----------

#[derive(Deserialize)]
pub struct OfficeBody {
    path: String,
}

/// Mở file office trong vault ở chế độ chỉ xem (sửa sẽ phải để DocumentServer ghi bản rõ).
pub async fn office_config(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    Json(b): Json<OfficeBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    use jsonwebtoken::{EncodingKey, Header, encode};
    let c = ctx(&s, &headers, &q).await?;
    let path = clean_path(&b.path)?;
    let (file_type, document_type) = crate::office::doc_type(&path).ok_or_else(|| AppError::BadRequest("loại office này chưa hỗ trợ".into()))?;
    let n = resolve(&s.db, c.user, &c.keys, &path).await?.ok_or_else(|| AppError::NotFound("not found".into()))?;
    let obj = n.object_key.ok_or_else(|| AppError::BadRequest("not a file".into()))?;
    let ticket = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    {
        let mut m = s.vaults.office.lock().unwrap();
        m.retain(|_, t| t.at.elapsed() < IDLE);
        m.insert(ticket.clone(), Ticket { user: c.user, keys: c.keys.clone(), path: path.clone(), at: Instant::now() });
    }
    let base = std::env::var("ONLYOFFICE_BACKEND_URL")
        .ok()
        .map(|v| v.trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| crate::office::public_base(&headers));
    let key = crate::office::doc_key(&c.user, &obj, 0, n.size as u64);
    let cfg = serde_json::json!({
        "document": {
            "fileType": file_type,
            "key": key,
            "title": path.rsplit('/').next().unwrap_or(&path),
            "url": format!("{base}/api/secret/office/file?t={ticket}"),
            "permissions": { "edit": false, "download": false, "print": true },
        },
        "documentType": document_type,
        "editorConfig": { "mode": "view", "lang": "vi", "user": { "id": c.user.to_string() } },
    });
    let secret = crate::office::oo_secret()?;
    let token = encode(&Header::default(), &cfg, &EncodingKey::from_secret(secret.as_bytes())).map_err(|e| AppError::Internal(format!("ký token office: {e}")))?;
    Ok(Json(serde_json::json!({ "config": cfg, "token": token })))
}

/// DocumentServer tải bản rõ qua vé (hết hạn sau 10 phút).
pub async fn office_file(State(s): State<AppState>, Query(q): Query<HashMap<String, String>>) -> Result<Response, AppError> {
    let t = q.get("t").ok_or_else(|| AppError::Auth("missing token".into()))?;
    let (user, keys, path) = {
        let m = s.vaults.office.lock().unwrap();
        let t = m.get(t).filter(|t| t.at.elapsed() < IDLE).ok_or_else(|| AppError::Forbidden("vé hết hạn".into()))?;
        (t.user, t.keys.clone(), t.path.clone())
    };
    let c = Ctx { user, keys, via_rec: false };
    let (n, key, dek, hdr) = open_file(&s, &c, &path).await?;
    let size = n.size as u64;
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, crate::routes::content_type(&path))
        .header(header::CONTENT_LENGTH, size)
        .body(Body::from_stream(plain_stream(s.store.clone(), key, dek, hdr, size, 0, size)))
        .unwrap())
}

// ---------- upload: mã hoá ngay khi nhận, đĩa staging chỉ có bản mã ----------

#[derive(Deserialize)]
pub struct InitBody {
    path: String,
    size: u64,
    mtime: Option<i64>,
}

pub async fn init_upload(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    Json(b): Json<InitBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let c = ctx(&s, &headers, &q).await?;
    if b.size > 100 * 1024 * 1024 * 1024 {
        return Err(AppError::BadRequest("file too large (max 100GiB)".into()));
    }
    let p = clean_path(&b.path)?;
    if p.is_empty() {
        return Err(AppError::BadRequest("path rỗng".into()));
    }
    let row = load_user(&s.db, c.user).await?;
    Quota::new(row.quota_limit.max(0) as u64, row.quota_used.max(0) as u64)
        .can_add(HDR + ct_len(b.size))
        .map_err(|e| AppError::Conflict(format!("quota exceeded: {e}")))?;
    let (hdr, dek) = new_header(&c.keys.vk, b.size);
    let id = Uuid::new_v4().to_string();
    let tmp = s.store.stage_path(c.user, &format!("s-{id}"));
    std::fs::create_dir_all(tmp.parent().unwrap()).map_err(|e| AppError::Internal(e.to_string()))?;
    std::fs::write(&tmp, &hdr).map_err(|e| AppError::Internal(e.to_string()))?;
    let up = Up { user: c.user, path: p, size: b.size, mtime: b.mtime, dek, hdr, buf: vec![], received: 0, block: 0 };
    s.vaults.uploads.lock().unwrap().insert(id.clone(), up);
    Ok(Json(serde_json::json!({ "id": id })))
}

fn append(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::OpenOptions::new().append(true).open(path)?.write_all(data)
}

#[derive(Deserialize)]
pub struct ChunkQuery {
    offset: u64,
}

pub async fn put_chunk(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(raw): Query<HashMap<String, String>>,
    Query(q): Query<ChunkQuery>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, AppError> {
    let c = ctx(&s, &headers, &raw).await?;
    let mut up = s.vaults.uploads.lock().unwrap().remove(&id).ok_or_else(|| AppError::NotFound("upload gone".into()))?;
    let res = (|| {
        if up.user != c.user {
            return Err(AppError::Forbidden("not your upload".into()));
        }
        if q.offset != up.received || up.received + body.len() as u64 > up.size {
            return Err(AppError::Conflict(format!("offset mismatch: have {}, got {}", up.received, q.offset)));
        }
        up.buf.extend_from_slice(&body);
        let mut out = Vec::with_capacity(up.buf.len() + 1024);
        let full = up.buf.len() / BLOCK as usize * BLOCK as usize;
        for blk in up.buf[..full].chunks(BLOCK as usize) {
            out.extend(enc_block(&up.dek, &up.hdr, up.block, blk));
            up.block += 1;
        }
        up.buf.drain(..full);
        append(&s.store.stage_path(c.user, &format!("s-{id}")), &out).map_err(|e| AppError::Internal(e.to_string()))?;
        up.received += body.len() as u64;
        Ok(up.received)
    })();
    s.vaults.uploads.lock().unwrap().insert(id, up);
    Ok(Json(serde_json::json!({ "received": res? })))
}

pub async fn complete_upload(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<StatusCode, AppError> {
    let c = ctx(&s, &headers, &q).await?;
    let up = s.vaults.uploads.lock().unwrap().remove(&id).ok_or_else(|| AppError::NotFound("upload gone".into()))?;
    if up.user != c.user {
        return Err(AppError::Forbidden("not your upload".into()));
    }
    let tmp = s.store.stage_path(c.user, &format!("s-{id}"));
    let r = finish(&s, &c, up, &tmp).await;
    let _ = tokio::fs::remove_file(&tmp).await;
    r
}

async fn finish(s: &AppState, c: &Ctx, up: Up, tmp: &std::path::Path) -> Result<StatusCode, AppError> {
    if up.received != up.size {
        return Err(AppError::Conflict(format!("incomplete: staged {}, want {}", up.received, up.size)));
    }
    if !up.buf.is_empty() {
        append(tmp, &enc_block(&up.dek, &up.hdr, up.block, &up.buf)).map_err(|e| AppError::Internal(e.to_string()))?;
    }
    let (dir, name) = split(&up.path);
    let existing = resolve(&s.db, c.user, &c.keys, &up.path).await?;
    if existing.as_ref().is_some_and(|n| n.is_dir) {
        return Err(AppError::Conflict("đã có thư mục cùng tên".into()));
    }
    let parent = ensure_dir(&s.db, c.user, &c.keys, dir).await?;
    let row = load_user(&s.db, c.user).await?;
    let mut q = Quota::new(row.quota_limit.max(0) as u64, row.quota_used.max(0) as u64);
    let obj = format!("{OBJ_DIR}/{}", Uuid::new_v4());
    let mtime = s.store.put_local(c.user, &obj, tmp, up.mtime, &mut q).await?;
    let stored = (HDR + ct_len(up.size)) as i64;
    match existing {
        Some(old) => {
            sqlx::query("UPDATE secret_files SET object_key = $2, size = $3, mtime = $4, stored = $5 WHERE id = $1")
                .bind(old.id)
                .bind(&obj)
                .bind(up.size as i64)
                .bind(mtime)
                .bind(stored)
                .execute(&s.db)
                .await?;
            drop_objects(&s.db, &s.store, c.user, &[(old.object_key, old.stored)]).await?;
        }
        None => {
            if let Err(e) = insert(&s.db, c.user, &c.keys, parent, name, false, up.size as i64, mtime, Some(&obj), stored).await {
                let _ = s.store.op.delete(&s.store.key(c.user, &obj)?).await;
                return Err(e);
            }
        }
    }
    sqlx::query("UPDATE users SET quota_used = GREATEST(0, quota_used + $1) WHERE id = $2")
        .bind(stored)
        .bind(c.user)
        .execute(&s.db)
        .await?;
    crate::index::add_file(&s.db, c.user, &obj, stored as u64, mtime).await?;
    Ok(StatusCode::CREATED)
}

/// Xoá object + hoàn quota (không cần khoá vault).
async fn drop_objects(db: &PgPool, store: &Store, user: Uuid, objs: &[(Option<String>, i64)]) -> Result<(), AppError> {
    let mut freed = 0;
    for (k, stored) in objs {
        let Some(k) = k else { continue };
        match store.op.delete(&store.key(user, k)?).await {
            Ok(()) => {}
            Err(e) if e.kind() == opendal::ErrorKind::NotFound => {}
            Err(e) => return Err(StoreError::from(e).into()),
        }
        crate::index::remove_file(db, user, k).await?;
        freed += stored;
    }
    sqlx::query("UPDATE users SET quota_used = GREATEST(0, quota_used - $2) WHERE id = $1").bind(user).bind(freed).execute(db).await?;
    Ok(())
}

// ---------- thùng rác riêng ----------

const SUBTREE: &str = "WITH RECURSIVE t AS (
    SELECT id, is_dir, size, object_key, stored FROM secret_files WHERE user_id = $1 AND id = $2
    UNION ALL SELECT f.id, f.is_dir, f.size, f.object_key, f.stored FROM secret_files f JOIN t ON f.parent_id = t.id)";

pub async fn list_trash(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Vec<TrashItem>>, AppError> {
    let c = ctx(&s, &headers, &q).await?;
    let rows: Vec<(Uuid, Option<Uuid>, Vec<u8>, bool, i64)> = sqlx::query_as(
        "SELECT id, parent_id, name_enc, is_dir, trashed_at FROM secret_files WHERE user_id = $1 AND trashed_at IS NOT NULL ORDER BY trashed_at DESC",
    )
    .bind(c.user)
    .fetch_all(&s.db)
    .await?;
    let mut out = vec![];
    for (id, mut parent, name, is_dir, at) in rows {
        let mut parts = vec![c.keys.dec_name(&name)];
        while let Some(p) = parent {
            let (pp, n): (Option<Uuid>, Vec<u8>) = sqlx::query_as("SELECT parent_id, name_enc FROM secret_files WHERE id = $1").bind(p).fetch_one(&s.db).await?;
            parts.push(c.keys.dec_name(&n));
            parent = pp;
        }
        parts.reverse();
        let (size, files): (i64, i64) = sqlx::query_as(&format!("{SUBTREE} SELECT COALESCE(SUM(size), 0)::BIGINT, COUNT(*) FILTER (WHERE NOT is_dir) FROM t"))
            .bind(c.user)
            .bind(id)
            .fetch_one(&s.db)
            .await?;
        out.push(TrashItem { id, orig_path: parts.join("/"), is_dir, size, files, deleted_at: at });
    }
    Ok(Json(out))
}

pub async fn restore(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<StatusCode, AppError> {
    let c = ctx(&s, &headers, &q).await?;
    let r = sqlx::query("UPDATE secret_files SET trashed_at = NULL WHERE user_id = $1 AND id = $2 AND trashed_at IS NOT NULL")
        .bind(c.user)
        .bind(id)
        .execute(&s.db)
        .await
        .map_err(|e| match dup(e) {
            AppError::Conflict(_) => AppError::Conflict("chỗ cũ đã có mục cùng tên, đổi tên mục đó trước".into()),
            e => e,
        })?;
    if r.rows_affected() == 0 {
        return Err(AppError::NotFound("not found".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn purge_item(db: &PgPool, store: &Store, user: Uuid, id: Uuid) -> Result<(), AppError> {
    let objs: Vec<(Option<String>, i64)> = sqlx::query_as(&format!("{SUBTREE} SELECT object_key, stored FROM t")).bind(user).bind(id).fetch_all(db).await?;
    drop_objects(db, store, user, &objs).await?;
    sqlx::query("DELETE FROM secret_files WHERE user_id = $1 AND id = $2").bind(user).bind(id).execute(db).await?;
    Ok(())
}

pub async fn purge(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<StatusCode, AppError> {
    let c = ctx(&s, &headers, &q).await?;
    let ok: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM secret_files WHERE user_id = $1 AND id = $2 AND trashed_at IS NOT NULL")
        .bind(c.user)
        .bind(id)
        .fetch_optional(&s.db)
        .await?;
    ok.ok_or_else(|| AppError::NotFound("not found".into()))?;
    purge_item(&s.db, &s.store, c.user, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn empty_trash(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Result<StatusCode, AppError> {
    let c = ctx(&s, &headers, &q).await?;
    let ids: Vec<(Uuid,)> = sqlx::query_as("SELECT id FROM secret_files WHERE user_id = $1 AND trashed_at IS NOT NULL").bind(c.user).fetch_all(&s.db).await?;
    for (id,) in ids {
        purge_item(&s.db, &s.store, c.user, id).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Dọn thùng rác vault quá `days` ngày (chạy cùng job dọn thùng rác thường).
pub async fn purge_expired(db: &PgPool, store: &Store, days: i64) -> Result<usize, AppError> {
    let old: Vec<(Uuid, Uuid)> = sqlx::query_as("SELECT user_id, id FROM secret_files WHERE trashed_at < $1")
        .bind(now() - days * 86400)
        .fetch_all(db)
        .await?;
    let mut done = 0;
    for (u, id) in old {
        match purge_item(db, store, u, id).await {
            Ok(()) => done += 1,
            Err(e) => tracing::warn!("dọn thùng rác vault thất bại user={u} id={id}: {e:?}"),
        }
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crypto_roundtrip() {
        let k = Keys::new(rand_key());
        assert_eq!(k.dec_name(&k.enc_name("Ảnh riêng.jpg")), "Ảnh riêng.jpg");
        assert_eq!(k.name_mac("a"), k.name_mac("a"));
        assert_ne!(k.name_mac("a"), k.name_mac("A"));
        let (hdr, dek) = new_header(&k.vk, 5);
        assert_eq!(hdr.len() as u64, HDR);
        assert_eq!(parse_header(&k.vk, &hdr, 5), Some(dek));
        assert_eq!(parse_header(&k.vk, &hdr, 6), None); // size bị sửa
        assert_eq!(parse_header(&rand_key(), &hdr, 5), None);
        let ct = enc_block(&dek, &hdr, 3, b"hello");
        assert_eq!(dec_block(&dek, &hdr, 3, &ct).unwrap(), b"hello");
        assert!(dec_block(&dek, &hdr, 4, &ct).is_none()); // đổi chỗ khối bị phát hiện
    }

    #[test]
    fn kek_and_codes() {
        let salt = rand_key();
        let vk = rand_key();
        let w = seal(&kek_pin("123456", &salt, b"pepper-pepper-pepper"), &vk, b"vk");
        assert_eq!(open(&kek_pin("123456", &salt, b"pepper-pepper-pepper"), &w, b"vk").unwrap(), vk);
        assert!(open(&kek_pin("123457", &salt, b"pepper-pepper-pepper"), &w, b"vk").is_none());
        assert!(open(&kek_pin("123456", &salt, b"other-pepper-xxxxxx"), &w, b"vk").is_none());
        let code = new_code();
        assert_eq!(code.len(), 29);
        let wr = seal(&kek_rec(&code, &salt), &vk, b"vk");
        assert_eq!(open(&kek_rec(&code.to_lowercase().replace('-', " "), &salt), &wr, b"vk").unwrap(), vk);
        assert!(valid_pin("012345") && !valid_pin("12345") && !valid_pin("12345a"));
    }

    #[tokio::test]
    async fn stream_ranges() {
        let (store, u) = crate::files::tests::tmp_store("sec");
        let k = Keys::new(rand_key());
        let data: Vec<u8> = (0..(BLOCK * 3 + 100)).map(|i| (i % 251) as u8).collect();
        let (hdr, dek) = new_header(&k.vk, data.len() as u64);
        let mut obj = hdr.clone();
        for (i, b) in data.chunks(BLOCK as usize).enumerate() {
            obj.extend(enc_block(&dek, &hdr, i as u64, b));
        }
        assert_eq!(obj.len() as u64, HDR + ct_len(data.len() as u64));
        let key = store.key(u, "x").unwrap();
        store.op.write(&key, obj).await.unwrap();
        use futures::TryStreamExt;
        for (a, b) in [(0, data.len() as u64), (5, 10), (BLOCK - 3, BLOCK + 3), (BLOCK * 3 + 50, data.len() as u64), (0, 0)] {
            let parts: Vec<Bytes> = plain_stream(store.clone(), key.clone(), dek, hdr.clone(), data.len() as u64, a, b).try_collect().await.unwrap();
            assert_eq!(parts.concat(), &data[a as usize..b as usize], "range {a}..{b}");
        }
    }

    /// Luồng đầy đủ qua HTTP: tạo, upload, đọc Range, đổi tên, thùng rác, khoá/mở, đổi PIN, xoá vault.
    #[tokio::test]
    async fn vault_http_flow() {
        use axum::http::Request;
        use serde_json::{Value, json};
        use tower::ServiceExt;
        let Ok(url) = std::env::var("NK_TEST_DB") else { return };
        std::env::set_var("SECRET_PEPPER", "test-pepper-0123456789");
        let db = PgPool::connect(&url).await.unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        let (store, u) = crate::files::tests::tmp_store("secret-http");
        sqlx::query("INSERT INTO users (id, username, pass_hash, quota_limit) VALUES ($1, $2, 'x', 0)").bind(u).bind(u.to_string()).execute(&db).await.unwrap();
        let state = AppState {
            db: db.clone(),
            store,
            enc: jsonwebtoken::EncodingKey::from_secret(b"t"),
            dec: jsonwebtoken::DecodingKey::from_secret(b"t"),
            uploads: Default::default(),
            http: reqwest::Client::new(),
            tts_url: String::new(),
            vaults: Default::default(),
        };
        let tok = crate::auth::create_jwt(u, false, &state.enc).unwrap();
        let app = crate::routes::api_router(state);
        let call = |m: &'static str, uri: String, vt: Option<String>, body: Option<Vec<u8>>, range: Option<&'static str>| {
            let (app, tok) = (app.clone(), tok.clone());
            async move {
                let mut r = Request::builder().method(m).uri(uri).header("authorization", format!("Bearer {tok}")).header("content-type", "application/json");
                if let Some(v) = vt {
                    r = r.header("x-vault-token", v);
                }
                if let Some(x) = range {
                    r = r.header("range", x);
                }
                let res = app.oneshot(r.body(body.map_or(Body::empty(), Body::from)).unwrap()).await.unwrap();
                let st = res.status();
                (st, axum::body::to_bytes(res.into_body(), 1 << 24).await.unwrap().to_vec())
            }
        };
        let js = |v: Value| Some(v.to_string().into_bytes());
        let val = |b: &[u8]| serde_json::from_slice::<Value>(b).unwrap_or(Value::Null);

        let (st, b) = call("GET", "/api/secret".into(), None, None, None).await;
        assert_eq!((st, val(&b)["exists"].clone()), (StatusCode::OK, json!(false)));
        assert_eq!(call("POST", "/api/secret/setup".into(), None, js(json!({"pin": "12a456"})), None).await.0, StatusCode::BAD_REQUEST);
        let (st, b) = call("POST", "/api/secret/setup".into(), None, js(json!({"pin": "123456"})), None).await;
        assert_eq!(st, StatusCode::OK);
        let v = val(&b);
        let (vt, code) = (v["token"].as_str().unwrap().to_string(), v["recovery"].as_str().unwrap().to_string());
        let t = || Some(vt.clone());
        let (_, b) = call("GET", "/api/secret".into(), t(), None, None).await;
        assert_eq!((val(&b)["unlocked"].clone(), val(&b)["left"].as_u64().unwrap() > 590), (json!(true), true));
        // không có token mở khoá -> 403
        assert_eq!(call("GET", "/api/secret/list".into(), None, None, None).await.0, StatusCode::FORBIDDEN);

        // upload 2 chunk lệch khối 64KB
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 253) as u8).collect();
        let (_, b) = call("POST", "/api/secret/uploads".into(), t(), js(json!({"path": "Ảnh/x.bin", "size": data.len()})), None).await;
        let id = val(&b)["id"].as_str().unwrap().to_string();
        assert_eq!(call("PUT", format!("/api/secret/uploads/{id}?offset=0"), t(), Some(data[..70_000].to_vec()), None).await.0, StatusCode::OK);
        assert_eq!(call("PUT", format!("/api/secret/uploads/{id}?offset=1"), t(), Some(vec![1]), None).await.0, StatusCode::CONFLICT);
        assert_eq!(call("PUT", format!("/api/secret/uploads/{id}?offset=70000"), t(), Some(data[70_000..].to_vec()), None).await.0, StatusCode::OK);
        assert_eq!(call("POST", format!("/api/secret/uploads/{id}/complete"), t(), None, None).await.0, StatusCode::CREATED);
        let (st, b) = call("GET", "/api/secret/list?prefix=".into(), t(), None, None).await;
        assert_eq!((st, val(&b)["entries"][0]["name"].clone()), (StatusCode::OK, json!("Ảnh")));
        let (_, b) = call("GET", format!("/api/secret/list?prefix={}", urlencode("Ảnh")), t(), None, None).await;
        assert_eq!(val(&b)["entries"][0]["size"], json!(200_000));
        let p = format!("/api/secret/files/{}/x.bin", urlencode("Ảnh"));
        let (st, b) = call("GET", p.clone(), t(), None, None).await;
        assert_eq!((st, b == data), (StatusCode::OK, true));
        let (st, b) = call("GET", p.clone(), t(), None, Some("bytes=65530-65545")).await;
        assert_eq!((st, b.as_slice()), (StatusCode::PARTIAL_CONTENT, &data[65530..65546]));
        // tên file không nằm trong DB/S3 dạng rõ
        let names: Vec<(Vec<u8>,)> = sqlx::query_as("SELECT name_enc FROM secret_files WHERE user_id = $1").bind(u).fetch_all(&db).await.unwrap();
        assert!(names.iter().all(|(n,)| !String::from_utf8_lossy(n).contains("x.bin")));
        let used: i64 = sqlx::query_scalar("SELECT quota_used FROM users WHERE id = $1").bind(u).fetch_one(&db).await.unwrap();
        assert_eq!(used as u64, HDR + ct_len(200_000));

        // đổi tên + thư mục mới, chặn chuyển thư mục vào chính nó
        assert_eq!(call("POST", "/api/secret/move".into(), t(), js(json!({"from": "Ảnh/x.bin", "to": "B/y.bin"})), None).await.0, StatusCode::OK);
        assert_eq!(call("GET", p.clone(), t(), None, None).await.0, StatusCode::NOT_FOUND);
        assert_eq!(call("POST", "/api/secret/move".into(), t(), js(json!({"from": "B", "to": "B/C/B"})), None).await.0, StatusCode::BAD_REQUEST);
        assert_eq!(call("POST", "/api/secret/mkdir".into(), t(), js(json!({"path": "B"})), None).await.0, StatusCode::CONFLICT);

        // thùng rác: xoá -> khôi phục -> xoá -> dọn hẳn (hoàn quota)
        assert_eq!(call("DELETE", "/api/secret/files/B".into(), t(), None, None).await.0, StatusCode::OK);
        let (_, b) = call("GET", "/api/secret/trash".into(), t(), None, None).await;
        let tr = val(&b);
        assert_eq!((tr[0]["orig_path"].clone(), tr[0]["files"].clone()), (json!("B"), json!(1)));
        let tid = tr[0]["id"].as_str().unwrap().to_string();
        assert_eq!(call("POST", format!("/api/secret/trash/{tid}/restore"), t(), None, None).await.0, StatusCode::NO_CONTENT);
        assert_eq!(call("GET", "/api/secret/files/B/y.bin".into(), t(), None, None).await.0, StatusCode::OK);
        call("DELETE", "/api/secret/files/B".into(), t(), None, None).await;
        assert_eq!(call("DELETE", format!("/api/secret/trash/{tid}"), t(), None, None).await.0, StatusCode::NO_CONTENT);
        let used: i64 = sqlx::query_scalar("SELECT quota_used FROM users WHERE id = $1").bind(u).fetch_one(&db).await.unwrap();
        assert_eq!(used, 0);

        // khoá; sai PIN đếm lần; mở bằng mã khôi phục rồi đặt PIN mới không cần PIN cũ
        call("POST", "/api/secret/lock".into(), t(), None, None).await;
        assert_eq!(call("GET", "/api/secret/list".into(), t(), None, None).await.0, StatusCode::FORBIDDEN);
        let (st, b) = call("POST", "/api/secret/unlock".into(), None, js(json!({"pin": "000000"})), None).await;
        assert_eq!(st, StatusCode::FORBIDDEN);
        assert!(val(&b)["error"].as_str().unwrap().contains("còn 4"));
        let (st, b) = call("POST", "/api/secret/unlock".into(), None, js(json!({"recovery": code.to_lowercase()})), None).await;
        assert_eq!(st, StatusCode::OK);
        let vt2 = Some(val(&b)["token"].as_str().unwrap().to_string());
        assert_eq!(call("POST", "/api/secret/pin".into(), vt2.clone(), js(json!({"new": "654321"})), None).await.0, StatusCode::NO_CONTENT);
        assert_eq!(call("POST", "/api/secret/unlock".into(), None, js(json!({"pin": "123456"})), None).await.0, StatusCode::FORBIDDEN);
        let (st, b) = call("POST", "/api/secret/unlock".into(), None, js(json!({"pin": "654321"})), None).await;
        assert_eq!(st, StatusCode::OK);
        let vt3 = Some(val(&b)["token"].as_str().unwrap().to_string());
        // mở bằng PIN thì đổi PIN phải có PIN cũ
        assert_eq!(call("POST", "/api/secret/pin".into(), vt3.clone(), js(json!({"new": "111111"})), None).await.0, StatusCode::BAD_REQUEST);
        assert_eq!(call("DELETE", "/api/secret".into(), vt3.clone(), js(json!({"pin": "654321"})), None).await.0, StatusCode::NO_CONTENT);
        let (_, b) = call("GET", "/api/secret".into(), vt3.clone(), None, None).await;
        assert_eq!((val(&b)["exists"].clone(), val(&b)["unlocked"].clone()), (json!(false), json!(false)));
    }
}
