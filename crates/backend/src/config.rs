//! Cấu hình chung cho client (không nhạy cảm).
use axum::Json;
use serde::Serialize;

#[derive(Serialize)]
pub struct ConfigOut {
    pub trash_days: i64,
    pub version: &'static str,
}

pub async fn get() -> Json<ConfigOut> {
    Json(ConfigOut { trash_days: crate::ops::trash_days(), version: env!("CARGO_PKG_VERSION") })
}
