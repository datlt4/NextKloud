import { For, Index, Match, Show, Switch, batch, createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { api, isSelfHostedWeb, serverOrigin, type Entry, type Me } from "./api";
import { PRESETS, previewDim, setDim, setImage, setPreset, setTheme, syncFromServer, ui, wallCss } from "./background";
import { fmtSize } from "./format";
import { Icon, ext, kindOf } from "./icons";
import UsersAdmin from "./UsersAdmin";
import MarkdownView from "./viewers/MarkdownView";

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
  const [open, setOpen] = createSignal<{ path: string; text: string } | null>(null);
  const [query, setQuery] = createSignal("");
  const [sort, setSort] = createSignal<{ key: SortKey; asc: boolean }>({ key: "name", asc: true });
  const [view, setView] = createSignal<"list" | "grid">((localStorage.getItem("nk-view") as "grid") || "list");
  const [sideOpen, setSideOpen] = createSignal(false);
  const [menuFor, setMenuFor] = createSignal<string | null>(null);
  const [menuUpFor, setMenuUpFor] = createSignal<string | null>(null);
  const [userMenu, setUserMenu] = createSignal(false);
  const [userMenuUp, setUserMenuUp] = createSignal(false);
  const [showLook, setShowLook] = createSignal(false);
  const [page, setPage] = createSignal<"files" | "users">("files");
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
    try {
      if (!(await fetchSet(p, [0]))) return;
      setPage("files");
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

  const doLogin = async () => {
    setErr("");
    try {
      localStorage.setItem("nk-server", server());
      const r = await api.login(username(), password());
      localStorage.setItem("nk-token", r.token);
      if (r.must_reset) setNeedReset(true);
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
    setErr("");
    try {
      if (/\.md$/i.test(f.name)) setOpen({ path, text: await api.readText(path) });
      else download(path, f.name);
    } catch (e) {
      fail(e, "Không mở được file");
    }
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

  const remove = async (f: Entry) => {
    if (!confirm(`Xoá "${f.name}"?`)) return;
    try {
      await api.remove(f.path);
      const r = results();
      if (r) setResults(r.filter((x) => x.path !== f.path));
      else await refresh();
      refreshMe();
    } catch (e) {
      fail(e, "Không xoá được");
    }
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
            <button onClick={() => { setMenuFor(null); setMenuUpFor(null); download(p.f.path, p.f.name); }}>
              <Icon name="download" /> Tải xuống
            </button>
            <button class="danger" onClick={() => { setMenuFor(null); setMenuUpFor(null); remove(p.f); }}>
              <Icon name="trash" /> Xoá
            </button>
          </Show>
          <Show when={p.f.is_dir}>
            <button onClick={() => { setMenuFor(null); setMenuUpFor(null); load(p.f.path); }}>
              <Icon name="folder" /> Mở
            </button>
          </Show>
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
              <h2>Đổi mật khẩu lần đầu</h2>
              <p class="muted">Tài khoản mới cấp — đặt mật khẩu mới (tối thiểu 8 ký tự).</p>
              <Show when={err()}><p class="err">{err()}</p></Show>
              <ResetForm
                onDone={async (o, n) => {
                  setErr("");
                  try {
                    await api.changePassword(o, n);
                    setNeedReset(false);
                    enter();
                  } catch (e) {
                    setErr("Đổi mật khẩu thất bại: " + String((e as Error).message));
                  }
                }}
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
              }} title={me()?.username}>
                {(me()?.username ?? "?").slice(0, 1).toUpperCase()}
              </button>
              <Show when={userMenu()}>
                <div class="menu right" classList={{ up: userMenuUp() }}>
                  <div class="menu-head">
                    <b>{me()?.username}</b>
                    <Show when={me()?.is_admin}><span class="tag">admin</span></Show>
                  </div>
                  <button onClick={() => { setUserMenu(false); setShowLook(true); }}>
                    <Icon name="palette" /> Giao diện & nền
                  </button>
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
                <Show when={me()?.is_admin}>
                  <button class="nav" classList={{ active: page() === "users" }} onClick={() => { setSideOpen(false); setPage("users"); }}>
                    <Icon name="users" /> Người dùng
                  </button>
                </Show>
                <button class="nav" onClick={() => { setSideOpen(false); setShowLook(true); }}>
                  <Icon name="palette" /> Giao diện & nền
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
              <Show when={page() === "users" && me()}>
                <Show when={err()}>
                  <div class="err banner">
                    <span>{err()}</span>
                    <button class="icon-btn sm" onClick={() => setErr("")}><Icon name="close" size={14} /></button>
                  </div>
                </Show>
                <UsersAdmin meId={me()!.id} onError={setErr} onChanged={refreshMe} />
              </Show>
              <Show when={page() === "files"}>
              <div class="toolbar">
                <button class="btn primary" onClick={() => fileInput.click()} disabled={!!upload()}>
                  <Icon name="upload" /> <span class="hide-xs">Tải lên</span>
                </button>
                <input ref={fileInput} type="file" multiple hidden onChange={(e) => doUpload(e.currentTarget.files)} />
                <nav class="crumbs">
                  <button class="crumb" onClick={() => load("")}>
                    <Icon name="home" size={16} />
                  </button>
                  <For each={crumbs()}>
                    {(c, i) => (
                      <>
                        <Icon name="chevron" size={14} class="ic dim" />
                        <button
                          class="crumb"
                          classList={{ cur: i() === crumbs().length - 1 && !open() }}
                          onClick={() => load(crumbs().slice(0, i() + 1).join("/"))}
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
                    <MarkdownView path={open()!.path} text={open()!.text} />
                  </div>
                }
              >
                <div
                  class="scroll"
                  classList={{ busy: loading() }}
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
                                  <div class="row vrow" style={pos()} classList={{ "menu-open": menuFor() === f().path }} onClick={() => openEntry(f())}>
                                    <span class="c-name">
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
                                      <div class="tile" classList={{ "menu-open": menuFor() === f().path }} onClick={() => openEntry(f())}>
                                        <div class="tile-art" style={{ color: k().color }}>
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

          <Show when={showLook()}>
            <LookDialog onClose={() => setShowLook(false)} onError={(m) => setErr(m)} />
          </Show>
        </Match>
      </Switch>
    </div>
  );
}

function LookDialog(props: { onClose: () => void; onError: (m: string) => void }) {
  const [busy, setBusy] = createSignal(false);
  let pick!: HTMLInputElement;
  const run = async (f: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await f();
    } catch (e) {
      props.onError("Không lưu được giao diện: " + String((e as Error).message));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div class="modal-bg" onClick={props.onClose}>
      <div class="modal glass" onClick={(e) => e.stopPropagation()}>
        <div class="modal-head">
          <h3>Giao diện & nền</h3>
          <button class="icon-btn" onClick={props.onClose}><Icon name="close" /></button>
        </div>
        <p class="muted sm">Lưu trên server — đồng bộ mọi thiết bị.</p>
        <div class="field-label">Chế độ màu</div>
        <div class="seg wide">
          <For each={[["auto", "Theo hệ thống"], ["light", "Sáng"], ["dark", "Tối"]] as const}>
            {([id, label]) => (
              <button classList={{ on: ui().theme === id }} disabled={busy()} onClick={() => run(() => setTheme(id))}>
                {label}
              </button>
            )}
          </For>
        </div>
        <div class="field-label">Hình nền</div>
        <div class="swatches">
          <button
            class="swatch upload"
            classList={{ on: ui().bg === "image" }}
            disabled={busy()}
            onClick={() => pick.click()}
            title="Ảnh của bạn"
          >
            <Icon name={busy() ? "cloud" : "image"} size={22} />
            <span>{busy() ? "Đang lưu…" : "Ảnh của bạn"}</span>
          </button>
          <For each={PRESETS}>
            {(p) => (
              <button
                class="swatch"
                classList={{ on: ui().bg === p.id }}
                style={{ background: p.css }}
                disabled={busy()}
                onClick={() => run(() => setPreset(p.id))}
              >
                <Show when={ui().bg === p.id}><Icon name="check" size={20} /></Show>
                <span>{p.name}</span>
              </button>
            )}
          </For>
        </div>
        <input
          ref={pick}
          type="file"
          accept="image/*"
          hidden
          onChange={(e) => {
            const f = e.currentTarget.files?.[0];
            if (f) run(() => setImage(f));
            e.currentTarget.value = "";
          }}
        />
        <Show when={ui().bg === "image"}>
          <label class="slider">
            <span>Làm tối ảnh nền</span>
            <input
              type="range"
              min="0"
              max="0.8"
              step="0.05"
              value={ui().dim}
              onInput={(e) => previewDim(+e.currentTarget.value)}
              onChange={(e) => run(() => setDim(+e.currentTarget.value))}
            />
          </label>
        </Show>
      </div>
    </div>
  );
}

function ResetForm(props: { onDone: (o: string, n: string) => void }) {
  const [o, setO] = createSignal("");
  const [n, setN] = createSignal("");
  return (
    <form onSubmit={(e) => { e.preventDefault(); props.onDone(o(), n()); }}>
      <label>Mật khẩu hiện tại</label>
      <input type="password" value={o()} onInput={(e) => setO(e.currentTarget.value)} autocomplete="current-password" />
      <label>Mật khẩu mới</label>
      <input type="password" value={n()} onInput={(e) => setN(e.currentTarget.value)} autocomplete="new-password" />
      <button type="submit" class="btn primary block">Đổi mật khẩu</button>
    </form>
  );
}
