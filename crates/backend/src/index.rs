//! Chỉ mục metadata file trong Postgres. Store (RustFS) giữ bytes, bảng `files` giữ cây thư mục.
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::files::{Store, StoreError};

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SortKey {
    #[default]
    Name,
    Size,
    Modified,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Row {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: i64,
    pub modified: i64,
}

#[derive(Debug, Serialize)]
pub struct Page {
    pub entries: Vec<Row>,
    pub dirs: i64,
    pub files: i64,
    pub bytes: i64,
}

/// "a/b/c" -> ("a/b", "c"); "c" -> ("", "c").
pub fn split(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

/// "a/b/c.txt" -> ["a", "a/b"].
pub fn ancestors(path: &str) -> Vec<&str> {
    path.match_indices('/').map(|(i, _)| &path[..i]).collect()
}

fn order(sort: SortKey, desc: bool) -> String {
    let d = if desc { "DESC" } else { "ASC" };
    match sort {
        SortKey::Name => format!("name {d}"),
        SortKey::Size => format!("size {d}, name {d}"),
        SortKey::Modified => format!("modified {d}, name {d}"),
    }
}

/// Escape cho ILIKE (escape char mặc định là `\`).
pub fn like_escape(q: &str) -> String {
    q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

const COLS: &str = "path, name, is_dir, size, modified";
/// Dotfile (.nextkloud chứa cài đặt) không hiện ra.
const VISIBLE: &str = "name NOT LIKE '.%'";

pub async fn list(
    db: &PgPool,
    user: Uuid,
    parent: &str,
    sort: SortKey,
    desc: bool,
    offset: i64,
    limit: i64,
) -> Result<Page, sqlx::Error> {
    let counts: Vec<(bool, i64, i64)> = sqlx::query_as(&format!(
        "SELECT is_dir, COUNT(*), COALESCE(SUM(size), 0)::BIGINT FROM files
         WHERE user_id = $1 AND parent = $2 AND {VISIBLE} GROUP BY is_dir"
    ))
    .bind(user)
    .bind(parent)
    .fetch_all(db)
    .await?;
    let (mut dirs, mut files, mut bytes) = (0, 0, 0);
    for (d, n, b) in counts {
        if d { dirs = n } else { files = n; bytes = b }
    }
    // Thư mục luôn đứng trước: 2 truy vấn riêng, mỗi cái đi thẳng index.
    let sql = format!(
        "SELECT {COLS} FROM files WHERE user_id = $1 AND parent = $2 AND is_dir = $3 AND {VISIBLE}
         ORDER BY {} OFFSET $4 LIMIT $5",
        order(sort, desc)
    );
    let fetch = |is_dir: bool, off: i64, lim: i64| {
        sqlx::query_as::<_, Row>(&sql).bind(user).bind(parent).bind(is_dir).bind(off).bind(lim).fetch_all(db)
    };
    let mut entries = vec![];
    if offset < dirs {
        entries = fetch(true, offset, limit.min(dirs - offset)).await?;
    }
    let rest = limit - entries.len() as i64;
    if rest > 0 && files > 0 {
        entries.extend(fetch(false, (offset - dirs).max(0), rest).await?);
    }
    Ok(Page { entries, dirs, files, bytes })
}

/// Tìm theo tên trên toàn bộ cây của user (trigram index).
pub async fn search(db: &PgPool, user: Uuid, q: &str, limit: i64) -> Result<Vec<Row>, sqlx::Error> {
    sqlx::query_as::<_, Row>(&format!(
        "SELECT {COLS} FROM files
         WHERE user_id = $1 AND name ILIKE $2 AND path NOT LIKE '.%' AND path NOT LIKE '%/.%'
         ORDER BY is_dir DESC, name LIMIT $3"
    ))
    .bind(user)
    .bind(format!("%{}%", like_escape(q)))
    .bind(limit)
    .fetch_all(db)
    .await
}

const UPSERT_DIR: &str = "INSERT INTO files (user_id, path, parent, name, is_dir, size, modified)
     VALUES ($1, $2, $3, $4, TRUE, 0, $5)
     ON CONFLICT (user_id, path) DO UPDATE SET modified = GREATEST(files.modified, EXCLUDED.modified)";

/// Tạo các dòng thư mục còn thiếu (đã có thì chỉ nâng modified).
pub async fn ensure_dirs(c: &mut sqlx::PgConnection, user: Uuid, dirs: &[&str], modified: i64) -> Result<(), sqlx::Error> {
    for d in dirs {
        let (parent, name) = split(d);
        sqlx::query(UPSERT_DIR).bind(user).bind(*d).bind(parent).bind(name).bind(modified).execute(&mut *c).await?;
    }
    Ok(())
}

/// Ghi 1 file (thêm/ghi đè) + tạo các thư mục cha còn thiếu.
pub async fn add_file(db: &PgPool, user: Uuid, path: &str, size: u64, modified: i64) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;
    for d in ancestors(path) {
        let (parent, name) = split(d);
        sqlx::query(UPSERT_DIR).bind(user).bind(d).bind(parent).bind(name).bind(modified).execute(&mut *tx).await?;
    }
    let (parent, name) = split(path);
    sqlx::query(
        "INSERT INTO files (user_id, path, parent, name, is_dir, size, modified)
         VALUES ($1, $2, $3, $4, FALSE, $5, $6)
         ON CONFLICT (user_id, path) DO UPDATE SET is_dir = FALSE, size = EXCLUDED.size, modified = EXCLUDED.modified",
    )
    .bind(user)
    .bind(path)
    .bind(parent)
    .bind(name)
    .bind(size as i64)
    .bind(modified)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

pub async fn remove_file(db: &PgPool, user: Uuid, path: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM files WHERE user_id = $1 AND path = $2 AND NOT is_dir")
        .bind(user)
        .bind(path)
        .execute(db)
        .await?;
    Ok(())
}

/// Dựng lại chỉ mục của 1 user từ store (sau migrate / lệch dữ liệu). Cập nhật luôn quota_used.
/// Trả về (số file, tổng bytes).
pub async fn rebuild(db: &PgPool, store: &Store, user: Uuid) -> Result<(u64, u64), StoreError> {
    let objs = store.scan(user).await?;
    let mut dirs: HashMap<String, i64> = HashMap::new();
    for (p, _, m) in &objs {
        for d in ancestors(p) {
            let e = dirs.entry(d.to_string()).or_insert(*m);
            *e = (*e).max(*m);
        }
    }
    // Giữ lại thư mục rỗng do user tạo: S3 không có thư mục rỗng nên quét không thấy.
    // So sánh bằng Rust để khỏi dính wildcard SQL khi tên có '%' hoặc '_'.
    // Bỏ qua thư mục hệ thống (dấu chấm): avatar/cài đặt vẫn được quét lại bình thường.
    let old: Vec<(String, i64)> = sqlx::query_as(
        "SELECT path, modified FROM files WHERE user_id = $1 AND is_dir AND left(path, 1) <> '.' ORDER BY path",
    )
    .bind(user)
    .fetch_all(db)
    .await
    .map_err(db_err)?;
    {
        let scanned: std::collections::HashSet<&str> = objs.iter().map(|(p, _, _)| p.as_str()).collect();
        for (p, m) in old {
            if scanned.iter().any(|f| f.starts_with(&format!("{p}/"))) {
                continue;
            }
            dirs.entry(p).or_insert(m);
        }
    }
    let total: u64 = objs.iter().map(|o| o.1).sum();
    let rows = dirs
        .iter()
        .map(|(p, m)| (p.as_str(), true, 0u64, *m))
        .chain(objs.iter().map(|(p, s, m)| (p.as_str(), false, *s, *m)));
    let rows: Vec<_> = rows.collect();

    let mut tx = db.begin().await.map_err(db_err)?;
    sqlx::query("DELETE FROM files WHERE user_id = $1").bind(user).execute(&mut *tx).await.map_err(db_err)?;
    for chunk in rows.chunks(5000) {
        let (mut path, mut parent, mut name, mut is_dir, mut size, mut modified) =
            (vec![], vec![], vec![], vec![], vec![], vec![]);
        for (p, d, s, m) in chunk {
            let (pa, n) = split(p);
            path.push(*p);
            parent.push(pa);
            name.push(n);
            is_dir.push(*d);
            size.push(*s as i64);
            modified.push(*m);
        }
        sqlx::query(
            "INSERT INTO files (user_id, path, parent, name, is_dir, size, modified)
             SELECT $1, * FROM UNNEST($2::text[], $3::text[], $4::text[], $5::bool[], $6::int8[], $7::int8[])",
        )
        .bind(user)
        .bind(&path)
        .bind(&parent)
        .bind(&name)
        .bind(&is_dir)
        .bind(&size)
        .bind(&modified)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    }
    sqlx::query("UPDATE users SET quota_used = $1 WHERE id = $2")
        .bind(total as i64)
        .bind(user)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok((objs.len() as u64, total))
}

fn db_err(e: sqlx::Error) -> StoreError {
    StoreError::Io(std::io::Error::other(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_ancestors() {
        assert_eq!(split("a/b/c.txt"), ("a/b", "c.txt"));
        assert_eq!(split("c.txt"), ("", "c.txt"));
        assert_eq!(ancestors("a/b/c.txt"), vec!["a", "a/b"]);
        assert!(ancestors("c.txt").is_empty());
    }

    #[test]
    fn escape_like() {
        assert_eq!(like_escape("50%_a\\b"), "50\\%\\_a\\\\b");
    }

    /// Chạy với Postgres thật: NK_TEST_DB=postgres://... cargo test
    async fn test_db() -> Option<PgPool> {
        let url = std::env::var("NK_TEST_DB").ok()?;
        let db = PgPool::connect(&url).await.unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        Some(db)
    }

    async fn new_user(db: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, username, pass_hash) VALUES ($1, $2, 'x')")
            .bind(id)
            .bind(id.to_string())
            .execute(db)
            .await
            .unwrap();
        id
    }

    #[tokio::test]
    async fn paging_dirs_first_natural_order() {
        let Some(db) = test_db().await else { return };
        let u = new_user(&db).await;
        for i in 1..=12 {
            add_file(&db, u, &format!("f{i}.txt"), i, 100 + i as i64).await.unwrap();
        }
        add_file(&db, u, "d2/x.txt", 5, 50).await.unwrap();
        add_file(&db, u, "d10/y/z.txt", 7, 70).await.unwrap();
        add_file(&db, u, ".nextkloud/settings.json", 1, 1).await.unwrap();

        let p = list(&db, u, "", SortKey::Name, false, 0, 3).await.unwrap();
        assert_eq!((p.dirs, p.files, p.bytes), (2, 12, 78));
        let names: Vec<_> = p.entries.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["d2", "d10", "f1.txt"]); // dir trước, tự nhiên 2 < 10
        let p = list(&db, u, "", SortKey::Name, false, 3, 3).await.unwrap();
        let names: Vec<_> = p.entries.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["f2.txt", "f3.txt", "f4.txt"]);
        let p = list(&db, u, "", SortKey::Size, true, 2, 2).await.unwrap();
        let names: Vec<_> = p.entries.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["f12.txt", "f11.txt"]);
        // dir cha mang mtime lớn nhất của con
        let p = list(&db, u, "d10", SortKey::Name, false, 0, 10).await.unwrap();
        assert_eq!((p.entries[0].path.as_str(), p.entries[0].modified), ("d10/y", 70));

        // ghi đè cập nhật size, xoá bỏ khỏi index
        add_file(&db, u, "f1.txt", 1000, 1).await.unwrap();
        assert_eq!(list(&db, u, "", SortKey::Name, false, 0, 0).await.unwrap().bytes, 1077);
        remove_file(&db, u, "f1.txt").await.unwrap();
        assert_eq!(list(&db, u, "", SortKey::Name, false, 0, 0).await.unwrap().files, 11);

        let hits = search(&db, u, "Z.T", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "d10/y/z.txt");
        assert!(search(&db, u, "settings", 10).await.unwrap().is_empty()); // dotfolder ẩn
        assert!(search(&db, u, "%", 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn rebuild_from_store() {
        let Some(db) = test_db().await else { return };
        let (s, _) = crate::files::tests::tmp_store("reidx");
        let u = new_user(&db).await;
        let mut q = crate::quota::Quota::new(0, 0);
        for (i, p) in ["a/b/one.txt", "a/two.txt", "three.txt"].iter().enumerate() {
            s.write_chunk(u, &i.to_string(), 0, b"12345").unwrap();
            s.complete(u, &i.to_string(), p, 5, Some(1_600_000_000 + i as i64), &mut q).await.unwrap();
        }
        add_file(&db, u, "stale.txt", 9, 1).await.unwrap();
        assert_eq!(rebuild(&db, &s, u).await.unwrap(), (3, 15));
        let root = list(&db, u, "", SortKey::Name, false, 0, 10).await.unwrap();
        let names: Vec<_> = root.entries.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["a", "three.txt"]);
        let a = list(&db, u, "a/b", SortKey::Name, false, 0, 10).await.unwrap();
        assert_eq!(a.entries[0].size, 5);
        let used: (i64,) = sqlx::query_as("SELECT quota_used FROM users WHERE id = $1").bind(u).fetch_one(&db).await.unwrap();
        assert_eq!(used.0, 15);
    }
}
