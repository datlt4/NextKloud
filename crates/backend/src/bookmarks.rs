//! Ghim thư mục ở sidebar (đồng bộ đa thiết bị, thay localStorage `nk-bookmarks`).
use axum::{Json, extract::State};
use serde::Deserialize;
use uuid::Uuid;

use crate::state::{AppError, AppState, AuthUser};

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Danh sách path đã ghim (cũ nhất trước), để sidebar render.
pub async fn list(State(s): State<AppState>, user: AuthUser) -> Result<Json<Vec<String>>, AppError> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT path FROM bookmarks WHERE user_id = $1 ORDER BY created_at")
        .bind(user.id)
        .fetch_all(&s.db)
        .await?;
    Ok(Json(rows.into_iter().map(|(p,)| p).collect()))
}

#[derive(Deserialize)]
pub struct Toggle {
    pub path: String,
}

/// Bật/tắt ghim 1 thư mục, trả về danh sách mới (client khỏi refetch).
pub async fn toggle(State(s): State<AppState>, user: AuthUser, Json(b): Json<Toggle>) -> Result<Json<Vec<String>>, AppError> {
    let p = crate::ops::user_path(&b.path)?;
    let gone: Option<(Uuid,)> = sqlx::query_as("DELETE FROM bookmarks WHERE user_id = $1 AND path = $2 RETURNING user_id")
        .bind(user.id)
        .bind(&p)
        .fetch_optional(&s.db)
        .await?;
    if gone.is_none() {
        sqlx::query("INSERT INTO bookmarks (user_id, path, created_at) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING")
            .bind(user.id)
            .bind(&p)
            .bind(now())
            .execute(&s.db)
            .await?;
    }
    list(State(s), user).await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup() -> Option<(sqlx::PgPool, Uuid)> {
        let db = sqlx::PgPool::connect(&std::env::var("NK_TEST_DB").ok()?).await.unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        let id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, username, pass_hash) VALUES ($1, $2, 'x')")
            .bind(id)
            .bind(format!("bm-{id}"))
            .execute(&db)
            .await
            .unwrap();
        Some((db, id))
    }

    fn auth(id: Uuid) -> AuthUser {
        AuthUser { id, admin: false }
    }

    #[tokio::test]
    async fn toggle_roundtrip() {
        let Some((db, u)) = setup().await else { return };
        let s = State(AppState {
            db,
            store: crate::files::tests::tmp_store("bookmarks").0,
            enc: jsonwebtoken::EncodingKey::from_secret(b"t"),
            dec: jsonwebtoken::DecodingKey::from_secret(b"t"),
            uploads: Default::default(),
            http: reqwest::Client::new(),
            tts_url: String::new(),
            vaults: Default::default(),
        });
        let t = Toggle { path: "Obsidian Vaults".into() };
        let l = toggle(s.clone(), auth(u), Json(t)).await.unwrap().0;
        assert_eq!(l, vec!["Obsidian Vaults".to_string()]);
        // bật lần nữa = bỏ ghim
        let l = toggle(
            s.clone(),
            auth(u),
            Json(Toggle { path: "Obsidian Vaults".into() }),
        )
        .await
        .unwrap()
        .0;
        assert!(l.is_empty());
        // path xấu / thư mục hệ thống bị chặn
        assert!(toggle(s.clone(), auth(u), Json(Toggle { path: "../x".into() })).await.is_err());
        assert!(toggle(s, auth(u), Json(Toggle { path: ".nextkloud/x".into() })).await.is_err());
    }

    #[tokio::test]
    async fn isolated_between_users() {
        let Some((db, u)) = setup().await else { return };
        let s = State(AppState {
            db,
            store: crate::files::tests::tmp_store("bookmarks2").0,
            enc: jsonwebtoken::EncodingKey::from_secret(b"t"),
            dec: jsonwebtoken::DecodingKey::from_secret(b"t"),
            uploads: Default::default(),
            http: reqwest::Client::new(),
            tts_url: String::new(),
            vaults: Default::default(),
        });
        toggle(s.clone(), auth(u), Json(Toggle { path: "A".into() })).await.unwrap();
        let u2 = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, username, pass_hash) VALUES ($1, $2, 'x')")
            .bind(u2)
            .bind(format!("bm-{u2}"))
            .execute(&s.db)
            .await
            .unwrap();
        let l = list(s, auth(u2)).await.unwrap().0;
        assert!(l.is_empty(), "user khác không thấy ghim của nhau");
    }
}
