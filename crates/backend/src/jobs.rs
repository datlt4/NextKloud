//! Job chạy nền cho thao tác lâu (chuyển/chép thư mục lớn): giữ trong RAM, chỉ chủ job xem được.
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use axum::{Json, extract::Path};
use uuid::Uuid;

use crate::state::{AppError, AuthUser};

/// Từ bao nhiêu file thì chạy nền thay vì chờ trong request.
pub const MIN_FILES: i64 = 500;
/// Xong rồi giữ kết quả bao lâu (giây).
const KEEP_SECS: i64 = 600;

#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
}

impl Progress {
    pub fn tick(&self) {
        self.done.fetch_add(1, Ordering::Relaxed);
    }
    pub fn set_total(&self, n: u64) {
        self.total.store(n, Ordering::Relaxed);
    }
}

struct Job {
    user: Uuid,
    prog: Arc<Progress>,
    /// (lúc xong, kết quả); None = đang chạy
    end: Mutex<Option<(i64, Result<u64, String>)>>,
}

static JOBS: LazyLock<Mutex<HashMap<Uuid, Arc<Job>>>> = LazyLock::new(Default::default);

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn prune(m: &mut HashMap<Uuid, Arc<Job>>) {
    let t = now();
    m.retain(|_, j| j.end.lock().unwrap().as_ref().is_none_or(|e| t - e.0 < KEEP_SECS));
}

/// Thông báo lỗi cho người dùng (giống phần `error` của response).
fn msg(e: AppError) -> String {
    match e {
        AppError::Auth(m)
        | AppError::Forbidden(m)
        | AppError::NotFound(m)
        | AppError::Conflict(m)
        | AppError::BadRequest(m)
        | AppError::TooMany(m)
        | AppError::Internal(m) => m,
        AppError::Store(e) => e.to_string(),
        AppError::Db(e) => {
            tracing::error!("job db: {e}");
            "database error".into()
        }
    }
}

/// Chạy `f` nền, trả id ngay.
pub fn spawn<F, Fut>(user: Uuid, f: F) -> Uuid
where
    F: FnOnce(Arc<Progress>) -> Fut,
    Fut: Future<Output = Result<u64, AppError>> + Send + 'static,
{
    let id = Uuid::new_v4();
    let job = Arc::new(Job { user, prog: Arc::default(), end: Mutex::new(None) });
    {
        let mut m = JOBS.lock().unwrap();
        prune(&mut m);
        m.insert(id, job.clone());
    }
    let h = tokio::spawn(f(job.prog.clone()));
    tokio::spawn(async move {
        // task panic cũng phải thành "error", không treo "running" mãi
        let r = match h.await {
            Ok(r) => r.map_err(msg),
            Err(e) => Err(e.to_string()),
        };
        *job.end.lock().unwrap() = Some((now(), r));
    });
    id
}

/// GET /api/jobs/:id -> {state: running|done|error, done, total, files?, error?}
pub async fn status(user: AuthUser, Path(id): Path<Uuid>) -> Result<Json<serde_json::Value>, AppError> {
    let job = {
        let mut m = JOBS.lock().unwrap();
        prune(&mut m);
        m.get(&id).filter(|j| j.user == user.id).cloned()
    }
    .ok_or_else(|| AppError::NotFound("job not found".into()))?;
    let (done, total) = (job.prog.done.load(Ordering::Relaxed), job.prog.total.load(Ordering::Relaxed));
    let end = job.end.lock().unwrap();
    Ok(Json(match &*end {
        None => serde_json::json!({ "state": "running", "done": done, "total": total }),
        Some((_, Ok(n))) => serde_json::json!({ "state": "done", "done": done, "total": total, "files": n }),
        Some((_, Err(e))) => serde_json::json!({ "state": "error", "done": done, "total": total, "error": e }),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn get(u: Uuid, id: Uuid) -> Result<serde_json::Value, AppError> {
        status(AuthUser { id: u, admin: false }, Path(id)).await.map(|j| j.0)
    }

    #[tokio::test]
    async fn job_lifecycle_and_owner_only() {
        let (u, other) = (Uuid::new_v4(), Uuid::new_v4());
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let id = spawn(u, |p| async move {
            p.set_total(3);
            p.tick();
            rx.await.ok();
            p.tick();
            p.tick();
            Ok(3)
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let r = get(u, id).await.unwrap();
        assert_eq!((r["state"].as_str(), r["done"].as_u64(), r["total"].as_u64()), (Some("running"), Some(1), Some(3)));
        // user khác không xem được
        assert!(matches!(get(other, id).await, Err(AppError::NotFound(_))));
        tx.send(()).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let r = get(u, id).await.unwrap();
        assert_eq!((r["state"].as_str(), r["done"].as_u64(), r["files"].as_u64()), (Some("done"), Some(3), Some(3)));

        let id = spawn(u, |_| async { Err(AppError::Conflict("hỏng".into())) });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let r = get(u, id).await.unwrap();
        assert_eq!((r["state"].as_str(), r["error"].as_str()), (Some("error"), Some("hỏng")));

        // quá hạn giữ thì bị dọn
        JOBS.lock().unwrap().get(&id).unwrap().end.lock().unwrap().as_mut().unwrap().0 -= KEEP_SECS + 1;
        assert!(matches!(get(u, id).await, Err(AppError::NotFound(_))));
    }
}
