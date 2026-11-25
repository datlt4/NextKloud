//! Engine đồng bộ: cấu hình cặp thư mục, vòng quét 30 giây, thực thi kế hoạch từ `plan`.
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::{BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, State};

use super::plan::{self, Action, Base, Meta};

const EVERY: Duration = Duration::from_secs(30);
const CHUNK: u64 = 8 * 1024 * 1024;
const EVENT: &str = "nk-sync";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pair {
    pub id: String,
    pub local: String,
    pub remote: String,
    #[serde(default)]
    pub paused: bool,
    /// "<server>|<user id>": chỉ chạy khi đang đăng nhập đúng tài khoản này
    pub account: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Status {
    pub last_sync: Option<i64>,
    pub up: u64,
    pub down: u64,
    pub conflicts: u64,
    pub errors: u64,
    pub last_error: Option<String>,
    pub done: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct View {
    #[serde(flatten)]
    pub pair: Pair,
    #[serde(flatten)]
    pub status: Status,
    /// idle | syncing | error | paused | auth (cần đăng nhập) | other (tài khoản khác)
    pub state: &'static str,
}

#[derive(Clone)]
struct Auth {
    server: String,
    token: String,
    account: String,
    tz: i64,
}

#[derive(Default)]
struct Inner {
    pairs: Vec<Pair>,
    status: HashMap<String, Status>,
    auth: Option<Auth>,
    gen: u64,
    running: Option<String>,
}

pub struct Engine {
    inner: Mutex<Inner>,
    wake: tokio::sync::Notify,
    dir: PathBuf,
    host: String,
}

enum E {
    Auth,
    Stop,
    Msg(String),
}

impl From<std::io::Error> for E {
    fn from(e: std::io::Error) -> Self {
        E::Msg(e.to_string())
    }
}

impl From<reqwest::Error> for E {
    fn from(e: reqwest::Error) -> Self {
        E::Msg(e.to_string())
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn mtime_of(md: &fs::Metadata) -> i64 {
    md.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Meta hiện tại trên đĩa (không theo symlink); symlink/khác coi như thư mục để không bao giờ khớp file.
fn local_meta(p: &Path) -> Option<Meta> {
    let md = fs::symlink_metadata(p).ok()?;
    Some(if md.is_file() { Meta::file(md.len(), mtime_of(&md)) } else { Meta::dir() })
}

/// Percent-encode (giữ `/` nếu `slash`).
fn enc(s: &str, slash: bool) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) || (slash && b == b'/') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn host_name() -> String {
    let h = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| std::process::Command::new("hostname").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()))
        .unwrap_or_default();
    let h = h.trim().trim_end_matches(".local");
    let h: String = h.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(24).collect();
    if h.is_empty() { "desktop".into() } else { h }
}

fn write_json<T: Serialize>(p: &Path, v: &T) -> std::io::Result<()> {
    let tmp = p.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec(v)?)?;
    fs::rename(tmp, p)
}

/// Quét cây local. Trả về (cây, các path không đọc được/symlink: bỏ khỏi kế hoạch để không bị coi là đã xoá).
fn scan(root: &Path) -> std::io::Result<(BTreeMap<String, Meta>, Vec<String>)> {
    let mut out = BTreeMap::new();
    let mut skip = vec![];
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        let rd = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) if rel.is_empty() => return Err(e),
            Err(_) => {
                skip.push(rel);
                continue;
            }
        };
        for e in rd {
            let Ok(e) = e else { continue };
            let Ok(name) = e.file_name().into_string() else { continue };
            let r = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            if plan::ignored(&r) {
                continue;
            }
            match fs::symlink_metadata(e.path()) {
                Ok(md) if md.is_dir() => {
                    out.insert(r.clone(), Meta::dir());
                    stack.push((e.path(), r));
                }
                Ok(md) if md.is_file() => {
                    out.insert(r, Meta::file(md.len(), mtime_of(&md)));
                }
                _ => skip.push(r),
            }
        }
    }
    Ok((out, skip))
}

fn under(k: &str, p: &str) -> bool {
    k == p || (k.len() > p.len() && k.starts_with(p) && k.as_bytes()[p.len()] == b'/')
}

/// Bỏ `p` và mọi path con khỏi map.
fn remove_tree<T>(m: &mut BTreeMap<String, T>, p: &str) {
    m.remove(p);
    let pre = format!("{p}/");
    let keys: Vec<String> = m.range(pre.clone()..).take_while(|(k, _)| k.starts_with(&pre)).map(|(k, _)| k.clone()).collect();
    for k in keys {
        m.remove(&k);
    }
}

