import { For, Index, Match, Show, Switch, batch, createEffect, createMemo, createSignal, lazy, onCleanup, onMount, Suspense } from "solid-js";
import { api, isSelfHostedWeb, serverOrigin, type Entry, type Me } from "./api";
import { syncFromServer, ui, wallCss } from "./background";
import { ResetForm } from "./forms";
import { LookDialog } from "./look";
import { fmtSize } from "./format";
import { Icon, ext, kindOf } from "./icons";
import Settings, { type SettingsTab } from "./Settings";
import { Avatar } from "./avatar";
import MarkdownView from "./viewers/MarkdownView";
import { viewerKind, hasThumb, type VKind } from "./viewers/kinds";
const TextView = lazy(() => import("./viewers/TextView"));
const ImageView = lazy(() => import("./viewers/ImageView"));
const VideoView = lazy(() => import("./viewers/VideoView"));
const PdfView = lazy(() => import("./viewers/PdfView"));
const ZipView = lazy(() => import("./viewers/ZipView"));
const EpubView = lazy(() => import("./viewers/EpubView"));
const DrawioView = lazy(() => import("./viewers/DrawioView"));
const ExcalidrawView = lazy(() => import("./viewers/ExcalidrawView"));
const OfficeView = lazy(() => import("./viewers/OfficeView"));

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

type SortKey = "name" | "size" | "modified";

