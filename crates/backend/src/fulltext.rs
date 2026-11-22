//! Tìm theo nội dung file chữ (.md/.txt): bảng `file_text` + index trigram (không cần stemming, chạy tốt với tiếng Việt).
use futures::StreamExt;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{files::Store, index::like_escape};

pub const MAX_BYTES: u64 = 2 * 1024 * 1024;

pub fn indexable(path: &str, size: u64) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    size <= MAX_BYTES && matches!(ext.as_str(), "md" | "markdown" | "txt")
}

pub async fn put(db: &PgPool, user: Uuid, path: &str, bytes: &[u8]) -> Result<(), sqlx::Error> {
    // Postgres TEXT không nhận byte 0.
    let body = String::from_utf8_lossy(bytes).replace('\0', "");
    sqlx::query("INSERT INTO file_text (user_id, path, body) VALUES ($1, $2, $3) ON CONFLICT (user_id, path) DO UPDATE SET body = EXCLUDED.body")
        .bind(user)
        .bind(path)
        .bind(body)
        .execute(db)
        .await?;
    Ok(())
}

/// Đọc file từ store rồi ghi vào chỉ mục nội dung. Lỗi chỉ log: tìm theo nội dung là phụ, không chặn upload.
pub async fn put_from_store(db: &PgPool, store: &Store, user: Uuid, path: &str, size: u64) {
    if !indexable(path, size) {
        return;
    }
    let r = async {
        let key = store.key(user, path).map_err(|e| e.to_string())?;
        let data = store.op.read(&key).await.map_err(|e| format!("{}", e.kind()))?.to_vec();
        put(db, user, path, &data).await.map_err(|e| e.to_string())
    }
    .await;
    if let Err(e) = r {
        tracing::warn!("fulltext {path}: {e}");
    }
}

pub async fn move_paths(c: &mut sqlx::PgConnection, user: Uuid, from: &str, to: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE file_text SET path = $3 || substr(path, length($2) + 1) WHERE user_id = $1 AND (path = $2 OR path LIKE $4)")
        .bind(user)
        .bind(from)
        .bind(to)
        .bind(format!("{}/%", like_escape(from)))
        .execute(c)
        .await?;
    Ok(())
}

pub async fn delete_under(c: &mut sqlx::PgConnection, user: Uuid, path: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM file_text WHERE user_id = $1 AND (path = $2 OR path LIKE $3)")
        .bind(user)
        .bind(path)
        .bind(format!("{}/%", like_escape(path)))
        .execute(c)
        .await?;
    Ok(())
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Hit {
    pub path: String,
    pub name: String,
    pub size: i64,
    pub modified: i64,
    /// Đoạn chữ quanh chỗ khớp đầu tiên.
    pub snippet: String,
}

pub async fn search(db: &PgPool, user: Uuid, q: &str, limit: i64) -> Result<Vec<Hit>, sqlx::Error> {
    sqlx::query_as(
        "SELECT t.path, f.name, f.size, f.modified,
                substr(t.body, greatest(1, strpos(lower(t.body), lower($3)) - 60), 220) AS snippet
         FROM file_text t JOIN files f ON f.user_id = t.user_id AND f.path = t.path
         WHERE t.user_id = $1 AND t.body ILIKE $2 AND t.path NOT LIKE '.%' AND t.path NOT LIKE '%/.%'
         ORDER BY f.modified DESC LIMIT $4",
    )
    .bind(user)
    .bind(format!("%{}%", like_escape(q)))
    .bind(q)
    .bind(limit)
    .fetch_all(db)
    .await
}

/// Bù chỉ mục nội dung cho file chữ chưa có (lần đầu nâng cấp, hoặc sau reindex). Trả về số file đã thêm.
pub async fn backfill(db: &PgPool, store: &Store, user: Uuid) -> Result<usize, sqlx::Error> {
    let todo: Vec<(String, i64)> = sqlx::query_as(
        "SELECT f.path, f.size FROM files f
         WHERE f.user_id = $1 AND NOT f.is_dir AND f.size <= $2
           AND (lower(f.name) LIKE '%.md' OR lower(f.name) LIKE '%.markdown' OR lower(f.name) LIKE '%.txt')
           AND NOT EXISTS (SELECT 1 FROM file_text t WHERE t.user_id = f.user_id AND t.path = f.path)",
    )
    .bind(user)
    .bind(MAX_BYTES as i64)
    .fetch_all(db)
    .await?;
    let n = todo.len();
    futures::stream::iter(todo)
        .for_each_concurrent(16, |(p, size)| async move { put_from_store(db, store, user, &p, size as u64).await })
        .await;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn which_files() {
        assert!(indexable("a/b.MD", 10) && indexable("x.txt", 10));
        assert!(!indexable("a.pdf", 10) && !indexable("big.md", MAX_BYTES + 1));
    }

    #[tokio::test]
    async fn content_search_follows_files() {
        let Ok(url) = std::env::var("NK_TEST_DB") else { return };
        let db = PgPool::connect(&url).await.unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        let (s, _) = crate::files::tests::tmp_store("ft");
        let u = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, username, pass_hash) VALUES ($1, $2, 'x')").bind(u).bind(u.to_string()).execute(&db).await.unwrap();
        let mut q = crate::quota::Quota::new(0, 0);
        let body = "# Động học\n\nVận tốc là đạo hàm của vị trí theo thời gian.";
        s.write_chunk(u, "f1", 0, body.as_bytes()).unwrap();
        let m = s.complete(u, "f1", "v/a.md", body.len() as u64, None, &mut q).await.unwrap();
        crate::index::add_file(&db, u, "v/a.md", body.len() as u64, m).await.unwrap();
        assert_eq!(backfill(&db, &s, u).await.unwrap(), 1);
        assert_eq!(backfill(&db, &s, u).await.unwrap(), 0); // đã có thì bỏ qua
        let hits = search(&db, u, "ĐẠO HÀM", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.contains("đạo hàm"));
        crate::ops::move_path(&db, &s, u, "v", "w").await.unwrap();
        assert_eq!(search(&db, u, "vận tốc", 10).await.unwrap()[0].path, "w/a.md");
        let t = crate::ops::trash(&db, &s, u, "w").await.unwrap();
        assert!(search(&db, u, "vận tốc", 10).await.unwrap().is_empty()); // trong thùng rác thì ẩn
        crate::ops::purge(&db, &s, u, t.id).await.unwrap();
        let left: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM file_text WHERE user_id = $1").bind(u).fetch_one(&db).await.unwrap();
        assert_eq!(left.0, 0);
    }
}
