//! Thao tác cây thư mục: tạo thư mục, đổi tên/di chuyển, thùng rác. Store giữ bytes, chỉ mục giữ cây.
use std::collections::{HashMap, HashSet};

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    files::{Store, StoreError, clean_path},
    index::{ancestors, ensure_dirs, like_escape, split},
    jobs::Progress,
    state::AppError,
};

/// Thư mục hệ thống của user (avatar, thùng rác): user không được đụng trực tiếp.
pub const SYS: &str = ".nextkloud";
pub const TRASH: &str = ".nextkloud/trash";

pub fn is_reserved(p: &str) -> bool {
    p == SYS || p.starts_with(".nextkloud/")
}

/// Path user gửi: chuẩn hoá, không rỗng, không nằm trong thư mục hệ thống.
pub fn user_path(p: &str) -> Result<String, AppError> {
    let p = clean_path(p)?;
    if p.is_empty() {
        return Err(AppError::BadRequest("empty path".into()));
    }
    if is_reserved(&p) {
        return Err(AppError::Forbidden("reserved path".into()));
    }
    Ok(p)
}

/// File cài đặt client được tự ghi trong `.nextkloud` (giao diện, ảnh nền).
const CLIENT_FILES: [&str; 2] = [".nextkloud/settings.json", ".nextkloud/background.webp"];

/// Path cho upload: như `user_path` nhưng cho phép các file cài đặt của client.
pub fn upload_path(p: &str) -> Result<String, AppError> {
    let c = clean_path(p)?;
    if CLIENT_FILES.contains(&c.as_str()) { Ok(c) } else { user_path(p) }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Some(is_dir) nếu path có trong chỉ mục.
pub(crate) async fn kind(db: &PgPool, user: Uuid, path: &str) -> Result<Option<bool>, sqlx::Error> {
    let r: Option<(bool,)> = sqlx::query_as("SELECT is_dir FROM files WHERE user_id = $1 AND path = $2")
        .bind(user)
        .bind(path)
        .fetch_optional(db)
        .await?;
    Ok(r.map(|x| x.0))
}

/// Không cho tạo con bên dưới một file.
pub(crate) async fn check_parents(db: &PgPool, user: Uuid, path: &str) -> Result<(), AppError> {
    for a in ancestors(path) {
        if kind(db, user, a).await? == Some(false) {
            return Err(AppError::Conflict(format!("'{a}' là file, không phải thư mục")));
        }
    }
    Ok(())
}

/// Ghi file vào `path`: không được trùng thư mục có sẵn, không nằm dưới 1 file.
pub(crate) async fn check_file_target(db: &PgPool, user: Uuid, path: &str) -> Result<(), AppError> {
    if kind(db, user, path).await? == Some(true) {
        return Err(AppError::Conflict(format!("'{path}' là thư mục")));
    }
    check_parents(db, user, path).await
}

/// Thư mục chỉ tồn tại trong chỉ mục (S3 không có thư mục rỗng).
pub async fn mkdir(db: &PgPool, user: Uuid, path: &str) -> Result<(), AppError> {
    match kind(db, user, path).await? {
        Some(true) => return Err(AppError::Conflict("thư mục đã tồn tại".into())),
        Some(false) => return Err(AppError::Conflict("đã có file cùng tên".into())),
        None => {}
    }
    check_parents(db, user, path).await?;
    let mut dirs = ancestors(path);
    dirs.push(path);
    let mut tx = db.begin().await?;
    ensure_dirs(&mut tx, user, &dirs, now()).await?;
    tx.commit().await?;
    Ok(())
}

/// Lỗi 409 khi đích đã có mà chưa chọn cách xử lý (handler gắn thêm `code: exists`).
pub const EXISTS: &str = "đích đã tồn tại";

/// Cách xử lý file trùng tên khi gộp vào đích đã có.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnConflict {
    Overwrite,
    Skip,
    KeepBoth,
}

/// Đích đã có: khác loại -> 409; cùng loại mà chưa chọn `pol` -> 409 EXISTS. Ok(true) = cần gộp.
pub async fn check_dest(db: &PgPool, user: Uuid, from: &str, to: &str, pol: Option<OnConflict>) -> Result<bool, AppError> {
    if to == from {
        return Ok(false);
    }
    let is_dir = kind(db, user, from).await?.ok_or_else(|| AppError::NotFound("not found".into()))?;
    match kind(db, user, to).await? {
        None => Ok(false),
        Some(d) if d != is_dir => Err(AppError::Conflict("đích đã có nhưng khác loại (file/thư mục)".into())),
        Some(_) if pol.is_none() => Err(AppError::Conflict(EXISTS.into())),
        Some(_) => Ok(true),
    }
}

