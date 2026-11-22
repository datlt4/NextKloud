//! Proxy /oo/* -> OnlyOffice DocumentServer: trình soạn thảo chạy cùng origin (https) với NextKloud,
//! không cần mở cổng DocumentServer ra ngoài. Cấu hình: ONLYOFFICE_URL, ONLYOFFICE_HEADERS.
use std::sync::OnceLock;

use axum::{
    body::Body,
    extract::{
        Request,
        ws::{Message as AMsg, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::{Message as TMsg, client::IntoClientRequest};

pub const PREFIX: &str = "/oo";

/// Gốc DocumentServer phía server (vd http://192.168.0.186:8027). None = tắt proxy.
pub fn upstream() -> Option<String> {
    std::env::var("ONLYOFFICE_URL").ok().map(|v| v.trim().trim_end_matches('/').to_string()).filter(|v| !v.is_empty())
}

/// Header thêm vào mọi request lên DocumentServer: "Tên: giá trị; Tên2: giá trị2".
pub fn extra_headers() -> Vec<(HeaderName, HeaderValue)> {
    parse_headers(&std::env::var("ONLYOFFICE_HEADERS").unwrap_or_default())
}

fn parse_headers(s: &str) -> Vec<(HeaderName, HeaderValue)> {
    s.split([';', '\n'])
        .filter_map(|kv| {
            let (k, v) = kv.split_once(':')?;
            Some((HeaderName::from_bytes(k.trim().as_bytes()).ok()?, HeaderValue::from_str(v.trim()).ok()?))
        })
        .collect()
}

fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().expect("http client"))
}

const HOP: [&str; 9] = ["connection", "keep-alive", "proxy-connection", "transfer-encoding", "upgrade", "te", "trailer", "host", "content-length"];

/// DocumentServer dựng URL tuyệt đối theo X-Forwarded-Host -> phải là host của NextKloud + /oo.
fn forwarded(h: &HeaderMap) -> [(HeaderName, HeaderValue); 2] {
    let get = |k: &str| h.get(k).and_then(|v| v.to_str().ok()).map(|v| v.split(',').next().unwrap_or("").trim().to_string());
    let host = get("x-forwarded-host").or_else(|| get("host")).unwrap_or_default();
    let proto = get("x-forwarded-proto").unwrap_or_else(|| "http".into());
    [
        (HeaderName::from_static("x-forwarded-host"), HeaderValue::from_str(&format!("{host}{PREFIX}")).unwrap_or(HeaderValue::from_static(""))),
        (HeaderName::from_static("x-forwarded-proto"), HeaderValue::from_str(&proto).unwrap_or(HeaderValue::from_static("http"))),
    ]
}

/// URL DocumentServer trả về (qua /oo của NextKloud) -> URL nội bộ để backend tải trực tiếp.
pub fn internal_url(url: &str) -> String {
    match (upstream(), url.find(&format!("{PREFIX}/"))) {
        (Some(up), Some(i)) => format!("{up}{}", &url[i + PREFIX.len()..]),
        _ => url.to_string(),
    }
}

pub async fn proxy(ws: Option<WebSocketUpgrade>, req: Request) -> Response {
    let Some(up) = upstream() else {
        return (StatusCode::NOT_FOUND, "chưa cấu hình ONLYOFFICE_URL").into_response();
    };
    let pq = req.uri().path_and_query().map(|p| p.as_str()).unwrap_or("/");
    let rest = pq.strip_prefix(PREFIX).unwrap_or(pq);
    let target = format!("{up}{}{rest}", if rest.starts_with('/') { "" } else { "/" });
    let headers = req.headers().clone();
    if let Some(ws) = ws {
        return ws_proxy(ws, target, &headers).await;
    }
    let method = req.method().clone();
    let body = match axum::body::to_bytes(req.into_body(), 200 << 20).await {
        Ok(b) => b,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    let mut out = HeaderMap::new();
    for (k, v) in &headers {
        if !HOP.contains(&k.as_str()) && k != header::COOKIE && k != header::AUTHORIZATION {
            out.append(k.clone(), v.clone());
        }
    }
    for (k, v) in forwarded(&headers).into_iter().chain(extra_headers()) {
        out.insert(k, v);
    }
    let resp = match client().request(method, &target).headers(out).body(body).send().await {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("DocumentServer: {e}")).into_response(),
    };
    let mut b = Response::builder().status(resp.status());
    for (k, v) in resp.headers() {
        if HOP.contains(&k.as_str()) {
            continue;
        }
        if k == header::LOCATION {
            if let Some(rest) = v.to_str().ok().and_then(|l| l.strip_prefix(&up)) {
                b = b.header(k, format!("{PREFIX}{rest}"));
                continue;
            }
        }
        b = b.header(k, v);
    }
    b.body(Body::from_stream(resp.bytes_stream())).unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
}

async fn ws_proxy(ws: WebSocketUpgrade, target: String, h: &HeaderMap) -> Response {
    let url = target.replacen("http", "ws", 1); // http->ws, https->wss
    let Ok(mut r) = url.into_client_request() else { return StatusCode::BAD_GATEWAY.into_response() };
    for (k, v) in forwarded(h).into_iter().chain(extra_headers()) {
        r.headers_mut().insert(k, v);
    }
    let up = match tokio_tungstenite::connect_async(r).await {
        Ok((s, _)) => s,
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("DocumentServer ws: {e}")).into_response(),
    };
    ws.on_upgrade(move |client| async move {
        let (mut ct, mut cr) = client.split();
        let (mut ut, mut ur) = up.split();
        let to_up = async {
            while let Some(Ok(m)) = cr.next().await {
                let m = match m {
                    AMsg::Text(t) => TMsg::Text(t),
                    AMsg::Binary(b) => TMsg::Binary(b),
                    AMsg::Ping(p) => TMsg::Ping(p),
                    AMsg::Pong(p) => TMsg::Pong(p),
                    AMsg::Close(_) => break,
                };
                if ut.send(m).await.is_err() {
                    break;
                }
            }
        };
        let to_client = async {
            while let Some(Ok(m)) = ur.next().await {
                let m = match m {
                    TMsg::Text(t) => AMsg::Text(t),
                    TMsg::Binary(b) => AMsg::Binary(b),
                    TMsg::Ping(p) => AMsg::Ping(p),
                    TMsg::Pong(p) => AMsg::Pong(p),
                    TMsg::Close(_) => break,
                    TMsg::Frame(_) => continue,
                };
                if ct.send(m).await.is_err() {
                    break;
                }
            }
        };
        tokio::select! { _ = to_up => {}, _ = to_client => {} }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_and_urls() {
        let h = parse_headers("P-Access-Token-Id: a1; P-Access-Token: b2;bad");
        assert_eq!(h.len(), 2);
        assert_eq!((h[1].0.as_str(), h[1].1.to_str().unwrap()), ("p-access-token", "b2"));
        std::env::set_var("ONLYOFFICE_URL", "http://ds:8027/");
        assert_eq!(internal_url("https://nk.example/oo/cache/files/x?md5=1"), "http://ds:8027/cache/files/x?md5=1");
        assert_eq!(internal_url("http://other/cache/x"), "http://other/cache/x");
    }
}