fn same_content(a: &Path, b: &Path) -> std::io::Result<bool> {
    let (mut fa, mut fb) = (BufReader::new(fs::File::open(a)?), BufReader::new(fs::File::open(b)?));
    let (mut ba, mut bb) = (vec![0u8; 64 * 1024], vec![0u8; 64 * 1024]);
    loop {
        let n = fa.read(&mut ba)?;
        if n == 0 {
            return Ok(fb.read(&mut bb[..1])? == 0);
        }
        fb.read_exact(&mut bb[..n]).map_err(|_| std::io::Error::other("short"))?;
        if ba[..n] != bb[..n] {
            return Ok(false);
        }
    }
}

impl Engine {
    fn new(dir: PathBuf) -> Self {
        let pairs: Vec<Pair> = fs::read(dir.join("pairs.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Engine { inner: Mutex::new(Inner { pairs, ..Default::default() }), wake: tokio::sync::Notify::new(), dir, host: host_name() }
    }
    fn pairs_file(&self) -> PathBuf {
        self.dir.join("pairs.json")
    }
    fn state_file(&self, id: &str) -> PathBuf {
        self.dir.join(format!("state-{id}.json"))
    }

    fn views(&self) -> Vec<View> {
        let i = self.inner.lock().unwrap();
        i.pairs
            .iter()
            .map(|p| {
                let st = i.status.get(&p.id).cloned().unwrap_or_default();
                let state = if p.paused {
                    "paused"
                } else if i.running.as_deref() == Some(p.id.as_str()) {
                    "syncing"
                } else if i.auth.is_none() {
                    "auth"
                } else if i.auth.as_ref().is_some_and(|a| a.account != p.account) {
                    "other"
                } else if st.last_error.is_some() {
                    "error"
                } else {
                    "idle"
                };
                View { pair: p.clone(), status: st, state }
            })
            .collect()
    }

    fn emit(&self, app: &AppHandle) {
        let _ = app.emit(EVENT, self.views());
    }

    fn save_pairs(&self, i: &Inner) -> Result<(), String> {
        write_json(&self.pairs_file(), &i.pairs).map_err(|e| e.to_string())
    }

    /// Cặp còn chạy tiếp được không (chưa tạm dừng/gỡ, phiên đăng nhập chưa đổi).
    fn alive(&self, id: &str, gen: u64) -> bool {
        let i = self.inner.lock().unwrap();
        i.gen == gen && i.pairs.iter().any(|p| p.id == id && !p.paused)
    }

    fn status_mut<R>(&self, id: &str, f: impl FnOnce(&mut Status) -> R) -> R {
        let mut i = self.inner.lock().unwrap();
        f(i.status.entry(id.to_string()).or_default())
    }
}

pub fn start(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let dir = app.path().app_data_dir()?.join("sync");
    fs::create_dir_all(&dir)?;
    let s = Arc::new(Engine::new(dir));
    app.manage(s.clone());
    tauri::async_runtime::spawn(run(app.clone(), s));
    Ok(())
}

async fn run(app: AppHandle, s: Arc<Engine>) {
    // reqwest dùng rustls không kèm provider (giống tauri-plugin-updater): cài ring
    let _ = rustls::crypto::ring::default_provider().install_default();
    let http = match reqwest::Client::builder()
        .user_agent(concat!("NextKloud-desktop-sync/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(120))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("sync: http client: {e}");
            return;
        }
    };
    loop {
        let (todo, auth, gen) = {
            let i = s.inner.lock().unwrap();
            match &i.auth {
                Some(a) => (i.pairs.iter().filter(|p| !p.paused && p.account == a.account).cloned().collect(), Some(a.clone()), i.gen),
                None => (vec![], None, i.gen),
            }
        };
        if let Some(auth) = auth {
            for p in todo {
                if !s.alive(&p.id, gen) {
                    continue;
                }
                s.inner.lock().unwrap().running = Some(p.id.clone());
                s.status_mut(&p.id, |st| {
                    st.errors = 0;
                    st.last_error = None;
                    st.done = 0;
                    st.total = 0;
                });
                s.emit(&app);
                let mut cx = Ctx::new(&s, Some(&app), &http, auth.clone(), gen, p.clone());
                let r = cx.round().await;
                let unauthorized = matches!(r, Err(E::Auth));
                s.status_mut(&p.id, |st| {
                    st.done = 0;
                    st.total = 0;
                    match r {
                        Ok(()) => st.last_sync = Some(now()),
                        Err(E::Auth) => st.last_error = Some("sync.e.auth".into()),
                        Err(E::Stop) => {}
                        Err(E::Msg(m)) => {
                            st.errors += 1;
                            st.last_error = Some(m);
                        }
                    }
                });
                {
                    let mut i = s.inner.lock().unwrap();
                    i.running = None;
                    // token hết hạn/bị thu hồi: dừng hẳn tới khi webview đăng nhập lại
                    if unauthorized && i.gen == gen {
                        i.auth = None;
                        i.gen += 1;
                    }
                }
                s.emit(&app);
                if unauthorized {
                    break;
                }
            }
        }
        let _ = tokio::time::timeout(EVERY, s.wake.notified()).await;
    }
}

struct Ctx<'a> {
    s: &'a Engine,
    app: Option<&'a AppHandle>,
    http: &'a reqwest::Client,
    auth: Auth,
    gen: u64,
    pair: Pair,
    root: PathBuf,
    base: BTreeMap<String, Base>,
    last_emit: Instant,
}

impl<'a> Ctx<'a> {
    fn new(s: &'a Engine, app: Option<&'a AppHandle>, http: &'a reqwest::Client, auth: Auth, gen: u64, pair: Pair) -> Self {
        let root = PathBuf::from(&pair.local);
        Ctx { s, app, http, auth, gen, pair, root, base: BTreeMap::new(), last_emit: Instant::now() }
    }

