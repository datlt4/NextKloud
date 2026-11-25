/** Hàng đợi upload: tạm dừng/tiếp tục từng file, giữ qua F5 (IndexedDB), file nhỏ gửi theo lô. */
import { createSignal } from "solid-js";
import { api, isSecret } from "./api";
import { errText } from "./fileops";

export type UpState = "queued" | "running" | "paused" | "missing" | "done" | "error";
export type UpItem = {
  id: string;
  path: string;
  rel: string;
  owner?: string;
  name: string;
  size: number;
  lastModified: number;
  file: File | null;
  state: UpState;
  sent: number;
  err?: string;
};
export type UpRow = UpItem & { pausable: boolean };

const SMALL = 1024 * 1024; // ≤ 1MB: gửi theo lô
const BATCH_FILES = 200;
const BATCH_BYTES = 8 * 1024 * 1024;
const KEEP_BLOB = 200 * 1024 * 1024; // lớn hơn: chỉ lưu metadata, sau F5 phải chọn lại file
const KEEP_TOTAL = 2 * 1024 * 1024 * 1024; // tổng bytes chép vào IndexedDB mỗi lần thêm
const WORKERS = 3;
const ROWS = 100;

let items: UpItem[] = [];
let uid = "";
let workers = 0;
let finished = false; // có file xong từ lần rảnh trước -> refresh danh sách
let stopped = false;
let closeTimer: number | undefined;
let rate = { t0: 0, b0: 0 };
const ctl = new Map<string, AbortController>();
let seq = 0;
const newId = () => `${Date.now().toString(36)}-${(seq++).toString(36)}-${Math.random().toString(36).slice(2, 8)}`;

let hooks = { drain: () => {}, authFail: (_e: unknown) => {} };
export const upHooks = (h: typeof hooks) => (hooks = h);

const [ver, setVer] = createSignal(0);
let tickT: number | undefined;
const touch = () => {
  if (tickT === undefined) tickT = window.setTimeout(() => ((tickT = undefined), setVer((v) => v + 1)), 200);
};

// --- IndexedDB: mọi lỗi đều bỏ qua, app vẫn chạy khi không có IDB ---
let dbp: Promise<IDBDatabase> | undefined;
const idb = () =>
  (dbp ??= new Promise<IDBDatabase>((ok, no) => {
    const r = indexedDB.open("nk-upq", 1);
    r.onupgradeneeded = () => r.result.createObjectStore("items", { keyPath: "id" });
    r.onsuccess = () => ok(r.result);
    r.onerror = () => no(r.error);
  }));
const tx = async (mode: IDBTransactionMode, fn: (s: IDBObjectStore) => void) => {
  const db = await idb();
  await new Promise<void>((ok, no) => {
    const t = db.transaction("items", mode);
    fn(t.objectStore("items"));
    t.oncomplete = () => ok();
    t.onerror = t.onabort = () => no(t.error);
  });
};
async function idbPut(xs: UpItem[]) {
  xs = xs.filter((x) => !isSecret(x.path)); // file Bí mật không lưu lại trên máy
  if (!uid || !xs.length) return;
  let room = KEEP_TOTAL;
  const rec = (x: UpItem, blob: boolean) => {
    const keep = blob && !!x.file && x.size <= KEEP_BLOB && (room -= x.size) >= 0;
    return { id: x.id, uid, path: x.path, rel: x.rel, owner: x.owner, name: x.name, size: x.size, lastModified: x.lastModified, file: keep ? x.file : null };
  };
  try {
    await tx("readwrite", (s) => xs.forEach((x) => s.put(rec(x, true))));
  } catch {
    try {
      // hết chỗ chứa blob: chỉ giữ metadata
      await tx("readwrite", (s) => xs.forEach((x) => s.put(rec(x, false))));
    } catch {
      /* không có IDB */
    }
  }
}
async function idbDel(ids: string[]) {
  if (!ids.length) return;
  try {
    await tx("readwrite", (s) => ids.forEach((id) => s.delete(id)));
  } catch {
    /* không có IDB */
  }
}

