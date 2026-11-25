// Nhỏ gọn: client cho backend /api + TTS service. Không dep ngoài solid.
import { t } from "./i18n";
/** Chạy trong app Tauri (desktop/mobile) thay vì trình duyệt web. */
export function isTauri(): boolean {
  return typeof window !== "undefined" && !!(window as any).__TAURI_INTERNALS__;
}

export function isSelfHostedWeb(): boolean {
  // Web serve từ chính backend -> cùng origin, khỏi nhập tay.
  // App Tauri (kể cả URL tauri.localhost bắt đầu bằng http) luôn cần nhập server tay.
  return window.location.protocol.startsWith("http") && !isTauri();
}

export function serverOrigin(): string {
  // Web serve từ chính backend -> cùng origin, khỏi nhập tay (bỏ qua giá trị cũ).
  if (isSelfHostedWeb()) return window.location.origin;
  const saved = localStorage.getItem("nk-server");
  if (saved) return saved.replace(/\/+$/, "");
  return "http://mini:8011";
}

export interface Entry {
  path: string;
  name: string;
  is_dir: boolean;
  size: number;
  modified: number;
  /** Tìm theo nội dung: đoạn chữ quanh chỗ khớp. */
  snippet?: string;
}

export interface Page {
  entries: Entry[];
  dirs: number;
  files: number;
  bytes: number;
}

export interface ListOpts {
  sort: "name" | "size" | "modified";
  desc: boolean;
  offset: number;
  limit: number;
}

export interface TrashItem {
  id: string;
  orig_path: string;
  is_dir: boolean;
  size: number;
  files: number;
  deleted_at: number;
}

export interface GitRepo {
  id: string;
  url: string;
  branch: string;
  folder: string;
  ssh_pub: string | null;
  interval_h: number;
  state: "pending" | "idle" | "syncing" | "error";
  last_sync: number | null;
  last_commit: string | null;
  last_error: string | null;
  files: number;
  created_at: number;
}

export interface KLabel {
  title: string;
  color: string;
}
export interface KBoard {
  id: string;
  title: string;
  labels: KLabel[];
  archived: boolean;
  position: number;
  created_at: number;
}
export interface KColumn {
  id: string;
  title: string;
  position: number;
  is_done: boolean;
  is_cancel: boolean;
  total: number;
}
export interface KCard {
  id: string;
  column_id: string;
  parent_id: string | null;
  title: string;
  description: string;
  position: number;
  labels: string[];
  color: string | null;
  due_at: number | null;
  done_at: number | null;
  created_at: number;
  updated_at: number;
  attachments: string[];
  parent_title: string | null;
  parent_cancelled: boolean | null;
  sub_total: number;
  sub_done: number;
}
export type KCardPatch = Partial<Pick<KCard, "title" | "description" | "column_id" | "position" | "labels" | "color" | "due_at" | "parent_id" | "attachments">> & { archived?: boolean };
export interface DeckPreview {
  id: number;
  title: string;
  archived: boolean;
  imported: boolean;
  stacks: { id: number; title: string; count: number; merge_into: string | null; is_done: boolean }[];
  cards: { id: number; title: string; stack_id: number; big: boolean; done: boolean; parent: number | null }[];
}
export type TaskView = "overdue" | "today" | "week" | "all" | "done";
export interface TaskItem {
  id: string;
  board_id: string;
  board_title: string;
  column_id: string;
  column_title: string;
  parent_id: string | null;
  title: string;
  due_at: number | null;
  done_at: number | null;
  labels: string[];
}
export interface Habit {
  id: string;
  title: string;
  schedule: string;
  archived: boolean;
  created_at: number;
  streak: number;
  total: number;
  done_today: boolean;
  days: string[];
}
export interface LinkMeta {
  title: string;
  is_dir: boolean;
  locked: boolean;
  expires_at: number | null;
}
export interface ShareRow {
  id: string;
  path: string;
  is_dir: boolean;
  target: string | null;
  /** người nhận đã chia sẻ lại (null = chủ file tạo) */
  granted_by: string | null;
  token: string | null;
  has_password: boolean;
  permission: string;
  expires_at: number | null;
  created_at: number;
}
export interface SharedItem {
  id: string;
  owner_id: string;
  owner_name: string;
  path: string;
  is_dir: boolean;
  permission: string;
  expires_at: number | null;
  /** người chia sẻ lại cho tôi (null = chủ file) */
  via: string | null;
}
export interface Backlink {
  path: string;
  name: string;
  snippet: string;
}
export interface VaultOutline {
  root: string;
  files: { path: string; headings: [number, string][] }[];
  assets?: string[];
}
export type NoteState = "todo" | "doing" | "done";
export interface Annotation {
  id: string;
  path: string;
  kind: "bookmark" | "note";
  anchor: string;
  quote: string;
  body: string;
  state: NoteState;
  created_at: number;
  updated_at: number;
}

