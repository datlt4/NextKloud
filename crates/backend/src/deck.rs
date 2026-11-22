//! Nhập Nextcloud Deck (JSON do scripts/deck_export.py xuất) vào Kanban.
//! Xem trước: đề xuất gom card con vào "task lớn" (cột Bigtask) theo tiền tố tên; người dùng sửa rồi mới ghi.
use std::collections::{HashMap, HashSet};

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::state::{AppError, AppState, AuthUser};

#[derive(Debug, Deserialize)]
pub struct Export {
    /// Chủ dữ liệu Deck (username Nextcloud = username NextKloud) được tự nhập; admin nhập được mọi file.
    #[serde(default)]
    pub owner: Option<String>,
    pub boards: Vec<DBoard>,
}

#[derive(Debug, Deserialize)]
pub struct DBoard {
    pub id: i64,
    pub title: String,
    pub archived: bool,
    pub labels: Vec<crate::kanban::Label>,
    pub stacks: Vec<DStack>,
    pub cards: Vec<DCard>,
}

#[derive(Debug, Deserialize)]
pub struct DStack {
    pub id: i64,
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct DCard {
    pub id: i64,
    pub title: String,
    pub description: String,
    pub stack_id: i64,
    pub created_at: i64,
    pub last_modified: i64,
    pub due_at: Option<i64>,
    pub done_at: Option<i64>,
    pub labels: Vec<String>,
    pub color: Option<String>,
    /// File đính kèm (đường dẫn trong file của user, vd "Deck/a.pdf").
    #[serde(default)]
    pub attachments: Vec<String>,
}

fn path() -> String {
    std::env::var("DECK_IMPORT").unwrap_or_else(|_| "/data/import/deck.json".into())
}

fn load() -> Result<Export, AppError> {
    let raw = std::fs::read(path()).map_err(|_| AppError::NotFound("chưa có file xuất Deck trên server (scripts/deck_export.py)".into()))?;
    serde_json::from_slice(&raw).map_err(|e| AppError::BadRequest(format!("file Deck hỏng: {e}")))
}

/// File xuất Deck, nếu user được nhập: admin, hoặc đúng chủ dữ liệu.
async fn load_for(s: &AppState, user: &AuthUser) -> Result<Export, AppError> {
    let ex = load()?;
    if !user.admin {
        let me = crate::state::load_user(&s.db, user.id).await?;
        if ex.owner.as_deref() != Some(me.username.as_str()) {
            return Err(AppError::Forbidden("dữ liệu Deck này không phải của bạn".into()));
        }
    }
    Ok(ex)
}

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn is(s: &DStack, name: &str) -> bool {
    norm(&s.title) == name
}

/// Task cha đề xuất: task lớn có tên dài nhất là tiền tố của tên card (cắt ở ranh giới từ).
pub fn propose(b: &DBoard) -> HashMap<i64, i64> {
    let big: HashSet<i64> = b.stacks.iter().filter(|s| is(s, "bigtask")).map(|s| s.id).collect();
    let parents: Vec<(String, i64)> = b.cards.iter().filter(|c| big.contains(&c.stack_id)).map(|c| (norm(&c.title), c.id)).collect();
    let mut out = HashMap::new();
    for c in b.cards.iter().filter(|c| !big.contains(&c.stack_id)) {
        let t = norm(&c.title);
        let best = parents
            .iter()
            .filter(|(p, _)| !p.is_empty() && t.len() > p.len() && t.starts_with(p.as_str()) && !t[p.len()..].starts_with(|ch: char| ch.is_alphanumeric()))
            .max_by_key(|(p, _)| p.len());
        if let Some((_, id)) = best {
            out.insert(c.id, *id);
        }
    }
    // Nhóm "Tên - phần" (>= 2 card) chưa có cha: cha là card tên đúng "Tên", hoặc card duy nhất "Tên ..." (vd "Hành Thư 8000").
    let mut groups: HashMap<String, Vec<i64>> = HashMap::new();
    for c in b.cards.iter().filter(|c| !out.contains_key(&c.id) && !big.contains(&c.stack_id)) {
        if let Some((pre, _)) = norm(&c.title).split_once(" - ") {
            groups.entry(pre.trim().to_string()).or_default().push(c.id);
        }
    }
    for (pre, kids) in groups.into_iter().filter(|(_, v)| v.len() >= 2) {
        let exact: Vec<i64> = b.cards.iter().filter(|c| norm(&c.title) == pre).map(|c| c.id).collect();
        let loose: Vec<i64> = b.cards.iter().filter(|c| { let t = norm(&c.title); t.starts_with(&format!("{pre} ")) && !t.contains(" - ") }).map(|c| c.id).collect();
        let parent = if exact.len() == 1 { exact[0] } else if exact.is_empty() && loose.len() == 1 { loose[0] } else { continue };
        if out.contains_key(&parent) {
            continue; // chỉ 1 cấp
        }
        for k in kids {
            out.insert(k, parent);
        }
    }
    out
}

#[derive(Serialize)]
pub struct PStack {
    pub id: i64,
    pub title: String,
    pub count: usize,
    /// Cột "Subtask" của Deck: card được gộp vào cột này (theo tên), không tạo cột riêng.
    pub merge_into: Option<String>,
    pub is_done: bool,
}

#[derive(Serialize)]
pub struct PCard {
    pub id: i64,
    pub title: String,
    pub stack_id: i64,
    pub big: bool,
    pub done: bool,
    pub parent: Option<i64>,
}

#[derive(Serialize)]
pub struct PBoard {
    pub id: i64,
    pub title: String,
    pub archived: bool,
    pub imported: bool,
    pub stacks: Vec<PStack>,
    pub cards: Vec<PCard>,
}

fn merge_target(b: &DBoard, s: &DStack) -> Option<String> {
    is(s, "subtask").then(|| b.stacks.iter().find(|x| is(x, "to do") || is(x, "todo")).map(|x| x.title.clone())).flatten()
}

async fn imported(db: &PgPool, user: Uuid) -> Result<HashSet<String>, AppError> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT source FROM boards WHERE owner = $1 AND source IS NOT NULL").bind(user).fetch_all(db).await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

pub async fn preview(State(s): State<AppState>, user: AuthUser) -> Result<Json<Vec<PBoard>>, AppError> {
    let ex = load_for(&s, &user).await?;
    let done_ids = imported(&s.db, user.id).await?;
    Ok(Json(
        ex.boards
            .iter()
            .map(|b| {
                let prop = propose(b);
                let big: HashSet<i64> = b.stacks.iter().filter(|s| is(s, "bigtask")).map(|s| s.id).collect();
                // card làm cha (đề xuất) cũng là "task lớn" để chọn được trong danh sách
                let heads: HashSet<i64> = prop.values().copied().collect();
                PBoard {
                    id: b.id,
                    title: b.title.clone(),
                    archived: b.archived,
                    imported: done_ids.contains(&format!("deck:{}", b.id)),
                    stacks: b
                        .stacks
                        .iter()
                        .map(|st| PStack {
                            id: st.id,
                            title: st.title.clone(),
                            count: b.cards.iter().filter(|c| c.stack_id == st.id).count(),
                            merge_into: merge_target(b, st),
                            is_done: is(st, "done"),
                        })
                        .collect(),
                    cards: b
                        .cards
                        .iter()
                        .map(|c| PCard { id: c.id, title: c.title.clone(), stack_id: c.stack_id, big: big.contains(&c.stack_id) || heads.contains(&c.id), done: c.done_at.is_some(), parent: prop.get(&c.id).copied() })
                        .collect(),
                }
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct Apply {
    pub board_id: i64,
    /// card id -> task cha (null = không có cha). Card không có trong map: dùng đề xuất.
    #[serde(default)]
    pub parents: HashMap<i64, Option<i64>>,
    /// Cột Deck bỏ qua (cùng card của nó).
    #[serde(default)]
    pub skip_stacks: Vec<i64>,
}

#[derive(Serialize)]
pub struct Applied {
    pub board: Uuid,
    pub cards: usize,
    pub subtasks: usize,
}

pub async fn apply(State(s): State<AppState>, user: AuthUser, Json(a): Json<Apply>) -> Result<Json<Applied>, AppError> {
    let ex = load_for(&s, &user).await?;
    let b = ex.boards.iter().find(|b| b.id == a.board_id).ok_or_else(|| AppError::NotFound("board Deck không có".into()))?;
    Ok(Json(import(&s.db, user.id, b, &a).await?))
}

pub async fn import(db: &PgPool, user: Uuid, b: &DBoard, a: &Apply) -> Result<Applied, AppError> {
    let source = format!("deck:{}", b.id);
    if imported(db, user).await?.contains(&source) {
        return Err(AppError::Conflict("board này đã nhập rồi".into()));
    }
    let skip: HashSet<i64> = a.skip_stacks.iter().copied().collect();
    let mut parents = propose(b);
    for (k, v) in &a.parents {
        match v {
            Some(p) => parents.insert(*k, *p),
            None => parents.remove(k),
        };
    }
    let now = chrono::Utc::now().timestamp();
    let mut tx = db.begin().await?;
    let board = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO boards (id, owner, title, labels, archived, position, source, created_at)
         VALUES ($1, $2, $3, $4, $5, (SELECT COALESCE(MAX(position), 0) + 1 FROM boards WHERE owner = $2), $6, $7)",
    )
    .bind(board)
    .bind(user)
    .bind(&b.title)
    .bind(sqlx::types::Json(&b.labels))
    .bind(b.archived)
    .bind(&source)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    // Cột: giữ thứ tự Deck; cột Subtask gộp vào To do; cột Done là cột xong.
    let mut col_of: HashMap<i64, (Uuid, bool)> = HashMap::new();
    let mut by_title: HashMap<String, (Uuid, bool)> = HashMap::new();
    for (i, st) in b.stacks.iter().enumerate().filter(|(_, st)| !skip.contains(&st.id) && merge_target(b, st).is_none()) {
        let id = Uuid::new_v4();
        let done = is(st, "done");
        sqlx::query("INSERT INTO board_columns (id, board_id, title, position, is_done) VALUES ($1, $2, $3, $4, $5)")
            .bind(id)
            .bind(board)
            .bind(&st.title)
            .bind(i as f64)
            .bind(done)
            .execute(&mut *tx)
            .await?;
        col_of.insert(st.id, (id, done));
        by_title.insert(st.title.clone(), (id, done));
    }
    for st in b.stacks.iter().filter(|st| !skip.contains(&st.id)) {
        if let Some(t) = merge_target(b, st) {
            if let Some(c) = by_title.get(&t) {
                col_of.insert(st.id, *c);
            }
        }
    }
    let cards: Vec<&DCard> = b.cards.iter().filter(|c| col_of.contains_key(&c.stack_id)).collect();
    let ids: HashMap<i64, Uuid> = cards.iter().map(|c| (c.id, Uuid::new_v4())).collect();
    // Cha phải được nhập và không là con của ai (1 cấp).
    let parent_of = |c: &DCard| parents.get(&c.id).filter(|p| ids.contains_key(p) && !parents.contains_key(p) && **p != c.id).map(|p| ids[p]);
    // Ghi task cha trước để khoá ngoại parent_id hợp lệ.
    let mut ordered = cards.clone();
    ordered.sort_by_key(|c| parent_of(c).is_some());
    let mut subtasks = 0;
    for (i, c) in ordered.iter().enumerate() {
        let (col, done) = col_of[&c.stack_id];
        let parent = parent_of(c);
        subtasks += parent.is_some() as usize;
        let done_at = done.then(|| c.done_at.unwrap_or(c.last_modified));
        sqlx::query(
            "INSERT INTO cards (id, board_id, column_id, parent_id, title, description, position, labels, color, due_at, done_at, created_at, updated_at, attachments)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
        )
        .bind(ids[&c.id])
        .bind(board)
        .bind(col)
        .bind(parent)
        .bind(&c.title)
        .bind(&c.description)
        .bind(i as f64)
        .bind(&c.labels)
        .bind(&c.color)
        .bind(c.due_at)
        .bind(done_at)
        .bind(c.created_at)
        .bind(c.last_modified)
        .bind(&c.attachments)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Applied { board, cards: cards.len(), subtasks })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: i64, title: &str, stack: i64) -> DCard {
        DCard { id, title: title.into(), description: String::new(), stack_id: stack, created_at: 1, last_modified: 2, due_at: None, done_at: None, labels: vec![], color: None, attachments: vec![] }
    }

    fn board() -> DBoard {
        DBoard {
            id: 3,
            title: "Personal".into(),
            archived: false,
            labels: vec![],
            stacks: vec![
                DStack { id: 1, title: "Bigtask".into() },
                DStack { id: 2, title: "Subtask".into() },
                DStack { id: 3, title: "To do".into() },
                DStack { id: 4, title: "Done".into() },
            ],
            cards: vec![
                card(10, "HSK 1", 1),
                card(11, "ESP32", 1),
                card(12, "HSK 1 - Lesson 4", 2),
                card(13, "ESP32 IDF - M16", 3),
                card(14, "HSK 12 khác", 2),
                card(15, "esp32", 4),
                card(16, "Viết blog", 3),
                card(20, "Khải thư 8000", 4),
                card(21, "Khải thư - p55", 4),
                card(22, "Khải thư - p56", 3),
                card(23, "CUDA Mastery", 3),
                card(24, "CUDA Mastery - Chapter 4", 3),
                card(25, "CUDA Mastery - Chapter 5", 4),
                card(26, "Lẻ - một mình", 3),
            ],
        }
    }

    #[test]
    fn proposes_by_prefix() {
        let p = propose(&board());
        assert_eq!(p.get(&12), Some(&10));
        assert_eq!(p.get(&13), Some(&11));
        assert_eq!(p.get(&14), None); // "HSK 12" không phải "HSK 1"
        assert_eq!(p.get(&15), None); // trùng tên, không phải con
        assert_eq!(p.get(&16), None);
        assert_eq!((p.get(&21), p.get(&22)), (Some(&20), Some(&20))); // cha "Khải thư 8000"
        assert_eq!((p.get(&24), p.get(&25)), (Some(&23), Some(&23))); // cha đúng tên "CUDA Mastery"
        assert_eq!(p.get(&26), None); // nhóm 1 card
    }

    #[tokio::test]
    async fn imports_board() {
        let Some((db, u)) = crate::kanban::tests::db_user().await else { return };
        let mut b = board();
        b.cards[5].done_at = Some(99);
        b.cards[6].attachments = vec!["Deck/a.pdf".into()];
        let r = import(&db, u, &b, &Apply { board_id: 3, parents: HashMap::from([(16, Some(10)), (13, None)]), skip_stacks: vec![] }).await.unwrap();
        assert_eq!((r.cards, r.subtasks), (14, 6)); // 12, 16 (sửa tay) -> HSK 1; 13 bỏ cha; 21,22 -> Khải thư 8000; 24,25 -> CUDA Mastery
        let att: Vec<String> = sqlx::query_scalar("SELECT attachments FROM cards WHERE board_id = $1 AND title = 'Viết blog'").bind(r.board).fetch_one(&db).await.unwrap();
        assert_eq!(att, vec!["Deck/a.pdf".to_string()]);
        let cols: Vec<(String, bool)> = sqlx::query_as("SELECT title, is_done FROM board_columns WHERE board_id = $1 ORDER BY position").bind(r.board).fetch_all(&db).await.unwrap();
        assert_eq!(cols.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["Bigtask", "To do", "Done"]);
        let done: (Option<i64>,) = sqlx::query_as("SELECT done_at FROM cards WHERE board_id = $1 AND title = 'esp32'").bind(r.board).fetch_one(&db).await.unwrap();
        assert_eq!(done.0, Some(99));
        let lesson: (String,) = sqlx::query_as("SELECT k.title FROM cards c JOIN board_columns k ON k.id = c.column_id WHERE c.board_id = $1 AND c.title = 'HSK 1 - Lesson 4'")
            .bind(r.board)
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(lesson.0, "To do"); // cột Subtask gộp vào To do
        assert!(matches!(import(&db, u, &b, &Apply { board_id: 3, parents: HashMap::new(), skip_stacks: vec![] }).await, Err(AppError::Conflict(_))));
    }
}
