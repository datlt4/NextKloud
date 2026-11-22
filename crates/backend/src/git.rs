//! Mirror 1 chiều: git repo → thư mục của user (vd vault Obsidian). Dùng git CLI: clone nông, fetch, diff.
//! Bản clone nằm ở `<staging>/../git/<id>`, file được đẩy vào store + chỉ mục như upload thường.
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use axum::{
    Json,
    extract::{Path as UrlPath, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize, Serializer};
use sqlx::PgPool;
use tokio::process::Command;
use uuid::Uuid;

use crate::{
    files::{Store, StoreError},
    index, ops,
    quota::Quota,
    state::{AppError, AppState, AuthUser, load_user},
};

// Chặn ext::/file:// (chạy lệnh / đọc file của server). Test dùng repo local qua file://.
const ALLOW: &str = if cfg!(test) { "https:http:ssh:file" } else { "https:http:ssh" };

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Repo {
    pub id: Uuid,
    #[serde(skip)]
    pub user_id: Uuid,
    #[serde(serialize_with = "ser_redact")]
    pub url: String,
    pub branch: String,
    pub folder: String,
    #[serde(skip)]
    pub ssh_key: Option<String>,
    pub ssh_pub: Option<String>,
    pub interval_h: i32,
    pub state: String,
    pub last_sync: Option<i64>,
    pub last_commit: Option<String>,
    pub last_error: Option<String>,
    pub files: i64,
    pub created_at: i64,
}

/// Ẩn mật khẩu/token trong URL: `https://user:token@host` và `https://token@host`.
pub fn redact(url: &str) -> String {
    if let Some((scheme, rest)) = url.split_once("://") {
        if let Some((cred, host)) = rest.split_once('@') {
            if let Some((user, _)) = cred.split_once(':') {
                return format!("{scheme}://{user}:***@{host}");
            }
            if !cred.is_empty() {
                return format!("{scheme}://***@{host}");
            }
        }
    }
    url.to_string()
}

fn ser_redact<S: Serializer>(u: &str, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&redact(u))
}

/// Chỉ https/http/ssh và dạng scp `git@host:path`; không cho ký tự điều khiển hay `-` đầu (chèn option).
pub fn check_url(u: &str) -> Result<(), String> {
    if u.is_empty() || u.starts_with('-') || u.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("URL không hợp lệ".into());
    }
    let scheme_ok = ["https://", "http://", "ssh://"].iter().any(|s| u.starts_with(s));
    let scp_ok = !u.contains("://")
        && u.split_once(':').is_some_and(|(h, p)| h.contains('@') && !h.contains('/') && !p.is_empty());
    if scheme_ok || scp_ok { Ok(()) } else { Err("Chỉ hỗ trợ URL https:// hoặc ssh (git@host:user/repo.git)".into()) }
}

pub fn check_branch(b: &str) -> Result<(), String> {
    let ok = b.is_empty()
        || (!b.starts_with('-')
            && !b.contains("..")
            && b.chars().all(|c| c.is_ascii_alphanumeric() || "._/-".contains(c)));
    if ok { Ok(()) } else { Err("Tên nhánh không hợp lệ".into()) }
}

/// Hai thư mục có chồng lấn nhau không (bằng nhau hoặc một cái nằm trong cái kia).
/// So sánh segment chính xác, tránh wildcard SQL khi tên có `_` hoặc `%`.
pub fn path_clash(a: &str, b: &str) -> bool {
    a == b || a.starts_with(&format!("{b}/")) || b.starts_with(&format!("{a}/"))
}

/// "git@github.com:me/My Vault.git" -> "My Vault".
pub fn repo_name(url: &str) -> String {
    let last = url.trim_end_matches('/').rsplit(['/', ':']).next().unwrap_or("repo");
    let n = last.strip_suffix(".git").unwrap_or(last);
    if n.is_empty() { "repo".into() } else { n.into() }
}

pub fn base_dir(store: &Store) -> PathBuf {
    store.staging.with_file_name("git")
}

