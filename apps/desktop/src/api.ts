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

export interface UserRow {
  id: string;
  username: string;
  must_reset: boolean;
  is_admin: boolean;
  quota_limit: number;
  quota_used: number;
}

export interface Me {
  id: string;
  username: string;
  is_admin: boolean;
  quota_limit: number;
  quota_used: number;
}

const encPath = (p: string) => p.split("/").map(encodeURIComponent).join("/");

export function token(): string | null {
  return localStorage.getItem("nk-token");
}

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
  async me() {
    const r = await req("/api/me");
    return r.json() as Promise<Me>;
  },
  async remove(path: string) {
    await req(`/api/files/${encPath(path)}`, { method: "DELETE" });
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

export async function speak(text: string): Promise<string> {
  // Qua backend proxy (/api/tts/*) để chạy được trên domain public,
  // không cần mở port 8016 ra ngoài.
  const key = hashText(text);
  const hit = audioCache.get(key);
  if (hit) return hit;
  const t = token();
  const r = await fetch(serverOrigin() + "/api/tts/synthesize", {
    method: "POST",
    headers: { "Content-Type": "application/json", ...(t ? { Authorization: `Bearer ${t}` } : {}) },
    body: JSON.stringify({ text }),
  });
  if (!r.ok) throw new Error("tts failed");
  const url = URL.createObjectURL(await r.blob());
  audioCache.set(key, url);
  return url;
}