/// Số file (không tính thư mục) của 1 path: quyết định chạy nền hay không.
pub async fn count_files(db: &PgPool, user: Uuid, path: &str) -> Result<i64, sqlx::Error> {
    let (n,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM files WHERE user_id = $1 AND NOT is_dir AND (path = $2 OR path LIKE $3)")
            .bind(user)
            .bind(path)
            .bind(format!("{}/%", like_escape(path)))
            .fetch_one(db)
            .await?;
    Ok(n)
}

/// Mỗi user 1 khoá: move/copy/trash không chạy đè lên nhau (job nền + sync + web cùng lúc).
static TREE_LOCKS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<Uuid, std::sync::Arc<tokio::sync::Mutex<()>>>>> =
    std::sync::LazyLock::new(Default::default);
async fn tree_lock(user: Uuid) -> tokio::sync::OwnedMutexGuard<()> {
    let m = TREE_LOCKS.lock().unwrap().entry(user).or_default().clone();
    m.lock_owned().await
}

/// Đổi tên / di chuyển file hoặc cả thư mục. Trả về số file đã chuyển.
pub async fn move_path(db: &PgPool, store: &Store, user: Uuid, from: &str, to: &str) -> Result<u64, AppError> {
    move_with(db, store, user, from, to, None, &Progress::default()).await
}

/// Như `move_path`, thêm gộp khi đích đã có (`pol`) và báo tiến độ.
pub async fn move_with(
    db: &PgPool,
    store: &Store,
    user: Uuid,
    from: &str,
    to: &str,
    pol: Option<OnConflict>,
    prog: &Progress,
) -> Result<u64, AppError> {
    if to == from {
        return Ok(0);
    }
    if to.starts_with(&format!("{from}/")) {
        return Err(AppError::BadRequest("không thể chuyển thư mục vào chính nó".into()));
    }
    let _g = tree_lock(user).await;
    if check_dest(db, user, from, to, pol).await? {
        return merge(db, store, user, from, to, pol.unwrap(), false, prog).await;
    }
    let is_dir = kind(db, user, from).await?.ok_or_else(|| AppError::NotFound("not found".into()))?;
    check_parents(db, user, to).await?;

    let sub = format!("{}/%", like_escape(from));
    let (files, modified): (Vec<String>, i64) = if is_dir {
        let rows: Vec<(String, i64)> =
            sqlx::query_as("SELECT path, modified FROM files WHERE user_id = $1 AND path LIKE $2 AND NOT is_dir")
                .bind(user)
                .bind(&sub)
                .fetch_all(db)
                .await?;
        let m = rows.iter().map(|r| r.1).max().unwrap_or_else(now);
        (rows.into_iter().map(|r| r.0).collect(), m)
    } else {
        let m: (i64,) = sqlx::query_as("SELECT modified FROM files WHERE user_id = $1 AND path = $2")
            .bind(user)
            .bind(from)
            .fetch_one(db)
            .await?;
        (vec![from.to_string()], m.0)
    };
    let dest = |p: &str| format!("{to}{}", &p[from.len()..]);
    prog.set_total(files.len() as u64);

    // Fs đổi tên được; S3 thì copy (giữ user metadata mtime) rồi xoá.
    let op = &store.op;
    let rename = op.info().capability().rename;
    let results: Vec<Result<String, StoreError>> = futures::stream::iter(files.iter().cloned())
        .map(|p: String| {
            let (a, b) = (format!("{user}/{p}"), format!("{user}/{}", dest(&p)));
            async move {
                let r = if rename {
                    op.rename(&a, &b).await.map(|_| ())
                } else {
                    match op.copy(&a, &b).await {
                        Ok(_) => op.delete(&a).await,
                        Err(e) => Err(e),
                    }
                };
                prog.tick();
                match r.map_err(StoreError::from) {
                    // Mất trên store từ trước: vẫn dời dòng chỉ mục, reindex sẽ dọn.
                    Ok(()) | Err(StoreError::NotFound) => Ok(p),
                    Err(e) => Err(e),
                }
            }
        })
        .buffer_unordered(64)
        .collect()
        .await;
    let ok: Vec<String> = results.iter().filter_map(|r| r.as_ref().ok().cloned()).collect();
    let all_ok = ok.len() == files.len();
    // Mọi lỗi ghi chỉ mục (UPDATE, ensure_dirs, commit) đều dời S3 về chỗ cũ: chỉ mục chưa đổi nên khỏi mồ côi.
    let indexed: Result<(), sqlx::Error> = async {
        let mut tx = db.begin().await?;
        if all_ok {
            let (tp, tn) = split(to);
            sqlx::query(
                "UPDATE files SET
                   path = $3 || substr(path, length($2) + 1),
                   parent = CASE WHEN path = $2 THEN $4 ELSE $3 || substr(parent, length($2) + 1) END,
                   name = CASE WHEN path = $2 THEN $5 ELSE name END
                 WHERE user_id = $1 AND (path = $2 OR path = ANY($7) OR (is_dir AND path LIKE $6))",
            )
            .bind(user)
            .bind(from)
            .bind(to)
            .bind(tp)
            .bind(tn)
            .bind(&sub)
            .bind(&files)
            .execute(&mut *tx)
            .await?;
            ensure_dirs(&mut tx, user, &ancestors(to), modified).await?;
            // file mới ghi vào nguồn trong lúc chuyển (sync/upload) không bị dời: dựng lại thư mục cha cho chúng
            let left: Vec<(String,)> = sqlx::query_as("SELECT path FROM files WHERE user_id = $1 AND path LIKE $2 AND NOT is_dir")
                .bind(user)
                .bind(&sub)
                .fetch_all(&mut *tx)
                .await?;
            for (p,) in &left {
                ensure_dirs(&mut tx, user, &ancestors(p), modified).await?;
            }
            crate::notes::move_paths(&mut tx, user, from, to).await?;
            crate::fulltext::move_paths(&mut tx, user, from, to).await?;
        } else {
            // Hỏng giữa chừng: chỉ dời những file đã chuyển xong, phần còn lại giữ chỗ cũ.
            for p in &ok {
                let np = dest(p);
                let (pa, n) = split(&np);
                sqlx::query("UPDATE files SET path = $3, parent = $4, name = $5 WHERE user_id = $1 AND path = $2")
                    .bind(user)
                    .bind(p)
                    .bind(&np)
                    .bind(pa)
                    .bind(n)
                    .execute(&mut *tx)
                    .await?;
                ensure_dirs(&mut tx, user, &ancestors(&np), modified).await?;
                crate::notes::move_paths(&mut tx, user, p, &np).await?;
                crate::fulltext::move_paths(&mut tx, user, p, &np).await?;
            }
        }
        tx.commit().await
    }
    .await;
    if let Err(e) = indexed {
        for p in &ok {
            let (a, b) = (format!("{user}/{}", dest(p)), format!("{user}/{p}"));
            let r = if rename {
                store.op.rename(&a, &b).await.map(|_| ())
            } else {
                match store.op.copy(&a, &b).await {
                    Ok(_) => store.op.delete(&a).await,
                    Err(e) => Err(e),
                }
            };
            if let Err(e) = r.map_err(StoreError::from) {
                if !matches!(e, StoreError::NotFound) {
                    tracing::warn!("hoàn tác move thất bại {a} -> {b}: {e}");
                }
            }
        }
        return Err(e.into());
    }
    if let Some(Err(e)) = results.into_iter().find(|r| r.is_err()) {
        return Err(AppError::Internal(format!("chuyển được {}/{} file: {e}", ok.len(), files.len())));
    }
    Ok(ok.len() as u64)
}

/// Một file/thư mục trong chỉ mục: các file (path, size, modified) và thư mục con (kể cả chính nó nếu là thư mục).
pub(crate) struct Tree {
    pub files: Vec<(String, i64, i64)>,
    pub dirs: Vec<String>,
}

pub(crate) async fn tree(db: &PgPool, user: Uuid, path: &str) -> Result<Tree, AppError> {
    let is_dir = kind(db, user, path).await?.ok_or_else(|| AppError::NotFound("not found".into()))?;
    let rows: Vec<(String, bool, i64, i64)> = sqlx::query_as(
        "SELECT path, is_dir, size, modified FROM files WHERE user_id = $1 AND (path = $2 OR ($3 AND path LIKE $4))",
    )
    .bind(user)
    .bind(path)
    .bind(is_dir)
    .bind(format!("{}/%", like_escape(path)))
    .fetch_all(db)
    .await?;
    let (mut files, mut dirs) = (vec![], vec![]);
    for (p, d, size, m) in rows {
        if d { dirs.push(p) } else { files.push((p, size, m)) }
    }
    Ok(Tree { files, dirs })
}

/// Sao chép file hoặc cả thư mục (S3 copy phía server, không qua client). Trả về số file.
pub async fn copy_path(db: &PgPool, store: &Store, user: Uuid, from: &str, to: &str) -> Result<u64, AppError> {
    copy_with(db, store, user, from, to, None, &Progress::default()).await
}

/// Như `copy_path`, thêm gộp khi đích đã có (`pol`) và báo tiến độ (chép + ghi chỉ mục = 2 bước/file).
pub async fn copy_with(
    db: &PgPool,
    store: &Store,
    user: Uuid,
    from: &str,
    to: &str,
    pol: Option<OnConflict>,
    prog: &Progress,
) -> Result<u64, AppError> {
    if to == from || to.starts_with(&format!("{from}/")) {
        return Err(AppError::BadRequest("không thể sao chép thư mục vào chính nó".into()));
    }
    let _g = tree_lock(user).await;
    if check_dest(db, user, from, to, pol).await? {
        return merge(db, store, user, from, to, pol.unwrap(), true, prog).await;
    }
    let t = tree(db, user, from).await?;
    check_parents(db, user, to).await?;
    prog.set_total(2 * t.files.len() as u64);
    let total: i64 = t.files.iter().map(|f| f.1).sum();
    let row = crate::state::load_user(db, user).await?;
    crate::quota::Quota::new(row.quota_limit.max(0) as u64, row.quota_used.max(0) as u64)
        .can_add(total as u64)
        .map_err(|e| AppError::Conflict(format!("quota exceeded: {e}")))?;
    let dest = |p: &str| format!("{to}{}", &p[from.len()..]);
    let mut tx = db.begin().await?;
    let mut dirs: Vec<String> = t.dirs.iter().map(|d| dest(d)).collect();
    dirs.extend(ancestors(to).into_iter().map(String::from));
    let dirs: Vec<&str> = dirs.iter().map(String::as_str).collect();
    ensure_dirs(&mut tx, user, &dirs, now()).await?;
    tx.commit().await?;
    let op = &store.op;
    let results: Vec<Result<(String, i64, i64), StoreError>> = futures::stream::iter(t.files.iter().cloned())
        .map(|(p, size, m)| {
            let (a, b) = (format!("{user}/{p}"), format!("{user}/{}", dest(&p)));
            async move {
                let r = op.copy(&a, &b).await.map(|_| (dest(&p), size, m)).map_err(StoreError::from);
                prog.tick();
                r
            }
        })
        .buffer_unordered(16)
        .collect()
        .await;
    let mut n = 0u64;
    let mut added = 0i64;
    let mut first_err = None;
    for r in results {
        prog.tick();
        match r {
            Ok((p, size, m)) => {
                crate::index::add_file(db, user, &p, size as u64, m).await?;
                crate::fulltext::put_from_store(db, store, user, &p, size as u64).await;
                n += 1;
                added += size;
            }
            Err(e) => first_err = first_err.or(Some(e)),
        }
    }
    sqlx::query("UPDATE users SET quota_used = quota_used + $2 WHERE id = $1").bind(user).bind(added).execute(db).await?;
    if let Some(e) = first_err {
        return Err(AppError::Internal(format!("sao chép được {n}/{} file: {e}", t.files.len())));
    }
    Ok(n)
}

/// "a/b.txt" -> "a/b (2).txt" (hoặc (3), ...) chưa có trong `taken`.
fn unique_name(p: &str, taken: &HashSet<String>) -> String {
    let (dir, name) = split(p);
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..)
        .map(|k| {
            let n = format!("{stem} ({k}){ext}");
            if dir.is_empty() { n } else { format!("{dir}/{n}") }
        })
        .find(|c| !taken.contains(c))
        .unwrap()
}

