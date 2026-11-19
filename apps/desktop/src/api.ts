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
    if (!r.ok) throw new Error("change password failed");
  },
  async files(prefix: string) {
    const r = await req(`/api/files?prefix=${encodeURIComponent(prefix)}`);
    return r.json() as Promise<{ name: string; is_dir: boolean; size: number }[]>;
  },
  async readText(path: string) {
    const r = await req(`/api/files/${path.split("/").map(encodeURIComponent).join("/")}`);
    return r.text();
  },
  async downloadUrl(path: string): Promise<string> {
    const r = await req(`/api/files/${path.split("/").map(encodeURIComponent).join("/")}`);
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
