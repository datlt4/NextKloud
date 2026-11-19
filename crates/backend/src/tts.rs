//! TTS proxy: frontend gọi cùng origin (/api/tts/*), backend forward sang service tts.
// Tránh CORS + mở port 8016 ra ngoài (cần cho domain public https).
use axum::{
    Json,
    body::Body,
    extract::{Query, State},
    http::header,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::collections::HashMap;

use crate::state::{AppError, AppState};

async fn passthrough(res: reqwest::Response, content_type: &str) -> Result<Response, AppError> {
    let status = res.status();
    let bytes = res.bytes().await.map_err(|e| AppError::Internal(format!("tts: {e}")))?;
    Ok(Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(bytes))
        .unwrap())
}

pub async fn voices(State(s): State<AppState>) -> Result<Response, AppError> {
    let res = s
        .http
        .get(format!("{}/voices", s.tts_url))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("tts unreachable: {e}")))?;
    passthrough(res, "application/json").await
}

pub async fn detect(
    State(s): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Response, AppError> {
    let res = s
        .http
        .get(format!("{}/detect", s.tts_url))
        .query(&q)
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("tts unreachable: {e}")))?;
    passthrough(res, "application/json").await
}

#[derive(Deserialize)]
pub struct SynthBody {
    pub text: String,
    pub voice: Option<String>,
    pub rate: Option<String>,
}

pub async fn synthesize(State(s): State<AppState>, Json(b): Json<SynthBody>) -> Result<Response, AppError> {
    if b.text.len() > 20000 {
        return Err(AppError::BadRequest("text too long".into()));
    }
    let res = s
        .http
        .post(format!("{}/synthesize", s.tts_url))
        .json(&serde_json::json!({
            "text": b.text,
            "voice": b.voice.unwrap_or_default(),
            "rate": b.rate.unwrap_or_default(),
        }))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("tts unreachable: {e}")))?;
    passthrough(res, "audio/mpeg").await
}