/// Gộp `from` vào `to` đã có (cùng loại): thư mục gộp đệ quy, file trùng xử lý theo `pol`.
/// Ghi đè: chụp version bản cũ trước, quota trừ phần bị thay. Trả về số file đã chuyển/chép.
#[allow(clippy::too_many_arguments)]
async fn merge(
    db: &PgPool,
    store: &Store,
    user: Uuid,
    from: &str,
    to: &str,
    pol: OnConflict,
    copy: bool,
    prog: &Progress,
) -> Result<u64, AppError> {
    if from.starts_with(&format!("{to}/")) {
        return Err(AppError::BadRequest("không thể gộp vào thư mục cha của chính nó".into()));
    }
    check_parents(db, user, to).await?;
    let t = tree(db, user, from).await?;
    let dest = |p: &str| format!("{to}{}", &p[from.len()..]);
    let have: HashMap<String, (bool, i64)> = sqlx::query_as::<_, (String, bool, i64)>(
        "SELECT path, is_dir, size FROM files WHERE user_id = $1 AND (path = $2 OR path LIKE $3)",
    )
    .bind(user)
    .bind(to)
    .bind(format!("{}/%", like_escape(to)))
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|(p, d, s)| (p, (d, s)))
    .collect();
    // file đè thư mục (hoặc ngược lại) ở tầng sâu: không gộp được
    for d in &t.dirs {
        if have.get(&dest(d)).is_some_and(|x| !x.0) {
            return Err(AppError::Conflict(format!("'{}' là file, không phải thư mục", dest(d))));
        }
    }
    for f in &t.files {
        if have.get(&dest(&f.0)).is_some_and(|x| x.0) {
            return Err(AppError::Conflict(format!("'{}' là thư mục, không phải file", dest(&f.0))));
        }
    }
    // kế hoạch: (nguồn, đích, size, modified, size bản bị ghi đè)
    let mut taken: HashSet<String> = have.keys().cloned().chain(t.files.iter().map(|f| dest(&f.0))).collect();
    let mut plan: Vec<(String, String, i64, i64, Option<i64>)> = vec![];
    let mut left: Vec<String> = vec![]; // file nguồn còn ở lại (bỏ qua/lỗi)
    for (p, size, m) in &t.files {
        let d = dest(p);
        match (have.get(&d).map(|x| x.1), pol) {
            (None, _) => plan.push((p.clone(), d, *size, *m, None)),
            (Some(_), OnConflict::Skip) => left.push(p.clone()),
            (Some(old), OnConflict::Overwrite) => plan.push((p.clone(), d, *size, *m, Some(old))),
            (Some(_), OnConflict::KeepBoth) => {
                let d = unique_name(&d, &taken);
                taken.insert(d.clone());
                plan.push((p.clone(), d, *size, *m, None));
            }
        }
    }
    if copy {
        let net: i64 = plan.iter().map(|x| x.2 - x.4.unwrap_or(0)).sum();
        let row = crate::state::load_user(db, user).await?;
        crate::quota::Quota::new(row.quota_limit.max(0) as u64, row.quota_used.max(0) as u64)
            .can_add(net.max(0) as u64)
            .map_err(|e| AppError::Conflict(format!("quota exceeded: {e}")))?;
    }
    prog.set_total(plan.len() as u64 * if copy { 2 } else { 1 });
    let mut tx = db.begin().await?;
    let mut dirs: Vec<String> = t.dirs.iter().map(|d| dest(d)).collect();
    dirs.extend(ancestors(to).into_iter().map(String::from));
    let dirs: Vec<&str> = dirs.iter().map(String::as_str).collect();
    ensure_dirs(&mut tx, user, &dirs, now()).await?;
    tx.commit().await?;

    let op = &store.op;
    let rename = op.info().capability().rename;
    let results: Vec<(usize, Result<(), StoreError>)> = futures::stream::iter(plan.iter().cloned().enumerate())
        .map(|(i, (p, d, _, _, old)): (usize, (String, String, i64, i64, Option<i64>))| {
            let (a, b) = (format!("{user}/{p}"), format!("{user}/{d}"));
            async move {
                let r = async {
                    if old.is_some() {
                        crate::versions::snapshot(store, user, &d).await?;
                    }
                    let r = if copy {
                        op.copy(&a, &b).await.map(|_| ())
                    } else if rename {
                        op.rename(&a, &b).await.map(|_| ())
                    } else {
                        match op.copy(&a, &b).await {
                            Ok(_) => op.delete(&a).await,
                            Err(e) => Err(e),
                        }
                    };
                    match r.map_err(StoreError::from) {
                        // move: mất trên store từ trước vẫn dời dòng chỉ mục (như move_path), trừ khi đè lên file có thật ở đích
                        Err(StoreError::NotFound) if !copy && old.is_none() => Ok(()),
                        r => r,
                    }
                }
                .await;
                prog.tick();
                (i, r)
            }
        })
        .buffer_unordered(16)
        .collect()
        .await;
    let mut first_err = None;
    let mut ok: Vec<usize> = vec![];
    for (i, r) in results {
        match r {
            Ok(()) => ok.push(i),
            Err(e) => {
                left.push(plan[i].0.clone());
                first_err = first_err.or(Some(e));
            }
        }
    }
    ok.sort_unstable();
    let freed: i64 = ok.iter().filter_map(|&i| plan[i].4).sum();
    if copy {
        let mut added = 0i64;
        for &i in &ok {
            let (_, d, size, m, _) = &plan[i];
            crate::index::add_file(db, user, d, *size as u64, *m).await?;
            crate::fulltext::put_from_store(db, store, user, d, *size as u64).await;
            added += size;
            prog.tick();
        }
        sqlx::query("UPDATE users SET quota_used = GREATEST(0, quota_used + $2) WHERE id = $1")
            .bind(user)
            .bind(added - freed)
            .execute(db)
            .await?;
    } else {
        // store đã chuyển xong (có thể đã đè) nên không hoàn tác được: lỗi ở đây để reindex sửa
        let mut tx = db.begin().await?;
        for &i in &ok {
            let (p, d, _, _, old) = &plan[i];
            if old.is_some() {
                sqlx::query("DELETE FROM files WHERE user_id = $1 AND path = $2").bind(user).bind(d).execute(&mut *tx).await?;
                crate::fulltext::delete_under(&mut tx, user, d).await?;
            }
            let (pa, n) = split(d);
            sqlx::query("UPDATE files SET path = $3, parent = $4, name = $5 WHERE user_id = $1 AND path = $2")
                .bind(user)
                .bind(p)
                .bind(d)
                .bind(pa)
                .bind(n)
                .execute(&mut *tx)
                .await?;
            crate::notes::move_paths(&mut tx, user, p, d).await?;
            crate::fulltext::move_paths(&mut tx, user, p, d).await?;
        }
        // thư mục nguồn đã trống thì bỏ, còn file ở lại thì giữ cây cha của chúng
        let keep: HashSet<&str> = left.iter().flat_map(|p| ancestors(p)).collect();
        let gone: Vec<&str> = t.dirs.iter().map(String::as_str).filter(|d| !keep.contains(d)).collect();
        sqlx::query("DELETE FROM files WHERE user_id = $1 AND is_dir AND path = ANY($2)")
            .bind(user)
            .bind(gone)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE users SET quota_used = GREATEST(0, quota_used - $2) WHERE id = $1")
            .bind(user)
            .bind(freed)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    }
    if let Some(e) = first_err {
        return Err(AppError::Internal(format!("xong {}/{} file: {e}", ok.len(), plan.len())));
    }
    Ok(ok.len() as u64)
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct TrashItem {
    pub id: Uuid,
    pub orig_path: String,
    pub is_dir: bool,
    pub size: i64,
    pub files: i64,
    pub deleted_at: i64,
}

