//! Task tracker (lọc card có hạn từ mọi board) + Habit tracker (thói quen + streak).
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashSet;
use uuid::Uuid;

use crate::state::{AppError, AppState, AuthUser};

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Nửa đêm hôm nay theo giờ địa phương của server (mini PC để giờ VN).
fn day_bounds() -> (i64, i64, i64) {
    let today = chrono::Local::now().date_naive();
    let t0 = today.and_hms_opt(0, 0, 0).unwrap().and_local_timezone(chrono::Local).single().map(|d| d.timestamp()).unwrap_or(0);
    (t0, t0 + 86_400, t0 + 7 * 86_400)
}

// ---------- task ----------

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct TaskItem {
    pub id: Uuid,
    pub board_id: Uuid,
    pub board_title: String,
    pub column_id: Uuid,
    pub column_title: String,
    pub parent_id: Option<Uuid>,
    pub title: String,
    pub due_at: Option<i64>,
    pub done_at: Option<i64>,
    pub labels: Vec<String>,
}

#[derive(Deserialize)]
pub struct TaskQuery {
    /// overdue | today | week | done | all (mặc định overdue)
    #[serde(default)]
    pub view: String,
}

pub async fn tasks(State(s): State<AppState>, user: AuthUser, Query(q): Query<TaskQuery>) -> Result<Json<Vec<TaskItem>>, AppError> {
    let (t0, t1, w1) = day_bounds();
    let (cond, lim): (&str, i64) = match q.view.as_str() {
        "today" => ("c.due_at >= $2 AND c.due_at < $3 AND c.done_at IS NULL", 200),
        "week" => ("c.due_at >= $2 AND c.due_at < $3 AND c.done_at IS NULL", 300),
        "all" => ("c.done_at IS NULL AND c.due_at IS NOT NULL", 300),
        "done" => ("c.done_at IS NOT NULL AND c.due_at IS NOT NULL", 100),
        _ => ("c.due_at < $2 AND c.done_at IS NULL", 300), // overdue
    };
    let (from, to) = match q.view.as_str() {
        "today" => (t0, t1),
        "week" => (t0, w1),
        _ => (t0, t0),
    };
    let order = if q.view == "done" { "c.done_at DESC" } else { "c.due_at ASC NULLS LAST" };
    let sql = format!(
        "SELECT c.id, c.board_id, b.title AS board_title, c.column_id, k.title AS column_title,
                c.parent_id, c.title, c.due_at, c.done_at, c.labels
         FROM cards c JOIN boards b ON b.id = c.board_id JOIN board_columns k ON k.id = c.column_id
         WHERE b.owner = $1 AND NOT c.archived AND NOT k.is_cancel AND {cond}
         ORDER BY {order} LIMIT {lim}"
    );
    let rows: Vec<TaskItem> = if q.view == "all" || q.view == "done" {
        sqlx::query_as(&sql).bind(user.id).fetch_all(&s.db).await?
    } else if q.view == "today" || q.view == "week" {
        sqlx::query_as(&sql).bind(user.id).bind(from).bind(to).fetch_all(&s.db).await?
    } else {
        // overdue + view lạ (rớt về overdue, khớp cond chỉ dùng $2)
        sqlx::query_as(&sql).bind(user.id).bind(from).fetch_all(&s.db).await?
    };
    Ok(Json(rows))
}

// ---------- habit ----------

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Habit {
    pub id: Uuid,
    pub title: String,
    pub schedule: String,
    pub archived: bool,
    pub created_at: i64,
}

#[derive(Debug, Serialize)]
pub struct HabitOut {
    pub id: Uuid,
    pub title: String,
    pub schedule: String,
    pub archived: bool,
    pub created_at: i64,
    pub streak: i64,
    pub total: i64,
    pub done_today: bool,
    /// các ngày đã làm trong ~13 tháng gần nhất (YYYY-MM-DD), để vẽ lưới tháng
    pub days: Vec<String>,
}

fn parse_day(s: &str) -> Result<NaiveDate, AppError> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| AppError::BadRequest("ngày phải dạng YYYY-MM-DD".into()))
}

/// Chuỗi ngày liên tục tính tới hôm nay (nếu hôm nay chưa làm thì tính từ hôm qua).
fn streak_of(set: &HashSet<NaiveDate>, today: NaiveDate) -> i64 {
    let mut s = 0;
    let mut d = today;
    if !set.contains(&d) {
        d = d.pred_opt().unwrap_or(d);
    }
    while set.contains(&d) {
        s += 1;
        d = match d.pred_opt() {
            Some(p) => p,
            None => break,
        };
    }
    s
}

