//! Lịch sử phiên bản: trước khi ghi đè 1 file, chép bản cũ sang `.nextkloud/versions/<path>/<mtime>`.
//! Giữ tối đa KEEP bản/file và MAX_DAYS ngày; không tính quota (giống thumbs), không vào chỉ mục.
use axum::{
    Json,
    body::Body,
    extract::{Query, State},
    http::header,
    response::Response,
};
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    files::{Store, StoreError, clean_path, mtime_of},
    ops::user_path,
    state::{AppError, AppState, AuthUser},
};

pub const DIR: &str = ".nextkloud/versions";
const KEEP: usize = 10;
const MAX_DAYS: i64 = 30;

fn vdir(user: Uuid, path: &str) -> String {
    format!("{user}/{DIR}/{path}/")
}

/// Danh sách (mtime, size) các bản cũ, mới nhất trước.
async fn list_raw(store: &Store, user: Uuid, path: &str) -> Result<Vec<(i64, u64)>, StoreError> {
    let dir = vdir(user, path);
    let mut v: Vec<(i64, u64)> = match store.op.lister_with(&dir).await {
        Ok(l) => l
            .try_filter_map(|e| async move {
                let name = e.name().trim_end_matches('/');
                Ok(if e.metadata().is_dir() { None } else { name.parse::<i64>().ok().map(|t| (t, e.metadata().content_length())) })
            })
            .try_collect()
            .await?,
        Err(e) if e.kind() == opendal::ErrorKind::NotFound => vec![],
        Err(e) => return Err(e.into()),
    };
    v.sort_by(|a, b| b.0.cmp(&a.0));
    // fs backend list không kèm size -> stat bù
    for it in v.iter_mut().filter(|x| x.1 == 0) {
        if let Ok(m) = store.op.stat(&format!("{dir}{}", it.0)).await {
            it.1 = m.content_length();
        }
    }
    Ok(v)
}

/// Gọi trước khi ghi đè: chép bản hiện tại (nếu có) thành 1 version, rồi cắt bớt bản cũ.
pub async fn snapshot(store: &Store, user: Uuid, path: &str) -> Result<(), StoreError> {
    let p = clean_path(path)?;
    if p.starts_with(".nextkloud/") {
        return Ok(());
    }
    let key = store.key(user, &p)?;
    let Ok(m) = store.op.stat(&key).await else { return Ok(()) };
    let dir = vdir(user, &p);
    store.op.copy(&key, &format!("{dir}{}", mtime_of(&m))).await?;
    for (t, _) in list_raw(store, user, &p).await?.into_iter().skip(KEEP) {
        let _ = store.op.delete(&format!("{dir}{t}")).await;
    }
    Ok(())
}

/// Xoá bản cũ quá MAX_DAYS ngày của mọi user (chạy định kỳ). Trả về số bản đã xoá.
pub async fn purge_expired(db: &sqlx::PgPool, store: &Store) -> usize {
    let cutoff = chrono::Utc::now().timestamp() - MAX_DAYS * 86_400;
    let users: Vec<(Uuid,)> = sqlx::query_as("SELECT id FROM users").fetch_all(db).await.unwrap_or_default();
    let mut n = 0;
    for (u,) in users {
        let Ok(l) = store.op.lister_with(&format!("{u}/{DIR}/")).recursive(true).await else { continue };
        let keys: Vec<String> = l
            .try_filter_map(|e| async move {
                let old = !e.metadata().is_dir() && e.name().parse::<i64>().is_ok_and(|t| t < cutoff);
                Ok(old.then(|| e.path().to_string()))
            })
            .try_collect()
            .await
            .unwrap_or_default();
        for k in keys {
            if store.op.delete(&k).await.is_ok() {
                n += 1;
            }
        }
    }
    n
}

#[derive(Deserialize)]
pub struct VQuery {
    pub path: String,
    pub ts: Option<i64>,
}

#[derive(Serialize)]
pub struct VersionOut {
    pub ts: i64,
    pub size: u64,
}