fn trash_dir(id: Uuid) -> String {
    format!("{TRASH}/{id}")
}

fn trash_path(t: &TrashItem) -> String {
    format!("{}/{}", trash_dir(t.id), split(&t.orig_path).1)
}

/// Chuyển vào thùng rác. Quota vẫn tính cho tới khi dọn hẳn.
pub async fn trash(db: &PgPool, store: &Store, user: Uuid, path: &str) -> Result<TrashItem, AppError> {
    let is_dir = kind(db, user, path).await?.ok_or_else(|| AppError::NotFound("not found".into()))?;
    let (size, files): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(size), 0)::BIGINT, COUNT(*) FILTER (WHERE NOT is_dir)
         FROM files WHERE user_id = $1 AND (path = $2 OR path LIKE $3)",
    )
    .bind(user)
    .bind(path)
    .bind(format!("{}/%", like_escape(path)))
    .fetch_one(db)
    .await?;
    let t = TrashItem { id: Uuid::new_v4(), orig_path: path.into(), is_dir, size, files, deleted_at: now() };
    // Chuyển xong mới ghi mục: "Dọn sạch" không thể chạy đè lúc đang copy.
    // Lỗi chỉ mục -> move_path đã dời S3 về, không có gì để ghi. Chuyển được một phần (Internal)
    // -> phần đã chuyển nằm trong thùng rác, vẫn phải ghi mục để khôi phục/dọn được.
    let moved = move_path(db, store, user, path, &trash_path(&t)).await;
    if !matches!(moved, Ok(_) | Err(AppError::Internal(_))) {
        return moved.map(|_| t);
    }
    let ins = sqlx::query("INSERT INTO trash (id, user_id, orig_path, is_dir, size, files, deleted_at) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(t.id)
        .bind(user)
        .bind(&t.orig_path)
        .bind(t.is_dir)
        .bind(t.size)
        .bind(t.files)
        .bind(t.deleted_at)
        .execute(db)
        .await;
    if let Err(e) = ins {
        // Không ghi được mục: trả file về chỗ cũ để khỏi mồ côi.
        if let Err(back) = move_path(db, store, user, &trash_path(&t), path).await {
            tracing::error!("trash {}: không ghi được mục và không trả lại được: {back:?}", t.id);
        }
        return Err(e.into());
    }
    moved.map(|_| t)
}