export interface UserRow {
  id: string;
  username: string;
  display_name: string | null;
  avatar_v: number;
  must_reset: boolean;
  is_admin: boolean;
  quota_limit: number;
  quota_used: number;
}

export interface Me {
  id: string;
  username: string;
  display_name: string | null;
  avatar_v: number;
  is_admin: boolean;
  quota_limit: number;
  quota_used: number;
}

const encPath = (p: string) => p.split("/").map(encodeURIComponent).join("/");

/** Gốc Secret Vault trong app: "\u0000s/<path>" (NUL không có trong tên file thật) -> gọi /api/secret/*. */
export const SECRET_ROOT = "\u0000s";
export const isSecret = (p: string) => p === SECRET_ROOT || p.startsWith(SECRET_ROOT + "/");
const sRel = (p: string) => (p === SECRET_ROOT ? "" : p.slice(SECRET_ROOT.length + 1));
const sAbs = (p: string) => (p ? `${SECRET_ROOT}/${p}` : SECRET_ROOT);
/** Token mở khoá vault: sessionStorage -> đóng tab là khoá. */
export const vaultToken = () => sessionStorage.getItem("nk-vault");
export const setVaultToken = (t: string | null) => (t ? sessionStorage.setItem("nk-vault", t) : sessionStorage.removeItem("nk-vault"));
/** Lần cuối gọi API vault (để đếm ngược tự khoá). */
export let vaultSeen = 0;
const fileUrl = (path: string) => (isSecret(path) ? `/api/secret/files/${encPath(sRel(path))}` : `/api/files/${encPath(path)}`);
// Đang xem file người khác chia sẻ (ViewerOverlay trong "Được chia sẻ"): đọc file kèm ?owner=
let viewOwner: string | null = null;
export const setViewOwner = (o: string | null) => (viewOwner = o);
const ownQ = (path: string, sep: "?" | "&") => (viewOwner && !isSecret(path) ? `${sep}owner=${encodeURIComponent(viewOwner)}` : "");
const vtQ = (path: string) => (isSecret(path) ? `&vt=${encodeURIComponent(vaultToken() ?? "")}` : "");
const noSecret = (path: string, what: string) => {
  if (isSecret(path)) throw new Error(`${what} không dùng được với file Bí mật`);
};

export function token(): string | null {
  return localStorage.getItem("nk-token");
}

const JSON_H = { "Content-Type": "application/json" };

async function req(path: string, init?: RequestInit) {
  const tok = token();
  const sec = path.startsWith("/api/secret");
  const vt = sec ? vaultToken() : null;
  const r = await fetch(serverOrigin() + path, {
    ...init,
    headers: { ...(init?.headers || {}), ...(tok ? { Authorization: `Bearer ${tok}` } : {}), ...(vt ? { "X-Vault-Token": vt } : {}) },
  });
  if (r.status === 401) {
    // sai mat khau link cong khai (/api/s/*) khong phai token login het han -> giu nguyen
    if (!path.startsWith("/api/s/")) localStorage.removeItem("nk-token");
    throw new Error("unauthorized");
  }
  if (sec) {
    if (r.status === 403 && (await r.clone().text()).includes("vault locked")) {
      setVaultToken(null);
      window.dispatchEvent(new Event("nk-vault-locked"));
      throw new Error("Khu Bí mật đã khoá");
    }
    if (vt) vaultSeen = Date.now();
  }
  if (!r.ok) throw new Error(`${r.status} ${await r.text()}`);
  // Server trả HTML thay vì JSON (sai địa chỉ server, proxy chặn...): báo rõ thay vì "Unexpected token '<'".
  const ct = r.headers.get("content-type") ?? "";
  if (ct.includes("text/html") && (path.startsWith("/api/") || path.startsWith("/s/"))) {
    throw new Error(t("c.badJson"));
  }
  return r;
}

