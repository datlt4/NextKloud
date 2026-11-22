// Nhỏ gọn: client cho backend /api + TTS service. Không dep ngoài solid.
export function isSelfHostedWeb(): boolean {
  return window.location.protocol.startsWith("http");
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
  sub_total: number;
  sub_done: number;
}
export type KCardPatch = Partial<Pick<KCard, "title" | "description" | "column_id" | "position" | "labels" | "color" | "due_at" | "parent_id">> & { archived?: boolean };
export interface DeckPreview {
  id: number;
  title: string;
  archived: boolean;
  imported: boolean;
  stacks: { id: number; title: string; count: number; merge_into: string | null; is_done: boolean }[];
  cards: { id: number; title: string; stack_id: number; big: boolean; done: boolean; parent: number | null }[];
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

export function token(): string | null {
  return localStorage.getItem("nk-token");
}

const JSON_H = { "Content-Type": "application/json" };

async function req(path: string, init?: RequestInit) {
  const t = token();
  const r = await fetch(serverOrigin() + path, {
    ...init,
    headers: { ...(init?.headers || {}), ...(t ? { Authorization: `Bearer ${t}` } : {}) },
  });
  if (r.status === 401) {
    localStorage.removeItem("nk-token");
    throw new Error("unauthorized");
  }
  if (!r.ok) throw new Error(`${r.status} ${await r.text()}`);
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
  /** 1 trang của thư mục (server phân trang, thư mục luôn trước). */
  async files(prefix: string, o: ListOpts) {
    const q = new URLSearchParams({ prefix, sort: o.sort, desc: String(o.desc), offset: String(o.offset), limit: String(o.limit) });
    const r = await req(`/api/files?${q}`);
    return r.json() as Promise<Page>;
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
  async me() {
    const r = await req("/api/me");
    return r.json() as Promise<Me>;
  },
  /** Xoá = chuyển vào thùng rác. */
  async remove(path: string) {
    return (await req(`/api/files/${encPath(path)}`, { method: "DELETE" })).json() as Promise<TrashItem>;
  },
  async mkdir(path: string) {
    await req("/api/mkdir", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ path }) });
  },
  /** Ghi đè / tạo file text (dùng khi editor drawio/excalidraw lưu). */
  async saveText(path: string, text: string) {
    await this.upload(path, new Blob([text], { type: "text/plain;charset=utf-8" }));
  },
  /** URL stream trực tiếp (kèm ?t= cho <video>/<audio>/<img> không gửi được Bearer). */
  streamUrl(path: string) {
    const t = token();
    return `${serverOrigin()}/api/files/${encPath(path)}?t=${encodeURIComponent(t ?? "")}`;
  },
  thumbUrl(path: string, s = 256) {
    const t = token();
    return `${serverOrigin()}/api/thumb/${encPath(path)}?s=${s}&t=${encodeURIComponent(t ?? "")}`;
  },
  async officeConfig(path: string) {
    const r = await req("/api/office/config", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ path }) });
    return r.json() as Promise<{ config: any; token: string }>;
  },
  async zipList(path: string, limit = 20000) {
    const r = await req(`/api/zip/list?path=${encodeURIComponent(path)}&limit=${limit}`);
    return r.json() as Promise<{ entries: { name: string; size: number; is_dir: boolean }[]; total: number; truncated: boolean }>;
  },
  zipFileUrl(path: string, entry: string) {
    const t = token();
    return `${serverOrigin()}/api/zip/file?path=${encodeURIComponent(path)}&entry=${encodeURIComponent(entry)}&t=${encodeURIComponent(t ?? "")}`;
  },
  /** Đổi tên hoặc di chuyển (file/thư mục). */
  async move(from: string, to: string) {
    const r = await req("/api/move", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ from, to }) });
    return r.json() as Promise<{ files: number }>;
  },
  // --- bookmark + ghi chú ---
  async annotations(path: string) {
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
  async updateColumn(id: string, b: { title?: string; position?: number; is_done?: boolean }) {
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
  async deleteCard(id: string) {
    await req(`/api/cards/${id}`, { method: "DELETE" });
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
  /** Mục lục cả vault Obsidian chứa file (null nếu không thuộc vault nào). */
  async vaultOutline(path: string) {
    return (await req(`/api/vault/outline?path=${encodeURIComponent(path)}`)).json() as Promise<VaultOutline | null>;
  },
  async allNotes(state?: NoteState) {
    return (await req(`/api/annotations${state ? `?state=${state}` : ""}`)).json() as Promise<Annotation[]>;
  },
  async addAnnotation(b: { path: string; kind: "bookmark" | "note"; anchor: string; quote: string; body?: string }) {
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
  async trash() {
    return (await req("/api/trash")).json() as Promise<TrashItem[]>;
  },
  async config() {
    return (await req("/api/config")).json() as Promise<{ trash_days: number; version: string }>;
  },
  async restore(id: string) {
    await req(`/api/trash/${id}/restore`, { method: "POST" });
  },
  async purge(id: string) {
    await req(`/api/trash/${id}`, { method: "DELETE" });
  },
  async emptyTrash() {
    await req("/api/trash", { method: "DELETE" });
  },
  /** Ném lỗi nếu file không tồn tại (HEAD, không tải nội dung). */
  async exists(path: string) {
    await req(`/api/files/${encPath(path)}`, { method: "HEAD" });
  },
  async readBlob(path: string) {
    return (await req(`/api/files/${encPath(path)}`)).blob();
  },
  /** Chunked upload 8MB tuần tự — không giữ cả file trong RAM. */
  async upload(path: string, data: Blob, onProgress?: (done: number) => void) {
    const json = { "Content-Type": "application/json" };
    const init = await req("/api/files/uploads", {
      method: "POST",
      headers: json,
      body: JSON.stringify({
        path,
        size: data.size,
        mtime: data instanceof File ? Math.floor(data.lastModified / 1000) : undefined,
      }),
    });
    const { id } = (await init.json()) as { id: string };
    const CHUNK = 8 * 1024 * 1024;
    for (let off = 0; off < data.size; off += CHUNK) {
      await req(`/api/files/uploads/${id}?offset=${off}`, { method: "PUT", body: data.slice(off, off + CHUNK) });
      onProgress?.(Math.min(off + CHUNK, data.size));
    }
    await req(`/api/files/uploads/${id}/complete`, { method: "POST" });
  },
  async readText(path: string) {
    const r = await req(`/api/files/${encPath(path)}`);
    return r.text();
  },
  /** Link tải trực tiếp (token 10 phút, chỉ cho file này): trình duyệt stream xuống đĩa, không qua RAM. */
  async directDownloadUrl(path: string): Promise<string> {
    const r = await req("/api/dl", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ path }) });
    const { token: t } = (await r.json()) as { token: string };
    return `${serverOrigin()}/api/files/${encPath(path)}?dl=${encodeURIComponent(t)}`;
  },
  async downloadUrl(path: string): Promise<string> {
    const r = await req(`/api/files/${encPath(path)}`);
    const b = await r.blob();
    return URL.createObjectURL(b);
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
