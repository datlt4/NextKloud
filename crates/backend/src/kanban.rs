//! Kanban: board -> cột -> card, card có subtask (parent_id). Cột "xong" chỉ trả N card mới nhất, phần còn lại phân trang.
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::state::{AppError, AppState, AuthUser};

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Board {
    pub id: Uuid,
    pub title: String,
    pub labels: sqlx::types::Json<Vec<Label>>,
    pub archived: bool,
    pub position: f64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub title: String,
    pub color: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Column {
    pub id: Uuid,
    pub title: String,
    pub position: f64,
    pub is_done: bool,
    /// Tổng số card (cột xong có thể chỉ trả về một phần).
    pub total: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Card {
    pub id: Uuid,
    pub column_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub title: String,
    pub description: String,
    pub position: f64,
    pub labels: Vec<String>,
    pub color: Option<String>,
    pub due_at: Option<i64>,
    pub done_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub attachments: Vec<String>,
    /// Số subtask / số subtask đã xong.
    pub sub_total: i64,
    pub sub_done: i64,
}

const CARD_COLS: &str = "c.id, c.column_id, c.parent_id, c.title, c.description, c.position, c.labels, c.color, c.due_at, c.done_at, c.created_at, c.updated_at, c.attachments,
    (SELECT COUNT(*) FROM cards s WHERE s.parent_id = c.id AND NOT s.archived) AS sub_total,
    (SELECT COUNT(*) FROM cards s WHERE s.parent_id = c.id AND NOT s.archived AND s.done_at IS NOT NULL) AS sub_done";

async fn own_board(db: &PgPool, user: Uuid, id: Uuid) -> Result<(), AppError> {
    let ok: Option<(i32,)> = sqlx::query_as("SELECT 1 FROM boards WHERE id = $1 AND owner = $2").bind(id).bind(user).fetch_optional(db).await?;
    ok.map(|_| ()).ok_or_else(|| AppError::NotFound("board not found".into()))
}

/// (board_id, is_done) của 1 cột thuộc user.
async fn own_column(db: &PgPool, user: Uuid, id: Uuid) -> Result<(Uuid, bool), AppError> {
    sqlx::query_as("SELECT c.board_id, c.is_done FROM board_columns c JOIN boards b ON b.id = c.board_id WHERE c.id = $1 AND b.owner = $2")
        .bind(id)
        .bind(user)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| AppError::NotFound("column not found".into()))
}

async fn card(db: &PgPool, user: Uuid, id: Uuid) -> Result<(Card, Uuid), AppError> {
    let row: Option<(Uuid,)> = sqlx::query_as("SELECT c.board_id FROM cards c JOIN boards b ON b.id = c.board_id WHERE c.id = $1 AND b.owner = $2")
        .bind(id)
        .bind(user)
        .fetch_optional(db)
        .await?;
    let (board,) = row.ok_or_else(|| AppError::NotFound("card not found".into()))?;
    let c = sqlx::query_as(&format!("SELECT {CARD_COLS} FROM cards c WHERE c.id = $1")).bind(id).fetch_one(db).await?;
    Ok((c, board))
}

fn check_title(t: &str) -> Result<String, AppError> {
    let t = t.trim();
    if t.is_empty() || t.len() > 500 {
        return Err(AppError::BadRequest("tiêu đề trống hoặc quá dài".into()));
    }
    Ok(t.to_string())
}

// ---------- board ----------

pub async fn list_boards(State(s): State<AppState>, user: AuthUser) -> Result<Json<Vec<Board>>, AppError> {
    Ok(Json(
        sqlx::query_as("SELECT id, title, labels, archived, position, created_at FROM boards WHERE owner = $1 ORDER BY archived, position, created_at")
            .bind(user.id)
            .fetch_all(&s.db)
            .await?,
    ))
}

#[derive(Deserialize)]
pub struct NewBoard {
    pub title: String,
}

pub async fn create_board(State(s): State<AppState>, user: AuthUser, Json(b): Json<NewBoard>) -> Result<(StatusCode, Json<Board>), AppError> {
    let title = check_title(&b.title)?;
    let id = Uuid::new_v4();
    let mut tx = s.db.begin().await?;
    let board: Board = sqlx::query_as(
        "INSERT INTO boards (id, owner, title, position, created_at)
         VALUES ($1, $2, $3, (SELECT COALESCE(MAX(position), 0) + 1 FROM boards WHERE owner = $2), $4)
         RETURNING id, title, labels, archived, position, created_at",
    )
    .bind(id)
    .bind(user.id)
    .bind(title)
    .bind(now())
    .fetch_one(&mut *tx)
    .await?;
    for (i, (t, done)) in [("Cần làm", false), ("Đang làm", false), ("Xong", true)].into_iter().enumerate() {
        sqlx::query("INSERT INTO board_columns (id, board_id, title, position, is_done) VALUES ($1, $2, $3, $4, $5)")
            .bind(Uuid::new_v4())
            .bind(id)
            .bind(t)
            .bind(i as f64)
            .bind(done)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(board)))
}

