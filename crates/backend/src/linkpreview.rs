//! Xem trước link (như Google Keep): tiêu đề, ảnh, tên trang lấy từ thẻ og:/twitter:/<title>.
//! GET /api/linkpreview?url=… — lưu DB 7 ngày (lỗi thì 1 ngày) để khỏi tải lại trang.
use std::collections::HashMap;

use axum::{
    Json,
    extract::{Query, State},
};
use serde::Serialize;

use crate::state::{AppError, AppState, AuthUser};

const MAX_HTML: usize = 1536 * 1024;

#[derive(Debug, Default, Serialize, PartialEq)]
pub struct Preview {
    pub url: String,
    pub title: Option<String>,
    pub image: Option<String>,
    pub site: Option<String>,
}

fn decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest[..rest.len().min(10)].find(';');
        let Some(end) = end else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "nbsp" => Some(' '),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Thuộc tính của 1 thẻ: key="v" | key='v' | key=v (key viết thường).
fn attrs(tag: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    let b = tag.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && !(b[i].is_ascii_alphabetic()) {
            i += 1;
        }
        let ks = i;
        while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'-' || b[i] == b':' || b[i] == b'_') {
            i += 1;
        }
        let key = tag[ks..i].to_ascii_lowercase();
        while i < b.len() && b[i] == b' ' {
            i += 1;
        }
        if i >= b.len() || b[i] != b'=' {
            continue;
        }
        i += 1;
        while i < b.len() && b[i] == b' ' {
            i += 1;
        }
        let val = if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
            let q = b[i];
            let vs = i + 1;
            let ve = tag[vs..].find(q as char).map_or(tag.len(), |p| vs + p);
            i = ve + 1;
            &tag[vs..ve.min(tag.len())]
        } else {
            let vs = i;
            while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                i += 1;
            }
            &tag[vs..i]
        };
        m.insert(key, decode(val));
    }
    m
}

/// Lấy tiêu đề/ảnh/tên trang từ HTML. `base` để đổi link ảnh tương đối thành tuyệt đối.
pub fn parse(html: &str, base: &reqwest::Url) -> Preview {
    let lower = html.to_ascii_lowercase();
    let mut meta: HashMap<String, String> = HashMap::new();
    let mut from = 0;
    while let Some(p) = lower[from..].find("<meta") {
        let s = from + p;
        let e = lower[s..].find('>').map_or(lower.len(), |x| s + x);
        let a = attrs(&html[s + 5..e]);
        if let (Some(k), Some(v)) = (a.get("property").or_else(|| a.get("name")).or_else(|| a.get("itemprop")), a.get("content")) {
            meta.entry(k.to_ascii_lowercase()).or_insert_with(|| v.clone());
        }
        from = e;
    }
    let title_tag = lower.find("<title").and_then(|s| {
        let a = lower[s..].find('>')? + s + 1;
        let b = lower[a..].find("</title")? + a;
        Some(decode(&html[a..b]))
    });
    let pick = |ks: &[&str]| ks.iter().find_map(|k| meta.get(*k).filter(|v| !v.is_empty()).cloned());
    let image = pick(&["og:image", "og:image:url", "og:image:secure_url", "twitter:image", "twitter:image:src", "image"])
        .and_then(|i| base.join(&i).ok())
        .filter(|u| matches!(u.scheme(), "http" | "https"))
        .map(|u| u.to_string());
    Preview {
        url: base.to_string(),
        title: pick(&["og:title", "twitter:title"]).or(title_tag).filter(|t| !t.is_empty()),
        image,
        site: pick(&["og:site_name"]).or_else(|| base.host_str().map(|h| h.trim_start_matches("www.").to_string())),
    }
}