async fn habit_out(db: &PgPool, h: Habit) -> Result<HabitOut, AppError> {
    let today = chrono::Local::now().date_naive();
    let rows: Vec<(String,)> = sqlx::query_as("SELECT day FROM habit_logs WHERE habit_id = $1 ORDER BY day DESC LIMIT 400")
        .bind(h.id)
        .fetch_all(db)
        .await?;
    let set: HashSet<NaiveDate> = rows.iter().filter_map(|(d,)| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()).collect();
    let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM habit_logs WHERE habit_id = $1").bind(h.id).fetch_one(db).await?;
    let cutoff = (today - chrono::Duration::days(400)).format("%Y-%m-%d").to_string();
    let mut days: Vec<String> = set
        .iter()
        .filter(|d| d.format("%Y-%m-%d").to_string() >= cutoff)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .collect();
    days.sort();
    Ok(HabitOut {
        id: h.id,
        title: h.title,
        schedule: h.schedule,
        archived: h.archived,
        created_at: h.created_at,
        streak: streak_of(&set, today),
        total: total.0,
        done_today: set.contains(&today),
        days,
    })
}

async fn own_habit(db: &PgPool, user: Uuid, id: Uuid) -> Result<(), AppError> {
    let ok: Option<(i32,)> = sqlx::query_as("SELECT 1 FROM habits WHERE id = $1 AND owner = $2").bind(id).bind(user).fetch_optional(db).await?;
    ok.map(|_| ()).ok_or_else(|| AppError::NotFound("habit not found".into()))
}