export const api = {
  async login(username: string, password: string) {
    const r = await fetch(serverOrigin() + "/api/auth/login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ username, password }),
    });
    if (!r.ok) throw new Error("login failed");
    return r.json() as Promise<{ token: string; must_reset: boolean; is_admin: boolean }>;
  },
  async changePassword(old_: string, next: string) {
    const t = token();
    const r = await fetch(serverOrigin() + "/api/auth/password", {
      method: "POST",
      headers: { "Content-Type": "application/json", ...(t ? { Authorization: `Bearer ${t}` } : {}) },
      body: JSON.stringify({ old: old_, new: next }),
    });
    if (r.status === 401) throw new Error("Mật khẩu hiện tại không đúng");
    if (r.status === 400) throw new Error("Mật khẩu mới phải có ít nhất 8 ký tự");
    if (!r.ok) throw new Error(`Đổi mật khẩu thất bại (${r.status})`);
  },
  /** 1 trang của thư mục (server phân trang, thư mục luôn trước). owner: duyệt vùng được chia sẻ. */
  async files(prefix: string, o: ListOpts, owner?: string) {
    const sec = isSecret(prefix);
    const q = new URLSearchParams({ prefix: sec ? sRel(prefix) : prefix, sort: o.sort, desc: String(o.desc), offset: String(o.offset), limit: String(o.limit) });
    if (owner) q.set("owner", owner);
    const r = (await (await req(`${sec ? "/api/secret/list" : "/api/files"}?${q}`)).json()) as Page;
    if (sec) r.entries.forEach((e) => (e.path = sAbs(e.path)));
    return r;
  },
  /** Tìm theo nội dung file .md/.txt. */
  async searchContent(q: string): Promise<Entry[]> {
    const r = await req(`/api/search/content?q=${encodeURIComponent(q)}&limit=200`);
    const hits = (await r.json()) as { path: string; name: string; size: number; modified: number; snippet: string }[];
    return hits.map((h) => ({ ...h, is_dir: false }));
  },
  /** Tìm theo tên trên toàn bộ file. */
  async search(q: string) {
    const r = await req(`/api/search?q=${encodeURIComponent(q)}&limit=300`);
    return r.json() as Promise<Entry[]>;
  },
  // --- admin ---
  async users() {
    return (await req("/api/users")).json() as Promise<UserRow[]>;
  },
  async createUser(b: { username: string; password: string; quota_limit: number; is_admin: boolean }) {
    const r = await req("/api/users", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(b) });
    return r.json() as Promise<UserRow>;
  },
  async setQuota(id: string, quota_limit: number) {
    const r = await req(`/api/users/${id}/quota`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ quota_limit }),
    });
    return r.json() as Promise<UserRow>;
  },
  async resetPassword(id: string, password: string) {
    await req(`/api/users/${id}/password`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ password }),
    });
  },
  async deleteUser(id: string) {
    await req(`/api/users/${id}`, { method: "DELETE" });
  },
  async adminStats() {
    return (await req("/api/admin/stats")).json() as Promise<{
      version: string;
      storage: string;
      users: number;
      files: number;
      dirs: number;
      bytes: number;
    }>;
  },
  async adminReindex(id: string) {
    return (await req(`/api/admin/reindex/${id}`, { method: "POST" })).json() as Promise<{ files: number; bytes: number }>;
  },
  // --- hồ sơ ---
  async updateMe(display_name: string) {
    const r = await req("/api/me", {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ display_name }),
    });
    return r.json() as Promise<Me>;
  },
  async putAvatar(img: Blob) {
    return (await req("/api/me/avatar", { method: "PUT", body: img })).json() as Promise<Me>;
  },
  async deleteAvatar() {
    return (await req("/api/me/avatar", { method: "DELETE" })).json() as Promise<Me>;
  },
  async avatarBlob(id: string, v: number) {
    return (await req(`/api/users/${id}/avatar?v=${v}`)).blob();
  },
  /** Vé 1 phút để mở SSE (không để JWT đăng nhập trong URL). */
  async eventsTicket() {
    return ((await (await req("/api/events/ticket", { method: "POST" })).json()) as { ticket: string }).ticket;
  },
  async me() {
    const r = await req("/api/me");
    return r.json() as Promise<Me>;
  },
  /** Xoá = chuyển vào thùng rác. owner: xoá trong vùng được chia sẻ (cần quyền sửa). */
  async remove(path: string, owner?: string) {
    const u = fileUrl(path) + (owner ? `?owner=${encodeURIComponent(owner)}` : "");
    const t = (await req(u, { method: "DELETE" })).json() as Promise<TrashItem>;
    return isSecret(path) ? t.then((x) => ({ ...x, orig_path: sAbs(x.orig_path) })) : t;
  },
  async mkdir(path: string, owner?: string) {
    const sec = isSecret(path);
    await req(sec ? "/api/secret/mkdir" : "/api/mkdir", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ path: sec ? sRel(path) : path, ...(owner ? { owner } : {}) }) });
  },
  /** Ghi đè / tạo file text (dùng khi editor drawio/excalidraw lưu). */
  async saveText(path: string, text: string) {
    await this.upload(path, new Blob([text], { type: "text/plain;charset=utf-8" }));
  },
  /** URL stream trực tiếp (kèm ?t= cho <video>/<audio>/<img> không gửi được Bearer). */
  streamUrl(path: string, owner?: string) {
    const t = token();
    return `${serverOrigin()}${fileUrl(path)}?t=${encodeURIComponent(t ?? "")}${vtQ(path)}${owner ? `&owner=${encodeURIComponent(owner)}` : ownQ(path, "&")}`;
  },
  /** owner: thumbnail file người khác chia sẻ (mặc định theo file đang xem trong ViewerOverlay). */
  thumbUrl(path: string, s = 256, owner?: string) {
    const t = token();
    const base = isSecret(path) ? `/api/secret/thumb/${encPath(sRel(path))}` : `/api/thumb/${encPath(path)}`;
    return `${serverOrigin()}${base}?s=${s}&t=${encodeURIComponent(t ?? "")}${vtQ(path)}${owner ? `&owner=${encodeURIComponent(owner)}` : ownQ(path, "&")}`;
  },
  async officeConfig(path: string) {
    const sec = isSecret(path); // file bí mật: chỉ xem
    const owner = !sec && viewOwner ? { owner: viewOwner } : {}; // file được chia sẻ: server quyết xem/sửa theo quyền
    const r = await req(sec ? "/api/secret/office/config" : "/api/office/config", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ path: sec ? sRel(path) : path, ...owner }) });
    return r.json() as Promise<{ config: any; token: string }>;
  },
  async zipList(path: string, limit = 20000) {
    noSecret(path, "Xem file zip");
    const r = await req(`/api/zip/list?path=${encodeURIComponent(path)}&limit=${limit}`);
    return r.json() as Promise<{ entries: { name: string; size: number; is_dir: boolean }[]; total: number; truncated: boolean }>;
  },
  zipFileUrl(path: string, entry: string) {
    const t = token();
    return `${serverOrigin()}/api/zip/file?path=${encodeURIComponent(path)}&entry=${encodeURIComponent(entry)}&t=${encodeURIComponent(t ?? "")}`;
  },
  /** Đổi tên hoặc di chuyển (file/thư mục). owner: di chuyển trong vùng được chia sẻ. */
  async move(from: string, to: string, owner?: string) {
    return this.xfer(from, to, false, owner);
  },
  async copy(from: string, to: string, owner?: string) {
    return this.xfer(from, to, true, owner);
  },
  /** Chuyển/sao chép; qua lại giữa file thường và Bí mật thì server mã hoá/giải mã. */
  async xfer(from: string, to: string, keep: boolean, owner?: string) {
    const [a, b] = [isSecret(from), isSecret(to)];
    const url = a && b ? (keep ? "/api/secret/copy" : "/api/secret/move") : !a && !b ? (keep ? "/api/copy" : "/api/move") : a ? "/api/secret/export" : "/api/secret/import";
    const body = { from: a ? sRel(from) : from, to: b ? sRel(to) : to, keep, ...(owner && !a && !b ? { owner } : {}) };
    const r = await req(url, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
    return r.json() as Promise<{ files: number }>;
  },
  // --- bookmark + ghi chú ---
  async annotations(path: string) {
    if (isSecret(path)) return [] as Annotation[]; // không lưu trích dẫn của file bí mật vào DB thường
    return (await req(`/api/annotations?path=${encodeURIComponent(path)}`)).json() as Promise<Annotation[]>;
  },
  // --- kanban ---
  async boards() {
    return (await req("/api/boards")).json() as Promise<KBoard[]>;
  },
  async createBoard(title: string) {
    return (await req("/api/boards", { method: "POST", headers: JSON_H, body: JSON.stringify({ title }) })).json() as Promise<KBoard>;
  },
  async updateBoard(id: string, b: { title?: string; archived?: boolean; labels?: KLabel[] }) {
    await req(`/api/boards/${id}`, { method: "PATCH", headers: JSON_H, body: JSON.stringify(b) });
  },
  async deleteBoard(id: string) {
    await req(`/api/boards/${id}`, { method: "DELETE" });
  },
  async board(id: string, doneLimit = 10) {
    return (await req(`/api/boards/${id}?done_limit=${doneLimit}`)).json() as Promise<{ board: KBoard; columns: KColumn[]; cards: KCard[] }>;
  },
  async columnCards(id: string, offset: number, limit = 50) {
    return (await req(`/api/columns/${id}/cards?offset=${offset}&limit=${limit}`)).json() as Promise<KCard[]>;
  },
  async createColumn(board: string, title: string, is_done = false) {
    return (await req(`/api/boards/${board}/columns`, { method: "POST", headers: JSON_H, body: JSON.stringify({ title, is_done }) })).json() as Promise<KColumn>;
  },
  async updateColumn(id: string, b: { title?: string; position?: number; is_done?: boolean; is_cancel?: boolean }) {
    await req(`/api/columns/${id}`, { method: "PATCH", headers: JSON_H, body: JSON.stringify(b) });
  },
  async deleteColumn(id: string) {
    await req(`/api/columns/${id}`, { method: "DELETE" });
  },
  async createCard(column_id: string, title: string, parent_id?: string | null) {
    return (await req("/api/cards", { method: "POST", headers: JSON_H, body: JSON.stringify({ column_id, title, parent_id: parent_id ?? null }) })).json() as Promise<KCard>;
  },
  async updateCard(id: string, b: KCardPatch) {
    return (await req(`/api/cards/${id}`, { method: "PATCH", headers: JSON_H, body: JSON.stringify(b) })).json() as Promise<KCard>;
  },
  async card(id: string) {
    return (await req(`/api/cards/${id}`)).json() as Promise<KCard>;
  },
  async deleteCard(id: string) {
    await req(`/api/cards/${id}`, { method: "DELETE" });
  },
  /** Task có thể làm task cha (tìm cả task chưa tải). */
  async parentSearch(board: string, q: string) {
    return (await req(`/api/boards/${board}/parents?q=${encodeURIComponent(q)}`)).json() as Promise<{ id: string; title: string; done: boolean }[]>;
  },
  async linkPreview(url: string) {
    return (await req(`/api/linkpreview?url=${encodeURIComponent(url)}`)).json() as Promise<{ url: string; title: string | null; image: string | null; site: string | null }>;
  },
  async subtasks(id: string) {
    return (await req(`/api/cards/${id}/subtasks`)).json() as Promise<KCard[]>;
  },
  async deckPreview() {
    return (await req("/api/kanban/import/deck")).json() as Promise<DeckPreview[]>;
  },
  async deckApply(board_id: number, parents: Record<number, number | null>, skip_stacks: number[]) {
    return (await req("/api/kanban/import/deck", { method: "POST", headers: JSON_H, body: JSON.stringify({ board_id, parents, skip_stacks }) })).json() as Promise<{ board: string; cards: number; subtasks: number }>;
  },
  // --- chia sẻ ---
  /** owner: mục được chia sẻ cho tôi -> chỉ các share lại do tôi tạo. */
  async shares(path?: string, owner?: string) {
    const q = new URLSearchParams({ ...(path ? { path } : {}), ...(owner ? { owner } : {}) }).toString();
    return (await req(`/api/shares${q ? `?${q}` : ""}`)).json() as Promise<ShareRow[]>;
  },
  /** owner: chia sẻ lại mục người khác chia sẻ cho tôi (quyền ≤ quyền của tôi). */
  async shareWith(path: string, username: string, permission: string, owner?: string) {
    return (await req("/api/shares", { method: "POST", headers: JSON_H, body: JSON.stringify({ path, username, permission, ...(owner ? { owner } : {}) }) })).json() as Promise<ShareRow>;
  },
  async unshare(id: string) {
    await req(`/api/shares/${id}`, { method: "DELETE" });
  },
  async createLink(path: string, password?: string, expires_in_days?: number | null) {
    return (
      await req("/api/shares/link", {
        method: "POST",
        headers: JSON_H,
        body: JSON.stringify({ path, password: password ?? "", expires_in_days: expires_in_days ?? null }),
      })
    ).json() as Promise<ShareRow>;
  },
  async shared() {
    return (await req("/api/shared")).json() as Promise<SharedItem[]>;
  },
  /** Tìm theo tên trong cây được chia sẻ (root = gốc mục được chia sẻ). */
  async sharedSearch(owner: string, root: string, q: string) {
    return (await req(`/api/shared/search?${new URLSearchParams({ owner, root, q })}`)).json() as Promise<Entry[]>;
  },
  async saveCopy(owner: string, path: string, to: string) {
    return (await req("/api/shared/save", { method: "POST", headers: JSON_H, body: JSON.stringify({ owner, path, to }) })).json() as Promise<{ files: number }>;
  },
  /** Tải trực tiếp file được chia sẻ (không qua RAM tab). */
  async sharedDirectUrl(owner: string, path: string): Promise<string> {
    const r = await req("/api/dl", { method: "POST", headers: JSON_H, body: JSON.stringify({ path, owner }) });
    const { token: t } = (await r.json()) as { token: string };
    return `${serverOrigin()}/api/files/${encPath(path)}?dl=${encodeURIComponent(t)}`;
  },
  /** Link tải file qua link công khai (không cần đăng nhập). */
  linkFileUrl(token: string, path: string, pw?: string) {
    return `${serverOrigin()}/api/s/${token}/file/${encPath(path)}${pw ? `?pw=${encodeURIComponent(pw)}` : ""}`;
  },
  async linkMeta(token: string, pw?: string) {
    return (await req(`/api/s/${token}${pw ? `?pw=${encodeURIComponent(pw)}` : ""}`)).json() as Promise<LinkMeta>;
  },
  async linkList(token: string, prefix: string, pw?: string) {
    const q = new URLSearchParams({ prefix, ...(pw ? { pw } : {}) });
    return (await req(`/api/s/${token}/list?${q}`)).json() as Promise<Page>;
  },
  // --- task + habit tracker ---
  async tasks(view: TaskView) {
    return (await req(`/api/tasks?view=${view}`)).json() as Promise<TaskItem[]>;
  },
  async habits() {
    return (await req("/api/habits")).json() as Promise<Habit[]>;
  },
  async createHabit(title: string) {
    return (await req("/api/habits", { method: "POST", headers: JSON_H, body: JSON.stringify({ title }) })).json() as Promise<Habit>;
  },
  async updateHabit(id: string, b: { title?: string; archived?: boolean }) {
    return (await req(`/api/habits/${id}`, { method: "PATCH", headers: JSON_H, body: JSON.stringify(b) })).json() as Promise<Habit>;
  },
  async deleteHabit(id: string) {
    await req(`/api/habits/${id}`, { method: "DELETE" });
  },
  async habitCheck(id: string, day: string, done: boolean) {
    return (await req(`/api/habits/${id}/check`, { method: "POST", headers: JSON_H, body: JSON.stringify({ day, done }) })).json() as Promise<Habit>;
  },
  // --- Secret Vault ---
  async secretStatus() {
    return (await req("/api/secret")).json() as Promise<{ exists: boolean; unlocked: boolean; wait: number; idle: number; left: number }>;
  },
  async secretSetup(pin: string) {
    const r = (await (await req("/api/secret/setup", { method: "POST", headers: JSON_H, body: JSON.stringify({ pin }) })).json()) as { token: string; recovery: string };
    setVaultToken(r.token);
    return r.recovery;
  },
  async secretUnlock(b: { pin: string } | { recovery: string }) {
    const r = (await (await req("/api/secret/unlock", { method: "POST", headers: JSON_H, body: JSON.stringify(b) })).json()) as { token: string };
    setVaultToken(r.token);
  },
  async secretLock() {
    if (vaultToken()) await req("/api/secret/lock", { method: "POST" }).catch(() => {});
    setVaultToken(null);
  },
  async secretChangePin(old: string | null, next: string) {
    await req("/api/secret/pin", { method: "POST", headers: JSON_H, body: JSON.stringify({ old, new: next }) });
  },
  async secretNewRecovery(pin: string) {
    return ((await (await req("/api/secret/recovery", { method: "POST", headers: JSON_H, body: JSON.stringify({ pin }) })).json()) as { recovery: string }).recovery;
  },
  async secretDestroy(pin: string) {
    await req("/api/secret", { method: "DELETE", headers: JSON_H, body: JSON.stringify({ pin }) });
    setVaultToken(null);
  },
  /** Mục lục cả vault Obsidian chứa file (null nếu không thuộc vault nào). */
  async vaultOutline(path: string) {
    if (isSecret(path)) return null;
    return (await req(`/api/vault/outline?path=${encodeURIComponent(path)}`)).json() as Promise<VaultOutline | null>;
  },
  /** File nào nhắc tới file này qua [[wikilink]]. */
  async backlinks(path: string) {
    return (await req(`/api/vault/backlinks?path=${encodeURIComponent(path)}`)).json() as Promise<Backlink[]>;
  },
  async allNotes(state?: NoteState) {
    return (await req(`/api/annotations${state ? `?state=${state}` : ""}`)).json() as Promise<Annotation[]>;
  },
  async addAnnotation(b: { path: string; kind: "bookmark" | "note"; anchor: string; quote: string; body?: string }) {
    noSecret(b.path, "Bookmark/ghi chú");
    const r = await req("/api/annotations", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(b) });
    return r.json() as Promise<Annotation>;
  },
  async updateAnnotation(id: string, b: { body?: string; state?: NoteState }) {
    const r = await req(`/api/annotations/${id}`, { method: "PATCH", headers: { "Content-Type": "application/json" }, body: JSON.stringify(b) });
    return r.json() as Promise<Annotation>;
  },
  async removeAnnotation(id: string) {
    await req(`/api/annotations/${id}`, { method: "DELETE" });
  },
  // --- git mirror ---
  async gitRepos() {
    return (await req("/api/git")).json() as Promise<GitRepo[]>;
  },
  async gitAdd(b: { url: string; branch: string; folder: string; key: "generate" | "paste" | "none"; private_key?: string; interval_h: number }) {
    const r = await req("/api/git", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(b) });
    return r.json() as Promise<GitRepo>;
  },
  async gitUpdate(id: string, b: { interval_h?: number; branch?: string; url?: string }) {
    const r = await req(`/api/git/${id}`, { method: "PATCH", headers: { "Content-Type": "application/json" }, body: JSON.stringify(b) });
    return r.json() as Promise<GitRepo>;
  },
  async gitSync(id: string) {
    await req(`/api/git/${id}/sync`, { method: "POST" });
  },
  async gitRemove(id: string, files: boolean) {
    await req(`/api/git/${id}?files=${files}`, { method: "DELETE" });
  },
  async trash(secret = false) {
    const r = (await (await req(secret ? "/api/secret/trash" : "/api/trash")).json()) as TrashItem[];
    if (secret) r.forEach((t) => (t.orig_path = sAbs(t.orig_path)));
    return r;
  },
  async config() {
    return (await req("/api/config")).json() as Promise<{ trash_days: number; version: string }>;
  },
  async restore(id: string, secret = false) {
    await req(`${secret ? "/api/secret" : "/api"}/trash/${id}/restore`, { method: "POST" });
  },
  async purge(id: string, secret = false) {
    await req(`${secret ? "/api/secret" : "/api"}/trash/${id}`, { method: "DELETE" });
  },
  async emptyTrash(secret = false) {
    await req(secret ? "/api/secret/trash" : "/api/trash", { method: "DELETE" });
  },
  /** Ném lỗi nếu file không tồn tại (HEAD, không tải nội dung). */
  async exists(path: string) {
    await req(fileUrl(path) + ownQ(path, "?"), { method: "HEAD" });
  },
  /** Bản cũ của file (mới nhất trước), ts = mtime bản cũ. */
  async versions(path: string) {
    return (await req(`/api/versions?path=${encodeURIComponent(path)}`)).json() as Promise<{ ts: number; size: number }[]>;
  },
  async versionBlob(path: string, ts: number) {
    return (await req(`/api/versions/file?path=${encodeURIComponent(path)}&ts=${ts}`)).blob();
  },
  async restoreVersion(path: string, ts: number) {
    await req("/api/versions/restore", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ path, ts }) });
  },
  async readBlob(path: string) {
    return (await req(fileUrl(path) + ownQ(path, "?"))).blob();
  },
  /** Upload đang dở của chính mình (để resume): cùng vùng, path và size. */
  async uploadActive(path: string, size: number, mtime: number, owner?: string) {
    const q = new URLSearchParams({ path, size: String(size), mtime: String(mtime), ...(owner ? { owner } : {}) });
    const r = await req(`/api/files/uploads/active?${q}`);
    return r.json() as Promise<{ id: string; received: number; size: number } | null>;
  },
  /** Chunked upload 8MB tuần tự + resume: rớt mạng/F5 xong gửi lại cùng file là chạy tiếp từ chỗ dở. */
  async upload(path: string, data: Blob, onProgress?: (done: number) => void, owner?: string) {
    const json = { "Content-Type": "application/json" };
    const up = isSecret(path) ? "/api/secret/uploads" : "/api/files/uploads";
    const rel = isSecret(path) ? sRel(path) : path;
    let id = "";
    let off = 0;
    // chỉ resume File thật (có mtime): cùng path+size+mtime mới coi là cùng nội dung
    const mtime = data instanceof File ? Math.floor(data.lastModified / 1000) : undefined;
    try {
      const a = isSecret(path) || mtime === undefined ? null : await this.uploadActive(rel, data.size, mtime, owner);
      if (a && a.received > 0 && a.received < data.size) {
        id = a.id;
        off = a.received;
        onProgress?.(off);
      }
    } catch {
      /* không có gì để resume */
    }
    if (!id) {
      const init = await req(up, {
        method: "POST",
        headers: json,
        body: JSON.stringify({
          path: rel,
          size: data.size,
          mtime,
          ...(owner ? { owner } : {}),
        }),
      });
      id = ((await init.json()) as { id: string }).id;
    }
    const CHUNK = 8 * 1024 * 1024;
    const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
    for (; off < data.size; ) {
      try {
        await req(`${up}/${id}?offset=${off}`, { method: "PUT", body: data.slice(off, off + CHUNK) });
        off = Math.min(off + CHUNK, data.size);
        onProgress?.(off);
      } catch (e) {
        const msg = String((e as Error)?.message ?? e);
        if (msg.includes("unauthorized")) throw e;
        // lệch offset hoặc lỗi mạng: hỏi server đang nhận tới đâu rồi gửi tiếp (tối đa 5 lần)
        let waited = 0;
        for (let i = 0; i < 5; i++) {
          await sleep(500 * (i + 1));
          try {
            const st = (await (await req(`${up}/${id}`)).json()) as { received: number };
            if (st.received !== off) {
              off = st.received;
              onProgress?.(off);
            }
            break;
          } catch {
            waited++;
          }
        }
        if (waited >= 5) throw e;
      }
    }
    await req(`${up}/${id}/complete`, { method: "POST" });
  },
  async readText(path: string) {
    const r = await req(fileUrl(path) + ownQ(path, "?"));
    return r.text();
  },
  /** Link tải cả thư mục thành zip (stream trực tiếp, không qua RAM). */
  zipDownloadUrl(path: string, owner?: string) {
    const t = token();
    return `${serverOrigin()}/api/zip/download?path=${encodeURIComponent(path)}&t=${encodeURIComponent(t ?? "")}${owner ? `&owner=${encodeURIComponent(owner)}` : ""}`;
  },
  /** Link tải trực tiếp (token 10 phút, chỉ cho file này): trình duyệt stream xuống đĩa, không qua RAM. */
  async directDownloadUrl(path: string): Promise<string> {
    if (isSecret(path)) return `${this.streamUrl(path)}&download=1`;
    const r = await req("/api/dl", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ path }) });
    const { token: t } = (await r.json()) as { token: string };
    return `${serverOrigin()}/api/files/${encPath(path)}?dl=${encodeURIComponent(t)}`;
  },
};

/** djb2 — đủ để phát hiện "md đã sửa" cho banner stale. */
export function hashText(s: string): string {
  let h = 5381;
  for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) | 0;
  return (h >>> 0).toString(16);
}

const audioCache = new Map<string, string>();

export async function speak(text: string, provider?: "auto" | "edge" | "google"): Promise<string> {
  // Qua backend proxy (/api/tts/*) để chạy được trên domain public,
  // không cần mở port 8016 ra ngoài.
  const key = hashText(text);
  const hit = audioCache.get(key);
  if (hit) return hit;
  const t = token();
  const r = await fetch(serverOrigin() + "/api/tts/synthesize", {
    method: "POST",
    headers: { "Content-Type": "application/json", ...(t ? { Authorization: `Bearer ${t}` } : {}) },
    body: JSON.stringify({ text, provider: provider ?? "auto" }),
  });
  if (!r.ok) throw new Error("tts failed");
  const url = URL.createObjectURL(await r.blob());
  audioCache.set(key, url);
  return url;
}