async fn fetch(s: &AppState, url: &reqwest::Url) -> Result<Preview, String> {
    let mut r = s
        .http
        .get(url.clone())
        .timeout(std::time::Duration::from_secs(8))
        .header("User-Agent", "Mozilla/5.0 (compatible; NextKloud link preview)")
        .header("Accept", "text/html,application/xhtml+xml")
        .header("Accept-Language", "vi,en;q=0.8")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let final_url = r.url().clone();
    let html = r.headers().get("content-type").and_then(|v| v.to_str().ok()).is_none_or(|t| t.contains("html"));
    if !html {
        // file (ảnh, pdf...): chỉ có tên file
        let name = final_url.path_segments().and_then(|mut p| p.next_back().map(str::to_string)).filter(|n| !n.is_empty());
        return Ok(Preview { url: url.to_string(), title: name, image: None, site: final_url.host_str().map(str::to_string) });
    }
    let mut body = Vec::new();
    while let Some(c) = r.chunk().await.map_err(|e| e.to_string())? {
        body.extend_from_slice(&c);
        if body.len() > MAX_HTML || String::from_utf8_lossy(&body).to_ascii_lowercase().contains("</head>") {
            break;
        }
    }
    let mut p = parse(&String::from_utf8_lossy(&body), &final_url);
    p.url = url.to_string();
    Ok(p)
}

pub async fn get(State(s): State<AppState>, _user: AuthUser, Query(q): Query<HashMap<String, String>>) -> Result<Json<Preview>, AppError> {
    let raw = q.get("url").map(|u| u.trim()).unwrap_or("");
    let url = reqwest::Url::parse(raw).map_err(|_| AppError::BadRequest("url không hợp lệ".into()))?;
    if !matches!(url.scheme(), "http" | "https") || raw.len() > 2000 {
        return Err(AppError::BadRequest("chỉ hỗ trợ link http/https".into()));
    }
    let now = chrono::Utc::now().timestamp();
    let cached: Option<(Option<String>, Option<String>, Option<String>, i64)> =
        sqlx::query_as("SELECT title, image, site, fetched_at FROM link_previews WHERE url = $1").bind(raw).fetch_optional(&s.db).await?;
    if let Some((title, image, site, at)) = cached {
        let ttl = if title.is_some() { 7 * 86400 } else { 86400 };
        if now - at < ttl {
            return Ok(Json(Preview { url: raw.to_string(), title, image, site }));
        }
    }
    let p = fetch(&s, &url).await.unwrap_or_else(|_| Preview { url: raw.to_string(), site: url.host_str().map(str::to_string), ..Default::default() });
    sqlx::query(
        "INSERT INTO link_previews (url, title, image, site, fetched_at) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (url) DO UPDATE SET title = EXCLUDED.title, image = EXCLUDED.image, site = EXCLUDED.site, fetched_at = EXCLUDED.fetched_at",
    )
    .bind(raw)
    .bind(&p.title)
    .bind(&p.image)
    .bind(&p.site)
    .bind(now)
    .execute(&s.db)
    .await?;
    Ok(Json(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_og_and_fallbacks() {
        let base = reqwest::Url::parse("https://www.youtube.com/watch?v=x").unwrap();
        let html = r#"<html><head><title>Fallback &amp; title</title>
            <meta property="og:site_name" content="YouTube">
            <meta property='og:title' content='Javascript c&#243; th&#7875; l&#224;m &quot;g&#236;&quot;'>
            <meta content="/img/a.jpg" property="og:image" /></head>"#;
        let p = parse(html, &base);
        assert_eq!(p.title.as_deref(), Some("Javascript có thể làm \"gì\""));
        assert_eq!(p.image.as_deref(), Some("https://www.youtube.com/img/a.jpg"));
        assert_eq!(p.site.as_deref(), Some("YouTube"));
        let p = parse("<title>Chỉ có title</title>", &reqwest::Url::parse("http://24.8.0.2:8000/a").unwrap());
        assert_eq!((p.title.as_deref(), p.image, p.site.as_deref()), (Some("Chỉ có title"), None, Some("24.8.0.2")));
    }
}