/** Sau đăng nhập: nạp hàng đợi còn dở của user này (tạm dừng; file lớn cần chọn lại). */
export async function upRestore(user: string) {
  if (uid && uid !== user) {
    // đổi tài khoản: bỏ hàng đợi của người trước khỏi RAM (IDB vẫn giữ)
    ctl.forEach((c) => c.abort());
    items = [];
  }
  uid = user;
  let recs: (Omit<UpItem, "state" | "sent"> & { uid: string })[] = [];
  try {
    const db = await idb();
    recs = await new Promise((ok, no) => {
      const r = db.transaction("items").objectStore("items").getAll();
      r.onsuccess = () => ok(r.result);
      r.onerror = () => no(r.error);
    });
  } catch {
    return;
  }
  const have = new Set(items.map((x) => x.id));
  for (const r of recs) {
    if (r.uid !== user || have.has(r.id)) continue;
    const file = r.file instanceof File ? r.file : null;
    items.push({ id: r.id, path: r.path, rel: r.rel, owner: r.owner, name: r.name, size: r.size, lastModified: r.lastModified, file, state: file ? "paused" : "missing", sent: 0 });
  }
  touch();
}

export function upEnqueue(jobs: { file: File; path: string; rel: string; owner?: string }[]) {
  if (!jobs.length) return;
  clearTimeout(closeTimer);
  stopped = false;
  // panel chỉ còn file xong: bắt đầu lô mới
  if (items.every((x) => x.state === "done")) items = [];
  const add = jobs.map((j): UpItem => ({ id: newId(), path: j.path, rel: j.rel, owner: j.owner, name: j.file.name, size: j.file.size, lastModified: j.file.lastModified, file: j.file, state: "queued", sent: 0 }));
  items.push(...add);
  void idbPut(add);
  pump();
}

const small = (x: UpItem) => x.size <= SMALL && !isSecret(x.path);

/** Việc tiếp theo: 1 lô file nhỏ cùng owner, hoặc 1 file lớn. */
function take(): UpItem[] | UpItem | null {
  const first = items.find((x) => x.state === "queued" && x.file);
  if (!first) return null;
  if (!small(first)) return first;
  const out: UpItem[] = [];
  let bytes = 0;
  for (const x of items) {
    if (x.state !== "queued" || !x.file || !small(x) || x.owner !== first.owner) continue;
    if (out.length >= BATCH_FILES || bytes + x.size > BATCH_BYTES) break;
    out.push(x);
    bytes += x.size;
  }
  return out;
}

function pump() {
  if (!workers) rate = { t0: Date.now(), b0: sentBytes() };
  while (workers < WORKERS && items.some((x) => x.state === "queued" && x.file)) {
    workers++;
    void worker();
  }
  touch();
}

async function worker() {
  for (let job = take(); job; job = take()) await (Array.isArray(job) ? runBatch(job) : runOne(job));
  workers--;
  touch();
  if (workers) return;
  if (finished) {
    finished = false;
    hooks.drain();
  }
  // xong hết, không lỗi: tự đóng sau vài giây
  if (items.length && items.every((x) => x.state === "done")) closeTimer = window.setTimeout(upClose, 4000);
}

const done = (xs: UpItem[]) => {
  for (const x of xs) {
    x.state = "done";
    x.sent = x.size;
    x.file = null;
  }
  finished = true;
  void idbDel(xs.map((x) => x.id));
};
const isAuth = (e: unknown) => String((e as Error)?.message).includes("unauthorized");
const failed = (x: UpItem, e: unknown) => {
  x.state = "error";
  x.err = typeof e === "string" ? e : errText(e);
};
// hết phiên đăng nhập: tạm dừng tất cả, giữ hàng đợi
const authStop = (xs: UpItem[], e: unknown) => {
  xs.forEach((x) => x.state === "running" && (x.state = "paused"));
  upPauseAll();
  hooks.authFail(e);
};

async function runOne(x: UpItem) {
  const ac = new AbortController();
  ctl.set(x.id, ac);
  x.state = "running";
  x.err = undefined;
  touch();
  try {
    await api.upload(x.path, x.file!, (d) => ((x.sent = d), touch()), x.owner, ac.signal);
    done([x]);
  } catch (e) {
    if (ac.signal.aborted) {
      /* tạm dừng / huỷ */
    } else if (isAuth(e)) authStop([x], e);
    else failed(x, e);
  }
  ctl.delete(x.id);
  touch();
}

async function runBatch(xs: UpItem[]) {
  xs.forEach((x) => ((x.state = "running"), (x.err = undefined)));
  touch();
  try {
    const res = await api.uploadBatch(xs.map((x) => ({ path: x.path, data: x.file! })), xs[0].owner);
    done(xs.filter((_, i) => res[i]?.ok));
    xs.forEach((x, i) => !res[i]?.ok && failed(x, res[i]?.error ?? "?"));
  } catch (e) {
    if (isAuth(e)) authStop(xs, e);
    else xs.forEach((x) => x.state === "running" && failed(x, e));
  }
  touch();
}