    fn url(&self, p: &str) -> String {
        format!("{}{p}", self.auth.server)
    }
    fn rpath(&self, rel: &str) -> String {
        if self.pair.remote.is_empty() { rel.to_string() } else { format!("{}/{rel}", self.pair.remote) }
    }
    fn lpath(&self, rel: &str) -> PathBuf {
        let mut p = self.root.clone();
        for c in rel.split('/') {
            p.push(c);
        }
        p
    }

    /// Gửi kèm token; 401 -> dừng đồng bộ. Trả về response cho mọi mã khác.
    async fn raw(&self, rb: reqwest::RequestBuilder) -> Result<reqwest::Response, E> {
        let r = rb.bearer_auth(&self.auth.token).send().await?;
        if r.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(E::Auth);
        }
        Ok(r)
    }
    async fn ok(&self, rb: reqwest::RequestBuilder) -> Result<reqwest::Response, E> {
        let r = self.raw(rb).await?;
        if r.status().is_success() {
            return Ok(r);
        }
        let code = r.status().as_u16();
        let body: String = r.text().await.unwrap_or_default().chars().take(200).collect();
        Err(E::Msg(format!("HTTP {code} {body}")))
    }

    fn load_base(&mut self) {
        self.base = fs::read(self.s.state_file(&self.pair.id)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    }
    fn save_base(&self) {
        // cặp đã bị gỡ giữa chừng: đừng ghi lại file state
        if self.s.inner.lock().unwrap().pairs.iter().any(|p| p.id == self.pair.id) {
            if let Err(e) = write_json(&self.s.state_file(&self.pair.id), &self.base) {
                eprintln!("sync: save state: {e}");
            }
        }
    }

    /// Cây server dưới thư mục của cặp. None = thư mục chưa có trên server.
    async fn list_remote(&self) -> Result<Option<BTreeMap<String, Meta>>, E> {
        #[derive(Deserialize)]
        struct Row {
            path: String,
            size: i64,
            mtime: i64,
            is_dir: bool,
        }
        #[derive(Deserialize)]
        struct Pg {
            entries: Vec<Row>,
            next: Option<String>,
        }
        let pre = if self.pair.remote.is_empty() { String::new() } else { format!("{}/", self.pair.remote) };
        let mut out = BTreeMap::new();
        let mut cursor = String::new();
        loop {
            let u = self.url(&format!("/api/sync/list?prefix={}&cursor={}&limit=5000", enc(&self.pair.remote, false), enc(&cursor, false)));
            let r = self.raw(self.http.get(u)).await?;
            if r.status() == reqwest::StatusCode::NOT_FOUND {
                return Ok(None);
            }
            if !r.status().is_success() {
                return Err(E::Msg(format!("list: HTTP {}", r.status().as_u16())));
            }
            let pg: Pg = r.json().await?;
            for e in pg.entries {
                let Some(rel) = e.path.strip_prefix(&pre) else { continue };
                if rel.is_empty() || plan::ignored(rel) {
                    continue;
                }
                let m = if e.is_dir { Meta::dir() } else { Meta::file(e.size.max(0) as u64, e.mtime) };
                out.insert(rel.to_string(), m);
            }
            match pg.next {
                Some(n) => cursor = n,
                None => return Ok(Some(out)),
            }
        }
    }

    async fn round(&mut self) -> Result<(), E> {
        if !self.root.is_dir() {
            return Err(E::Msg("sync.e.localMissing".into()));
        }
        self.load_base();
        let root = self.root.clone();
        let (local, skip) = tauri::async_runtime::spawn_blocking(move || scan(&root))
            .await
            .map_err(|e| E::Msg(e.to_string()))??;
        let remote = match self.list_remote().await? {
            Some(r) => r,
            None if self.base.is_empty() => {
                if !self.pair.remote.is_empty() {
                    let r = self.raw(self.http.post(self.url("/api/mkdir")).json(&json!({ "path": self.pair.remote }))).await?;
                    if !r.status().is_success() && r.status() != reqwest::StatusCode::CONFLICT {
                        return Err(E::Msg(format!("mkdir: HTTP {}", r.status().as_u16())));
                    }
                }
                BTreeMap::new()
            }
            // từng đồng bộ mà thư mục server biến mất (đổi tên/xoá): không xoá sạch bản máy
            None => return Err(E::Msg("sync.e.remoteMissing".into())),
        };
        // 1 bên trống trơn trong khi bên kia còn: nhiều khả năng ổ chưa mount/đổi tên, không xoá hàng loạt
        if !self.base.is_empty() && local.is_empty() && !remote.is_empty() {
            return Err(E::Msg("sync.e.localEmpty".into()));
        }
        if !self.base.is_empty() && remote.is_empty() && !local.is_empty() {
            return Err(E::Msg("sync.e.remoteEmpty".into()));
        }
        let (mut r_in, mut b_in) = (remote.clone(), self.base.clone());
        for p in &skip {
            remove_tree(&mut r_in, p);
            remove_tree(&mut b_in, p);
        }
        let todo = plan::plan(&local, &r_in, &b_in);
        // xoá quá nhiều trong 1 vòng (ổ con chưa mount, thư mục bị đổi tên...): dừng, không xoá hàng loạt
        let dels: usize = todo
            .iter()
            .filter(|(_, a)| matches!(a, Action::DeleteRemote | Action::TrashLocal))
            .map(|(k, _)| 1 + self.base.range(format!("{k}/")..).take_while(|(p, _)| p.starts_with(&format!("{k}/"))).count())
            .sum();
        if dels > 50 && dels * 10 > self.base.len() * 3 {
            return Err(E::Msg("sync.e.massDelete".into()));
        }
        let is_dir = |k: &str| local.get(k).or(remote.get(k)).is_some_and(|m| m.is_dir);
        // tạo thư mục trước (cha trước con), rồi file, cuối cùng xoá thư mục (con trước cha)
        let mut order: Vec<(&String, &Action)> = todo.iter().filter(|(_, a)| matches!(a, Action::MkdirLocal | Action::MkdirRemote)).collect();
        order.extend(todo.iter().filter(|(k, a)| !matches!(a, Action::MkdirLocal | Action::MkdirRemote) && !(matches!(a, Action::DeleteRemote | Action::TrashLocal) && is_dir(k))));
        order.extend(todo.iter().rev().filter(|(k, a)| matches!(a, Action::DeleteRemote | Action::TrashLocal) && is_dir(k)));
        let total = order.iter().filter(|(_, a)| !matches!(a, Action::Record | Action::Forget)).count() as u64;
        self.s.status_mut(&self.pair.id, |st| st.total = total);
        let mut since_save = 0;
        for (rel, a) in order {
            if !self.s.alive(&self.pair.id, self.gen) {
                self.save_base();
                return Err(E::Stop);
            }
            let (l, r) = (local.get(rel).copied(), remote.get(rel).copied());
            let res = self.apply(rel, *a, l, r).await;
            match res {
                Ok(()) => {}
                Err(E::Auth) => {
                    self.save_base();
                    return Err(E::Auth);
                }
                Err(E::Stop) => {}
                Err(E::Msg(m)) => self.s.status_mut(&self.pair.id, |st| {
                    st.errors += 1;
                    st.last_error = Some(format!("{rel}: {m}"));
                }),
            }
            if !matches!(a, Action::Record | Action::Forget) {
                self.s.status_mut(&self.pair.id, |st| st.done += 1);
                since_save += 1;
                if self.last_emit.elapsed() > Duration::from_millis(300) {
                    self.last_emit = Instant::now();
                    if let Some(app) = self.app {
                        self.s.emit(app);
                    }
                }
            }
            if since_save >= 200 {
                since_save = 0;
                self.save_base();
            }
        }
        self.save_base();
        Ok(())
    }

    async fn apply(&mut self, rel: &str, a: Action, l: Option<Meta>, r: Option<Meta>) -> Result<(), E> {
        match a {
            Action::Upload => self.upload(rel).await,
            Action::Download => self.download(rel, r.ok_or(E::Stop)?, l).await,
            Action::Conflict => self.conflict(rel, l.ok_or(E::Stop)?, r.ok_or(E::Stop)?).await,
            Action::DeleteRemote => {
                let r = self.raw(self.http.delete(self.url(&format!("/api/files/{}", enc(&self.rpath(rel), true))))).await?;
                if !r.status().is_success() && r.status() != reqwest::StatusCode::NOT_FOUND {
                    return Err(E::Msg(format!("delete: HTTP {}", r.status().as_u16())));
                }
                remove_tree(&mut self.base, rel);
                Ok(())
            }
            Action::TrashLocal => {
                // file vừa bị sửa sau lúc quét: không bỏ vào thùng rác, vòng sau sẽ thành "sửa thắng xoá"
                if l.is_some_and(|m| !m.is_dir) && local_meta(&self.lpath(rel)) != l {
                    return Err(E::Stop);
                }
                self.trash_local(rel)?;
                remove_tree(&mut self.base, rel);
                Ok(())
            }
            Action::MkdirRemote => {
                let r = self.raw(self.http.post(self.url("/api/mkdir")).json(&json!({ "path": self.rpath(rel) }))).await?;
                if !r.status().is_success() && r.status() != reqwest::StatusCode::CONFLICT {
                    return Err(E::Msg(format!("mkdir: HTTP {}", r.status().as_u16())));
                }
                self.base.insert(rel.into(), Base::dir());
                Ok(())
            }
            Action::MkdirLocal => {
                fs::create_dir_all(self.lpath(rel))?;
                self.base.insert(rel.into(), Base::dir());
                Ok(())
            }
            Action::Record => {
                let (l, r) = (l.ok_or(E::Stop)?, r.ok_or(E::Stop)?);
                self.base.insert(rel.into(), if l.is_dir { Base::dir() } else { Base::of(l, r) });
                Ok(())
            }
            Action::Forget => {
                self.base.remove(rel);
                Ok(())
            }
            Action::TypeClash => Err(E::Msg("file/folder name clash".into())),
        }
    }

    async fn upload(&mut self, rel: &str) -> Result<(), E> {
        let p = self.lpath(rel);
        let md = fs::symlink_metadata(&p)?;
        if !md.is_file() {
            return Err(E::Msg("not a file".into()));
        }
        let (size, mtime) = (md.len(), mtime_of(&md));
        let init: Value = self
            .ok(self.http.post(self.url("/api/files/uploads")).json(&json!({ "path": self.rpath(rel), "size": size, "mtime": mtime })))
            .await?
            .json()
            .await?;
        let id = init["id"].as_str().ok_or_else(|| E::Msg("upload: no id".into()))?.to_string();
        // server trả lại id cũ nếu cùng path+size+mtime: gửi tiếp từ chỗ dở
        let st: Value = self.ok(self.http.get(self.url(&format!("/api/files/uploads/{id}")))).await?.json().await?;
        let mut off = st["received"].as_u64().unwrap_or(0).min(size);
        let mut f = fs::File::open(&p)?;
        loop {
            let n = CHUNK.min(size - off);
            let mut buf = vec![0u8; n as usize];
            f.seek(SeekFrom::Start(off))?;
            f.read_exact(&mut buf).map_err(|_| E::Msg("file changed while uploading".into()))?;
            let r: Value = self.ok(self.http.put(self.url(&format!("/api/files/uploads/{id}?offset={off}"))).body(buf)).await?.json().await?;
            let got = r["received"].as_u64().unwrap_or(off + n);
            if got <= off && n > 0 {
                return Err(E::Msg("upload stalled".into()));
            }
            off = got;
            if off >= size {
                break;
            }
        }
        self.ok(self.http.post(self.url(&format!("/api/files/uploads/{id}/complete")))).await?;
        self.base.insert(rel.into(), Base { is_dir: false, ls: size, lm: mtime, rs: size, rm: mtime });
        self.s.status_mut(&self.pair.id, |st| st.up += 1);
        Ok(())
    }

    /// Tải về file tạm `<tên>.nkpart` cạnh đích, đặt mtime theo server.
    async fn fetch(&self, rel: &str, r: Meta) -> Result<PathBuf, E> {
        let target = self.lpath(rel);
        if let Some(d) = target.parent() {
            fs::create_dir_all(d)?;
        }
        let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let tmp = target.with_file_name(format!("{name}{}", plan::PART));
        let mut resp = self.ok(self.http.get(self.url(&format!("/api/files/{}", enc(&self.rpath(rel), true))))).await?;
        let mut f = fs::File::create(&tmp)?;
        let mut n = 0u64;
        let res: Result<(), E> = async {
            while let Some(c) = resp.chunk().await? {
                f.write_all(&c)?;
                n += c.len() as u64;
            }
            if n != r.size {
                return Err(E::Msg("size changed on server, retry later".into()));
            }
            f.set_modified(UNIX_EPOCH + Duration::from_secs(r.mtime.max(0) as u64))?;
            Ok(())
        }
        .await;
        drop(f);
        if res.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        res.map(|_| tmp)
    }

    async fn download(&mut self, rel: &str, r: Meta, l: Option<Meta>) -> Result<(), E> {
        let tmp = self.fetch(rel, r).await?;
        let target = self.lpath(rel);
        // file local vừa bị sửa trong lúc tải: để vòng sau xử lý (thành conflict)
        if local_meta(&target) != l {
            let _ = fs::remove_file(&tmp);
            return Err(E::Msg("changed locally during sync, retry later".into()));
        }
        fs::rename(&tmp, &target)?;
        let lm = local_meta(&target).ok_or_else(|| E::Msg("vanished".into()))?;
        self.base.insert(rel.into(), Base::of(lm, r));
        self.s.status_mut(&self.pair.id, |st| st.down += 1);
        Ok(())
    }

    async fn conflict(&mut self, rel: &str, l: Meta, r: Meta) -> Result<(), E> {
        let tmp = self.fetch(rel, r).await?;
        let target = self.lpath(rel);
        // cùng nội dung (vd thư mục chép tay từ server về): chỉ lấy mtime server, không tạo bản conflict
        if l.size == r.size && same_content(&target, &tmp).unwrap_or(false) {
            fs::rename(&tmp, &target)?;
            let lm = local_meta(&target).ok_or_else(|| E::Msg("vanished".into()))?;
            self.base.insert(rel.into(), Base::of(lm, r));
            return Ok(());
        }
        // thêm 4 ký tự ngẫu nhiên: 2 máy trùng hostname cùng phút không ghi đè bản conflict của nhau
        let rnd = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0) ^ std::process::id();
        let stamp = format!("{} {:04x}", plan::stamp(now() + self.auth.tz * 60), rnd & 0xffff);
        let mut cname = plan::conflict_name(rel, &self.s.host, &stamp);
        for i in 2.. {
            if local_meta(&self.lpath(&cname)).is_none() {
                break;
            }
            cname = plan::conflict_name(rel, &self.s.host, &format!("{stamp} {i}"));
        }
        fs::rename(&target, self.lpath(&cname))?;
        fs::rename(&tmp, &target)?;
        let lm = local_meta(&target).ok_or_else(|| E::Msg("vanished".into()))?;
        self.base.insert(rel.into(), Base::of(lm, r));
        self.s.status_mut(&self.pair.id, |st| {
            st.conflicts += 1;
            st.down += 1;
        });
        // upload lỗi thì vòng sau thấy file mới ở máy và upload lại
        self.upload(&cname).await
    }

