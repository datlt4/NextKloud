//! Bookmark + ghi chú (annotation) trên file markdown/pdf. Ghi chú có 3 trạng thái: todo → doing → done.
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    files::clean_path,
    index::like_escape,
    state::{AppError, AppState, AuthUser},
};

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Annotation {
    pub id: Uuid,
    pub path: String,
    pub kind: String,
    pub anchor: String,
    pub quote: String,
    pub body: String,
    pub state: String,
    pub created_at: i64,
    pub updated_at: i64,
}

const COLS: &str = "id, path, kind, anchor, quote, body, state, created_at, updated_at";
const KINDS: [&str; 2] = ["bookmark", "note"];
const STATES: [&str; 3] = ["todo", "doing", "done"];

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Dời annotation theo file/thư mục (gọi trong transaction của move).
pub async fn move_paths(c: &mut sqlx::PgConnection, user: Uuid, from: &str, to: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE annotations SET path = $3 || substr(path, length($2) + 1)
         WHERE user_id = $1 AND (path = $2 OR path LIKE $4)",
    )
    .bind(user)
    .bind(from)
    .bind(to)
    .bind(format!("{}/%", like_escape(from)))
    .execute(c)
    .await?;
    Ok(())
}

/// Xoá annotation của file/thư mục đã bị xoá hẳn.
pub async fn delete_under(c: &mut sqlx::PgConnection, user: Uuid, path: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM annotations WHERE user_id = $1 AND (path = $2 OR path LIKE $3)")
        .bind(user)
        .bind(path)
        .bind(format!("{}/%", like_escape(path)))
        .execute(c)
        .await?;
    Ok(())
}

#[derive(Deserialize)]
pub struct ListQuery {
    /// Annotation của 1 file.
    pub path: Option<String>,
    /// Không có path: mọi ghi chú của user (lọc theo trạng thái), dùng cho trang "Ghi chú".
    pub state: Option<String>,
}

pub async fn list(State(s): State<AppState>, user: AuthUser, Query(q): Query<ListQuery>) -> Result<Json<Vec<Annotation>>, AppError> {
    let rows = match q.path {
        Some(p) => {
            sqlx::query_as(&format!("SELECT {COLS} FROM annotations WHERE user_id = $1 AND path = $2 ORDER BY created_at"))
                .bind(user.id)
                .bind(clean_path(&p)?)
                .fetch_all(&s.db)
                .await?
        }
        None => {
            // Bỏ các ghi chú đang nằm trong thùng rác.
            sqlx::query_as(&format!(
                "SELECT {COLS} FROM annotations WHERE user_id = $1 AND kind = 'note'
                   AND ($2::text IS NULL OR state = $2) AND path NOT LIKE '.nextkloud/%'
                 ORDER BY updated_at DESC LIMIT 1000"
            ))
            .bind(user.id)
            .bind(q.state)
            .fetch_all(&s.db)
            .await?
        }
    };
    Ok(Json(rows))
}

#[derive(Deserialize)]
pub struct CreateBody {
    pub path: String,
    pub kind: String,
    #[serde(default)]
    pub anchor: String,
    #[serde(default)]
    pub quote: String,
    #[serde(default)]
    pub body: String,
}

fn check(kind: Option<&str>, state: Option<&str>, body: Option<&str>, quote: Option<&str>) -> Result<(), AppError> {
    if kind.is_some_and(|k| !KINDS.contains(&k)) {
        return Err(AppError::BadRequest("kind phải là bookmark hoặc note".into()));
    }
    if state.is_some_and(|k| !STATES.contains(&k)) {
        return Err(AppError::BadRequest("state phải là todo, doing hoặc done".into()));
    }
    if body.is_some_and(|b| b.len() > 20_000) || quote.is_some_and(|b| b.len() > 2_000) {
        return Err(AppError::BadRequest("ghi chú quá dài".into()));
    }
    Ok(())
}

async fn get(db: &PgPool, user: Uuid, id: Uuid) -> Result<Annotation, AppError> {
    sqlx::query_as(&format!("SELECT {COLS} FROM annotations WHERE user_id = $1 AND id = $2"))
        .bind(user)
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| AppError::NotFound("annotation not found".into()))
}