pub async fn list_trash(db: &PgPool, user: Uuid) -> Result<Vec<TrashItem>, sqlx::Error> {
    sqlx::query_as("SELECT id, orig_path, is_dir, size, files, deleted_at FROM trash WHERE user_id = $1 ORDER BY deleted_at DESC")
        .bind(user)
        .fetch_all(db)
        .await
}

async fn get_item(db: &PgPool, user: Uuid, id: Uuid) -> Result<TrashItem, AppError> {
    sqlx::query_as("SELECT id, orig_path, is_dir, size, files, deleted_at FROM trash WHERE user_id = $1 AND id = $2")
        .bind(user)
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| AppError::NotFound("not in trash".into()))
}

/// Trả về chỗ cũ (409 nếu chỗ cũ đã có thứ khác).
pub async fn restore(db: &PgPool, store: &Store, user: Uuid, id: Uuid) -> Result<TrashItem, AppError> {
    let t = get_item(db, user, id).await?;
    move_path(db, store, user, &trash_path(&t), &t.orig_path).await?;
    sqlx::query("DELETE FROM files WHERE user_id = $1 AND path = $2").bind(user).bind(trash_dir(id)).execute(db).await?;
    sqlx::query("DELETE FROM trash WHERE id = $1").bind(id).execute(db).await?;
    Ok(t)
}

