//! Server-Sent Events: nhắc task đến hạn + báo được chia sẻ file.
//! EventSource của trình duyệt không gửi được header Authorization nên chấp nhận token qua ?token=.
use std::{collections::HashMap, convert::Infallible, sync::OnceLock};

use axum::{
    async_trait,
    extract::{FromRequestParts, Query, State},
    http::{header, request::Parts},
    response::sse::{Event, KeepAlive, Sse},
};
use futures::stream::{Stream, unfold};
use serde::Serialize;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::{
    auth::verify_jwt,
    state::{AppError, AppState, AuthUser},
};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Ev {
    TaskDue { card: Uuid, board: Uuid, title: String, due_at: i64 },
    TaskOverdue { card: Uuid, board: Uuid, title: String, due_at: i64 },
    SharedWithYou { path: String, from: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct Push {
    pub user: Uuid,
    #[serde(flatten)]
    pub ev: Ev,
}

static BUS: OnceLock<broadcast::Sender<Push>> = OnceLock::new();

pub fn bus() -> &'static broadcast::Sender<Push> {
    BUS.get_or_init(|| broadcast::channel(256).0)
}

/// Gửi sự kiện cho 1 user (không ai nghe thì rơi mất, không lỗi).
pub fn emit(user: Uuid, ev: Ev) {
    let _ = bus().send(Push { user, ev });
}

/// Auth cho SSE: ưu tiên header Bearer, nếu không có thì ?token=.
pub struct EventUser(pub AuthUser);

#[async_trait]
impl FromRequestParts<AppState> for EventUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        if let Some(hdr) = parts.headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
            if let Some(tok) = hdr.strip_prefix("Bearer ") {
                let c = verify_jwt(tok, &state.dec).map_err(AppError::Auth)?;
                return Ok(Self(AuthUser { id: c.sub, admin: c.adm }));
            }
        }
        let q = Query::<HashMap<String, String>>::from_request_parts(parts, state)
            .await
            .map_err(|_| AppError::Auth("missing token".into()))?;
        let tok = q.get("token").ok_or_else(|| AppError::Auth("missing token".into()))?;
        let c = verify_jwt(tok, &state.dec).map_err(AppError::Auth)?;
        Ok(Self(AuthUser { id: c.sub, admin: c.adm }))
    }
}

pub async fn stream(State(_s): State<AppState>, user: EventUser) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let uid = user.0.id;
    let rx = bus().subscribe();
    let st = unfold(rx, move |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(p) if p.user == uid => {
                    let ev = Event::default().json_data(&p).unwrap_or_else(|_| Event::default().data("{\"type\":\"error\"}"));
                    return Some((Ok(ev), rx));
                }
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Sse::new(st).keep_alive(KeepAlive::default())
}