const fmtDate = (s: number) => (s ? new Date(s * 1000).toLocaleDateString("vi-VN") : "");
const parentOf = (p: string) => p.split("/").slice(0, -1).join("/");

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
  const [open, setOpen] = createSignal<{ path: string; name: string; kind: VKind; text?: string; anchor?: string | null } | null>(null);
  const [query, setQuery] = createSignal("");
  const [sort, setSort] = createSignal<{ key: SortKey; asc: boolean }>({ key: "name", asc: true });
  const [view, setView] = createSignal<"list" | "grid">((localStorage.getItem("nk-view") as "grid") || "list");
  const [sideOpen, setSideOpen] = createSignal(false);
  const [menuFor, setMenuFor] = createSignal<string | null>(null);
  const [menuUpFor, setMenuUpFor] = createSignal<string | null>(null);
  const [userMenu, setUserMenu] = createSignal(false);
  const [userMenuUp, setUserMenuUp] = createSignal(false);
  const [showLook, setShowLook] = createSignal(false);
  const [page, setPage] = createSignal<"files" | "settings" | "trash">("files");
  type Dlg = { kind: "mkdir" } | { kind: "rename"; f: Entry } | { kind: "move"; paths: string[] };
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
  let fileInput!: HTMLInputElement;
  let scrollEl: HTMLDivElement | undefined;
  let vbox: HTMLDivElement | undefined;
  let pages = new Map<number, Entry[]>();
  let inflight = new Set<number>();
  let gen = 0;

  const join = (name: string) => (prefix() ? `${prefix()}/${name}` : name);

  const logout = () => {
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
      if (g === gen) fail(e, "Không tải được danh sách file");
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
      fail(e, "Không tải được danh sách file");
    } finally {
      setLoading(false);
    }
  };

  const refreshMe = () => api.me().then(setMe).catch(() => {});

  const enter = () => {
    setAuthed(true);
    load("");
    refreshMe();
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
      setErr("Đăng nhập thất bại — kiểm tra server/username/password");
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
      fail(e, "Không tải được danh sách file");
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
  createEffect(() => {
    const q = query().trim();
    clearTimeout(searchTimer);
    if (q.length < 2) {
      setResults(null);
      return;
    }
    searchTimer = window.setTimeout(async () => {
      try {
        const r = await api.search(q);
        if (query().trim() !== q) return;
        setResults(r);
        if (scrollEl) scrollEl.scrollTop = 0;
        setScrollTop(0);
      } catch (e) {
        fail(e, "Tìm kiếm lỗi");
      }
    }, 250);
  });

  const crumbs = createMemo(() => prefix().split("/").filter(Boolean));

  const toggleSort = (key: SortKey) => {
    setSort((s) => ({ key, asc: s.key === key ? !s.asc : true }));
    if (!results()) {
      if (scrollEl) scrollEl.scrollTop = 0;
      setScrollTop(0);
      fetchSet(prefix(), [0]).catch((e) => fail(e, "Không tải được danh sách file"));
    }
  };

  const openEntry = async (f: Entry) => {
    if (f.is_dir) return load(f.path);
    const path = f.path;
    const name = f.name;
    setErr("");
    try {
      const kind = viewerKind(name);
      if (kind === "markdown") setOpen({ path, name, kind, text: await api.readText(path) });
      else if (kind === "other") download(path, name);
      else setOpen({ path, name, kind });
    } catch (e) {
      fail(e, "Không mở được file");
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
    const t = target.trim();
    if (!t) {
      if (anchor) scrollToAnchor(anchor);
      return;
    }
    setErr("");
    const cur = open()?.path ?? "";
    const dir = cur.includes("/") ? cur.slice(0, cur.lastIndexOf("/")) : "";
    const cands: string[] = [];
    const pushCand = (p: string | null) => {
      if (!p) return;
      cands.push(p);
      if (!/\.[a-zA-Z0-9]{1,5}$/.test(p)) cands.push(`${p}.md`);
    };
    if (t.startsWith("/")) pushCand(normPath(t.slice(1).split("/")));
    else {
      pushCand(normPath([...(dir ? dir.split("/") : []), ...t.split("/")]));
      const base = t.split("/").pop()!;
      if (base !== t && !t.startsWith("./") && !t.startsWith("../")) {
        pushCand(normPath([...(dir ? dir.split("/") : []), base]));
      }
    }
    for (const c of [...new Set(cands)]) {
      try {
        const name = c.split("/").pop() ?? c;
        const kind = viewerKind(name);
        if (kind === "markdown") {
          const text = await api.readText(c);
          setOpen({ path: c, name, kind, text, anchor: anchor ?? null });
        } else if (kind !== "other") {
          await api.exists(c); // không có -> ném lỗi, thử ứng viên kế
          setOpen({ path: c, name, kind });
        } else continue;
        return;
      } catch {
        /* thử ứng viên tiếp theo */
      }
    }
    // fallback: tìm theo tên file trong cả vault (giống Obsidian resolve by basename)
    try {
      const base = t.split("/").pop()!.replace(/\.md$/i, "");
      const hits = (await api.search(base)).filter(
        (e) => !e.is_dir && (e.name === `${base}.md` || e.name === base),
      );
      hits.sort((a, b) => a.path.length - b.path.length);
      if (hits.length) {
        const h = hits[0];
        const kind = viewerKind(h.name);
        if (kind === "markdown") setOpen({ path: h.path, name: h.name, kind, text: await api.readText(h.path), anchor: anchor ?? null });
        else setOpen({ path: h.path, name: h.name, kind });
        return;
      }
    } catch {
      /* bỏ qua, báo lỗi chung bên dưới */
    }
    fail(new Error("404"), `Không tìm thấy "${t}" trong vault`);
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
      const url = await api.downloadUrl(path);
      const a = document.createElement("a");
      a.href = url;
      a.download = name;
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 10_000);
    } catch (e) {
      fail(e, "Không tải được file");
    }
  };

  /** Tải lại sau thao tác: trang đang xem hoặc kết quả tìm kiếm. */
  const reload = async () => {
    const q = query().trim();
    if (results() && q.length >= 2) setResults(await api.search(q).catch(() => results()));
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
      const items = await each("Đang xoá", paths, (p) => api.remove(p));
      flash(items.length === 1 ? `Đã chuyển "${items[0].orig_path.split("/").pop()}" vào thùng rác` : `Đã chuyển ${items.length} mục vào thùng rác`, async () => {
        await each("Đang khôi phục", items.map((t) => t.id), (id) => api.restore(id));
        flash("Đã khôi phục");
      });
    } catch (e) {
      fail({ message: errText(e) }, "Không xoá được");
    }
  };

  /** Chuyển các mục vào `dest`. Lỗi ném ra để dialog hiện. */
  const moveTo = async (paths: string[], dest: string) => {
    const todo = paths.filter((p) => parentOf(p) !== dest && dest !== p && !dest.startsWith(p + "/"));
    if (!todo.length) return;
    await each("Đang di chuyển", todo, (p) => api.move(p, joinPath(dest, p.split("/").pop()!)));
    flash(`Đã chuyển ${todo.length} mục tới ${dest || "Tất cả file"}`);
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
    if (total() > 5000) return setErr("Chỉ chọn tất cả được khi thư mục có tối đa 5.000 mục");
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
      moveTo(JSON.parse(raw), dest()).catch((x) => fail({ message: errText(x) }, "Không di chuyển được"));
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

  const doUpload = async (list: FileList | null) => {
    if (!list?.length) return;
    const items = Array.from(list);
    try {
      for (let i = 0; i < items.length; i++) {
        const f = items[i];
        setUpload({ name: f.name, pct: 0, left: items.length - i });
        await api.upload(join(f.name), f, (done) =>
          setUpload({ name: f.name, pct: Math.round((done / Math.max(f.size, 1)) * 100), left: items.length - i }),
        );
      }
    } catch (e) {
      fail(e, "Upload lỗi");
    } finally {
      setUpload(null);
      fileInput.value = "";
      refresh();
      refreshMe();
    }
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
      <button class="icon-btn" title="Thao tác" onClick={(e) => {
        const n = p.f.path;
        if (menuFor() === n) { setMenuFor(null); setMenuUpFor(null); return; }
        setMenuUpFor(flipIfNeeded(e.currentTarget as HTMLElement) ? n : null);
        setMenuFor(n);
      }}>
        <Icon name="more" />
      </button>
      <Show when={menuFor() === p.f.path}>
        <div class="menu" classList={{ up: menuUpFor() === p.f.path }}>
          <Show when={!p.f.is_dir}>
            <button onClick={() => { closeMenu(); download(p.f.path, p.f.name); }}>
              <Icon name="download" /> Tải xuống
            </button>
          </Show>
          <Show when={p.f.is_dir}>
            <button onClick={() => { closeMenu(); load(p.f.path); }}>
              <Icon name="folder" /> Mở
            </button>
          </Show>
          <button onClick={() => { closeMenu(); setDlg({ kind: "rename", f: p.f }); }}>
            <Icon name="edit" /> Đổi tên
          </button>
          <button onClick={() => { closeMenu(); setDlg({ kind: "move", paths: [p.f.path] }); }}>
            <Icon name="move" /> Di chuyển
          </button>
          <button onClick={() => { closeMenu(); toggleSel(p.f.path, true); lastIdx = -1; }}>
            <Icon name="check" /> Chọn
          </button>
          <div class="menu-sep" />
          <button class="danger" onClick={() => { closeMenu(); remove([p.f.path]); }}>
            <Icon name="trash" /> Xoá
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
      <div class="wall" style={{ background: wallCss() }} />
      <div class="wall-dim" style={{ opacity: ui().bg === "image" ? ui().dim : 0 }} />

      <Switch>
        <Match when={needReset()}>
          <div class="auth">
            <div class="auth-card glass">
              <Logo />
              <h2>Đặt mật khẩu mới</h2>
              <p class="muted">Tài khoản mới hoặc vừa được quản trị viên đặt lại mật khẩu — hãy đặt mật khẩu của riêng bạn (tối thiểu 8 ký tự).</p>
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

        <Match when={!authed()}>
          <div class="auth">
            <form class="auth-card glass" onSubmit={(e) => { e.preventDefault(); doLogin(); }}>
              <Logo />
              <p class="muted">Đám mây riêng của bạn</p>
              <Show when={err()}><p class="err">{err()}</p></Show>
              <Show when={!isSelfHostedWeb()}>
                <label>Server</label>
                <input value={server()} onInput={(e) => setServer(e.currentTarget.value)} placeholder="http://mini:8011" />
              </Show>
              <label>Tên đăng nhập</label>
              <input value={username()} onInput={(e) => setUsername(e.currentTarget.value)} autocomplete="username" />
              <label>Mật khẩu</label>
              <input type="password" value={password()} onInput={(e) => setPassword(e.currentTarget.value)} autocomplete="current-password" />
              <button type="submit" class="btn primary block">Đăng nhập</button>
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
            <div class="hdr-search">
              <Icon name="search" />
              <input value={query()} onInput={(e) => { setPage("files"); setQuery(e.currentTarget.value); }} placeholder="Tìm tất cả file" />
              <Show when={query()}>
                <button class="icon-btn sm" onClick={() => setQuery("")}><Icon name="close" size={14} /></button>
              </Show>
            </div>
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
                    <Icon name="user" /> Hồ sơ & cài đặt
                  </button>
                  <button onClick={() => openSettings("security")}>
                    <Icon name="key" /> Đổi mật khẩu
                  </button>
                  <button onClick={() => { setUserMenu(false); setShowLook(true); }}>
                    <Icon name="palette" /> Giao diện & nền
                  </button>
                  <Show when={me()?.is_admin}>
                    <div class="menu-sep" />
                    <button onClick={() => openSettings("users")}>
                      <Icon name="users" /> Quản lý người dùng
                    </button>
                    <button onClick={() => openSettings("system")}>
                      <Icon name="server" /> Hệ thống
                    </button>
                  </Show>
                  <div class="menu-sep" />
                  <button onClick={logout}>
                    <Icon name="logout" /> Đăng xuất
                  </button>
                </div>
              </Show>
            </div>
          </header>

          <div class="body">
            <Show when={sideOpen()}>
              <div class="scrim only-mobile" onClick={() => setSideOpen(false)} />
            </Show>
            <aside class="side glass" classList={{ open: sideOpen() }}>
              <div class="side-brand only-mobile"><Logo /></div>
              <nav>
                <button class="nav" classList={{ active: page() === "files" && !prefix() && !open() }} onClick={() => load("")}>
                  <Icon name="home" /> Tất cả file
                </button>
                <button class="nav" classList={{ active: page() === "trash" }} onClick={() => { setPage("trash"); setSideOpen(false); }}>
                  <Icon name="trash" /> Thùng rác
                </button>
                <Show when={me()?.is_admin}>
                  <button class="nav" classList={{ active: page() === "settings" && (setTab() === "users" || setTab() === "system") }} onClick={() => openSettings("users")}>
                    <Icon name="users" /> Quản trị
                  </button>
                </Show>
                <button class="nav" classList={{ active: page() === "settings" && !["users", "system"].includes(setTab()) }} onClick={() => openSettings("profile")}>
                  <Icon name="settings" /> Cài đặt
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
              </div>
            </aside>

            <main class="panel glass">
              <Show when={page() === "settings" && me()}>
                <Show when={err()}>
                  <div class="err banner">
                    <span>{err()}</span>
                    <button class="icon-btn sm" onClick={() => setErr("")}><Icon name="close" size={14} /></button>
                  </div>
                </Show>
                <Settings me={me()!} tab={setTab()} onTab={setSetTab} onMe={setMe} onError={setErr} onOpen={(f) => load(f)} />
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
              <Show when={page() === "files"}>
              <Show when={sel().size}>
                <div class="toolbar selbar">
                  <button class="icon-btn" title="Bỏ chọn (Esc)" onClick={() => setSel(new Set<string>())}><Icon name="close" /></button>
                  <b>Đã chọn {sel().size.toLocaleString("vi-VN")}</b>
                  <button class="btn ghost sm" onClick={selectAll}>Chọn tất cả</button>
                  <span class="grow" />
                  <button class="btn ghost sm" onClick={() => setDlg({ kind: "move", paths: [...sel()] })}>
                    <Icon name="move" /> <span class="hide-xs">Di chuyển</span>
                  </button>
                  <button class="btn danger sm" onClick={() => remove([...sel()])}>
                    <Icon name="trash" /> <span class="hide-xs">Xoá</span>
                  </button>
                </div>
              </Show>
              <div class="toolbar" classList={{ hidden: sel().size > 0 }}>
                <button class="btn primary" onClick={() => fileInput.click()} disabled={!!upload()}>
                  <Icon name="upload" /> <span class="hide-xs">Tải lên</span>
                </button>
                <Show when={!results()}>
                  <button class="btn ghost" title="Thư mục mới" onClick={() => setDlg({ kind: "mkdir" })}>
                    <Icon name="folderPlus" /> <span class="hide-xs">Thư mục mới</span>
                  </button>
                </Show>
                <input ref={fileInput} type="file" multiple hidden onChange={(e) => doUpload(e.currentTarget.files)} />
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
                          {c}
                        </button>
                      </>
                    )}
                  </For>
                  <Show when={open()}>
                    <Icon name="chevron" size={14} class="ic dim" />
                    <span class="crumb cur">{open()!.path.split("/").pop()}</span>
                  </Show>
                </nav>
                <Show when={!open()}>
                  <div class="seg">
                    <button classList={{ on: view() === "list" }} title="Danh sách" onClick={() => { setView("list"); localStorage.setItem("nk-view", "list"); }}>
                      <Icon name="list" />
                    </button>
                    <button classList={{ on: view() === "grid" }} title="Lưới" onClick={() => { setView("grid"); localStorage.setItem("nk-view", "grid"); }}>
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
                    <button class="btn ghost" onClick={() => setOpen(null)}>
                      <Icon name="back" /> Quay lại
                    </button>
                    <Suspense fallback={<p class="muted">Đang mở…</p>}>
                      <Switch>
                        <Match when={open()!.kind === "markdown"}>
                          <MarkdownView path={open()!.path} text={open()!.text ?? ""} anchor={open()!.anchor ?? null} onOpenLink={openWikiLink} />
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
                          <PdfView path={open()!.path} />
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
                        Tên {sort().key === "name" ? (sort().asc ? "↑" : "↓") : ""}
                      </span>
                      <span class="c-size sortable" onClick={() => toggleSort("size")}>
                        Dung lượng {sort().key === "size" ? (sort().asc ? "↑" : "↓") : ""}
                      </span>
                      <span class="c-date sortable" onClick={() => toggleSort("modified")}>
                        Sửa đổi {sort().key === "modified" ? (sort().asc ? "↑" : "↓") : ""}
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
                                const k = () => kindOf(f().name, f().is_dir);
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
                                        <Show when={results() && parentOf(f().path)}>
                                          <span class="fpath">{parentOf(f().path)}</span>
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
                                    const k = () => kindOf(f().name, f().is_dir);
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
                                        <span class="tile-sub">{f().is_dir ? "Thư mục" : `${fmtSize(f().size)} · ${fmtDate(f().modified)}`}</span>
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
                      <p>{results() ? "Không có kết quả" : "Thư mục trống"}</p>
                      <Show when={!results()}>
                        <button class="btn primary" onClick={() => fileInput.click()}>
                          <Icon name="upload" /> Tải file lên
                        </button>
                      </Show>
                    </div>
                  </Show>
                </div>
                <div class="foot">
                  <Show
                    when={!results()}
                    fallback={<span>{results()!.length >= 300 ? "300+ kết quả (gõ cụ thể hơn)" : `${results()!.length} kết quả`}</span>}
                  >
                    <span>
                      {stats().dirs.toLocaleString("vi-VN")} thư mục · {stats().files.toLocaleString("vi-VN")} file
                    </span>
                    <span>{fmtSize(stats().bytes)}</span>
                  </Show>
                </div>
              </Show>
              </Show>
            </main>
          </div>

          <Show when={upload()}>
            <div class="toast glass">
              <div class="toast-row">
                <Icon name="upload" />
                <span class="fname">{upload()!.name}</span>
                <span class="muted">{upload()!.pct}%</span>
              </div>
              <div class="bar"><i style={{ width: `${upload()!.pct}%` }} /></div>
              <Show when={upload()!.left > 1}>
                <span class="muted sm">còn {upload()!.left - 1} file</span>
              </Show>
            </div>
          </Show>

          <Show when={working() || notice()}>
            <div class="toast glass note" classList={{ lift: !!upload() }}>
              <Show when={working()} fallback={
                <div class="toast-row">
                  <Icon name="check" />
                  <span class="fname">{notice()!.text}</span>
                  <Show when={notice()!.undo}>
                    <button class="btn ghost sm" onClick={() => { const u = notice()!.undo!; setNotice(null); u().catch((e) => fail({ message: errText(e) }, "Không hoàn tác được")); }}>
                      Hoàn tác
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
                  <NameDialog title="Thư mục mới" label={`Tạo trong ${prefix() || "Tất cả file"}`} ok="Tạo" onClose={() => setDlg(null)} onSubmit={async (n) => {
                    await api.mkdir(join(n));
                    setDlg(null);
                    await reload();
                  }} />
                </Match>
                <Match when={d().kind === "rename" && (d() as { f: Entry }).f}>
                  {(f) => (
                    <NameDialog title={f().is_dir ? "Đổi tên thư mục" : "Đổi tên file"} label="Tên mới" initial={f().name} ok="Đổi tên" onClose={() => setDlg(null)} onSubmit={async (n) => {
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
              </Switch>
            )}
          </Show>

          <Show when={showLook()}>
            <LookDialog onClose={() => setShowLook(false)} onError={(m) => setErr(m)} />
          </Show>
        </Match>
      </Switch>
    </div>
  );
}