const sentBytes = () => items.reduce((a, x) => a + (x.state === "done" ? x.size : x.sent), 0);

export function upPause(x: UpItem) {
  const it = items.find((y) => y.id === x.id);
  if (!it || (it.state !== "queued" && !ctl.has(it.id))) return;
  it.state = "paused";
  ctl.get(it.id)?.abort();
  touch();
}
export function upResume(x: UpItem) {
  const it = items.find((y) => y.id === x.id);
  if (it && (it.state === "paused" || it.state === "error") && it.file) {
    it.state = "queued";
    pump();
  }
}
export function upPauseAll() {
  for (const x of items) if (x.state === "queued" || ctl.has(x.id)) x.state = "paused";
  ctl.forEach((c) => c.abort());
  touch();
}
export function upResumeAll() {
  items.forEach((x) => x.state === "paused" && x.file && (x.state = "queued"));
  pump();
}
export function upRetryFails() {
  items.forEach((x) => x.state === "error" && x.file && ((x.state = "queued"), (x.sent = 0)));
  pump();
}
/** Bỏ 1 file khỏi hàng đợi (đang chạy thì huỷ). */
export function upCancel(x: UpItem) {
  ctl.get(x.id)?.abort();
  items = items.filter((y) => y.id !== x.id);
  void idbDel([x.id]);
  touch();
}
/** Dừng hẳn: huỷ file đang tải, bỏ mọi file chưa xong. */
export function upStopAll() {
  ctl.forEach((c) => c.abort());
  const drop = items.filter((x) => x.state !== "done" && x.state !== "error");
  items = items.filter((x) => x.state === "done" || x.state === "error");
  stopped = true;
  void idbDel(drop.map((x) => x.id));
  touch();
}
/** Đóng panel: chỉ khi không còn gì chờ; bỏ cả file lỗi. */
export function upClose() {
  clearTimeout(closeTimer);
  if (items.some((x) => x.state !== "done" && x.state !== "error")) return;
  void idbDel(items.filter((x) => x.state === "error").map((x) => x.id));
  items = [];
  stopped = false;
  touch();
}

/** Chọn lại file lớn mất sau F5: khớp theo tên + size + lastModified rồi chạy tiếp. Trả về [khớp, còn thiếu]. */
export function upReselect(): Promise<[number, number]> {
  return new Promise((ok) => {
    const inp = document.createElement("input");
    inp.type = "file";
    inp.multiple = true;
    inp.onchange = () => {
      let n = 0;
      for (const f of Array.from(inp.files ?? []))
        for (const x of items)
          if (x.state === "missing" && x.name === f.name && x.size === f.size && x.lastModified === f.lastModified) {
            x.file = f;
            x.state = "queued";
            n++;
          }
      pump();
      ok([n, items.filter((x) => x.state === "missing").length]);
    };
    inp.click();
  });
}

/** Ảnh chụp cho panel (cập nhật ~5 lần/giây). */
export function upView() {
  ver();
  if (!items.length) return null;
  let ok = 0, fails = 0, paused = 0, missing = 0, queued = 0, bytes = 0, left = 0;
  const cur: string[] = [];
  const rows: UpRow[] = [];
  for (const x of items) {
    bytes += x.size;
    if (x.state === "done") ok++;
    else if (x.state === "error") fails++;
    else if (x.state === "paused") paused++;
    else if (x.state === "missing") missing++;
    else {
      queued++;
      left += x.size - x.sent;
      if (x.state === "running" && cur.length < 3) cur.push(x.rel);
    }
    // liệt kê file lớn đang chạy/chờ + mọi file tạm dừng/thiếu/lỗi (lô file nhỏ chỉ đếm)
    const big = x.state === "running" ? ctl.has(x.id) : x.state === "queued" ? !small(x) : x.state !== "done";
    if (big && rows.length < ROWS) rows.push({ ...x, pausable: x.state === "queued" || ctl.has(x.id) });
  }
  const sent = sentBytes();
  const speed = workers ? (sent - rate.b0) / Math.max((Date.now() - rate.t0) / 1000, 0.5) : 0;
  return {
    total: items.length,
    ok,
    fails,
    paused,
    missing,
    pending: queued + paused + missing,
    running: workers > 0,
    stopped,
    cur: cur.join(" · "),
    pct: bytes ? Math.round((sent / bytes) * 100) : 100,
    speed,
    eta: speed > 0 ? left / speed : 0,
    rows,
    more: Math.max(0, queued + paused + missing + fails - rows.length),
  };
}