pub async fn list(State(s): State<AppState>, user: AuthUser, Query(q): Query<VQuery>) -> Result<Json<Vec<VersionOut>>, AppError> {
    let p = user_path(&q.path)?;
    let v = list_raw(&s.store, user.id, &p).await?;
    Ok(Json(v.into_iter().map(|(ts, size)| VersionOut { ts, size }).collect()))
}

fn version_key(user: Uuid, q: &VQuery) -> Result<(String, String), AppError> {
    let p = user_path(&q.path)?;
    let ts = q.ts.ok_or_else(|| AppError::BadRequest("ts required".into()))?;
    Ok((format!("{}{ts}", vdir(user, &p)), p))
}

/// Tải 1 bản cũ (stream).
pub async fn file(State(s): State<AppState>, user: AuthUser, Query(q): Query<VQuery>) -> Result<Response, AppError> {
    let (key, p) = version_key(user.id, &q)?;
    let m = s.store.op.stat(&key).await.map_err(StoreError::from)?;
    let stream = s
        .store
        .op
        .reader_with(&key)
        .chunk(4 * 1024 * 1024)
        .await
        .map_err(StoreError::from)?
        .into_bytes_stream(0..m.content_length())
        .await
        .map_err(StoreError::from)?;
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, crate::routes::content_type(&p))
        .header(header::CONTENT_LENGTH, m.content_length())
        .body(Body::from_stream(stream))
        .unwrap())
}

/// Khôi phục: bản hiện tại thành 1 version mới, bản cũ chép đè lên file.
pub async fn restore(State(s): State<AppState>, user: AuthUser, Json(q): Json<VQuery>) -> Result<Json<VersionOut>, AppError> {
    let (key, p) = version_key(user.id, &q)?;
    let vm = s.store.op.stat(&key).await.map_err(StoreError::from)?;
    let cur = s.store.key(user.id, &p)?;
    let old_size = s.store.op.stat(&cur).await.map(|m| m.content_length()).unwrap_or(0);
    snapshot(&s.store, user.id, &p).await?;
    s.store.op.copy(&key, &cur).await.map_err(StoreError::from)?;
    let size = vm.content_length();
    sqlx::query("UPDATE users SET quota_used = GREATEST(0, quota_used + $1) WHERE id = $2")
        .bind(size as i64 - old_size as i64)
        .bind(user.id)
        .execute(&s.db)
        .await?;
    let now = chrono::Utc::now().timestamp();
    crate::index::add_file(&s.db, user.id, &p, size, now).await?;
    crate::fulltext::put_from_store(&s.db, &s.store, user.id, &p, size).await;
    Ok(Json(VersionOut { ts: now, size }))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn keeps_latest(store: Store, u: Uuid) {
        let key = store.key(u, "a/b.txt").unwrap();
        // chưa có file -> không tạo version
        snapshot(&store, u, "a/b.txt").await.unwrap();
        assert!(list_raw(&store, u, "a/b.txt").await.unwrap().is_empty());
        let dir = vdir(u, "a/b.txt");
        for i in 0..(KEEP + 3) {
            store.op.write(&key, format!("v{i}").into_bytes()).await.unwrap();
            store.op.copy(&key, &format!("{dir}{}", 1000 + i)).await.unwrap();
        }
        snapshot(&store, u, "a/b.txt").await.unwrap();
        let v = list_raw(&store, u, "a/b.txt").await.unwrap();
        assert_eq!(v.len(), KEEP);
        assert!(v.windows(2).all(|w| w[0].0 > w[1].0));
        assert!(v.iter().all(|x| x.1 > 0));
        // file hệ thống không snapshot
        snapshot(&store, u, ".nextkloud/settings.json").await.unwrap();
        let _ = store.op.delete_with(&format!("{u}/")).recursive(true).await;
    }

    #[tokio::test]
    async fn snapshot_keeps_latest() {
        let (s, u) = crate::files::tests::tmp_store("ver");
        keeps_latest(s, u).await;
        if let Some((s, u)) = crate::files::tests::s3_store() {
            keeps_latest(s, u).await;
        }
    }
}