/// Xoá hẳn 1 mục, hoàn quota theo chỉ mục. Trả về số byte giải phóng.
pub async fn purge(db: &PgPool, store: &Store, user: Uuid, id: Uuid) -> Result<i64, AppError> {
    get_item(db, user, id).await?;
    let dir = trash_dir(id);
    store.op.delete_with(&format!("{user}/{dir}/")).recursive(true).await.map_err(StoreError::from)?;
    let mut tx = db.begin().await?;
    let (freed,): (i64,) = sqlx::query_as(
        "WITH d AS (DELETE FROM files WHERE user_id = $1 AND (path = $2 OR path LIKE $3) RETURNING size, is_dir)
         SELECT COALESCE(SUM(size) FILTER (WHERE NOT is_dir), 0)::BIGINT FROM d",
    )
    .bind(user)
    .bind(&dir)
    .bind(format!("{}/%", like_escape(&dir)))
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("UPDATE users SET quota_used = GREATEST(0, quota_used - $2) WHERE id = $1")
        .bind(user)
        .bind(freed)
        .execute(&mut *tx)
        .await?;
    crate::notes::delete_under(&mut tx, user, &dir).await?;
    crate::fulltext::delete_under(&mut tx, user, &dir).await?;
    sqlx::query("DELETE FROM trash WHERE id = $1").bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(freed)
}

/// Số ngày giữ thùng rác (env TRASH_DAYS, mặc định 30).
pub fn trash_days() -> i64 {
    std::env::var("TRASH_DAYS").ok().and_then(|v| v.parse().ok()).unwrap_or(30)
}