pub async fn list_habits(State(s): State<AppState>, user: AuthUser) -> Result<Json<Vec<HabitOut>>, AppError> {
    let rows: Vec<Habit> = sqlx::query_as("SELECT id, title, schedule, archived, created_at FROM habits WHERE owner = $1 ORDER BY archived, created_at")
        .bind(user.id)
        .fetch_all(&s.db)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for h in rows {
        out.push(habit_out(&s.db, h).await?);
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct NewHabit {
    pub title: String,
}

pub async fn create_habit(State(s): State<AppState>, user: AuthUser, Json(b): Json<NewHabit>) -> Result<(StatusCode, Json<HabitOut>), AppError> {
    let t = b.title.trim();
    if t.is_empty() || t.len() > 200 {
        return Err(AppError::BadRequest("tên thói quen trống hoặc quá dài".into()));
    }
    let h: Habit = sqlx::query_as(
        "INSERT INTO habits (id, owner, title, schedule, created_at) VALUES ($1, $2, $3, 'daily', $4)
         RETURNING id, title, schedule, archived, created_at",
    )
    .bind(Uuid::new_v4())
    .bind(user.id)
    .bind(t)
    .bind(now())
    .fetch_one(&s.db)
    .await?;
    Ok((StatusCode::CREATED, Json(habit_out(&s.db, h).await?)))
}

#[derive(Deserialize, Default)]
pub struct HabitPatch {
    pub title: Option<String>,
    pub archived: Option<bool>,
}

pub async fn update_habit(State(s): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<HabitPatch>) -> Result<Json<HabitOut>, AppError> {
    own_habit(&s.db, user.id, id).await?;
    if let Some(t) = b.title.as_deref() {
        let t = t.trim();
        if t.is_empty() || t.len() > 200 {
            return Err(AppError::BadRequest("tên thói quen trống hoặc quá dài".into()));
        }
        sqlx::query("UPDATE habits SET title = $2 WHERE id = $1").bind(id).bind(t).execute(&s.db).await?;
    }
    if let Some(a) = b.archived {
        sqlx::query("UPDATE habits SET archived = $2 WHERE id = $1").bind(id).bind(a).execute(&s.db).await?;
    }
    let h: Habit = sqlx::query_as("SELECT id, title, schedule, archived, created_at FROM habits WHERE id = $1").bind(id).fetch_one(&s.db).await?;
    Ok(Json(habit_out(&s.db, h).await?))
}

pub async fn delete_habit(State(s): State<AppState>, user: AuthUser, Path(id): Path<Uuid>) -> Result<StatusCode, AppError> {
    own_habit(&s.db, user.id, id).await?;
    sqlx::query("DELETE FROM habits WHERE id = $1").bind(id).execute(&s.db).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct HabitCheck {
    pub day: String,
    pub done: bool,
}

pub async fn check_habit(State(s): State<AppState>, user: AuthUser, Path(id): Path<Uuid>, Json(b): Json<HabitCheck>) -> Result<Json<HabitOut>, AppError> {
    own_habit(&s.db, user.id, id).await?;
    let d = parse_day(&b.day)?;
    let today = chrono::Local::now().date_naive();
    if d > today {
        return Err(AppError::BadRequest("không chấm ngày tương lai".into()));
    }
    if b.done {
        sqlx::query("INSERT INTO habit_logs (habit_id, day, created_at) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING")
            .bind(id)
            .bind(b.day)
            .bind(now())
            .execute(&s.db)
            .await?;
    } else {
        sqlx::query("DELETE FROM habit_logs WHERE habit_id = $1 AND day = $2").bind(id).bind(b.day).execute(&s.db).await?;
    }
    let h: Habit = sqlx::query_as("SELECT id, title, schedule, archived, created_at FROM habits WHERE id = $1").bind(id).fetch_one(&s.db).await?;
    Ok(Json(habit_out(&s.db, h).await?))
}

/// Quét card đến hạn trong 24h tới (kể cả đã quá hạn) mà chưa nhắc: bắn SSE + đánh dấu reminded_at.
pub async fn check_due_once(db: &PgPool) -> usize {
    let now = now();
    let rows: Vec<(Uuid, Uuid, String, i64, Uuid)> = sqlx::query_as(
        "SELECT c.id, c.board_id, c.title, c.due_at, b.owner FROM cards c JOIN boards b ON b.id = c.board_id JOIN board_columns k ON k.id = c.column_id
         WHERE c.done_at IS NULL AND NOT c.archived AND NOT k.is_cancel AND c.due_at IS NOT NULL AND c.due_at <= $1 AND c.reminded_at IS NULL
         LIMIT 200",
    )
    .bind(now + 86_400)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    let mut n = 0;
    for (id, board, title, due, owner) in rows {
        let ev = if due < now {
            crate::events::Ev::TaskOverdue { card: id, board, title, due_at: due }
        } else {
            crate::events::Ev::TaskDue { card: id, board, title, due_at: due }
        };
        crate::events::emit(owner, ev);
        if sqlx::query("UPDATE cards SET reminded_at = $2 WHERE id = $1 AND reminded_at IS NULL")
            .bind(id)
            .bind(now)
            .execute(db)
            .await
            .is_ok()
        {
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streak_counts_backwards() {
        let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let today = d("2026-11-23");
        let set: HashSet<NaiveDate> = ["2026-11-23", "2026-11-22", "2026-11-21", "2026-11-19"].iter().map(|s| d(s)).collect();
        assert_eq!(streak_of(&set, today), 3);
        // hôm nay chưa làm: tính từ hôm qua
        let today2 = d("2026-11-24");
        assert_eq!(streak_of(&set, today2), 3);
        // trống
        assert_eq!(streak_of(&HashSet::new(), today), 0);
        // 1 ngày duy nhất hôm nay
        let one: HashSet<NaiveDate> = [d("2026-11-23")].into_iter().collect();
        assert_eq!(streak_of(&one, today), 1);
    }

    #[test]
    fn day_rejects_bad_format() {
        assert!(parse_day("23-11-2026").is_err());
        assert!(parse_day("2026-11-23").is_ok());
    }

    #[tokio::test]
    async fn due_reminder_fires_once() {
        use crate::events::{Ev, bus};
        let Some((db, u)) = crate::kanban::tests::db_user().await else { return };
        let b: (Uuid,) = sqlx::query_as("INSERT INTO boards (id, owner, title, created_at) VALUES ($1, $2, 'B', 0) RETURNING id")
            .bind(Uuid::new_v4())
            .bind(u)
            .fetch_one(&db)
            .await
            .unwrap();
        let c: (Uuid,) = sqlx::query_as("INSERT INTO board_columns (id, board_id, title, position) VALUES ($1, $2, 'Todo', 0) RETURNING id")
            .bind(Uuid::new_v4())
            .bind(b.0)
            .fetch_one(&db)
            .await
            .unwrap();
        let cid = Uuid::new_v4();
        sqlx::query("INSERT INTO cards (id, board_id, column_id, title, position, due_at, created_at, updated_at) VALUES ($1,$2,$3,'x',0,$4,0,0)")
            .bind(cid)
            .bind(b.0)
            .bind(c.0)
            .bind(now() + 3600)
            .execute(&db)
            .await
            .unwrap();
        let mut rx = bus().subscribe();
        // card ở cột huỷ (Delete) thì không nhắc, không đánh dấu
        let cx: (Uuid,) = sqlx::query_as("INSERT INTO board_columns (id, board_id, title, position, is_cancel) VALUES ($1, $2, 'Delete', 99, true) RETURNING id")
            .bind(Uuid::new_v4())
            .bind(b.0)
            .fetch_one(&db)
            .await
            .unwrap();
        let del = Uuid::new_v4();
        sqlx::query("INSERT INTO cards (id, board_id, column_id, title, position, due_at, created_at, updated_at) VALUES ($1,$2,$3,'x',0,$4,0,0)")
            .bind(del)
            .bind(b.0)
            .bind(cx.0)
            .bind(now() + 3600)
            .execute(&db)
            .await
            .unwrap();
        // DB test dùng chung giữa các test chạy song song: chỉ xét event đúng card của mình
        let mine = |rx: &mut tokio::sync::broadcast::Receiver<crate::events::Push>| {
            let mut hit = false;
            while let Ok(p) = rx.try_recv() {
                if p.user == u {
                    if let Ev::TaskDue { card, .. } = p.ev {
                        if card == cid {
                            hit = true;
                        }
                    }
                }
            }
            hit
        };
        check_due_once(&db).await;
        assert!(mine(&mut rx), "thiếu event nhắc card của mình");
        let r: (Option<i64>,) = sqlx::query_as("SELECT reminded_at FROM cards WHERE id = $1").bind(cid).fetch_one(&db).await.unwrap();
        assert!(r.0.is_some(), "chưa đánh dấu reminded_at");
        // quét lại không nhắc lại card của mình
        check_due_once(&db).await;
        assert!(!mine(&mut rx), "bị nhắc trùng");
        // card cột huỷ không bị đánh dấu
        let r2: (Option<i64>,) = sqlx::query_as("SELECT reminded_at FROM cards WHERE id = $1").bind(del).fetch_one(&db).await.unwrap();
        assert!(r2.0.is_none(), "card cột huỷ không được nhắc");
    }

    #[tokio::test]
    async fn tasks_and_habits_roundtrip() {
        let Some((db, u)) = crate::kanban::tests::db_user().await else { return };
        // board + cột + card có hạn
        let b: (Uuid,) = sqlx::query_as("INSERT INTO boards (id, owner, title, created_at) VALUES ($1, $2, 'B', 0) RETURNING id")
            .bind(Uuid::new_v4())
            .bind(u)
            .fetch_one(&db)
            .await
            .unwrap();
        let c: (Uuid,) = sqlx::query_as("INSERT INTO board_columns (id, board_id, title, position) VALUES ($1, $2, 'Todo', 0) RETURNING id")
            .bind(Uuid::new_v4())
            .bind(b.0)
            .fetch_one(&db)
            .await
            .unwrap();
        let past = now() - 100_000;
        sqlx::query("INSERT INTO cards (id, board_id, column_id, title, position, due_at, created_at, updated_at) VALUES ($1,$2,$3,'x',0,$4,0,0)")
            .bind(Uuid::new_v4())
            .bind(b.0)
            .bind(c.0)
            .bind(past)
            .execute(&db)
            .await
            .unwrap();
        let (store, _) = crate::files::tests::tmp_store("tracker");
        let s = AppState {
            db: db.clone(),
            store,
            enc: jsonwebtoken::EncodingKey::from_secret(b"t"),
            dec: jsonwebtoken::DecodingKey::from_secret(b"t"),
            uploads: std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            http: reqwest::Client::new(),
            tts_url: String::new(),
            vaults: Default::default(),
        };
        let r = tasks(State(s.clone()), AuthUser { id: u, admin: false }, Query(TaskQuery { view: "overdue".into() })).await.unwrap();
        assert_eq!(r.0.len(), 1);
        // habit: chấm 2 ngày liên tiếp -> streak 2
        let h = create_habit(State(s.clone()), AuthUser { id: u, admin: false }, Json(NewHabit { title: "chạy".into() })).await.unwrap().1;
        let today = chrono::Local::now().date_naive();
        for d in [today, today.pred_opt().unwrap()] {
            check_habit(
                State(s.clone()),
                AuthUser { id: u, admin: false },
                Path(h.id),
                Json(HabitCheck { day: d.format("%Y-%m-%d").to_string(), done: true }),
            )
            .await
            .unwrap();
        }
        assert_eq!(list_habits(State(s), AuthUser { id: u, admin: false }).await.unwrap().0[0].streak, 2);
    }
}