async fn git(base: &Path, key: Option<&Path>, args: &[&str]) -> Result<String, String> {
    let kh = base.join("known_hosts");
    let mut ssh = format!(
        "ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile='{}'",
        kh.display()
    );
    if let Some(k) = key {
        ssh += &format!(" -o IdentitiesOnly=yes -i '{}'", k.display());
    }
    let mut c = Command::new("git");
    c.args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ALLOW_PROTOCOL", ALLOW)
        .env("GIT_SSH_COMMAND", ssh)
        .env("HOME", base)
        .env_remove("GIT_DIR")
        .stdin(Stdio::null())
        .kill_on_drop(true);
    let out = tokio::time::timeout(Duration::from_secs(1800), c.output())
        .await
        .map_err(|_| "git quá 30 phút, đã huỷ".to_string())?
        .map_err(|e| format!("không chạy được git: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let tail: Vec<&str> = err.lines().filter(|l| !l.trim().is_empty()).collect();
    Err(redact(&tail[tail.len().saturating_sub(3)..].join(" | ")))
}

async fn keygen(base: &Path, args: &[&str]) -> Result<(), AppError> {
    let out = Command::new("ssh-keygen")
        .args(args)
        .env("HOME", base)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| AppError::Internal(format!("ssh-keygen: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!("ssh-keygen: {}", String::from_utf8_lossy(&out.stderr).trim())))
    }
}

fn write_secret(p: &Path, body: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(p)?;
    f.write_all(body.trim_end().as_bytes())?;
    f.write_all(b"\n")
}

/// Cặp khoá ed25519 mới: (private, public).
pub async fn gen_key(base: &Path) -> Result<(String, String), AppError> {
    std::fs::create_dir_all(base).map_err(StoreError::from)?;
    let tmp = base.join(format!("keygen-{}", Uuid::new_v4()));
    let t = tmp.to_string_lossy().into_owned();
    let r = keygen(base, &["-q", "-t", "ed25519", "-N", "", "-C", "nextkloud", "-f", &t]).await;
    let pair = r.and_then(|_| {
        let private = std::fs::read_to_string(&tmp).map_err(StoreError::from)?;
        let public = std::fs::read_to_string(tmp.with_extension("pub")).map_err(StoreError::from)?;
        Ok((private, public.trim().to_string()))
    });
    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(tmp.with_extension("pub"));
    pair
}

/// Public key của private key user dán vào (kiểm tra luôn là key hợp lệ, không passphrase).
pub async fn pub_of(base: &Path, private: &str) -> Result<String, AppError> {
    std::fs::create_dir_all(base).map_err(StoreError::from)?;
    let tmp = base.join(format!("keycheck-{}", Uuid::new_v4()));
    write_secret(&tmp, private).map_err(StoreError::from)?;
    let t = tmp.to_string_lossy().into_owned();
    let out = Command::new("ssh-keygen").args(["-y", "-P", "", "-f", &t]).env("HOME", base).stdin(Stdio::null()).output().await;
    let _ = std::fs::remove_file(&tmp);
    match out {
        Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).trim().to_string()),
        _ => Err(AppError::BadRequest("Private key không hợp lệ hoặc có passphrase".into())),
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

const COLS: &str = "id, user_id, url, branch, folder, ssh_key, ssh_pub, interval_h, state, last_sync, last_commit, last_error, files, created_at";

/// Giữ khoá đồng bộ (state = syncing). `user` = None khi scheduler gọi.
pub async fn begin(db: &PgPool, id: Uuid, user: Option<Uuid>) -> Result<Repo, AppError> {
    let r: Option<Repo> = sqlx::query_as(&format!(
        "UPDATE git_repos SET state = 'syncing', last_error = NULL
         WHERE id = $1 AND ($2::uuid IS NULL OR user_id = $2) AND state <> 'syncing' RETURNING {COLS}"
    ))
    .bind(id)
    .bind(user)
    .fetch_optional(db)
    .await?;
    match r {
        Some(r) => Ok(r),
        None => {
            let exists: Option<(String,)> = sqlx::query_as("SELECT state FROM git_repos WHERE id = $1 AND ($2::uuid IS NULL OR user_id = $2)")
                .bind(id)
                .bind(user)
                .fetch_optional(db)
                .await?;
            Err(match exists {
                Some(_) => AppError::Conflict("đang đồng bộ".into()),
                None => AppError::NotFound("repo not found".into()),
            })
        }
    }
}

/// Chạy đồng bộ rồi ghi kết quả (gọi sau `begin`).
pub async fn finish(db: &PgPool, store: &Store, r: Repo) {
    let res = run(db, store, &r).await;
    let q = match &res {
        Ok(head) => sqlx::query(
            "UPDATE git_repos SET state = 'idle', last_sync = $2, last_commit = $3, last_error = NULL,
               files = (SELECT COUNT(*) FROM files WHERE user_id = $4 AND path LIKE $5 AND NOT is_dir)
             WHERE id = $1",
        )
        .bind(r.id)
        .bind(now())
        .bind(head)
        .bind(r.user_id)
        .bind(format!("{}/%", index::like_escape(&r.folder))),
        Err(e) => sqlx::query("UPDATE git_repos SET state = 'error', last_sync = $2, last_error = $3 WHERE id = $1")
            .bind(r.id)
            .bind(now())
            .bind(e),
    };
    if let Err(e) = q.execute(db).await {
        tracing::error!("git {}: save state: {e}", r.id);
    }
    if let Err(e) = res {
        tracing::warn!("git {} sync failed: {e}", r.id);
    }
}

/// Trả về commit đã đồng bộ.
async fn run(db: &PgPool, store: &Store, r: &Repo) -> Result<String, String> {
    let base = base_dir(store);
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let wd = base.join(r.id.to_string());
    let w = wd.to_string_lossy().into_owned();
    let key = match &r.ssh_key {
        Some(k) => {
            let p = base.join(format!("{}.key", r.id));
            write_secret(&p, k).map_err(|e| e.to_string())?;
            Some(p)
        }
        None => None,
    };
    let key = key.as_deref();

    let fresh = !wd.join(".git").exists();
    if fresh {
        let _ = std::fs::remove_dir_all(&wd);
        let mut a = vec!["clone", "--depth", "1", "--single-branch", "--no-tags"];
        if !r.branch.is_empty() {
            a.extend(["--branch", &r.branch]);
        }
        a.extend(["--", &r.url, &w]);
        git(&base, key, &a).await?;
    } else {
        let refspec = if r.branch.is_empty() { "HEAD" } else { &r.branch };
        // URL có thể vừa đổi (token mới) -> cập nhật remote trước khi fetch.
        git(&base, None, &["-C", &w, "remote", "set-url", "origin", "--", &r.url]).await?;
        git(&base, key, &["-C", &w, "fetch", "--depth", "1", "--no-tags", "origin", refspec]).await?;
        git(&base, None, &["-C", &w, "reset", "--hard", "-q", "FETCH_HEAD"]).await?;
    }
    let head = git(&base, None, &["-C", &w, "rev-parse", "HEAD"]).await?.trim().to_string();
    let ctime: i64 = git(&base, None, &["-C", &w, "log", "-1", "--format=%ct"]).await?.trim().parse().unwrap_or_else(|_| now());

    let prefix = format!("{}/", r.folder);
    let (upsert, delete): (Vec<String>, Vec<String>) = match &r.last_commit {
        Some(old) if !fresh && *old == head => (vec![], vec![]),
        Some(old) if !fresh => match git(&base, None, &["-C", &w, "diff", "--name-status", "-z", "--no-renames", old, &head]).await {
            Ok(out) => parse_diff(&out),
            Err(_) => full(db, &base, &w, r.user_id, &prefix).await?,
        },
        _ => full(db, &base, &w, r.user_id, &prefix).await?,
    };

    let row = load_user(db, r.user_id).await.map_err(|e| format!("{e:?}"))?;
    let mut q = Quota::new(row.quota_limit.max(0) as u64, row.quota_used.max(0) as u64);
    let start_used = row.quota_used;
    let res = apply(db, store, r.user_id, &prefix, &wd, &upsert, &delete, ctime, &mut q).await;
    // Cộng dồn thay vì ghi đè: trong lúc sync, user có thể tải file/dọn thùng rác ở chỗ khác.
    let delta = q.used_bytes as i64 - start_used;
    sqlx::query("UPDATE users SET quota_used = GREATEST(0, quota_used + $1) WHERE id = $2")
        .bind(delta)
        .bind(r.user_id)
        .execute(db)
        .await
        .map_err(|e| e.to_string())?;
    res?;
    Ok(head)
}

/// `git diff --name-status -z`: "M\0path\0D\0path\0..." -> (ghi, xoá).
fn parse_diff(out: &str) -> (Vec<String>, Vec<String>) {
    let (mut up, mut del) = (vec![], vec![]);
    let mut it = out.split('\0').filter(|s| !s.is_empty());
    while let (Some(st), Some(p)) = (it.next(), it.next()) {
        if st.starts_with('D') { del.push(p.to_string()) } else { up.push(p.to_string()) }
    }
    (up, del)
}

/// Đồng bộ toàn bộ: ghi mọi file trong repo, xoá file trong thư mục không còn trong repo.
async fn full(db: &PgPool, base: &Path, w: &str, user: Uuid, prefix: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let all: Vec<String> = git(base, None, &["-C", w, "ls-files", "-z"])
        .await?
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    let have: Vec<(String,)> = sqlx::query_as("SELECT path FROM files WHERE user_id = $1 AND path LIKE $2 AND NOT is_dir")
        .bind(user)
        .bind(format!("{}%", index::like_escape(prefix)))
        .fetch_all(db)
        .await
        .map_err(|e| e.to_string())?;
    let set: std::collections::HashSet<&str> = all.iter().map(String::as_str).collect();
    let del = have.into_iter().map(|(p,)| p[prefix.len()..].to_string()).filter(|p| !set.contains(p.as_str())).collect();
    Ok((all, del))
}

#[allow(clippy::too_many_arguments)]
async fn apply(
    db: &PgPool,
    store: &Store,
    user: Uuid,
    prefix: &str,
    wd: &Path,
    upsert: &[String],
    delete: &[String],
    mtime: i64,
    q: &mut Quota,
) -> Result<(), String> {
    for p in upsert {
        let src = wd.join(p);
        // Bỏ symlink, submodule: chỉ mirror file thường.
        let Ok(m) = std::fs::symlink_metadata(&src) else { continue };
        if !m.is_file() {
            // Upstream đổi file thành symlink/submodule: xoá bản thường cũ còn sót trong mirror.
            let dest = format!("{prefix}{p}");
            match store.delete(user, &dest, q).await {
                Ok(_) | Err(StoreError::NotFound) => {
                    index::remove_file(db, user, &dest).await.map_err(|e| e.to_string())?
                }
                Err(e) => return Err(format!("{dest}: {e}")),
            }
            continue;
        }
        let dest = format!("{prefix}{p}");
        store.put_local(user, &dest, &src, Some(mtime), q).await.map_err(|e| format!("{dest}: {e}"))?;
        index::add_file(db, user, &dest, m.len(), mtime).await.map_err(|e| e.to_string())?;
    }
    for p in delete {
        let dest = format!("{prefix}{p}");
        match store.delete(user, &dest, q).await {
            Ok(_) | Err(StoreError::NotFound) => index::remove_file(db, user, &dest).await.map_err(|e| e.to_string())?,
            Err(e) => return Err(format!("{dest}: {e}")),
        }
    }
    Ok(())
}

/// Repo tới hạn đồng bộ lại (bỏ qua 'pending': còn chờ user thêm deploy key).
pub async fn due(db: &PgPool) -> Result<Vec<Uuid>, sqlx::Error> {
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM git_repos WHERE state IN ('idle', 'error')
           AND (last_sync IS NULL OR last_sync + interval_h * 3600 <= $1) ORDER BY last_sync NULLS FIRST",
    )
    .bind(now())
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

/// Server vừa khởi động: sync đang dở coi như lỗi để chạy lại.
pub async fn reset_stuck(db: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE git_repos SET state = 'error', last_error = 'bị ngắt do server khởi động lại' WHERE state = 'syncing'")
        .execute(db)
        .await?;
    Ok(())
}

// ---------- HTTP ----------

async fn get(db: &PgPool, id: Uuid, user: Uuid) -> Result<Repo, AppError> {
    sqlx::query_as(&format!("SELECT {COLS} FROM git_repos WHERE id = $1 AND user_id = $2"))
        .bind(id)
        .bind(user)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| AppError::NotFound("repo not found".into()))
}

pub async fn list(State(s): State<AppState>, user: AuthUser) -> Result<Json<Vec<Repo>>, AppError> {
    Ok(Json(
        sqlx::query_as(&format!("SELECT {COLS} FROM git_repos WHERE user_id = $1 ORDER BY created_at"))
            .bind(user.id)
            .fetch_all(&s.db)
            .await?,
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyMode {
    Generate,
    Paste,
    None,
}

#[derive(Deserialize)]
pub struct CreateBody {
    pub url: String,
    #[serde(default)]
    pub branch: String,
    pub folder: Option<String>,
    pub key: KeyMode,
    pub private_key: Option<String>,
    pub interval_h: Option<i32>,
}

fn spawn_sync(s: &AppState, r: Repo) {
    let (db, store) = (s.db.clone(), s.store.clone());
    tokio::spawn(async move { finish(&db, &store, r).await });
}

pub async fn create(State(s): State<AppState>, user: AuthUser, Json(b): Json<CreateBody>) -> Result<(StatusCode, Json<Repo>), AppError> {
    let url = b.url.trim().to_string();
    let branch = b.branch.trim().to_string();
    check_url(&url).map_err(AppError::BadRequest)?;
    check_branch(&branch).map_err(AppError::BadRequest)?;
    let folder = ops::user_path(b.folder.as_deref().filter(|f| !f.trim().is_empty()).unwrap_or(&format!("Vaults/{}", repo_name(&url))))?;
    let interval = b.interval_h.unwrap_or(8).clamp(1, 24 * 7);
    // 2 repo không được ghi chồng lên nhau (so từng segment trong Rust,
    // không dùng LIKE để khỏi dính wildcard %/_ trong tên thư mục).
    let existing: Vec<(String,)> =
        sqlx::query_as("SELECT folder FROM git_repos WHERE user_id = $1").bind(user.id).fetch_all(&s.db).await?;
    if let Some(f) = existing.into_iter().map(|(f,)| f).find(|f| path_clash(f, &folder)) {
        return Err(AppError::Conflict(format!("thư mục trùng với repo đang mirror vào '{f}'")));
    }
    let base = base_dir(&s.store);
    let ssh = url.starts_with("ssh://") || !url.contains("://");
    let (key, public) = match b.key {
        KeyMode::Generate => {
            let (k, p) = gen_key(&base).await?;
            (Some(k), Some(p))
        }
        KeyMode::Paste => {
            let k = b.private_key.unwrap_or_default();
            let p = pub_of(&base, &k).await?;
            (Some(k.trim().to_string()), Some(p))
        }
        KeyMode::None => (None, None),
    };
    if ssh && key.is_none() {
        return Err(AppError::BadRequest("URL ssh cần khoá: tạo khoá mới hoặc dán private key".into()));
    }
    // Khoá vừa tạo chưa được thêm vào git host -> chờ user bấm "Đồng bộ ngay".
    let state = if matches!(b.key, KeyMode::Generate) { "pending" } else { "idle" };
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO git_repos (id, user_id, url, branch, folder, ssh_key, ssh_pub, interval_h, state, created_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
    )
    .bind(id)
    .bind(user.id)
    .bind(&url)
    .bind(&branch)
    .bind(&folder)
    .bind(&key)
    .bind(&public)
    .bind(interval)
    .bind(state)
    .bind(now())
    .execute(&s.db)
    .await?;
    let r = if state == "idle" {
        let r = begin(&s.db, id, Some(user.id)).await?;
        spawn_sync(&s, r.clone());
        r
    } else {
        get(&s.db, id, user.id).await?
    };
    Ok((StatusCode::CREATED, Json(r)))
}

pub async fn sync_now(State(s): State<AppState>, user: AuthUser, UrlPath(id): UrlPath<Uuid>) -> Result<(StatusCode, Json<Repo>), AppError> {
    let r = begin(&s.db, id, Some(user.id)).await?;
    spawn_sync(&s, r.clone());
    Ok((StatusCode::ACCEPTED, Json(r)))
}

#[derive(Deserialize)]
pub struct UpdateBody {
    pub interval_h: Option<i32>,
    pub branch: Option<String>,
    /// Đổi URL (vd token https mới).
    pub url: Option<String>,
}

pub async fn update(
    State(s): State<AppState>,
    user: AuthUser,
    UrlPath(id): UrlPath<Uuid>,
    Json(b): Json<UpdateBody>,
) -> Result<Json<Repo>, AppError> {
    let r = get(&s.db, id, user.id).await?;
    let url = b.url.map(|u| u.trim().to_string()).filter(|u| !u.is_empty()).unwrap_or(r.url);
    let branch = b.branch.map(|u| u.trim().to_string()).unwrap_or(r.branch.clone());
    check_url(&url).map_err(AppError::BadRequest)?;
    check_branch(&branch).map_err(AppError::BadRequest)?;
    // URL đã bị che (***) gửi ngược lên sẽ xoá mất token thật.
    if url.contains("***@") {
        return Err(AppError::BadRequest("URL còn chứa ***: dán lại URL đầy đủ hoặc để trống để giữ nguyên".into()));
    }
    let interval = b.interval_h.unwrap_or(r.interval_h).clamp(1, 24 * 7);
    if branch == r.branch {
        // URL mới được `run` set-url mỗi lần sync nên đổi lúc nào cũng được.
        sqlx::query("UPDATE git_repos SET url = $3, interval_h = $4 WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user.id)
            .bind(&url)
            .bind(interval)
            .execute(&s.db)
            .await?;
    } else {
        // Đổi nhánh: giữ khoá sync trong lúc xoá bản clone, rồi để lần sau đồng bộ lại toàn bộ.
        let prev: Option<(String,)> = sqlx::query_as(
            "WITH old AS (SELECT state FROM git_repos WHERE id = $1 AND user_id = $2 FOR UPDATE)
             UPDATE git_repos g SET state = 'syncing' FROM old
             WHERE g.id = $1 AND old.state <> 'syncing' RETURNING old.state",
        )
        .bind(id)
        .bind(user.id)
        .fetch_optional(&s.db)
        .await?;
        let Some((prev,)) = prev else {
            return Err(AppError::Conflict("đang đồng bộ, thử lại sau".into()));
        };
        let _ = std::fs::remove_dir_all(base_dir(&s.store).join(id.to_string()));
        sqlx::query(
            "UPDATE git_repos SET url = $3, branch = $4, interval_h = $5, state = $6, last_commit = NULL
             WHERE id = $1 AND user_id = $2",
        )
        .bind(id)
        .bind(user.id)
        .bind(&url)
        .bind(&branch)
        .bind(interval)
        .bind(&prev)
        .execute(&s.db)
        .await?;
    }
    Ok(Json(get(&s.db, id, user.id).await?))
}

#[derive(Deserialize)]
pub struct DeleteQuery {
    /// true = chuyển luôn thư mục đã mirror vào thùng rác.
    #[serde(default)]
    pub files: bool,
}

pub async fn remove(
    State(s): State<AppState>,
    user: AuthUser,
    UrlPath(id): UrlPath<Uuid>,
    Query(q): Query<DeleteQuery>,
) -> Result<StatusCode, AppError> {
    let r = get(&s.db, id, user.id).await?;
    if r.state == "syncing" {
        return Err(AppError::Conflict("đang đồng bộ, thử lại sau".into()));
    }
    sqlx::query("DELETE FROM git_repos WHERE id = $1").bind(id).execute(&s.db).await?;
    let base = base_dir(&s.store);
    let _ = std::fs::remove_dir_all(base.join(id.to_string()));
    let _ = std::fs::remove_file(base.join(format!("{id}.key")));
    if q.files {
        match ops::trash(&s.db, &s.store, user.id, &r.folder).await {
            Ok(_) | Err(AppError::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::SortKey;

    #[test]
    fn url_rules() {
        for ok in ["https://github.com/me/vault.git", "git@github.com:me/vault.git", "ssh://git@host:2222/me/v.git", "https://u:tok@gitea.local/me/v"] {
            assert!(check_url(ok).is_ok(), "{ok}");
        }
        for bad in ["ext::sh -c touch% /tmp/x", "-oProxyCommand=x", "file:///etc", "/srv/repo", "../x", "git@host:", "https://a b"] {
            assert!(check_url(bad).is_err(), "{bad}");
        }
        assert!(check_branch("main").is_ok() && check_branch("feat/x-1").is_ok() && check_branch("").is_ok());
        assert!(check_branch("-x").is_err() && check_branch("a..b").is_err() && check_branch("a b").is_err());
        assert_eq!(redact("https://me:secret@host/r.git"), "https://me:***@host/r.git");
        assert_eq!(redact("https://TOKEN123@github.com/me/r.git"), "https://***@github.com/me/r.git");
        assert_eq!(redact("https://host/r.git"), "https://host/r.git");
        assert_eq!(redact("git@github.com:me/r.git"), "git@github.com:me/r.git");
        assert_eq!(repo_name("git@github.com:me/My-Vault.git"), "My-Vault");
        assert_eq!(repo_name("https://gitea.local/me/notes/"), "notes");
        assert_eq!(parse_diff("M\0a.md\0D\0b/c.md\0A\0n.md\0"), (vec!["a.md".into(), "n.md".into()], vec!["b/c.md".into()]));
    }

    #[test]
    fn folder_clash_ignores_like_wildcards() {
        // "_" và "%" trong tên thư mục là ký tự thường, không phải wildcard.
        assert!(path_clash("a", "a"));
        assert!(path_clash("a/b", "a"));
        assert!(path_clash("a", "a/b"));
        assert!(!path_clash("a", "ab"));
        assert!(!path_clash("Vault_backup", "VaultXbackup"));
        assert!(!path_clash("100%_done", "100Xdone"));
        assert!(path_clash("a_b/c", "a_b"));
        assert!(!path_clash("a_b", "a/b"));
    }

    fn sh(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
    }

    #[tokio::test]
    async fn keys() {
        let (s, _) = crate::files::tests::tmp_store("gitkey");
        let base = base_dir(&s);
        let (k, p) = gen_key(&base).await.unwrap();
        assert!(k.contains("PRIVATE KEY") && p.starts_with("ssh-ed25519 "));
        assert_eq!(pub_of(&base, &k).await.unwrap(), p);
        assert!(pub_of(&base, "garbage").await.is_err());
    }

    #[tokio::test]
    async fn mirror_sync() {
        let Ok(url) = std::env::var("NK_TEST_DB") else { return };
        let db = PgPool::connect(&url).await.unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        let (s, _) = crate::files::tests::tmp_store("gitsync");
        let u = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, username, pass_hash) VALUES ($1, $2, 'x')").bind(u).bind(u.to_string()).execute(&db).await.unwrap();

        let src = std::env::temp_dir().join(format!("nk-gitsrc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&src);
        std::fs::create_dir_all(src.join("d")).unwrap();
        std::fs::write(src.join("a.md"), "hello").unwrap();
        std::fs::write(src.join("d/b.md"), "bee").unwrap();
        std::os::unix::fs::symlink("a.md", src.join("link.md")).unwrap();
        sh(&src, &["init", "-q", "-b", "main"]);
        sh(&src, &["add", "-A"]);
        sh(&src, &["commit", "-q", "-m", "1"]);

        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO git_repos (id, user_id, url, folder, created_at) VALUES ($1, $2, $3, 'Vaults/v', 0)")
            .bind(id)
            .bind(u)
            .bind(format!("file://{}", src.display()))
            .execute(&db)
            .await
            .unwrap();
        let sync = || async {
            let r = begin(&db, id, Some(u)).await.unwrap();
            finish(&db, &s, r).await;
            get(&db, id, u).await.unwrap()
        };
        let names = |dir: &'static str| {
            let db = db.clone();
            async move {
                let p = index::list(&db, u, dir, SortKey::Name, false, 0, 100).await.unwrap();
                p.entries.into_iter().map(|e| (e.name, e.size)).collect::<Vec<_>>()
            }
        };
        let used = || async {
            sqlx::query_as::<_, (i64,)>("SELECT quota_used FROM users WHERE id = $1").bind(u).fetch_one(&db).await.unwrap().0
        };

        let r = sync().await;
        assert_eq!((r.state.as_str(), r.files, r.last_error.as_deref()), ("idle", 2, None));
        assert_eq!(names("Vaults/v").await, [("d".to_string(), 0), ("a.md".to_string(), 5)]); // symlink bị bỏ
        assert_eq!(used().await, 8);
        assert_eq!(s.op.read(&format!("{u}/Vaults/v/d/b.md")).await.unwrap().to_vec(), b"bee");

        // đổi upstream: xoá a, sửa b, thêm c
        std::fs::remove_file(src.join("a.md")).unwrap();
        std::fs::remove_file(src.join("link.md")).unwrap();
        std::fs::write(src.join("d/b.md"), "beeee").unwrap();
        std::fs::write(src.join("c.md"), "c").unwrap();
        sh(&src, &["add", "-A"]);
        sh(&src, &["commit", "-q", "-m", "2"]);
        let r = sync().await;
        assert_eq!((r.state.as_str(), r.files), ("idle", 2));
        assert_eq!(names("Vaults/v").await, [("d".to_string(), 0), ("c.md".to_string(), 1)]);
        assert_eq!(names("Vaults/v/d").await, [("b.md".to_string(), 5)]);
        assert_eq!(used().await, 6);
        assert!(s.stat(u, "Vaults/v/a.md").await.is_err());

        // không đổi gì -> vẫn ok
        let head = r.last_commit.clone();
        assert_eq!(sync().await.last_commit, head);

        // mất bản clone local + có file lạ trong thư mục -> đồng bộ lại toàn bộ, dọn file lạ
        std::fs::remove_dir_all(base_dir(&s).join(id.to_string())).unwrap();
        index::add_file(&db, u, "Vaults/v/stale.md", 0, 1).await.unwrap();
        let r = sync().await;
        assert_eq!((r.state.as_str(), r.files), ("idle", 2));
        assert_eq!(names("Vaults/v").await, [("d".to_string(), 0), ("c.md".to_string(), 1)]);
        assert_eq!(used().await, 6);

        // repo hỏng -> state error, có thông báo
        sqlx::query("UPDATE git_repos SET url = 'file:///nonexistent/x' WHERE id = $1").bind(id).execute(&db).await.unwrap();
        sh(&base_dir(&s).join(id.to_string()), &["remote", "set-url", "origin", "file:///nonexistent/x"]);
        let r = sync().await;
        assert_eq!(r.state, "error");
        assert!(r.last_error.is_some());
        assert!(matches!(begin(&db, Uuid::new_v4(), Some(u)).await, Err(AppError::NotFound(_))));
    }
}
