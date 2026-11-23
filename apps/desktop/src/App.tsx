import { For, Index, Match, Show, Switch, batch, createEffect, createMemo, createSignal, lazy, onCleanup, onMount, Suspense } from "solid-js";
import { SECRET_ROOT, api, isSecret, isSelfHostedWeb, serverOrigin, vaultSeen, type Entry, type Me } from "./api";
import { syncFromServer, ui, wallCss } from "./background";
import { ResetForm } from "./forms";
import { LookDialog } from "./look";
import { fmtSize } from "./format";
import { fmtDate as fdate, fmtNum, t } from "./i18n";
import { Icon, ext, kindOf } from "./icons";
import Settings, { type SettingsTab } from "./Settings";
import { Avatar } from "./avatar";
import MarkdownView from "./viewers/MarkdownView";
import { viewerKind, hasThumb, type VKind } from "./viewers/kinds";
/** Server vừa cài bản mới thì file code cũ của trang đang mở không còn: tải lại trang (1 lần/phút) thay vì treo "Đang mở…". */
function reloadOnStale<T>(p: Promise<T>): Promise<T> {
  return p.catch((e) => {
    let last = 0;
    try {
      last = +(sessionStorage.getItem("nk-stale-reload") ?? 0);
      if (Date.now() - last > 60_000) {
        sessionStorage.setItem("nk-stale-reload", String(Date.now()));
        location.reload();
        return new Promise<T>(() => {});
      }
    } catch {}
    throw e;
  });
}
const TextView = lazy(() => reloadOnStale(import("./viewers/TextView")));
const ImageView = lazy(() => reloadOnStale(import("./viewers/ImageView")));
const VideoView = lazy(() => reloadOnStale(import("./viewers/VideoView")));
const PdfView = lazy(() => reloadOnStale(import("./viewers/PdfView")));
const ZipView = lazy(() => reloadOnStale(import("./viewers/ZipView")));
const EpubView = lazy(() => reloadOnStale(import("./viewers/EpubView")));
const DrawioView = lazy(() => reloadOnStale(import("./viewers/DrawioView")));
const ExcalidrawView = lazy(() => reloadOnStale(import("./viewers/ExcalidrawView")));
const OfficeView = lazy(() => reloadOnStale(import("./viewers/OfficeView")));

/** Thumbnail server cho tile grid; lỗi thì ẩn, giữ icon mặc định. */
function ThumbImg(props: { path: string; name: string }) {
  const [ok, setOk] = createSignal(true);
  return (
    <Show when={ok() && hasThumb(props.name)}>
      <img class="tile-thumb" loading="lazy" decoding="async" src={api.thumbUrl(props.path)} alt="" onError={() => setOk(false)} />
    </Show>
  );
}
import { MoveDialog, NameDialog, TrashPage, errText, joinPath } from "./fileops";
import { NotesPage } from "./annot";
import { KanbanPage } from "./kanban";
import { TrackerPage } from "./tracker";
import { PublicPage, ShareDialog, SharedPage } from "./share";
import { SecretGate, SecretSettings } from "./secret";

type SortKey = "name" | "size" | "modified";

const fmtDate = (s: number) => (s ? fdate(s) : "");
const parentOf = (p: string) => p.split("/").slice(0, -1).join("/");
/** Tên thư mục để hiện (gốc khu Bí mật hiện là "Bí mật"). */
const pretty = (p: string) => (isSecret(p) ? [t("f.secret"), ...p.split("/").slice(1)].join("/") : p || t("f.allFiles"));
const escHtml = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
/** Đoạn trích kết quả tìm nội dung: tô đậm chỗ khớp (không phân biệt hoa thường). */
const highlight = (snip: string, q: string) => {
  const t = q.trim();
  const flat = snip.replace(/\s+/g, " ");
  const i = t ? flat.toLowerCase().indexOf(t.toLowerCase()) : -1;
  return i < 0 ? escHtml(flat) : `${escHtml(flat.slice(0, i))}<mark>${escHtml(flat.slice(i, i + t.length))}</mark>${escHtml(flat.slice(i + t.length))}`;
};

// Danh sách ảo: chỉ render dòng đang thấy, tải từng trang PAGE dòng từ server.
const PAGE = 200;
const ROW_H = 48;