    /// Dời vào `.nextkloud-trash/<rel>` (trùng tên thì thêm " (n)").
    fn trash_local(&self, rel: &str) -> Result<(), E> {
        let src = self.lpath(rel);
        if local_meta(&src).is_none() {
            return Ok(());
        }
        let mut dst = self.root.join(plan::TRASH);
        for c in rel.split('/') {
            dst.push(c);
        }
        let name = dst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut i = 2;
        while local_meta(&dst).is_some() {
            let alt = match name.rsplit_once('.') {
                Some((stem, ext)) if !stem.is_empty() => format!("{stem} ({i}).{ext}"),
                _ => format!("{name} ({i})"),
            };
            dst.set_file_name(alt);
            i += 1;
        }
        if let Some(d) = dst.parent() {
            fs::create_dir_all(d)?;
        }
        fs::rename(src, dst)?;
        Ok(())
    }
}

// ---- lệnh gọi từ webview ----

type S<'a> = State<'a, Arc<Engine>>;

/// Webview báo phiên đăng nhập (token None = đăng xuất -> dừng đồng bộ).
#[tauri::command]
pub fn sync_auth(app: AppHandle, s: S<'_>, server: String, token: Option<String>, account: Option<String>, tz: i64) {
    let server = server.trim_end_matches('/').to_string();
    let next = match (token, account) {
        (Some(t), Some(a)) if !t.is_empty() && !a.is_empty() => Some(Auth { account: format!("{server}|{a}"), server, token: t, tz }),
        _ => None,
    };
    {
        let mut i = s.inner.lock().unwrap();
        let same = match (&i.auth, &next) {
            (Some(a), Some(b)) => a.token == b.token && a.account == b.account,
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        i.auth = next;
        i.gen += 1;
    }
    s.wake.notify_one();
    s.emit(&app);
}

#[tauri::command]
pub fn sync_get(s: S<'_>) -> Vec<View> {
    s.views()
}

/// Chuẩn hoá thư mục server: bỏ `/` thừa, chặn `..`, `\` và thư mục hệ thống.
fn clean_remote(r: &str) -> Result<String, String> {
    let parts: Vec<&str> = r.split('/').map(str::trim).filter(|c| !c.is_empty()).collect();
    if parts.iter().any(|c| *c == "." || *c == ".." || c.contains('\\')) || parts.first() == Some(&".nextkloud") {
        return Err("sync.e.badRemote".into());
    }
    Ok(parts.join("/"))
}

fn overlap(a: &str, b: &str) -> bool {
    a.is_empty() || b.is_empty() || under(a, b) || under(b, a)
}

#[tauri::command]
pub fn sync_add(app: AppHandle, s: S<'_>, local: String, remote: String) -> Result<Vec<View>, String> {
    let local = local.trim().to_string();
    let lp = Path::new(&local);
    if !lp.is_absolute() {
        return Err("sync.e.notAbs".into());
    }
    if !lp.is_dir() {
        return Err("sync.e.noDir".into());
    }
    let canon = fs::canonicalize(lp).map_err(|e| e.to_string())?;
    if canon.parent().is_none() || canon.starts_with(&s.dir) {
        return Err("sync.e.badLocal".into());
    }
    let remote = clean_remote(&remote)?;
    {
        let mut i = s.inner.lock().unwrap();
        let account = i.auth.as_ref().map(|a| a.account.clone()).ok_or("sync.e.login")?;
        for p in &i.pairs {
            if let Ok(c) = fs::canonicalize(&p.local) {
                if c.starts_with(&canon) || canon.starts_with(&c) {
                    return Err("sync.e.overlapLocal".into());
                }
            }
            if p.account == account && overlap(&p.remote, &remote) {
                return Err("sync.e.overlapRemote".into());
            }
        }
        let id = format!("{:x}", SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        i.pairs.push(Pair { id, local, remote, paused: false, account });
        s.save_pairs(&i)?;
    }
    s.wake.notify_one();
    s.emit(&app);
    Ok(s.views())
}

/// Gỡ cặp: chỉ ngừng đồng bộ + xoá state, không đụng file 2 bên.
#[tauri::command]
pub fn sync_remove(app: AppHandle, s: S<'_>, id: String) -> Result<Vec<View>, String> {
    {
        let mut i = s.inner.lock().unwrap();
        i.pairs.retain(|p| p.id != id);
        i.status.remove(&id);
        s.save_pairs(&i)?;
    }
    let _ = fs::remove_file(s.state_file(&id));
    s.emit(&app);
    Ok(s.views())
}

#[tauri::command]
pub fn sync_pause(app: AppHandle, s: S<'_>, id: String, paused: bool) -> Result<Vec<View>, String> {
    {
        let mut i = s.inner.lock().unwrap();
        if let Some(p) = i.pairs.iter_mut().find(|p| p.id == id) {
            p.paused = paused;
        }
        s.save_pairs(&i)?;
    }
    if !paused {
        s.wake.notify_one();
    }
    s.emit(&app);
    Ok(s.views())
}

#[tauri::command]
pub fn sync_now(s: S<'_>) {
    s.wake.notify_one();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nk-sync-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn scan_skips_ignored_and_trash() {
        let d = tmp("scan");
        fs::create_dir_all(d.join("a/b")).unwrap();
        fs::create_dir_all(d.join(".nextkloud-trash/old")).unwrap();
        fs::create_dir_all(d.join("empty")).unwrap();
        fs::write(d.join("a/b/x.txt"), "hello").unwrap();
        fs::write(d.join("a/~$x.docx"), "lock").unwrap();
        fs::write(d.join("a/y.txt.nkpart"), "part").unwrap();
        fs::write(d.join(".DS_Store"), "").unwrap();
        let (m, skip) = scan(&d).unwrap();
        let keys: Vec<&str> = m.keys().map(String::as_str).collect();
        assert_eq!(keys, ["a", "a/b", "a/b/x.txt", "empty"]);
        assert_eq!(m["a/b/x.txt"].size, 5);
        assert!(m["empty"].is_dir);
        assert!(skip.is_empty());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn helpers() {
        assert_eq!(enc("a b/č?.txt", true), "a%20b/%C4%8D%3F.txt");
        assert_eq!(enc("a/b", false), "a%2Fb");
        assert!(under("a/b", "a") && under("a", "a") && !under("ab", "a"));
        assert!(overlap("", "x") && overlap("a", "a/b") && !overlap("a", "ab"));
        assert_eq!(clean_remote("/Docs//Work/").unwrap(), "Docs/Work");
        assert!(clean_remote("a/../b").is_err() && clean_remote(".nextkloud/x").is_err());
        let mut m: BTreeMap<String, u8> = [("d", 1), ("d/a", 1), ("d x", 1), ("d/a/b", 1), ("e", 1)].iter().map(|(k, v)| (k.to_string(), *v)).collect();
        remove_tree(&mut m, "d");
        assert_eq!(m.keys().map(String::as_str).collect::<Vec<_>>(), ["d x", "e"]);
    }

    fn set(p: &Path, body: &str, mtime: u64) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
        fs::OpenOptions::new().write(true).open(p).unwrap().set_modified(UNIX_EPOCH + Duration::from_secs(mtime)).unwrap();
    }

    /// 2 "máy" A, B cùng đồng bộ 1 thư mục trên server thật.
    /// Chạy: NK_SYNC_SERVER=http://127.0.0.1:PORT NK_SYNC_TOKEN=<jwt> NK_SYNC_USER=<id> cargo test two_machines
    #[test]
    fn two_machines_against_server() {
        let (Ok(server), Ok(token)) = (std::env::var("NK_SYNC_SERVER"), std::env::var("NK_SYNC_TOKEN")) else { return };
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::new();
        let root = tmp("e2e");
        let e = Engine::new(root.join("app"));
        fs::create_dir_all(&e.dir).unwrap();
        let remote = format!("synctest-{}", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis());
        let auth = Auth { server, token, account: "t".into(), tz: 420 };
        let (a, b) = (root.join("A"), root.join("B"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        let pa = Pair { id: "a".into(), local: a.to_string_lossy().into(), remote: remote.clone(), paused: false, account: "t".into() };
        let pb = Pair { id: "b".into(), local: b.to_string_lossy().into(), remote: remote.clone(), paused: false, account: "t".into() };
        e.inner.lock().unwrap().pairs = vec![pa.clone(), pb.clone()];
        let round = |p: &Pair| {
            let mut cx = Ctx::new(&e, None, &http, auth.clone(), 0, p.clone());
            if let Err(E::Msg(m)) = tauri::async_runtime::block_on(cx.round()) {
                panic!("round {}: {m}", p.id);
            }
            let st = e.status_mut(&p.id, std::mem::take);
            assert_eq!(st.errors, 0, "{:?}", st.last_error);
            st
        };
        let read = |p: PathBuf| fs::read_to_string(p).unwrap();

        // A tạo file + thư mục rỗng -> server -> B
        set(&a.join("a.txt"), "hello", 1_700_000_000);
        set(&a.join("sub/b.txt"), "bee", 1_700_000_001);
        set(&a.join("zero.bin"), "", 1_700_000_002);
        fs::create_dir_all(a.join("emptyL")).unwrap();
        assert_eq!(round(&pa).up, 3);
        let st = round(&pb);
        assert_eq!(st.down, 3);
        assert_eq!(read(b.join("sub/b.txt")), "bee");
        assert_eq!(local_meta(&b.join("a.txt")), Some(Meta::file(5, 1_700_000_000)));
        assert!(b.join("emptyL").is_dir() && b.join("zero.bin").is_file());
        // không đổi gì -> không làm gì
        let st = round(&pa);
        assert_eq!((st.up, st.down, st.total), (0, 0, 0));

        // B sửa a.txt, xoá sub/b.txt -> A nhận bản sửa, b.txt vào rác local
        set(&b.join("a.txt"), "hello from B", 1_700_000_100);
        fs::remove_file(b.join("sub/b.txt")).unwrap();
        assert_eq!(round(&pb).up, 1);
        assert_eq!(round(&pa).down, 1);
        assert_eq!(read(a.join("a.txt")), "hello from B");
        assert!(!a.join("sub/b.txt").exists());
        assert_eq!(read(a.join(".nextkloud-trash/sub/b.txt")), "bee");

        // 2 bên cùng sửa -> conflict: bản server giữ tên, bản máy B thành file conflict
        set(&a.join("a.txt"), "A edit", 1_700_000_200);
        set(&b.join("a.txt"), "B edit!!", 1_700_000_300);
        round(&pa);
        let st = round(&pb);
        assert_eq!(st.conflicts, 1);
        assert_eq!(read(b.join("a.txt")), "A edit");
        let cf: Vec<String> = fs::read_dir(&b).unwrap().filter_map(|x| x.ok()?.file_name().into_string().ok()).filter(|n| n.contains("(conflict ")).collect();
        assert_eq!(cf.len(), 1, "{cf:?}");
        assert_eq!(read(b.join(&cf[0])), "B edit!!");
        round(&pa);
        assert_eq!(read(a.join(&cf[0])), "B edit!!");

        // 2 bên tạo cùng file giống hệt nội dung, khác mtime -> không sinh conflict
        set(&a.join("same.txt"), "identical", 1_700_000_400);
        set(&b.join("same.txt"), "identical", 1_700_000_500);
        round(&pa);
        assert_eq!(round(&pb).conflicts, 0);

        // A xoá thư mục -> server xoá -> B dời thư mục vào rác
        fs::remove_dir_all(a.join("sub")).unwrap();
        fs::remove_dir(a.join("emptyL")).unwrap();
        round(&pa);
        round(&pb);
        assert!(!b.join("sub").exists() && !b.join("emptyL").exists());
        assert!(b.join(".nextkloud-trash/emptyL").is_dir());
        // sau cùng 2 máy giống nhau
        let (la, _) = scan(&a).unwrap();
        let (lb, _) = scan(&b).unwrap();
        assert_eq!(la.keys().collect::<Vec<_>>(), lb.keys().collect::<Vec<_>>());
        // token sai -> dừng hẳn (E::Auth), không coi là lỗi file
        let bad = Auth { token: "bad".into(), ..auth.clone() };
        let mut cx = Ctx::new(&e, None, &http, bad, 0, pa.clone());
        assert!(matches!(tauri::async_runtime::block_on(cx.round()), Err(E::Auth)));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn content_compare() {
        let d = tmp("cmp");
        fs::write(d.join("a"), "same").unwrap();
        fs::write(d.join("b"), "same").unwrap();
        fs::write(d.join("c"), "diff").unwrap();
        fs::write(d.join("e"), "same!").unwrap();
        assert!(same_content(&d.join("a"), &d.join("b")).unwrap());
        assert!(!same_content(&d.join("a"), &d.join("c")).unwrap());
        assert!(!same_content(&d.join("a"), &d.join("e")).unwrap());
        let _ = fs::remove_dir_all(&d);
    }
}