/// Dọn các mục quá `days` ngày (mọi user). Trả về số mục đã dọn.
/// Một mục lỗi không chặn các mục khác.
pub async fn purge_expired(db: &PgPool, store: &Store, days: i64) -> Result<usize, AppError> {
    let old: Vec<(Uuid, Uuid)> = sqlx::query_as("SELECT user_id, id FROM trash WHERE deleted_at < $1")
        .bind(now() - days * 86400)
        .fetch_all(db)
        .await?;
    let mut done = 0;
    for (u, id) in &old {
        match purge(db, store, *u, *id).await {
            Ok(_) => done += 1,
            Err(e) => tracing::warn!("dọn thùng rác thất bại user={u} id={id}: {e:?}"),
        }
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{self, SortKey};

    async fn test_db() -> Option<PgPool> {
        let db = PgPool::connect(&std::env::var("NK_TEST_DB").ok()?).await.unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        Some(db)
    }

    async fn new_user(db: &PgPool) -> Uuid {
        let u = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, username, pass_hash) VALUES ($1, $2, 'x')")
            .bind(u)
            .bind(u.to_string())
            .execute(db)
            .await
            .unwrap();
        u
    }

    async fn put(db: &PgPool, s: &Store, u: Uuid, path: &str, body: &[u8]) {
        let mut q = crate::quota::Quota::new(0, 0);
        let id = Uuid::new_v4().to_string();
        s.write_chunk(u, &id, 0, body).unwrap();
        let m = s.complete(u, &id, path, body.len() as u64, Some(1_600_000_000), &mut q).await.unwrap();
        index::add_file(db, u, path, body.len() as u64, m).await.unwrap();
        sqlx::query("UPDATE users SET quota_used = quota_used + $2 WHERE id = $1")
            .bind(u)
            .bind(body.len() as i64)
            .execute(db)
            .await
            .unwrap();
    }

    async fn names(db: &PgPool, u: Uuid, dir: &str) -> Vec<String> {
        let p = index::list(db, u, dir, SortKey::Name, false, 0, 1000).await.unwrap();
        p.entries.into_iter().map(|r| r.name).collect()
    }

    async fn used(db: &PgPool, u: Uuid) -> i64 {
        sqlx::query_as::<_, (i64,)>("SELECT quota_used FROM users WHERE id = $1").bind(u).fetch_one(db).await.unwrap().0
    }

    #[test]
    fn reserved_paths() {
        assert!(upload_path(".nextkloud/settings.json").is_ok());
        assert!(upload_path("./.nextkloud/background.webp").is_ok());
        assert!(upload_path(".nextkloud/avatar").is_err());
        assert!(upload_path(".nextkloud/trash/x/a.txt").is_err());
        assert!(user_path(".nextkloud/settings.json").is_err());
        assert!(upload_path("a/b.txt").is_ok());
    }

    #[tokio::test]
    async fn ops_fs() {
        let Some(db) = test_db().await else { return };
        let (s, _) = crate::files::tests::tmp_store("ops");
        let u = new_user(&db).await;
        mkdir_move_trash_restore_purge(db, s, u).await;
    }

    #[tokio::test]
    async fn ops_s3() {
        let Some(db) = test_db().await else { return };
        let Some((s, _)) = crate::files::tests::s3_store() else { return };
        let u = new_user(&db).await;
        mkdir_move_trash_restore_purge(db, s, u).await;
    }

    #[test]
    fn unique_names() {
        let taken: HashSet<String> = ["d/a.txt", "d/a (2).txt", "b"].iter().map(|x| x.to_string()).collect();
        assert_eq!(unique_name("d/a.txt", &taken), "d/a (3).txt");
        assert_eq!(unique_name("b", &taken), "b (2)");
        assert_eq!(unique_name(".rc", &taken), ".rc (2)");
    }

    #[tokio::test]
    async fn merge_fs() {
        let Some(db) = test_db().await else { return };
        let (s, _) = crate::files::tests::tmp_store("merge");
        let u = new_user(&db).await;
        merge_policies(db, s, u).await;
    }

    #[tokio::test]
    async fn merge_s3() {
        let Some(db) = test_db().await else { return };
        let Some((s, _)) = crate::files::tests::s3_store() else { return };
        let u = new_user(&db).await;
        merge_policies(db, s, u).await;
    }

    async fn read(s: &Store, u: Uuid, p: &str) -> Vec<u8> {
        s.op.read(&format!("{u}/{p}")).await.unwrap().to_vec()
    }

    /// quota_used phải luôn bằng tổng size trong chỉ mục.
    async fn quota_ok(db: &PgPool, u: Uuid) {
        let (sum,): (i64,) = sqlx::query_as("SELECT COALESCE(SUM(size), 0)::BIGINT FROM files WHERE user_id = $1 AND NOT is_dir")
            .bind(u)
            .fetch_one(db)
            .await
            .unwrap();
        assert_eq!(used(db, u).await, sum);
    }

    async fn merge_policies(db: PgPool, s: Store, u: Uuid) {
        let p = Progress::default();
        let (ow, sk, kb) = (Some(OnConflict::Overwrite), Some(OnConflict::Skip), Some(OnConflict::KeepBoth));
        put(&db, &s, u, "src/a.txt", b"A1").await;
        put(&db, &s, u, "src/sub/b.txt", b"B1").await;
        put(&db, &s, u, "src/new.txt", b"N").await;
        put(&db, &s, u, "dst/a.txt", b"old-a").await;
        put(&db, &s, u, "dst/sub/b.txt", b"old-b").await;
        put(&db, &s, u, "dst/keep.txt", b"K").await;

        // chưa chọn cách xử lý -> 409 EXISTS
        assert!(matches!(copy_with(&db, &s, u, "src", "dst", None, &p).await, Err(AppError::Conflict(m)) if m == EXISTS));
        assert!(matches!(move_path(&db, &s, u, "src", "dst").await, Err(AppError::Conflict(m)) if m == EXISTS));

        // bỏ qua: chỉ chép file chưa có
        assert_eq!(copy_with(&db, &s, u, "src", "dst", sk, &p).await.unwrap(), 1);
        assert_eq!(read(&s, u, "dst/a.txt").await, b"old-a");
        assert_eq!(names(&db, u, "dst").await, ["sub", "a.txt", "keep.txt", "new.txt"]);
        quota_ok(&db, u).await;

        // giữ cả hai: "x (2).ext", kể cả trong thư mục con
        assert_eq!(copy_with(&db, &s, u, "src", "dst", kb, &p).await.unwrap(), 3);
        assert_eq!(names(&db, u, "dst").await, ["sub", "a (2).txt", "a.txt", "keep.txt", "new (2).txt", "new.txt"]);
        assert_eq!(names(&db, u, "dst/sub").await, ["b (2).txt", "b.txt"]);
        assert_eq!(read(&s, u, "dst/sub/b (2).txt").await, b"B1");
        quota_ok(&db, u).await;

        // ghi đè: chụp version bản cũ, quota trừ phần bị thay
        let before = used(&db, u).await;
        assert_eq!(copy_with(&db, &s, u, "src", "dst", ow, &p).await.unwrap(), 3);
        assert_eq!(read(&s, u, "dst/a.txt").await, b"A1");
        assert_eq!(used(&db, u).await, before - 6);
        quota_ok(&db, u).await;
        let vers = s.op.list(&format!("{u}/{}/dst/a.txt/", crate::versions::DIR)).await.unwrap();
        assert_eq!(vers.iter().filter(|e| !e.metadata().is_dir()).count(), 1);
        assert_eq!(p.done.load(std::sync::atomic::Ordering::Relaxed) > 0, true);

        // chuyển + bỏ qua: file trùng ở lại nguồn cùng cây cha, phần còn lại sang đích
        put(&db, &s, u, "src2/sub/b.txt", b"B2").await;
        put(&db, &s, u, "src2/x/y.txt", b"Y").await;
        mkdir(&db, u, "src2/emp").await.unwrap();
        assert_eq!(move_with(&db, &s, u, "src2", "dst", sk, &p).await.unwrap(), 1);
        assert_eq!(names(&db, u, "src2").await, ["sub"]);
        assert_eq!(names(&db, u, "src2/sub").await, ["b.txt"]);
        assert_eq!(read(&s, u, "dst/x/y.txt").await, b"Y");
        assert_eq!(names(&db, u, "dst").await, ["emp", "sub", "x", "a (2).txt", "a.txt", "keep.txt", "new (2).txt", "new.txt"]);
        quota_ok(&db, u).await;

        // chuyển + ghi đè: nguồn biến mất, quota trừ bản bị thay
        let before = used(&db, u).await;
        assert_eq!(move_with(&db, &s, u, "src2", "dst", ow, &p).await.unwrap(), 1);
        assert_eq!(read(&s, u, "dst/sub/b.txt").await, b"B2");
        assert!(kind(&db, u, "src2").await.unwrap().is_none());
        assert_eq!(used(&db, u).await, before - 2);
        quota_ok(&db, u).await;
        assert!(s.op.stat(&format!("{u}/src2/sub/b.txt")).await.is_err());

        // file đè file: giữ cả hai
        assert_eq!(move_with(&db, &s, u, "src/new.txt", "dst/keep.txt", kb, &p).await.unwrap(), 1);
        assert_eq!(read(&s, u, "dst/keep (2).txt").await, b"N");
        assert_eq!(names(&db, u, "src").await, ["sub", "a.txt"]);

        // khác loại: không gộp được, kể cả khi đã chọn cách xử lý
        mkdir(&db, u, "src3/a.txt").await.unwrap();
        assert!(matches!(move_with(&db, &s, u, "src3", "dst", ow, &p).await, Err(AppError::Conflict(m)) if m != EXISTS));
        assert!(matches!(move_with(&db, &s, u, "src/a.txt", "dst/sub", ow, &p).await, Err(AppError::Conflict(m)) if m != EXISTS));
        // gộp vào thư mục cha của chính nó
        assert!(matches!(move_with(&db, &s, u, "dst/sub", "dst", ow, &p).await, Err(AppError::BadRequest(_))));
        quota_ok(&db, u).await;
    }

    async fn mkdir_move_trash_restore_purge(db: PgPool, s: Store, u: Uuid) {
        mkdir(&db, u, "x/y/z").await.unwrap();
        assert_eq!(names(&db, u, "x/y").await, ["z"]);
        assert!(matches!(mkdir(&db, u, "x/y").await, Err(AppError::Conflict(_))));
        put(&db, &s, u, "f.txt", b"12345").await;
        assert!(matches!(mkdir(&db, u, "f.txt/sub").await, Err(AppError::Conflict(_))));

        // đổi tên file
        move_path(&db, &s, u, "f.txt", "g.txt").await.unwrap();
        assert_eq!(names(&db, u, "").await, ["x", "g.txt"]);
        assert_eq!(s.op.read(&format!("{u}/g.txt")).await.unwrap().to_vec(), b"12345");

        // di chuyển thư mục có 1.000 file + thư mục con rỗng
        for i in 0..1000 {
            put(&db, &s, u, &format!("a/b/{i}.bin"), b"x").await;
        }
        mkdir(&db, u, "a/empty").await.unwrap();
        assert!(matches!(move_path(&db, &s, u, "a", "a/b/c").await, Err(AppError::BadRequest(_))));
        assert!(matches!(move_path(&db, &s, u, "a", "g.txt").await, Err(AppError::Conflict(_))));
        assert_eq!(move_path(&db, &s, u, "a", "x/y/a2").await.unwrap(), 1000);
        assert_eq!(names(&db, u, "x/y").await, ["a2", "z"]);
        assert_eq!(names(&db, u, "x/y/a2").await, ["b", "empty"]);
        assert_eq!(index::list(&db, u, "x/y/a2/b", SortKey::Name, false, 0, 0).await.unwrap().files, 1000);
        assert_eq!(names(&db, u, "").await, ["x", "g.txt"]);
        let on_store = s.scan(u).await.unwrap();
        assert_eq!(on_store.len(), 1001);
        assert!(on_store.iter().all(|o| o.0 == "g.txt" || o.0.starts_with("x/y/a2/b/")));
        assert_eq!(on_store.iter().find(|o| o.0 == "x/y/a2/b/7.bin").unwrap().2, 1_600_000_000); // giữ mtime

        // thùng rác: ẩn khỏi listing, quota giữ nguyên
        let before = used(&db, u).await;
        let t = trash(&db, &s, u, "x/y/a2").await.unwrap();
        assert_eq!((t.files, t.size), (1000, 1000));
        assert_eq!(names(&db, u, "x/y").await, ["z"]);
        assert_eq!(used(&db, u).await, before);
        assert!(index::search(&db, u, "7.bin", 10).await.unwrap().is_empty());
        assert_eq!(list_trash(&db, u).await.unwrap().len(), 1);

        // khôi phục: chỗ cũ bị chiếm -> 409, giải phóng rồi thì ok
        mkdir(&db, u, "x/y/a2").await.unwrap();
        assert!(matches!(restore(&db, &s, u, t.id).await, Err(AppError::Conflict(_))));
        sqlx::query("DELETE FROM files WHERE user_id = $1 AND path = 'x/y/a2'").bind(u).execute(&db).await.unwrap();
        restore(&db, &s, u, t.id).await.unwrap();
        assert_eq!(index::list(&db, u, "x/y/a2/b", SortKey::Name, false, 0, 0).await.unwrap().files, 1000);
        assert!(list_trash(&db, u).await.unwrap().is_empty());
        assert_eq!(names(&db, u, TRASH).await, Vec::<String>::new());

        // xoá hẳn: hoàn quota, sạch store và chỉ mục
        let t = trash(&db, &s, u, "x/y/a2").await.unwrap();
        let t2 = trash(&db, &s, u, "g.txt").await.unwrap();
        assert_eq!(purge(&db, &s, u, t.id).await.unwrap(), 1000);
        assert_eq!(used(&db, u).await, before - 1000);
        assert_eq!(s.scan(u).await.unwrap().len(), 1);
        sqlx::query("UPDATE trash SET deleted_at = 0 WHERE id = $1").bind(t2.id).execute(&db).await.unwrap();
        purge_expired(&db, &s, 30).await.unwrap();
        assert_eq!(used(&db, u).await, before - 1005);
        assert!(s.scan(u).await.unwrap().is_empty());
        let left: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM files WHERE user_id = $1 AND NOT is_dir")
            .bind(u)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(left.0, 0);
    }
}