export default function App() {
  const [server, setServer] = createSignal(serverOrigin());
  const [username, setUsername] = createSignal("");
  const [password, setPassword] = createSignal("");
  const [authed, setAuthed] = createSignal(!!localStorage.getItem("nk-token"));
  const [me, setMe] = createSignal<Me | null>(null);
  const [needReset, setNeedReset] = createSignal(false);
  const [err, setErr] = createSignal("");
  const [prefix, setPrefix] = createSignal("");
  const [stats, setStats] = createSignal({ dirs: 0, files: 0, bytes: 0 });
  const [ver, setVer] = createSignal(0); // tăng mỗi khi cache trang thay đổi
  const [results, setResults] = createSignal<Entry[] | null>(null); // chế độ tìm kiếm
  const [scrollTop, setScrollTop] = createSignal(0);
  const [viewH, setViewH] = createSignal(800);
  const [viewW, setViewW] = createSignal(800);
  const [tileH, setTileH] = createSignal(0);
  const [loading, setLoading] = createSignal(false);
  const [open, setOpen] = createSignal<{ path: string; name: string; kind: VKind; text?: string; anchor?: string | null; mark?: string } | null>(null);
  const [query, setQuery] = createSignal("");
  const [sort, setSort] = createSignal<{ key: SortKey; asc: boolean }>({ key: "name", asc: true });
  const [view, setView] = createSignal<"list" | "grid">((localStorage.getItem("nk-view") as "grid") || "list");
  const [sideOpen, setSideOpen] = createSignal(false);
  const [menuFor, setMenuFor] = createSignal<string | null>(null);
  const [shareFor, setShareFor] = createSignal<{ path: string } | null>(null);
  const [menuUpFor, setMenuUpFor] = createSignal<string | null>(null);
  const [menuLeftFor, setMenuLeftFor] = createSignal<string | null>(null);
  const [userMenu, setUserMenu] = createSignal(false);
  const [userMenuUp, setUserMenuUp] = createSignal(false);
  const [mSearch, setMSearch] = createSignal(false);
  const [showLook, setShowLook] = createSignal(false);
  const [page, setPage] = createSignal<"files" | "settings" | "trash" | "notes" | "kanban" | "tasks" | "shared" | "secret" | "secret-trash">("files");
  const [secretSet, setSecretSet] = createSignal(false);
  const [vaultLeft, setVaultLeft] = createSignal(0);
  const inSecret = () => isSecret(prefix()) || page() === "secret-trash";
  const [boardId, setBoardId] = createSignal<string | null>(null);
  type Dlg = { kind: "mkdir" } | { kind: "rename"; f: Entry } | { kind: "move"; paths: string[] } | { kind: "copy"; paths: string[] };
  const [dlg, setDlg] = createSignal<Dlg | null>(null);
  const [sel, setSel] = createSignal<Set<string>>(new Set());
  const [notice, setNotice] = createSignal<{ text: string; undo?: () => Promise<void> } | null>(null);
  const [working, setWorking] = createSignal<string | null>(null);
  let lastIdx = -1;
  let noticeTimer: number | undefined;
  const flash = (text: string, undo?: () => Promise<void>) => {
    clearTimeout(noticeTimer);
    setNotice({ text, undo });
    noticeTimer = window.setTimeout(() => setNotice(null), 8000);
  };
  const [setTab, setSetTab] = createSignal<SettingsTab>("profile");
  const openSettings = (t: SettingsTab) => {
    setSetTab(t);
    setPage("settings");
    setSideOpen(false);
    setUserMenu(false);
  };
  const shownName = () => me()?.display_name || me()?.username || "";
  const [upload, setUpload] = createSignal<{ name: string; pct: number; left: number } | null>(null);
  type UpJob = { file: File; rel: string; base: string };
  const [upStat, setUpStat] = createSignal<{ total: number; done: number; ok: number; fails: (UpJob & { err: string })[]; running: boolean } | null>(null);
  let upCancel = false;
  const [upRate, setUpRate] = createSignal<{ speed: number; eta: number } | null>(null);
  const fmtEta = (s: number) => (s >= 3600 ? t("f.etaHMS", { h: Math.floor(s / 3600), m: Math.round((s % 3600) / 60) }) : s >= 60 ? t("f.etaMS", { m: Math.floor(s / 60), s: Math.round(s % 60) }) : t("f.etaS", { s: Math.max(1, Math.round(s)) }));
  let upCloseTimer: number | undefined;
  let fileInput!: HTMLInputElement;
  let dirInput!: HTMLInputElement;
  const [dropping, setDropping] = createSignal(false);
  let scrollEl: HTMLDivElement | undefined;
  let vbox: HTMLDivElement | undefined;
  let pages = new Map<number, Entry[]>();
  let inflight = new Set<number>();
  let gen = 0;

  const join = (name: string) => (prefix() ? `${prefix()}/${name}` : name);

  const logout = () => {
    api.secretLock();
    localStorage.removeItem("nk-token");
    setAuthed(false);
    setOpen(null);
    setMe(null);
    setUserMenu(false);
    setPage("files");
  };

  // Đo chỗ trống dưới nút bấm: còn < ~220px thì mở menu ngược lên trên,
  // tránh bị cắt khỏi màn hình (viewport ngắn, mobile ngang...).
  const flipIfNeeded = (el: HTMLElement | null) => {
    if (!el) return false;
    return window.innerHeight - el.getBoundingClientRect().bottom < 220;
  };

  const fail = (e: any, what: string) => {
    if (String(e?.message).includes("unauthorized")) logout();
    else setErr(`${what}: ${String(e?.message ?? e)}`);
  };

  const listOpts = (p: number) => {
    const { key, asc } = sort();
    return { sort: key, desc: !asc, offset: p * PAGE, limit: PAGE };
  };

  /** Tải các trang `need` của `pre`, thay cache 1 lần (giữ nội dung cũ tới khi có dữ liệu mới). */
  const fetchSet = async (pre: string, need: number[]) => {
    const g = ++gen;
    const got = await Promise.all(need.map((p) => api.files(pre, listOpts(p))));
    if (g !== gen) return false;
    pages = new Map(need.map((p, i) => [p, got[i].entries]));
    inflight = new Set();
    // Đổi prefix cùng lúc với cache: effect tải trang kế không được thấy prefix cũ.
    batch(() => {
      setPrefix(pre);
      setStats({ dirs: got[0].dirs, files: got[0].files, bytes: got[0].bytes });
      setVer((v) => v + 1);
    });
    return true;
  };

  const fetchPage = async (p: number) => {
    if (pages.has(p) || inflight.has(p)) return;
    const g = gen;
    inflight.add(p);
    try {
      const r = await api.files(prefix(), listOpts(p));
      if (g !== gen) return;
      pages.set(p, r.entries);
      setStats({ dirs: r.dirs, files: r.files, bytes: r.bytes });
      setVer((v) => v + 1);
    } catch (e) {
      if (g === gen) fail(e, t("f.loadList"));
    } finally {
      if (g === gen) inflight.delete(p);
    }
  };

  const load = async (p: string) => {
    setErr("");
    setLoading(true);
    setPage("files"); // ngay lúc bấm: tải xong mới đổi sẽ kéo user ra khỏi trang khác
    try {
      if (!(await fetchSet(p, [0]))) return;
      setSel(new Set<string>());
      setQuery("");
      setResults(null);
      setOpen(null);
      setSideOpen(false);
      if (scrollEl) scrollEl.scrollTop = 0;
      setScrollTop(0);
    } catch (e) {
      fail(e, t("f.loadList"));
    } finally {
      setLoading(false);
    }
  };

  const refreshMe = () => api.me().then(setMe).catch(() => {});
  const [srvVer, setSrvVer] = createSignal("");
  const refreshVer = () => api.config().then((c) => setSrvVer(c.version ?? "")).catch(() => {});

  // --- khu Bí mật ---
  const openSecret = async () => {
    setSideOpen(false);
    try {
      const st = await api.secretStatus();
      if (st.unlocked) return load(SECRET_ROOT);
    } catch (e) {
      return fail(e, t("f.secretErr"));
    }
    setOpen(null);
    setPage("secret");
  };
  /** Khoá: bỏ danh sách đang hiện (có tên file bí mật) rồi về cổng nhập PIN. */
  const lockUi = async () => {
    setOpen(null);
    setSecretSet(false);
    await load("");
    setPage("secret");
  };
  const lockSecret = async () => {
    await api.secretLock();
    await lockUi();
  };
  onMount(() => {
    const onLocked = () => {
      if (inSecret() || isSecret(open()?.path ?? "")) lockUi();
    };
    window.addEventListener("nk-vault-locked", onLocked);
    // đếm ngược tự khoá: mỗi lần gọi API vault thì đặt lại; 15s hỏi server 1 lần cho khớp
    let seen = 0;
    let n = 0;
    const tick = window.setInterval(async () => {
      if (!inSecret()) return;
      if (vaultSeen !== seen) {
        seen = vaultSeen;
        setVaultLeft(600);
      } else setVaultLeft((l) => Math.max(0, l - 1));
      if (++n % 15 === 0 || vaultLeft() === 0) {
        const st = await api.secretStatus().catch(() => null);
        if (st && !st.unlocked) onLocked();
        else if (st) setVaultLeft(st.left);
      }
    }, 1000);
    onCleanup(() => {
      window.removeEventListener("nk-vault-locked", onLocked);
      clearInterval(tick);
    });
  });

  // URL giữ vị trí (thư mục / file đang mở / trang): F5 hay gửi link vẫn về đúng chỗ, nút Back của trình duyệt dùng được.
  const encP = (p: string) => p.split("/").map(encodeURIComponent).join("/");
  const routeOf = () => {
    const pg = page();
    if (pg === "settings") return `#/settings/${setTab()}`;
    if (pg === "kanban") return boardId() ? `#/kanban/${boardId()}` : "#/kanban";
    // khu Bí mật: không đưa tên file/thư mục vào URL + lịch sử trình duyệt
    if (pg === "secret" || pg === "secret-trash" || isSecret(prefix()) || isSecret(open()?.path ?? "")) return "#/secret";
    if (pg !== "files") return `#/${pg}`;
    const o = open();
    if (o) return `#/o/${encP(o.path)}`;
    return prefix() ? `#/f/${encP(prefix())}` : "#/";
  };
  let routing = false;
  const [routed, setRouted] = createSignal(false); // chưa đọc URL lúc mở app thì chưa ghi đè nó
  createEffect(() => {
    if (!authed() || !routed()) return;
    const h = routeOf();
    if (!routing && location.hash !== h && !(h === "#/" && !location.hash)) history.pushState(null, "", h);
  });
  const go = async (h: string) => {
    routing = true;
    try {
      const m = /^#\/(f|o|notes|trash|tasks|shared|settings|kanban|secret)(?:\/(.*))?$/.exec(h);
      const rest = m?.[2] ? decodeURIComponent(m[2]) : "";
      if (m?.[1] === "secret") await openSecret();
      else if (m?.[1] === "f") await load(rest);
      else if (m?.[1] === "o" && rest) await openAt(rest, "");
      else if (m?.[1] === "notes" || m?.[1] === "trash" || m?.[1] === "tasks" || m?.[1] === "shared") setPage(m[1]);
      else if (m?.[1] === "kanban") {
        setBoardId(rest || null);
        setPage("kanban");
      } else if (m?.[1] === "settings") openSettings((rest || "profile") as SettingsTab);
      else await load("");
    } finally {
      routing = false;
      setRouted(true);
    }
  };
  onMount(() => {
    const pop = () => authed() && go(location.hash);
    window.addEventListener("popstate", pop);
    onCleanup(() => window.removeEventListener("popstate", pop));
  });

  const enter = () => {
    setAuthed(true);
    go(location.hash);
    refreshMe();
    refreshVer();
    syncFromServer();
  };

  onMount(() => {
    if (authed()) enter();
    // Solid delegate click ở document -> stopPropagation không chặn được, check target.
    const close = (e: MouseEvent) => {
      if ((e.target as Element).closest?.(".menu-wrap")) return;
      setMenuFor(null);
      setMenuUpFor(null);
      setUserMenu(false);
    };
    document.addEventListener("click", close);
    onCleanup(() => document.removeEventListener("click", close));
  });

  let loginPw = "";
  const doLogin = async () => {
    setErr("");
    try {
      localStorage.setItem("nk-server", server());
      const r = await api.login(username(), password());
      localStorage.setItem("nk-token", r.token);
      if (r.must_reset) {
        loginPw = password(); // vừa đăng nhập đúng -> khỏi bắt gõ lại "mật khẩu hiện tại"
        setNeedReset(true);
      }
      else enter();
    } catch {
      setErr(t("login.fail"));
    }
  };

  const sortedResults = createMemo(() => {
    const r = results();
    if (!r) return null;
    const { key, asc } = sort();
    const dir = asc ? 1 : -1;
    return r.slice().sort((a, b) => {
      if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
      const d = key === "name" ? a.name.localeCompare(b.name, undefined, { numeric: true }) : a[key] - b[key];
      return d * dir;
    });
  });
  const total = () => sortedResults()?.length ?? stats().dirs + stats().files;
  const entryAt = (i: number): Entry | undefined => {
    ver();
    const r = sortedResults();
    return r ? r[i] : pages.get(Math.floor(i / PAGE))?.[i % PAGE];
  };

  const small = () => window.innerWidth <= 600; // khớp media query trong style.css
  const gap = () => (small() ? 6 : 10);
  const cols = () => (view() === "grid" ? Math.max(1, Math.floor((viewW() + gap()) / ((small() ? 120 : 150) + gap()))) : 1);
  const rowH = () => {
    if (view() === "list") return ROW_H;
    const w = (viewW() - gap() * (cols() - 1)) / cols();
    return tileH() || Math.ceil((w - 16) * 0.75 + 66) + gap();
  };
  const nRows = () => Math.ceil(total() / cols());
  const range = createMemo(() => {
    const h = rowH();
    const a = Math.max(0, Math.floor(scrollTop() / h) - 4);
    const b = Math.min(nRows(), Math.ceil((scrollTop() + viewH()) / h) + 4);
    return [a, Math.max(a, b)] as const;
  });
  const visibleRows = createMemo(() => {
    const [a, b] = range();
    return Array.from({ length: b - a }, (_, k) => a + k);
  });
  const pagesInView = () => {
    const [a, b] = range();
    const c = cols();
    const first = Math.floor((a * c) / PAGE);
    const last = Math.floor(Math.max(a * c, b * c - 1) / PAGE);
    return Array.from({ length: last - first + 1 }, (_, k) => first + k).filter((p) => p * PAGE < Math.max(total(), 1));
  };

  // Kéo thanh cuộn nhanh qua 1 triệu dòng: chỉ tải khi dừng ~80ms, kèm trang kế tiếp.
  let fetchTimer: number | undefined;
  createEffect(() => {
    if (results()) return;
    const need = pagesInView();
    ver();
    clearTimeout(fetchTimer);
    if (need.every((p) => pages.has(p))) {
      const next = need[need.length - 1] + 1;
      if (next * PAGE < total()) fetchPage(next);
      return;
    }
    fetchTimer = window.setTimeout(() => need.forEach(fetchPage), 80);
  });

  /** Tải lại trang đang xem (sau xoá/upload) — giữ nguyên vị trí cuộn. */
  const refresh = async () => {
    try {
      await fetchSet(prefix(), pagesInView());
    } catch (e) {
      fail(e, t("f.loadList"));
    }
  };

  const ro = new ResizeObserver(() => {
    if (!scrollEl) return;
    setViewH(scrollEl.clientHeight);
    setViewW(scrollEl.clientWidth - 16); // padding 8px 2 bên
    measureTile();
  });
  onCleanup(() => ro.disconnect());
  const measureTile = () =>
    requestAnimationFrame(() => {
      const t = scrollEl?.querySelector<HTMLElement>(".vgrid .tile:not(.skel)");
      if (t) setTileH(t.offsetHeight + gap());
    });
  createEffect(() => {
    if (view() === "grid" && ver() >= 0) measureTile();
  });
  let rafPending = false;
  const onScroll = () => {
    if (rafPending) return;
    rafPending = true;
    requestAnimationFrame(() => {
      rafPending = false;
      if (scrollEl && vbox) setScrollTop(Math.max(0, scrollEl.scrollTop - vbox.offsetTop));
    });
  };

  // Tìm trên toàn bộ file (server), gõ xong 250ms mới gửi.
  let searchTimer: number | undefined;
  const [byContent, setByContent] = createSignal(false);
  createEffect(() => {
    const q = query().trim();
    byContent();
    clearTimeout(searchTimer);
    if (q.length < 2) {
      setResults(null);
      return;
    }
    searchTimer = window.setTimeout(async () => {
      try {
        const r = await (byContent() ? api.searchContent(q) : api.search(q));
        if (query().trim() !== q) return;
        setResults(r);
        if (scrollEl) scrollEl.scrollTop = 0;
        setScrollTop(0);
      } catch (e) {
        fail(e, t("search.err"));
      }
    }, 250);
  });

  const crumbs = createMemo(() => prefix().split("/").filter(Boolean));

  const toggleSort = (key: SortKey) => {
    setSort((s) => ({ key, asc: s.key === key ? !s.asc : true }));
    if (!results()) {
      if (scrollEl) scrollEl.scrollTop = 0;
      setScrollTop(0);
      fetchSet(prefix(), [0]).catch((e) => fail(e, t("f.loadList")));
    }
  };

  /** Mở file tại 1 vị trí annotation ("b12" đoạn md, "p3" trang pdf), từ trang Ghi chú. */
  const openAt = async (path: string, mark: string) => {
    await load(parentOf(path));
    const name = path.split("/").pop() ?? path;
    const kind = viewerKind(name);
    try {
      if (kind === "markdown") setOpen({ path, name, kind, text: await api.readText(path), mark });
      else setOpen({ path, name, kind, mark });
    } catch (e) {
      fail(e, t("f.openFile"));
    }
  };

  const openEntry = async (f: Entry) => {
    if (f.is_dir) return load(f.path);
    // kết quả tìm theo nội dung: mở tới đoạn khớp
    if (f.snippet !== undefined) return openAt(f.path, `q:${query().trim()}`);
    const path = f.path;
    const name = f.name;
    setErr("");
    try {
      const kind = viewerKind(name);
      if (kind === "markdown") setOpen({ path, name, kind, text: await api.readText(path) });
      else if (kind === "other") download(path, name);
      else setOpen({ path, name, kind });
    } catch (e) {
      fail(e, t("f.openFile"));
    }
  };

  /** Ảnh cùng thư mục (để lightbox vuốt qua lại). Ưu tiên cache trang đang xem. */
  const siblingImages = (path: string) => {
    const all: Entry[] = results()
      ? sortedResults()!
      : [...pages.keys()].sort((a, b) => a - b).flatMap((p) => pages.get(p) ?? []);
    const imgs = all.filter((e) => !e.is_dir && viewerKind(e.name) === "image");
    let idx = imgs.findIndex((e) => e.path === path);
    if (idx < 0) {
      return { files: [{ path, name: path.split("/").pop() ?? path }], index: 0 };
    }
    return { files: imgs.map((e) => ({ path: e.path, name: e.name })), index: idx };
  };

  /** Mở wikilink Obsidian [[note|alias]] / [[note#head]]: thử đường dẫn tương đối
   *  trước, rồi tìm theo tên file trong cả vault (giống Obsidian). */
  const normPath = (parts: string[]): string | null => {
    const out: string[] = [];
    for (const seg of parts) {
      if (!seg || seg === ".") continue;
      if (seg === "..") {
        if (!out.length) return null;
        out.pop();
      } else out.push(seg);
    }
    return out.join("/");
  };
  const openWikiLink = async (target: string, anchor?: string) => {
    const tt = target.trim();
    if (!tt) {
      if (anchor) scrollToAnchor(anchor);
      return;
    }
    setErr("");
    // breadcrumb theo thư mục của file vừa mở
    const show = async (o: NonNullable<ReturnType<typeof open>>) => {
      if (parentOf(o.path) !== prefix()) await load(parentOf(o.path));
      setOpen(o);
    };
    const cur = open()?.path ?? "";
    const dir = cur.includes("/") ? cur.slice(0, cur.lastIndexOf("/")) : "";
    const cands: string[] = [];
    const pushCand = (p: string | null) => {
      if (!p) return;
      cands.push(p);
      if (!/\.[a-zA-Z0-9]{1,5}$/.test(p)) cands.push(`${p}.md`);
    };
    if (tt.startsWith("/")) pushCand(normPath(tt.slice(1).split("/")));
    else {
      pushCand(normPath([...(dir ? dir.split("/") : []), ...tt.split("/")]));
      const base = tt.split("/").pop()!;
      if (base !== tt && !tt.startsWith("./") && !tt.startsWith("../")) {
        pushCand(normPath([...(dir ? dir.split("/") : []), base]));
      }
    }
    for (const c of [...new Set(cands)]) {
      try {
        const name = c.split("/").pop() ?? c;
        const kind = viewerKind(name);
        if (kind === "markdown") {
          const text = await api.readText(c);
          await show({ path: c, name, kind, text, anchor: anchor ?? null });
        } else if (kind !== "other") {
          await api.exists(c); // không có -> ném lỗi, thử ứng viên kế
          await show({ path: c, name, kind });
        } else continue;
        return;
      } catch {
        /* thử ứng viên tiếp theo */
      }
    }
    // fallback: tìm theo tên file trong cả vault (giống Obsidian resolve by basename)
    try {
      const base = tt.split("/").pop()!.replace(/\.md$/i, "");
      const hits = (await api.search(base)).filter(
        (e) => !e.is_dir && (e.name === `${base}.md` || e.name === base),
      );
      hits.sort((a, b) => a.path.length - b.path.length);
      if (hits.length) {
        const h = hits[0];
        const kind = viewerKind(h.name);
        if (kind === "markdown") await show({ path: h.path, name: h.name, kind, text: await api.readText(h.path), anchor: anchor ?? null });
        else await show({ path: h.path, name: h.name, kind });
        return;
      }
    } catch {
      /* bỏ qua, báo lỗi chung bên dưới */
    }
    fail(new Error("404"), t("f.wikiMiss", { name: tt }));
  };

  /** Cuộn tới heading sau khi mở file (link dạng [[note#section]]). */
  const scrollToAnchor = (a: string) => {
    const needle = a.toLowerCase().replace(/[-_]+/g, " ");
    let tries = 0;
    const tick = () => {
      tries += 1;
      const hs = document.querySelectorAll(".md-body h1, .md-body h2, .md-body h3, .md-body h4, .md-body h5, .md-body h6");
      for (const h of Array.from(hs)) {
        if ((h.textContent ?? "").toLowerCase().includes(needle)) {
          h.scrollIntoView({ block: "start" });
          return;
        }
      }
      if (tries < 20) setTimeout(tick, 100);
    };
    setTimeout(tick, 60);
  };

  const download = async (path: string, name: string) => {
    try {
      const a = document.createElement("a");
      a.href = await api.directDownloadUrl(path);
      a.download = name;
      a.click();
    } catch (e) {
      fail(e, t("f.downloadFile"));
    }
  };

  /** Tải lại sau thao tác: trang đang xem hoặc kết quả tìm kiếm. */
  const reload = async () => {
    const q = query().trim();
    if (results() && q.length >= 2) setResults(await (byContent() ? api.searchContent(q) : api.search(q)).catch(() => results()));
    else await refresh();
    refreshMe();
  };

  /** Chạy tuần tự trên nhiều mục, hiện tiến độ; trả về các kết quả thành công. */
  const each = async <T,>(label: string, paths: string[], f: (p: string) => Promise<T>) => {
    const done: T[] = [];
    try {
      for (let i = 0; i < paths.length; i++) {
        setWorking(paths.length > 1 ? `${label} ${i + 1}/${paths.length}…` : `${label}…`);
        done.push(await f(paths[i]));
      }
    } finally {
      setWorking(null);
      setSel(new Set<string>());
      await reload();
    }
    return done;
  };

  const remove = async (paths: string[]) => {
    try {
      const items = await each(t("f.deleting"), paths, (p) => api.remove(p));
      flash(items.length === 1 ? t("f.deleted1", { name: items[0].orig_path.split("/").pop() }) : t("f.deletedN", { n: items.length }), async () => {
        await each(t("f.restoring"), items.map((x) => x.id), (id) => api.restore(id, isSecret(paths[0])));
        flash(t("f.restored"));
      });
    } catch (e) {
      fail({ message: errText(e) }, t("f.deleteErr"));
    }
  };

  /** Chuyển các mục vào `dest`. Lỗi ném ra để dialog hiện. */
  const moveTo = async (paths: string[], dest: string) => {
    const todo = paths.filter((p) => parentOf(p) !== dest && dest !== p && !dest.startsWith(p + "/"));
    if (!todo.length) return;
    await each(t("f.movingN"), todo, (p) => api.move(p, joinPath(dest, p.split("/").pop()!)));
    flash(t("f.moved", { n: todo.length, dest: pretty(dest) }));
  };
  /** Sao chép vào `dest`; trùng tên trong cùng thư mục thì thêm " (bản sao)". */
  const copyTo = async (paths: string[], dest: string) => {
    const name = (p: string) => {
      const n = p.split("/").pop()!;
      if (parentOf(p) !== dest) return n;
      const i = n.lastIndexOf(".");
      return i > 0 ? `${n.slice(0, i)}${t("f.copySuffix")}${n.slice(i)}` : `${n}${t("f.copySuffix")}`;
    };
    await each(t("f.copyingN"), paths, (p) => api.copy(p, joinPath(dest, name(p))));
    flash(t("f.copied", { n: paths.length, dest: pretty(dest) }));
  };

  // --- chọn nhiều ---
  const toggleSel = (p: string, on?: boolean) => {
    const n = new Set(sel());
    (on ?? !n.has(p)) ? n.add(p) : n.delete(p);
    setSel(n);
  };
  /** Click chọn: shift = chọn dải từ lần chọn trước (chỉ các dòng đã tải). */
  const pick = (i: number, f: Entry, shift: boolean) => {
    if (shift && lastIdx >= 0) {
      const n = new Set(sel());
      for (let k = Math.min(lastIdx, i); k <= Math.max(lastIdx, i); k++) {
        const e = entryAt(k);
        if (e) n.add(e.path);
      }
      setSel(n);
    } else toggleSel(f.path);
    lastIdx = i;
  };
  const selectAll = async () => {
    if (results()) return setSel(new Set(results()!.map((e) => e.path)));
    if (total() > 5000) return setErr(t("f.selectLimit"));
    const need = Array.from({ length: Math.ceil(total() / PAGE) }, (_, k) => k);
    await Promise.all(need.map(fetchPage));
    setSel(new Set(need.flatMap((p) => (pages.get(p) ?? []).map((e) => e.path))));
  };
  onMount(() => {
    const key = (e: KeyboardEvent) => {
      if (page() !== "files" || dlg() || (e.target as Element).closest?.("input, textarea")) return;
      if ((e.ctrlKey || e.metaKey) && e.key === "a") {
        e.preventDefault();
        selectAll();
      } else if (e.key === "Escape") setSel(new Set<string>());
      else if (e.key === "Delete" && sel().size) remove([...sel()]);
    };
    document.addEventListener("keydown", key);
    onCleanup(() => document.removeEventListener("keydown", key));
  });

  // --- kéo thả để di chuyển (desktop) ---
  const DND = "application/x-nextkloud";
  const canDrag = matchMedia("(pointer: fine)").matches; // cảm ứng: nhấn giữ là để chọn
  const dragStart = (e: DragEvent, f: Entry) => {
    const paths = sel().has(f.path) ? [...sel()] : [f.path];
    e.dataTransfer!.setData(DND, JSON.stringify(paths));
    e.dataTransfer!.effectAllowed = "move";
  };
  const [dropOn, setDropOn] = createSignal<string | null>(null);
  const dropProps = (dest: () => string) => ({
    onDragOver: (e: DragEvent) => {
      if (!e.dataTransfer?.types.includes(DND)) return;
      e.preventDefault();
      setDropOn(dest());
    },
    onDragLeave: () => setDropOn(null),
    onDrop: (e: DragEvent) => {
      const raw = e.dataTransfer?.getData(DND);
      setDropOn(null);
      if (!raw) return;
      e.preventDefault();
      moveTo(JSON.parse(raw), dest()).catch((x) => fail({ message: errText(x) }, t("f.moveErr")));
    },
  });

  // Mobile: nhấn giữ 450ms để chọn.
  let pressTimer: number | undefined;
  let pressed = false;
  const pressProps = (i: () => number, f: () => Entry) => ({
    onPointerDown: (e: PointerEvent) => {
      pressed = false;
      if (e.pointerType !== "touch") return;
      pressTimer = window.setTimeout(() => {
        pressed = true;
        pick(i(), f(), false);
        navigator.vibrate?.(20);
      }, 450);
    },
    onPointerUp: () => clearTimeout(pressTimer),
    onPointerMove: () => clearTimeout(pressTimer),
    onPointerCancel: () => clearTimeout(pressTimer),
  });
  /** Click dòng: đang chọn nhiều hoặc giữ Ctrl/Shift thì chọn, không thì mở. */
  const rowClick = (e: MouseEvent, i: number, f: Entry) => {
    if (pressed) {
      pressed = false;
      return;
    }
    if (e.ctrlKey || e.metaKey || e.shiftKey || sel().size) pick(i, f, e.shiftKey);
    else openEntry(f);
  };

  /** Upload giữ cấu trúc thư mục: `rel` là đường dẫn tương đối ("A/b/c.txt"), `dirs` là thư mục rỗng cần tạo. */
  const doUpload = async (list: FileList | { file: File; rel: string }[] | null, dirs: string[] = []) => {
    const items = Array.isArray(list) ? list : Array.from(list ?? []).map((f) => ({ file: f, rel: f.webkitRelativePath || f.name }));
    fileInput.value = "";
    dirInput.value = "";
    if (!items.length && !dirs.length) return;
    const base = prefix();
    for (const d of dirs) await api.mkdir(joinPath(base, d)).catch(() => {}); // đã có thì thôi
    await runUploads(items.map((x) => ({ ...x, base })));
  };

  /** Chạy hàng đợi upload: lỗi 1 file không dừng cả lô, cuối cùng chỉ liệt kê file lỗi để thử lại. */
  const runUploads = async (jobs: UpJob[]) => {
    if (!jobs.length) return refresh();
    upCancel = false;
    setUpStat({ total: jobs.length, done: 0, ok: 0, fails: [], running: true });
    clearTimeout(upCloseTimer);
    // 3 file song song (lô ảnh nhỏ nhanh hơn nhiều); tiến độ + tốc độ tính theo byte cả lô
    const bytes = jobs.reduce((a, j) => a + j.file.size, 0);
    const t0 = Date.now();
    const partial = new Map<UpJob, number>();
    let doneBytes = 0;
    let next = 0;
    const tick = () => {
      const cur = doneBytes + [...partial.values()].reduce((a, b) => a + b, 0);
      const speed = cur / Math.max((Date.now() - t0) / 1000, 0.5);
      setUpload({ name: [...partial.keys()].map((j) => j.rel).join(" · "), pct: bytes ? Math.round((cur / bytes) * 100) : 100, left: 0 });
      setUpRate({ speed, eta: speed > 0 ? (bytes - cur) / speed : 0 });
    };
    const worker = async () => {
      while (!upCancel && next < jobs.length) {
        const j = jobs[next++];
        partial.set(j, 0);
        tick();
        try {
          await api.upload(joinPath(j.base, j.rel), j.file, (d) => {
            partial.set(j, d);
            tick();
          });
          setUpStat((s) => s && { ...s, ok: s.ok + 1 });
        } catch (e) {
          if (String((e as Error)?.message).includes("unauthorized")) {
            upCancel = true;
            fail(e, t("f.uploadErr"));
          } else setUpStat((s) => s && { ...s, fails: [...s.fails, { ...j, err: errText(e) }] });
        }
        partial.delete(j);
        doneBytes += j.file.size;
        setUpStat((s) => s && { ...s, done: s.done + 1 });
        tick();
      }
    };
    await Promise.all(Array.from({ length: Math.min(3, jobs.length) }, worker));
    setUpRate(null);
    setUpload(null);
    setUpStat((s) => s && { ...s, running: false });
    // xong hết, không lỗi: tự đóng sau vài giây
    if (!upStat()?.fails.length && !upCancel) upCloseTimer = window.setTimeout(() => setUpStat(null), 4000);
    refresh();
    refreshMe();
  };

  /** Kéo file/thư mục từ máy vào: duyệt cây thư mục (webkitGetAsEntry), giữ đường dẫn con. */
  const readDrop = async (dt: DataTransfer) => {
    const files: { file: File; rel: string }[] = [];
    const dirs: string[] = [];
    const walk = async (e: any, rel: string): Promise<void> => {
      if (e.isFile) {
        files.push({ file: await new Promise<File>((ok, no) => e.file(ok, no)), rel });
      } else if (e.isDirectory) {
        const r = e.createReader();
        let n = 0;
        // readEntries trả từng đợt (~100 mục), đọc tới khi rỗng
        for (;;) {
          const batch: any[] = await new Promise((ok, no) => r.readEntries(ok, no));
          if (!batch.length) break;
          n += batch.length;
          for (const c of batch) await walk(c, `${rel}/${c.name}`);
        }
        if (!n) dirs.push(rel);
      }
    };
    const entries = Array.from(dt.items).map((it) => it.webkitGetAsEntry?.()).filter(Boolean);
    if (!entries.length) return { files: Array.from(dt.files).map((f) => ({ file: f, rel: f.name })), dirs };
    for (const e of entries) await walk(e, e!.name);
    return { files, dirs };
  };
  const externalDrag = (e: DragEvent) => !!e.dataTransfer?.types.includes("Files") && !e.dataTransfer.types.includes(DND);
  const panelDrop = {
    onDragOver: (e: DragEvent) => {
      if (page() !== "files" || !externalDrag(e)) return;
      e.preventDefault();
      setDropping(true);
    },
    onDragLeave: (e: DragEvent) => {
      if (!(e.currentTarget as Element).contains(e.relatedTarget as Node)) setDropping(false);
    },
    onDrop: async (e: DragEvent) => {
      setDropping(false);
      if (page() !== "files" || !externalDrag(e)) return;
      e.preventDefault();
      const { files, dirs } = await readDrop(e.dataTransfer!);
      doUpload(files, dirs);
    },
  };

  const quotaPct = () => {
    const m = me();
    return m && m.quota_limit > 0 ? Math.min(100, (m.quota_used / m.quota_limit) * 100) : 0;
  };

  const closeMenu = () => {
    setMenuFor(null);
    setMenuUpFor(null);
  };
  const RowMenu = (p: { f: Entry }) => (
    <div class="menu-wrap" onClick={(e) => e.stopPropagation()}>
      <button class="icon-btn" title={t("f.menu")} onClick={(e) => {
        const n = p.f.path;
        if (menuFor() === n) { setMenuFor(null); setMenuUpFor(null); return; }
        setMenuUpFor(flipIfNeeded(e.currentTarget as HTMLElement) ? n : null);
        // ô sát mép trái: menu canh trái nút, không tràn ra ngoài khung
        const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
        const left = (e.currentTarget as HTMLElement).closest(".panel")?.getBoundingClientRect().left ?? 0;
        setMenuLeftFor(r.right - 210 < left ? n : null);
        setMenuFor(n);
      }}>
        <Icon name="more" />
      </button>
      <Show when={menuFor() === p.f.path}>
        <div class="menu" classList={{ up: menuUpFor() === p.f.path, left: menuLeftFor() === p.f.path }}>
          <Show when={!p.f.is_dir}>
            <button onClick={() => { closeMenu(); download(p.f.path, p.f.name); }}>
              <Icon name="download" /> {t("f.download")}
            </button>
          </Show>
          <Show when={p.f.is_dir}>
            <button onClick={() => { closeMenu(); load(p.f.path); }}>
              <Icon name="folder" /> {t("f.open")}
            </button>
          </Show>
          <Show when={p.f.is_dir && !isSecret(p.f.path)}>
            <button onClick={() => { closeMenu(); const a = document.createElement("a"); a.href = api.zipDownloadUrl(p.f.path); a.click(); }}>
              <Icon name="download" /> {t("f.zip")}
            </button>
          </Show>
          <button onClick={() => { closeMenu(); setDlg({ kind: "rename", f: p.f }); }}>
            <Icon name="edit" /> {t("f.rename")}
          </button>
          <button onClick={() => { closeMenu(); setDlg({ kind: "move", paths: [p.f.path] }); }}>
            <Icon name="move" /> {t("f.move")}
          </button>
          <button onClick={() => { closeMenu(); setDlg({ kind: "copy", paths: [p.f.path] }); }}>
            <Icon name="copy" /> {t("f.copy")}
          </button>
          <Show when={!isSecret(p.f.path)}>
            <button onClick={() => { closeMenu(); setShareFor({ path: p.f.path }); }}>
              <Icon name="users" /> {t("f.share")}
            </button>
          </Show>
          <button onClick={() => { closeMenu(); toggleSel(p.f.path, true); lastIdx = -1; }}>
            <Icon name="check" /> {t("f.select")}
          </button>
          <div class="menu-sep" />
          <button class="danger" onClick={() => { closeMenu(); remove([p.f.path]); }}>
            <Icon name="trash" /> {t("f.delete")}
          </button>
        </div>
      </Show>
    </div>
  );

  const Logo = () => (
    <div class="brand">
      <img src="/logo.svg" alt="" width="34" height="34" />
      <span>
        Next<b>Kloud</b>
      </span>
    </div>
  );

  return (
    <div class="app">
      <div class="wall" classList={{ "wall-light": ui().bg !== "image" && ui().bg !== "night" }} style={{ background: wallCss() }} />
      <div class="wall-dim" style={{ opacity: ui().bg === "image" ? ui().dim : 0 }} />

      <Switch>
        <Match when={needReset()}>
          <div class="auth">
            <div class="auth-card glass">
              <Logo />
              <h2>{t("login.newTitle")}</h2>
              <p class="muted">{t("login.newDesc")}</p>
              <Show when={err()}><p class="err">{err()}</p></Show>
              <ResetForm
                current={loginPw}
                onDone={async (o, n) => {
                  setErr("");
                  try {
                    await api.changePassword(o, n);
                    loginPw = "";
                    setPassword("");
                    setNeedReset(false);
                    enter();
                  } catch (e) {
                    setErr((e as Error).message);
                  }
                }}
                onError={setErr}
              />
            </div>
          </div>
        </Match>

        {/* Link công khai: xem được khi chưa đăng nhập */}
        <Match when={!authed() && /^#\/s\/[A-Za-z0-9]+/.test(location.hash)}>
          <PublicPage />
        </Match>

        <Match when={!authed()}>
          <div class="auth">
            <form class="auth-card glass" onSubmit={(e) => { e.preventDefault(); doLogin(); }}>
              <Logo />
              <p class="muted">{t("login.sub")}</p>
              <Show when={err()}><p class="err">{err()}</p></Show>
              <Show when={!isSelfHostedWeb()}>
                <label>{t("login.server")}</label>
                <input value={server()} onInput={(e) => setServer(e.currentTarget.value)} placeholder="http://mini:8011" />
              </Show>
              <label>{t("login.user")}</label>
              <input value={username()} onInput={(e) => setUsername(e.currentTarget.value)} autocomplete="username" />
              <label>{t("login.pass")}</label>
              <input type="password" value={password()} onInput={(e) => setPassword(e.currentTarget.value)} autocomplete="current-password" />
              <button type="submit" class="btn primary block">{t("login.go")}</button>
            </form>
          </div>
        </Match>

        <Match when={true}>
          <header class="hdr glass">
            <button class="icon-btn only-mobile" onClick={() => setSideOpen(!sideOpen())} title="Menu">
              <Icon name="menu" size={20} />
            </button>
            <a href="#" class="hdr-brand" onClick={(e) => { e.preventDefault(); load(""); }}>
              <Logo />
            </a>
            <div class="hdr-search hdr-search-inline">
              <Icon name="search" />
              <input value={query()} onInput={(e) => { setPage("files"); setQuery(e.currentTarget.value); }} placeholder={byContent() ? t("search.contentPh") : t("search.namePh")} />
              <button class="search-mode" classList={{ on: byContent() }} onClick={() => setByContent(!byContent())} title={t("search.mode")}>
                {byContent() ? t("search.content") : t("search.name")}
              </button>
              <Show when={query()}>
                <button class="icon-btn sm" onClick={() => setQuery("")}><Icon name="close" size={14} /></button>
              </Show>
            </div>
            <button class="icon-btn search-fab" onClick={() => setMSearch(true)} title={t("c.search")}>
              <Icon name="search" size={20} />
            </button>
            <div class="menu-wrap">
              <button class="avatar" onClick={(e) => {
                if (userMenu()) { setUserMenu(false); return; }
                setUserMenuUp(flipIfNeeded(e.currentTarget as HTMLElement));
                setUserMenu(true);
              }} title={shownName()}>
                <Show when={me()} fallback="?">
                  <Avatar id={me()!.id} v={me()!.avatar_v} name={shownName()} />
                </Show>
              </button>
              <Show when={userMenu()}>
                <div class="menu right" classList={{ up: userMenuUp() }}>
                  <button class="menu-head profile" onClick={() => openSettings("profile")}>
                    <Show when={me()}><Avatar id={me()!.id} v={me()!.avatar_v} name={shownName()} size={40} /></Show>
                    <span class="mh-text">
                      <b>{shownName()}</b>
                      <span class="muted sm">@{me()?.username}<Show when={me()?.is_admin}> · admin</Show></span>
                    </span>
                  </button>
                  <button onClick={() => openSettings("profile")}>
                    <Icon name="user" /> {t("nav.profile")}
                  </button>
                  <button onClick={() => openSettings("security")}>
                    <Icon name="key" /> {t("nav.password")}
                  </button>
                  <button onClick={() => { setUserMenu(false); setShowLook(true); }}>
                    <Icon name="palette" /> {t("nav.look")}
                  </button>
                  <Show when={me()?.is_admin}>
                    <div class="menu-sep" />
                    <button onClick={() => openSettings("users")}>
                      <Icon name="users" /> {t("nav.users")}
                    </button>
                    <button onClick={() => openSettings("system")}>
                      <Icon name="server" /> {t("nav.system")}
                    </button>
                  </Show>
                  <div class="menu-sep" />
                  <button onClick={logout}>
                    <Icon name="logout" /> {t("nav.logout")}
                  </button>
                </div>
              </Show>
            </div>
          </header>
          <Show when={mSearch()}>
            <div class="msearch-bg" onClick={() => setMSearch(false)}>
              <div class="msearch glass" onClick={(e) => e.stopPropagation()}>
                <div class="hdr-search hdr-search-modal">
                  <Icon name="search" />
                  <input autofocus value={query()} onInput={(e) => { setPage("files"); setQuery(e.currentTarget.value); }} onKeyDown={(e) => { if (e.key === "Enter" || e.key === "Escape") setMSearch(false); }} placeholder={byContent() ? t("search.contentPh") : t("search.namePh")} />
                  <button class="search-mode" classList={{ on: byContent() }} onClick={() => setByContent(!byContent())} title={t("search.mode")}>
                    {byContent() ? t("search.content") : t("search.name")}
                  </button>
                  <Show when={query()}>
                    <button class="icon-btn sm" onClick={() => setQuery("")}><Icon name="close" size={14} /></button>
                  </Show>
                </div>
                <button class="btn ghost" onClick={() => setMSearch(false)}>{t("c.close")}</button>
              </div>
            </div>
          </Show>

          <div class="body">
            <Show when={sideOpen()}>
              <div class="scrim only-mobile" onClick={() => setSideOpen(false)} />
            </Show>
            <aside class="side glass" classList={{ open: sideOpen() }}>
              <div class="side-brand only-mobile"><Logo /></div>
              <nav>
                <button class="nav" classList={{ active: page() === "files" && !prefix() && !open() }} onClick={() => load("")}>
                  <Icon name="home" /> {t("nav.files")}
                </button>
                <button class="nav" classList={{ active: page() === "kanban" }} onClick={() => { setPage("kanban"); setSideOpen(false); }}>
                  <Icon name="kanban" /> {t("nav.deck")}
                </button>
                <button class="nav" classList={{ active: page() === "tasks" }} onClick={() => { setPage("tasks"); setSideOpen(false); }}>
                  <Icon name="check" /> {t("nav.track")}
                </button>
                <button class="nav" classList={{ active: page() === "shared" }} onClick={() => { setPage("shared"); setSideOpen(false); }}>
                  <Icon name="users" /> {t("nav.shared")}
                </button>
                <button class="nav" classList={{ active: page() === "notes" }} onClick={() => { setPage("notes"); setSideOpen(false); }}>
                  <Icon name="note" /> {t("nav.notes")}
                </button>
                <button class="nav" classList={{ active: page() === "trash" }} onClick={() => { setPage("trash"); setSideOpen(false); }}>
                  <Icon name="trash" /> {t("nav.trash")}
                </button>
                <button class="nav" classList={{ active: page() === "secret" || inSecret() }} onClick={openSecret}>
                  <Icon name="lock" /> {t("nav.secret")}
                </button>
                <Show when={me()?.is_admin}>
                  <button class="nav" classList={{ active: page() === "settings" && (setTab() === "users" || setTab() === "system") }} onClick={() => openSettings("users")}>
                    <Icon name="users" /> {t("nav.admin")}
                  </button>
                </Show>
                <button class="nav" classList={{ active: page() === "settings" && !["users", "system"].includes(setTab()) }} onClick={() => openSettings("profile")}>
                  <Icon name="settings" /> {t("nav.settings")}
                </button>
              </nav>
              <div class="side-foot">
                <Show when={me()}>
                  <div class="quota">
                    <div class="quota-txt">
                      <Icon name="cloud" />
                      <span>
                        {fmtSize(me()!.quota_used)}
                        <Show when={me()!.quota_limit > 0}> / {fmtSize(me()!.quota_limit)}</Show>
                      </span>
                    </div>
                    <Show when={me()!.quota_limit > 0}>
                      <div class="bar"><i style={{ width: `${quotaPct()}%` }} classList={{ hot: quotaPct() > 90 }} /></div>
                    </Show>
                  </div>
                </Show>
                <Show when={srvVer()}>
                  <div class="muted xs srv-ver">Server v{srvVer()}</div>
                </Show>
              </div>
            </aside>

            <main class="panel glass" classList={{ dropping: dropping() }} {...panelDrop}>
              <Show when={page() === "settings" && me()}>
                <Show when={err()}>
                  <div class="err banner">
                    <span>{err()}</span>
                    <button class="icon-btn sm" onClick={() => setErr("")}><Icon name="close" size={14} /></button>
                  </div>
                </Show>
                <Settings me={me()!} tab={setTab()} onTab={setSetTab} onMe={setMe} onError={setErr} onOpen={(f) => load(f)} />
              </Show>
              <Show when={page() === "kanban"}>
                <Show when={err()}>
                  <div class="err banner">
                    <span>{err()}</span>
                    <button class="icon-btn sm" onClick={() => setErr("")}><Icon name="close" size={14} /></button>
                  </div>
                </Show>
                <KanbanPage boardId={boardId()} onBoard={setBoardId} onError={setErr} onOpenFile={(p) => openAt(p, "")} />
              </Show>
              <Show when={page() === "notes"}>
                <NotesPage onOpen={openAt} onError={setErr} />
              </Show>
              <Show when={page() === "tasks"}>
                <Show when={err()}>
                  <div class="err banner">
                    <span>{err()}</span>
                    <button class="icon-btn sm" onClick={() => setErr("")}><Icon name="close" size={14} /></button>
                  </div>
                </Show>
                <TrackerPage onOpenBoard={(id) => { setBoardId(id); setPage("kanban"); }} onError={setErr} />
              </Show>
              <Show when={page() === "trash"}>
                <Show when={err()}>
                  <div class="err banner">
                    <span>{err()}</span>
                    <button class="icon-btn sm" onClick={() => setErr("")}><Icon name="close" size={14} /></button>
                  </div>
                </Show>
                <TrashPage onError={setErr} onChanged={refreshMe} />
              </Show>
              <Show when={page() === "shared"}>
                <Show when={err()}>
                  <div class="err banner">
                    <span>{err()}</span>
                    <button class="icon-btn sm" onClick={() => setErr("")}><Icon name="close" size={14} /></button>
                  </div>
                </Show>
                <SharedPage onError={setErr} />
              </Show>
              <Show when={page() === "secret"}>
                <SecretGate onUnlocked={() => load(SECRET_ROOT)} />
              </Show>
              <Show when={page() === "secret-trash"}>
                <Show when={err()}>
                  <div class="err banner">
                    <span>{err()}</span>
                    <button class="icon-btn sm" onClick={() => setErr("")}><Icon name="close" size={14} /></button>
                  </div>
                </Show>
                <div class="toolbar"><button class="btn ghost sm" onClick={() => load(SECRET_ROOT)}><Icon name="back" /> {t("c.backToVault")}</button></div>
                <TrashPage secret onError={setErr} onChanged={refreshMe} />
              </Show>
              <Show when={page() === "files"}>
              <Show when={sel().size}>
                <div class="toolbar selbar">
                  <button class="icon-btn" title={t("f.deselect")} onClick={() => setSel(new Set<string>())}><Icon name="close" /></button>
                  <b>{t("f.selected", { n: fmtNum(sel().size) })}</b>
                  <button class="btn ghost sm" onClick={selectAll}>{t("f.selectAll")}</button>
                  <span class="grow" />
                  <button class="btn ghost sm" onClick={() => setDlg({ kind: "move", paths: [...sel()] })}>
                    <Icon name="move" /> <span class="hide-xs">{t("f.move")}</span>
                  </button>
                  <button class="btn ghost sm" onClick={() => setDlg({ kind: "copy", paths: [...sel()] })}>
                    <Icon name="copy" /> <span class="hide-xs">{t("f.copy")}</span>
                  </button>
                  <button class="btn danger sm" onClick={() => remove([...sel()])}>
                    <Icon name="trash" /> <span class="hide-xs">{t("f.delete")}</span>
                  </button>
                </div>
              </Show>
              <div class="toolbar" classList={{ hidden: sel().size > 0 }}>
                {/* đang mở file: thanh trên chỉ còn nút Quay lại (luôn thấy, không bị cuộn mất) */}
                <Show when={!open()} fallback={
                  <button class="btn ghost" onClick={() => setOpen(null)}>
                    <Icon name="back" /> {t("c.back")}
                  </button>
                }>
                  <button class="btn primary" onClick={() => fileInput.click()} disabled={!!upload()}>
                    <Icon name="upload" /> <span class="hide-xs">{t("c.upload")}</span>
                  </button>
                  <Show when={!results()}>
                    <button class="btn ghost" title={t("f.newFolder")} onClick={() => setDlg({ kind: "mkdir" })}>
                      <Icon name="folderPlus" /> <span class="hide-xs">{t("f.newFolder")}</span>
                    </button>
                  </Show>
                  <button class="btn ghost" title={t("f.uploadDir")} onClick={() => dirInput.click()} disabled={!!upload()}>
                    <Icon name="folder" /> <span class="hide-xs">{t("f.uploadDir")}</span>
                  </button>
                </Show>
                <input ref={fileInput} type="file" multiple hidden onChange={(e) => doUpload(e.currentTarget.files)} />
                <input ref={dirInput} type="file" multiple hidden {...{ webkitdirectory: true }} onChange={(e) => doUpload(e.currentTarget.files)} />
                <nav class="crumbs">
                  <button class="crumb" classList={{ drop: dropOn() === "" }} onClick={() => load("")} {...dropProps(() => "")}>
                    <Icon name="home" size={16} />
                  </button>
                  <For each={crumbs()}>
                    {(c, i) => (
                      <>
                        <Icon name="chevron" size={14} class="ic dim" />
                        <button
                          class="crumb"
                          classList={{ cur: i() === crumbs().length - 1 && !open(), drop: dropOn() === crumbs().slice(0, i() + 1).join("/") }}
                          onClick={() => load(crumbs().slice(0, i() + 1).join("/"))}
                          {...dropProps(() => crumbs().slice(0, i() + 1).join("/"))}
                        >
                          {c === SECRET_ROOT ? (<><Icon name="lock" size={14} /> {t("f.secret")}</>) : c}
                        </button>
                      </>
                    )}
                  </For>
                  <Show when={open()}>
                    <Icon name="chevron" size={14} class="ic dim" />
                    <span class="crumb cur">{open()!.path.split("/").pop()}</span>
                  </Show>
                </nav>
                <Show when={inSecret()}>
                  <div class="sec-tools">
                    <span class="sec-left" title={t("f.lockIdle")}>{Math.floor(vaultLeft() / 60)}:{String(vaultLeft() % 60).padStart(2, "0")}</span>
                    <button class="icon-btn" title={t("f.secretTrash")} onClick={() => { setOpen(null); setPage("secret-trash"); }}><Icon name="trash" /></button>
                    <button class="icon-btn" title={t("f.secretSettings")} onClick={() => setSecretSet(true)}><Icon name="settings" /></button>
                    <button class="btn ghost sm" title={t("f.lockNow")} onClick={lockSecret}><Icon name="lock" /> <span class="hide-xs">{t("f.lock")}</span></button>
                  </div>
                </Show>
                <Show when={!open()}>
                  <div class="seg">
                    <button classList={{ on: view() === "list" }} title={t("f.list")} onClick={() => { setView("list"); localStorage.setItem("nk-view", "list"); }}>
                      <Icon name="list" />
                    </button>
                    <button classList={{ on: view() === "grid" }} title={t("f.grid")} onClick={() => { setView("grid"); localStorage.setItem("nk-view", "grid"); }}>
                      <Icon name="grid" />
                    </button>
                  </div>
                </Show>
              </div>

              <Show when={err()}>
                <div class="err banner">
                  <span>{err()}</span>
                  <button class="icon-btn sm" onClick={() => setErr("")}><Icon name="close" size={14} /></button>
                </div>
              </Show>

              <Show
                when={!open()}
                fallback={
                  <div class="viewer">
                    <Suspense fallback={<p class="muted">{t("c.opening")}</p>}>
                      <Switch>
                        <Match when={open()!.kind === "markdown"}>
                          <MarkdownView path={open()!.path} text={open()!.text ?? ""} anchor={open()!.anchor ?? null} onOpenLink={openWikiLink} onError={setErr} mark={open()!.mark} onSaved={() => openAt(open()!.path, "")} />
                        </Match>
                        <Match when={open()!.kind === "text"}>
                          <TextView path={open()!.path} />
                        </Match>
                        <Match when={open()!.kind === "image"}>
                          {(() => {
                            const s = siblingImages(open()!.path);
                            return <ImageView files={s.files} index={s.index} />;
                          })()}
                        </Match>
                        <Match when={open()!.kind === "video" || open()!.kind === "audio"}>
                          <VideoView path={open()!.path} />
                        </Match>
                        <Match when={open()!.kind === "pdf"}>
                          <PdfView path={open()!.path} onError={setErr} mark={open()!.mark} />
                        </Match>
                        <Match when={open()!.kind === "zip"}>
                          <ZipView path={open()!.path} />
                        </Match>
                        <Match when={open()!.kind === "epub"}>
                          <EpubView path={open()!.path} />
                        </Match>
                        <Match when={open()!.kind === "drawio"}>
                          <DrawioView path={open()!.path} />
                        </Match>
                        <Match when={open()!.kind === "excalidraw"}>
                          <ExcalidrawView path={open()!.path} />
                        </Match>
                        <Match when={open()!.kind === "office"}>
                          <OfficeView path={open()!.path} />
                        </Match>
                      </Switch>
                    </Suspense>
                  </div>
                }
              >
                <div
                  class="scroll"
                  classList={{ busy: loading(), selecting: sel().size > 0 }}
                  onScroll={onScroll}
                  ref={(el) => {
                    scrollEl = el;
                    setScrollTop(0);
                    ro.observe(el);
                  }}
                >
                  <Show when={view() === "list"}>
                    <div class="row head">
                      <span class="c-name sortable" onClick={() => toggleSort("name")}>
                        {t("c.name")} {sort().key === "name" ? (sort().asc ? "↑" : "↓") : ""}
                      </span>
                      <span class="c-size sortable" onClick={() => toggleSort("size")}>
                        {t("set.quota")} {sort().key === "size" ? (sort().asc ? "↑" : "↓") : ""}
                      </span>
                      <span class="c-date sortable" onClick={() => toggleSort("modified")}>
                        {t("f.modified")}{sort().key === "modified" ? (sort().asc ? " ↑" : " ↓") : ""}
                      </span>
                      <span class="c-act" />
                    </div>
                    <div class="vbox" ref={vbox} style={{ height: `${nRows() * ROW_H}px` }}>
                      <Index each={visibleRows()}>
                        {(i) => {
                          const pos = () => ({ transform: `translateY(${i() * ROW_H}px)` });
                          return (
                            <Show when={entryAt(i())} fallback={<div class="row vrow skel" style={pos()}><span class="sk" /></div>}>
                              {(f) => {
                                const k = () => kindOf(f().name, f().is_dir, f().path);
                                return (
                                  <div
                                    class="row vrow"
                                    style={pos()}
                                    classList={{ "menu-open": menuFor() === f().path, sel: sel().has(f().path), drop: f().is_dir && dropOn() === f().path }}
                                    onClick={(e) => rowClick(e, i(), f())}
                                    draggable={canDrag}
                                    onDragStart={(e) => dragStart(e, f())}
                                    {...(f().is_dir ? dropProps(() => f().path) : {})}
                                    {...pressProps(i, f)}
                                  >
                                    <span class="c-name">
                                      <span class="ck" onClick={(e) => { e.stopPropagation(); pick(i(), f(), e.shiftKey); }}>
                                        <Icon name="check" size={14} />
                                      </span>
                                      <span class="fi" style={{ color: k().color }}>
                                        <Icon name={k().icon} size={20} />
                                      </span>
                                      <span class="fcol">
                                        <span class="fname" title={f().path}>{f().name}</span>
                                        <Show when={results() && parentOf(f().path) && f().snippet === undefined}>
                                          <span class="fpath">{parentOf(f().path)}</span>
                                        </Show>
                                        <Show when={f().snippet}>
                                          <span class="fsnip" innerHTML={highlight(f().snippet!, query())} />
                                        </Show>
                                      </span>
                                    </span>
                                    <span class="c-size">{f().is_dir ? "—" : fmtSize(f().size)}</span>
                                    <span class="c-date">{fmtDate(f().modified)}</span>
                                    <span class="c-act">
                                      <RowMenu f={f()} />
                                    </span>
                                  </div>
                                );
                              }}
                            </Show>
                          );
                        }}
                      </Index>
                    </div>
                  </Show>

                  <Show when={view() === "grid"}>
                    <div class="vbox vgrid" ref={vbox} style={{ height: `${nRows() * rowH()}px` }}>
                      <Index each={visibleRows()}>
                        {(r) => (
                          <div
                            class="grid vgrid-row"
                            style={{ transform: `translateY(${r() * rowH()}px)`, "grid-template-columns": `repeat(${cols()}, minmax(0, 1fr))` }}
                          >
                            <Index each={Array.from({ length: Math.min(cols(), total() - r() * cols()) }, (_, k) => r() * cols() + k)}>
                              {(i) => (
                                <Show when={entryAt(i())} fallback={<div class="tile skel"><div class="tile-art" /></div>}>
                                  {(f) => {
                                    const k = () => kindOf(f().name, f().is_dir, f().path);
                                    return (
                                      <div
                                        class="tile"
                                        classList={{ "menu-open": menuFor() === f().path, sel: sel().has(f().path), drop: f().is_dir && dropOn() === f().path }}
                                        onClick={(e) => rowClick(e, i(), f())}
                                        draggable={canDrag}
                                        onDragStart={(e) => dragStart(e, f())}
                                        {...(f().is_dir ? dropProps(() => f().path) : {})}
                                        {...pressProps(i, f)}
                                      >
                                        <span class="ck" onClick={(e) => { e.stopPropagation(); pick(i(), f(), e.shiftKey); }}>
                                          <Icon name="check" size={14} />
                                        </span>
                                        <div class="tile-art" style={{ color: k().color }}>
                                          <ThumbImg path={f().path} name={f().name} />
                                          <Icon name={k().icon} size={44} />
                                          <Show when={!f().is_dir && ext(f().name)}>
                                            <span class="ext" style={{ background: k().color }}>{ext(f().name)}</span>
                                          </Show>
                                        </div>
                                        <div class="tile-meta">
                                          <span class="fname" title={f().path}>{f().name}</span>
                                          <RowMenu f={f()} />
                                        </div>
                                        <span class="tile-sub">{f().is_dir ? t("f.dirLabel") : `${fmtSize(f().size)} · ${fmtDate(f().modified)}`}</span>
                                      </div>
                                    );
                                  }}
                                </Show>
                              )}
                            </Index>
                          </div>
                        )}
                      </Index>
                    </div>
                  </Show>

                  <Show when={!loading() && total() === 0}>
                    <div class="empty">
                      <Icon name={results() ? "search" : "folder"} size={48} />
                      <p>{results() ? t("f.noResults") : t("f.emptyFolder")}</p>
                      <Show when={!results()}>
                        <button class="btn primary" onClick={() => fileInput.click()}>
                          <Icon name="upload" /> {t("f.uploadFile")}
                        </button>
                      </Show>
                    </div>
                  </Show>
                </div>
                <div class="foot">
                  <Show
                    when={!results()}
                    fallback={<span>{results()!.length >= 300 ? t("f.tooMany") : t("f.results", { n: results()!.length })}</span>}
                  >
                    <span>
                      {t("f.folders", { n: fmtNum(stats().dirs) })} · {t("f.filesN", { n: fmtNum(stats().files) })}
                    </span>
                    <span>{fmtSize(stats().bytes)}</span>
                  </Show>
                </div>
              </Show>
              </Show>
            </main>
          </div>

          <Show when={upStat()}>
            {(u) => (
              <div class="toast glass up-panel">
                <div class="up-head">
                  <div>
                    <b>{u().running ? t("f.upRunning", { left: fmtNum(u().total - u().done), done: fmtNum(u().done), total: fmtNum(u().total) }) : upCancel ? t("f.upStopped") : t("f.upFinished", { done: fmtNum(u().done), total: fmtNum(u().total) })}</b>
                    <div class="sm">
                      <span class="up-ok">{t("f.upOk", { ok: fmtNum(u().ok) })}</span> · <span classList={{ "up-bad": u().fails.length > 0 }}>{t("f.upErr", { n: fmtNum(u().fails.length) })}</span>
                      <Show when={u().running && upRate()}>
                        {(r) => <> · {fmtSize(r().speed)}/s · ~{fmtEta(r().eta)}</>}
                      </Show>
                    </div>
                  </div>
                  <Show when={u().running} fallback={
                    <button class="icon-btn sm" title={t("f.upClose")} onClick={() => setUpStat(null)}><Icon name="close" size={15} /></button>
                  }>
                    <button class="btn ghost sm" title={t("f.upStop")} onClick={() => (upCancel = true)}>{t("f.upStopBtn")}</button>
                  </Show>
                </div>
                <Show when={upload()}>
                  <div class="toast-row up-cur">
                    <Icon name="upload" size={15} />
                    <span class="fname">{upload()!.name}</span>
                    <span class="muted">{upload()!.pct}%</span>
                  </div>
                  <div class="bar"><i style={{ width: `${upload()!.pct}%` }} /></div>
                </Show>
                <Show when={u().fails.length}>
                  <div class="up-fails">
                    <For each={u().fails}>
                      {(f) => (
                        <div class="up-fail" title={f.err}>
                          <span class="fname">{f.rel}</span>
                          <span class="muted xs">{f.err}</span>
                          <button class="icon-btn sm" title={t("c.skip")} onClick={() => setUpStat((s) => s && { ...s, fails: s.fails.filter((x) => x !== f) })}><Icon name="close" size={13} /></button>
                        </div>
                      )}
                    </For>
                  </div>
                  <Show when={!u().running}>
                    <button class="btn primary sm block" onClick={() => runUploads(u().fails.map(({ err: _e, ...j }) => j))}>{t("f.upRetryFails", { n: u().fails.length })}</button>
                  </Show>
                </Show>
              </div>
            )}
          </Show>

          <Show when={working() || notice()}>
            <div class="toast glass note" classList={{ lift: !!upStat() }}>
              <Show when={working()} fallback={
                <div class="toast-row">
                  <Icon name="check" />
                  <span class="fname">{notice()!.text}</span>
                  <Show when={notice()!.undo}>
                    <button class="btn ghost sm" onClick={() => { const u = notice()!.undo!; setNotice(null); u().catch((e) => fail({ message: errText(e) }, t("f.undoFail"))); }}>
                      {t("f.trashUndo")}
                    </button>
                  </Show>
                  <button class="icon-btn sm" onClick={() => setNotice(null)}><Icon name="close" size={14} /></button>
                </div>
              }>
                <div class="toast-row"><span class="spin" /><span class="fname">{working()}</span></div>
              </Show>
            </div>
          </Show>

          <Show when={dlg()}>
            {(d) => (
              <Switch>
                <Match when={d().kind === "mkdir"}>
                  <NameDialog title={t("f.mkdirTitle")} label={t("f.mkdirIn", { dir: pretty(prefix()) })} ok={t("f.create")} onClose={() => setDlg(null)} onSubmit={async (n) => {
                    await api.mkdir(join(n));
                    setDlg(null);
                    await reload();
                  }} />
                </Match>
                <Match when={d().kind === "rename" && (d() as { f: Entry }).f}>
                  {(f) => (
                    <NameDialog title={f().is_dir ? t("f.renameDir") : t("f.renameFile")} label={t("f.newName")} initial={f().name} ok={t("f.rename")} onClose={() => setDlg(null)} onSubmit={async (n) => {
                      await api.move(f().path, joinPath(parentOf(f().path), n));
                      setDlg(null);
                      await reload();
                    }} />
                  )}
                </Match>
                <Match when={d().kind === "move" && (d() as { paths: string[] }).paths}>
                  {(paths) => (
                    <MoveDialog paths={paths()} start={prefix()} onClose={() => setDlg(null)} onSubmit={async (dest) => {
                      await moveTo(paths(), dest);
                      setDlg(null);
                    }} />
                  )}
                </Match>
                <Match when={d().kind === "copy" && (d() as { paths: string[] }).paths}>
                  {(paths) => (
                    <MoveDialog copy paths={paths()} start={prefix()} onClose={() => setDlg(null)} onSubmit={async (dest) => {
                      await copyTo(paths(), dest);
                      setDlg(null);
                    }} />
                  )}
                </Match>
              </Switch>
            )}
          </Show>

          <Show when={secretSet()}>
            <SecretSettings onClose={() => setSecretSet(false)} onDestroyed={() => lockUi()} />
          </Show>

          <Show when={showLook()}>
            <LookDialog onClose={() => setShowLook(false)} onError={(m) => setErr(m)} />
          </Show>

          <Show when={shareFor()}>
            {(s) => <ShareDialog path={s().path} onClose={() => setShareFor(null)} onError={setErr} />}
          </Show>
        </Match>
      </Switch>
    </div>
  );
}
