//! Thao tác cây thư mục: tạo thư mục, đổi tên/di chuyển, thùng rác. Store giữ bytes, chỉ mục giữ cây.
use futures::StreamExt;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    files::{Store, StoreError, clean_path},
    index::{ancestors, ensure_dirs, like_escape, split},
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
async fn kind(db: &PgPool, user: Uuid, path: &str) -> Result<Option<bool>, sqlx::Error> {
    let r: Option<(bool,)> = sqlx::query_as("SELECT is_dir FROM files WHERE user_id = $1 AND path = $2")
        .bind(user)
        .bind(path)
        .fetch_optional(db)
        .await?;
    Ok(r.map(|x| x.0))
}

/// Không cho tạo con bên dưới một file.
async fn check_parents(db: &PgPool, user: Uuid, path: &str) -> Result<(), AppError> {
    for a in ancestors(path) {
        if kind(db, user, a).await? == Some(false) {
            return Err(AppError::Conflict(format!("'{a}' là file, không phải thư mục")));
        }
    }
    Ok(())
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

/// Đổi tên / di chuyển file hoặc cả thư mục. Trả về số file đã chuyển.
pub async fn move_path(db: &PgPool, store: &Store, user: Uuid, from: &str, to: &str) -> Result<u64, AppError> {
    if to == from {
        return Ok(0);
    }
    if to.starts_with(&format!("{from}/")) {
        return Err(AppError::BadRequest("không thể chuyển thư mục vào chính nó".into()));
    }
    let is_dir = kind(db, user, from).await?.ok_or_else(|| AppError::NotFound("not found".into()))?;
    if kind(db, user, to).await?.is_some() {
        return Err(AppError::Conflict("đích đã tồn tại".into()));
    }
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
                 WHERE user_id = $1 AND (path = $2 OR path LIKE $6)",
            )
            .bind(user)
            .bind(from)
            .bind(to)
            .bind(tp)
            .bind(tn)
            .bind(&sub)
            .execute(&mut *tx)
            .await?;
            ensure_dirs(&mut tx, user, &ancestors(to), modified).await?;
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