#[derive(Deserialize)]
pub struct BoardPatch {
    pub title: Option<String>,
    pub archived: Option<bool>,
    pub labels: Option<Vec<Label>>,
}

pub async fn update_board(State(s): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<BoardPatch>) -> Result<StatusCode, AppError> {
    own_board(&s.db, user.id, id).await?;
    let title = b.title.as_deref().map(check_title).transpose()?;
    sqlx::query("UPDATE boards SET title = COALESCE($2, title), archived = COALESCE($3, archived), labels = COALESCE($4, labels) WHERE id = $1")
        .bind(id)
        .bind(title)
        .bind(b.archived)
        .bind(b.labels.map(sqlx::types::Json))
        .execute(&s.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_board(State(s): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> Result<StatusCode, AppError> {
    own_board(&s.db, user.id, id).await?;
    sqlx::query("DELETE FROM boards WHERE id = $1").bind(id).execute(&s.db).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct BoardQuery {
    pub done_limit: Option<i64>,
}

#[derive(Serialize)]
pub struct BoardView {
    pub board: Board,
    pub columns: Vec<Column>,
    pub cards: Vec<Card>,
}

pub async fn get_board(State(s): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Query(q): Query<BoardQuery>) -> Result<Json<BoardView>, AppError> {
    own_board(&s.db, user.id, id).await?;
    let board = sqlx::query_as("SELECT id, title, labels, archived, position, created_at FROM boards WHERE id = $1").bind(id).fetch_one(&s.db).await?;
    let columns = sqlx::query_as(
        "SELECT k.id, k.title, k.position, k.is_done, (SELECT COUNT(*) FROM cards c WHERE c.column_id = k.id AND NOT c.archived) AS total
         FROM board_columns k WHERE k.board_id = $1 ORDER BY k.position",
    )
    .bind(id)
    .fetch_all(&s.db)
    .await?;
    // Cột thường: mọi card. Cột xong: N card xong gần nhất mỗi cột.
    let cards = sqlx::query_as(&format!(
        "SELECT {CARD_COLS} FROM cards c JOIN board_columns k ON k.id = c.column_id
         WHERE c.board_id = $1 AND NOT c.archived AND (NOT k.is_done OR c.id IN (
             SELECT x.id FROM cards x WHERE x.column_id = k.id AND NOT x.archived ORDER BY x.done_at DESC NULLS LAST, x.position LIMIT $2))
         ORDER BY c.position"
    ))
    .bind(id)
    .bind(q.done_limit.unwrap_or(10).clamp(0, 500))
    .fetch_all(&s.db)
    .await?;
    Ok(Json(BoardView { board, columns, cards }))
}

#[derive(Deserialize)]
pub struct PageQuery {
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

/// Card của 1 cột (dùng cho phần "Đã xong cũ hơn" của cột xong).
pub async fn column_cards(State(s): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Query(q): Query<PageQuery>) -> Result<Json<Vec<Card>>, AppError> {
    own_column(&s.db, user.id, id).await?;
    Ok(Json(
        sqlx::query_as(&format!(
            "SELECT {CARD_COLS} FROM cards c WHERE c.column_id = $1 AND NOT c.archived ORDER BY c.done_at DESC NULLS LAST, c.position OFFSET $2 LIMIT $3"
        ))
        .bind(id)
        .bind(q.offset.unwrap_or(0).max(0))
        .bind(q.limit.unwrap_or(50).clamp(1, 500))
        .fetch_all(&s.db)
        .await?,
    ))
}

// ---------- cột ----------

#[derive(Deserialize)]
pub struct NewColumn {
    pub title: String,
    #[serde(default)]
    pub is_done: bool,
}

pub async fn create_column(State(s): State<AppState>, user: AuthUser, Path(board): Path<Uuid>, Json(b): Json<NewColumn>) -> Result<(StatusCode, Json<Column>), AppError> {
    own_board(&s.db, user.id, board).await?;
    let c = sqlx::query_as(
        "INSERT INTO board_columns (id, board_id, title, position, is_done)
         VALUES ($1, $2, $3, (SELECT COALESCE(MAX(position), 0) + 1 FROM board_columns WHERE board_id = $2), $4)
         RETURNING id, title, position, is_done, 0::bigint AS total",
    )
    .bind(Uuid::new_v4())
    .bind(board)
    .bind(check_title(&b.title)?)
    .bind(b.is_done)
    .fetch_one(&s.db)
    .await?;
    Ok((StatusCode::CREATED, Json(c)))
}

#[derive(Deserialize)]
pub struct ColumnPatch {
    pub title: Option<String>,
    pub position: Option<f64>,
    pub is_done: Option<bool>,
}

pub async fn update_column(State(s): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<ColumnPatch>) -> Result<StatusCode, AppError> {
    own_column(&s.db, user.id, id).await?;
    let title = b.title.as_deref().map(check_title).transpose()?;
    let mut tx = s.db.begin().await?;
    sqlx::query("UPDATE board_columns SET title = COALESCE($2, title), position = COALESCE($3, position), is_done = COALESCE($4, is_done) WHERE id = $1")
        .bind(id)
        .bind(title)
        .bind(b.position.filter(|p| p.is_finite()))
        .bind(b.is_done)
        .execute(&mut *tx)
        .await?;
    // Đổi kiểu cột thì đồng bộ done_at của card trong cột.
    match b.is_done {
        Some(true) => sqlx::query("UPDATE cards SET done_at = COALESCE(done_at, $2) WHERE column_id = $1").bind(id).bind(now()).execute(&mut *tx).await?,
        Some(false) => sqlx::query("UPDATE cards SET done_at = NULL WHERE column_id = $1").bind(id).execute(&mut *tx).await?,
        None => Default::default(),
    };
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_column(State(s): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> Result<StatusCode, AppError> {
    own_column(&s.db, user.id, id).await?;
    sqlx::query("DELETE FROM board_columns WHERE id = $1").bind(id).execute(&s.db).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- card ----------

#[derive(Deserialize)]
pub struct NewCard {
    pub column_id: Uuid,
    pub title: String,
    pub parent_id: Option<Uuid>,
}

async fn check_parent(db: &PgPool, board: Uuid, card: Option<Uuid>, parent: Uuid) -> Result<(), AppError> {
    // Cùng board, không tự làm cha của mình, chỉ 1 cấp (cha không được là subtask).
    let row: Option<(Option<Uuid>,)> = sqlx::query_as("SELECT parent_id FROM cards WHERE id = $1 AND board_id = $2").bind(parent).bind(board).fetch_optional(db).await?;
    match row {
        None => Err(AppError::BadRequest("task cha không thuộc board này".into())),
        Some(_) if Some(parent) == card => Err(AppError::BadRequest("card không thể là cha của chính nó".into())),
        Some((Some(_),)) => Err(AppError::BadRequest("task cha đang là subtask của task khác".into())),
        Some((None,)) => {
            if let Some(c) = card {
                let kids: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM cards WHERE parent_id = $1").bind(c).fetch_one(db).await?;
                if kids.0 > 0 {
                    return Err(AppError::BadRequest("card đang có subtask, không thể làm subtask".into()));
                }
            }
            Ok(())
        }
    }
}

pub async fn create_card(State(s): State<AppState>, user: AuthUser, Json(b): Json<NewCard>) -> Result<(StatusCode, Json<Card>), AppError> {
    let (board, is_done) = own_column(&s.db, user.id, b.column_id).await?;
    if let Some(p) = b.parent_id {
        check_parent(&s.db, board, None, p).await?;
    }
    let id = Uuid::new_v4();
    let t = now();
    sqlx::query(
        "INSERT INTO cards (id, board_id, column_id, parent_id, title, position, done_at, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, (SELECT COALESCE(MAX(position), 0) + 1 FROM cards WHERE column_id = $3), $6, $7, $7)",
    )
    .bind(id)
    .bind(board)
    .bind(b.column_id)
    .bind(b.parent_id)
    .bind(check_title(&b.title)?)
    .bind(is_done.then_some(t))
    .bind(t)
    .execute(&s.db)
    .await?;
    Ok((StatusCode::CREATED, Json(card(&s.db, user.id, id).await?.0)))
}

/// Trường có thể đặt về null: vắng = giữ nguyên, null = xoá.
fn opt<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
    Ok(Some(Option::deserialize(d)?))
}

#[derive(Deserialize, Default)]
pub struct CardPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub column_id: Option<Uuid>,
    pub position: Option<f64>,
    pub labels: Option<Vec<String>>,
    #[serde(default, deserialize_with = "opt")]
    pub color: Option<Option<String>>,
    #[serde(default, deserialize_with = "opt")]
    pub due_at: Option<Option<i64>>,
    #[serde(default, deserialize_with = "opt")]
    pub parent_id: Option<Option<Uuid>>,
    pub archived: Option<bool>,
    pub attachments: Option<Vec<String>>,
}

pub async fn update_card(State(s): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<CardPatch>) -> Result<Json<Card>, AppError> {
    let (cur, board) = card(&s.db, user.id, id).await?;
    let title = b.title.as_deref().map(check_title).transpose()?;
    if b.description.as_ref().is_some_and(|d| d.len() > 100_000) {
        return Err(AppError::BadRequest("mô tả quá dài".into()));
    }
    // Đổi cột: chỉ trong cùng board; vào cột xong thì ghi done_at, ra khỏi thì xoá.
    let mut done_at = cur.done_at;
    if let Some(col) = b.column_id {
        let (cb, is_done) = own_column(&s.db, user.id, col).await?;
        if cb != board {
            return Err(AppError::BadRequest("cột không thuộc board này".into()));
        }
        done_at = if is_done { cur.done_at.or(Some(now())) } else { None };
    }
    if let Some(Some(p)) = b.parent_id {
        check_parent(&s.db, board, Some(id), p).await?;
    }
    let attachments = b.attachments.map(|v| v.iter().map(|p| crate::ops::user_path(p)).collect::<Result<Vec<_>, _>>()).transpose()?;
    sqlx::query(
        "UPDATE cards SET title = COALESCE($2, title), description = COALESCE($3, description), column_id = COALESCE($4, column_id),
            position = COALESCE($5, position), labels = COALESCE($6, labels),
            color = CASE WHEN $7 THEN $8 ELSE color END, due_at = CASE WHEN $9 THEN $10 ELSE due_at END,
            parent_id = CASE WHEN $11 THEN $12 ELSE parent_id END, archived = COALESCE($13, archived),
            done_at = $14, updated_at = $15, attachments = COALESCE($16, attachments)
         WHERE id = $1",
    )
    .bind(id)
    .bind(title)
    .bind(b.description)
    .bind(b.column_id)
    .bind(b.position.filter(|p| p.is_finite()))
    .bind(b.labels)
    .bind(b.color.is_some())
    .bind(b.color.flatten())
    .bind(b.due_at.is_some())
    .bind(b.due_at.flatten())
    .bind(b.parent_id.is_some())
    .bind(b.parent_id.flatten())
    .bind(b.archived)
    .bind(done_at)
    .bind(now())
    .bind(attachments)
    .execute(&s.db)
    .await?;
    Ok(Json(card(&s.db, user.id, id).await?.0))
}

pub async fn delete_card(State(s): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> Result<StatusCode, AppError> {
    card(&s.db, user.id, id).await?;
    sqlx::query("DELETE FROM cards WHERE id = $1").bind(id).execute(&s.db).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Subtask của 1 card (kể cả đã xong nằm trong phần thu gọn).
pub async fn subtasks(State(s): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> Result<Json<Vec<Card>>, AppError> {
    card(&s.db, user.id, id).await?;
    Ok(Json(
        sqlx::query_as(&format!("SELECT {CARD_COLS} FROM cards c WHERE c.parent_id = $1 AND NOT c.archived ORDER BY c.done_at NULLS FIRST, c.position"))
            .bind(id)
            .fetch_all(&s.db)
            .await?,
    ))
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub async fn db_user() -> Option<(PgPool, Uuid)> {
        let url = std::env::var("NK_TEST_DB").ok()?;
        let db = PgPool::connect(&url).await.unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        let u = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, username, pass_hash) VALUES ($1, $2, 'x')").bind(u).bind(u.to_string()).execute(&db).await.unwrap();
        Some((db, u))
    }

    #[test]
    fn patch_null_vs_missing() {
        let p: CardPatch = serde_json::from_str(r#"{"due_at": null}"#).unwrap();
        assert_eq!(p.due_at, Some(None));
        let p: CardPatch = serde_json::from_str(r#"{"due_at": 5}"#).unwrap();
        assert_eq!(p.due_at, Some(Some(5)));
        let p: CardPatch = serde_json::from_str("{}").unwrap();
        assert_eq!(p.due_at, None);
    }
}