pub async fn create(State(s): State<AppState>, user: AuthUser, Json(b): Json<CreateBody>) -> Result<(StatusCode, Json<Annotation>), AppError> {
    check(Some(&b.kind), None, Some(&b.body), Some(&b.quote))?;
    let path = clean_path(&b.path)?;
    let exists: Option<(bool,)> = sqlx::query_as("SELECT is_dir FROM files WHERE user_id = $1 AND path = $2")
        .bind(user.id)
        .bind(&path)
        .fetch_optional(&s.db)
        .await?;
    if exists.is_none_or(|(d,)| d) {
        return Err(AppError::NotFound("file not found".into()));
    }
    let (id, t) = (Uuid::new_v4(), now());
    sqlx::query(
        "INSERT INTO annotations (id, user_id, path, kind, anchor, quote, body, state, created_at, updated_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,'todo',$8,$8)",
    )
    .bind(id)
    .bind(user.id)
    .bind(&path)
    .bind(&b.kind)
    .bind(&b.anchor)
    .bind(&b.quote)
    .bind(&b.body)
    .bind(t)
    .execute(&s.db)
    .await?;
    Ok((StatusCode::CREATED, Json(get(&s.db, user.id, id).await?)))
}

#[derive(Deserialize)]
pub struct UpdateBody {
    pub body: Option<String>,
    pub state: Option<String>,
}

pub async fn update(State(s): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<UpdateBody>) -> Result<Json<Annotation>, AppError> {
    check(None, b.state.as_deref(), b.body.as_deref(), None)?;
    let n = sqlx::query(
        "UPDATE annotations SET body = COALESCE($3, body), state = COALESCE($4, state), updated_at = $5
         WHERE user_id = $1 AND id = $2",
    )
    .bind(user.id)
    .bind(id)
    .bind(b.body)
    .bind(b.state)
    .bind(now())
    .execute(&s.db)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound("annotation not found".into()));
    }
    Ok(Json(get(&s.db, user.id, id).await?))
}

pub async fn remove(State(s): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> Result<StatusCode, AppError> {
    sqlx::query("DELETE FROM annotations WHERE user_id = $1 AND id = $2").bind(user.id).bind(id).execute(&s.db).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{index, ops};

    #[tokio::test]
    async fn notes_follow_file() {
        let Ok(url) = std::env::var("NK_TEST_DB") else { return };
        let db = PgPool::connect(&url).await.unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        let (s, _) = crate::files::tests::tmp_store("notes");
        let u = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, username, pass_hash) VALUES ($1, $2, 'x')").bind(u).bind(u.to_string()).execute(&db).await.unwrap();
        let mut q = crate::quota::Quota::new(0, 0);
        s.write_chunk(u, "n1", 0, b"# hi").unwrap();
        let m = s.complete(u, "n1", "v/a.md", 4, None, &mut q).await.unwrap();
        index::add_file(&db, u, "v/a.md", 4, m).await.unwrap();
        sqlx::query("INSERT INTO annotations (id, user_id, path, kind, created_at, updated_at) VALUES ($1, $2, 'v/a.md', 'note', 0, 0)")
            .bind(Uuid::new_v4())
            .bind(u)
            .execute(&db)
            .await
            .unwrap();
        let path = || async {
            sqlx::query_as::<_, (String,)>("SELECT path FROM annotations WHERE user_id = $1").bind(u).fetch_optional(&db).await.unwrap().map(|r| r.0)
        };
        ops::move_path(&db, &s, u, "v", "w").await.unwrap();
        assert_eq!(path().await.as_deref(), Some("w/a.md"));
        let t = ops::trash(&db, &s, u, "w/a.md").await.unwrap();
        assert!(path().await.unwrap().starts_with(".nextkloud/trash/"));
        ops::restore(&db, &s, u, t.id).await.unwrap();
        assert_eq!(path().await.as_deref(), Some("w/a.md"));
        let t = ops::trash(&db, &s, u, "w").await.unwrap();
        ops::purge(&db, &s, u, t.id).await.unwrap();
        assert_eq!(path().await, None);
        assert!(check(Some("x"), None, None, None).is_err() && check(None, Some("doing"), None, None).is_ok());
    }
}
