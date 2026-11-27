import { For, Show, Suspense, createResource, createSignal, lazy, onCleanup } from "solid-js";
import { api, serverOrigin, type Entry, type LinkMeta, type ShareRow, type SharedItem } from "./api";
import { fmtDate, fmtNum, t } from "./i18n";
import { errText } from "./fileops";
import { Icon } from "./icons";
import { ViewerOverlay } from "./viewers/ViewerOverlay";
import { extOfName, hasThumb } from "./viewers/kinds";

const PdfThumb = lazy(() => import("./viewers/PdfThumb"));

const expText = (e: number | null) => (!e ? t("sh.forever") : e * 1000 < Date.now() ? t("sh.expired", { d: fmtDate(e) }) : t("sh.until", { d: fmtDate(e) }));

/** Dialog quản lý chia sẻ + link công khai cho 1 file/thư mục của mình.
 *  owner: chia sẻ lại mục người khác chia sẻ cho tôi (quyền ≤ myPerm, không có link công khai). */
export function ShareDialog(props: { path: string; owner?: string; myPerm?: string; onClose: () => void; onError: (m: string) => void }) {
  const [rows, { refetch }] = createResource(() => props.path, (p) => api.shares(p, props.owner).catch((e) => (props.onError(errText(e)), [] as ShareRow[])));
  const [user, setUser] = createSignal("");
  const [perm, setPerm] = createSignal("view");
  const [pw, setPw] = createSignal("");
  const [days, setDays] = createSignal("");
  const name = () => props.path.split("/").pop() ?? props.path;
  const run = async (f: () => Promise<unknown>) => {
    try {
      await f();
      refetch();
    } catch (e) {
      props.onError(errText(e));
    }
  };
  const userShares = () => (rows() ?? []).filter((r) => r.target);
  const links = () => (rows() ?? []).filter((r) => r.token);
  const linkUrl = (t: string) => `${serverOrigin()}/#/s/${t}`;
  const copyLink = async (t: string) => {
    try {
      await navigator.clipboard?.writeText(linkUrl(t));
    } catch {
      props.onError(`Link: ${linkUrl(t)}`);
    }
  };
  return (
    <div class="modal-bg" onClick={props.onClose}>
      <div class="modal glass" onClick={(e) => e.stopPropagation()}>
        <div class="modal-head">
          <h3>{t("sh.dlgTitle", { name: name() })}</h3>
          <button class="icon-btn" onClick={props.onClose}><Icon name="close" /></button>
        </div>
        <Show when={rows.loading}><p class="muted">{t("c.loading")}</p></Show>
        <div class="sh-sec">
          <b>{t("sh.toUser")}</b>
          <div class="sh-add">
            <input value={user()} onInput={(e) => setUser(e.currentTarget.value)} placeholder={t("u.username")} />
            <select value={perm()} onChange={(e) => setPerm(e.currentTarget.value)} title={t("sh.perm")}>
              <option value="view">{t("sh.canView")}</option>
              <Show when={!props.owner || props.myPerm === "edit"}><option value="edit">{t("sh.canEdit")}</option></Show>
            </select>
            <button class="btn primary sm" onClick={() => user().trim() && run(() => api.shareWith(props.path, user().trim(), perm(), props.owner).then(() => setUser("")))}>{t("c.add")}</button>
          </div>
          <For each={userShares()} fallback={<p class="muted xs">{t("sh.noShares")}</p>}>
            {(r) => (
              <div class="sh-row">
                <Icon name="user" size={15} class="ic muted" />
                <span>{r.target}</span>
                <span class="muted xs">{r.permission === "edit" ? t("sh.canEdit") : t("sh.canView")} · {expText(r.expires_at)}{r.granted_by && !props.owner ? ` · ${t("sh.via", { name: r.granted_by })}` : ""}</span>
                <span class="grow" />
                <button class="icon-btn sm danger" title={t("sh.revoke")} onClick={() => confirm(t("sh.revokeAsk", { name: r.target ?? "" })) && run(() => api.unshare(r.id))}><Icon name="trash" size={13} /></button>
              </div>
            )}
          </For>
        </div>
        <Show when={!props.owner}>
        <div class="sh-sec">
          <b>{t("sh.pubTitle")}</b>
          <div class="sh-add">
            <input type="password" value={pw()} onInput={(e) => setPw(e.currentTarget.value)} placeholder={t("sh.pwPh")} autocomplete="new-password" />
            <select value={days()} onChange={(e) => setDays(e.currentTarget.value)} title={t("sh.expiry")}>
              <option value="">{t("sh.forever")}</option>
              <option value="1">{t("sh.d1")}</option>
              <option value="7">{t("sh.d7")}</option>
              <option value="30">{t("sh.d30")}</option>
            </select>
            <button class="btn primary sm" onClick={() => run(() => api.createLink(props.path, pw(), days() ? +days() : null))}>{t("sh.makeLink")}</button>
          </div>
          <For each={links()} fallback={<p class="muted xs">{t("sh.noLinks")}</p>}>
            {(r) => (
              <div class="sh-row">
                <Icon name="copy" size={15} class="ic muted" />
                <input class="sh-link" readonly value={linkUrl(r.token!)} onFocus={(e) => e.currentTarget.select()} title={t("sh.copyHint")} />
                <span class="muted xs">{r.has_password ? t("sh.hasPw") : t("sh.noPw")} · {expText(r.expires_at)}</span>
                <span class="grow" />
                <button class="btn ghost sm" onClick={() => copyLink(r.token!)}>{t("c.copyBtn")}</button>
                <button class="icon-btn sm danger" title={t("sh.delLink")} onClick={() => confirm(t("sh.delLinkAsk")) && run(() => api.unshare(r.id))}><Icon name="trash" size={13} /></button>
              </div>
            )}
          </For>
        </div>
        </Show>
        <p class="muted xs">{props.owner ? t("sh.reshareNote") : t("sh.note")}</p>
      </div>
    </div>
  );
}

/** Trang "Được chia sẻ": duyệt + tải + (sửa nếu được phép) nội dung người khác chia sẻ. */
export function SharedPage(props: { onError: (m: string) => void }) {
  const [list, { refetch }] = createResource(() => api.shared().catch((e) => (props.onError(errText(e)), [] as SharedItem[])), { initialValue: [] });
  const [sel, setSel] = createSignal<SharedItem | null>(null);
  const [view, setView] = createSignal<string | null>(null);
  const [shareFor, setShareFor] = createSignal<string | null>(null);
  const [prefix, setPrefix] = createSignal("");
  const [query, setQuery] = createSignal("");
  // tìm theo tên trong cả mục được chia sẻ (chờ gõ xong 300ms)
  const [term, setTerm] = createSignal("");
  let qTimer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(qTimer));
  const onQuery = (v: string) => {
    setQuery(v);
    clearTimeout(qTimer);
    qTimer = setTimeout(() => setTerm(v.trim()), 300);
  };
  const [hits] = createResource(
    () => (sel()?.is_dir && term() ? { o: sel()!.owner_id, r: sel()!.path, q: term() } : null),
    (v) => api.sharedSearch(v.o, v.r, v.q).catch((e) => (props.onError(errText(e)), [] as Entry[])),
  );
  const abs = () => (prefix() ? `${sel()!.path}/${prefix()}` : sel()!.path);
  const [page, { refetch: refetchFiles }] = createResource(
    () => (sel() ? { o: sel()!.owner_id, p: abs() } : null),
    (v) => api.files(v.p, { sort: "name", desc: false, offset: 0, limit: 200 }, v.o).catch((e) => (props.onError(errText(e)), { entries: [], dirs: 0, files: 0, bytes: 0 })),
  );
  const crumbs = () => prefix().split("/").filter(Boolean);
  const clearQuery = () => {
    clearTimeout(qTimer);
    setQuery("");
    setTerm("");
  };
  const go = (i: number) => (clearQuery(), setPrefix(crumbs().slice(0, i).join("/")));
  const pick = (s: SharedItem) => {
    setSel(s);
    setPrefix("");
    clearQuery();
  };
  const openDir = (e: Entry) => (clearQuery(), setPrefix(e.path.slice(sel()!.path.length + 1)));
  const rows = () => (term() ? hits() ?? [] : page()?.entries ?? []);
  const reload = () => {
    refetch();
    refetchFiles();
  };
  const run = async (f: () => Promise<unknown>) => {
    try {
      await f();
    } catch (e) {
      props.onError(errText(e));
    }
    reload();
  };
  const dl = async (e: Entry) => {
    try {
      const u = await api.sharedDirectUrl(sel()!.owner_id, e.path);
      const a = document.createElement("a");
      a.href = u;
      a.download = e.name;
      a.click();
    } catch (err) {
      props.onError(errText(err));
    }
  };
  const saveCopy = (path: string) => {
    const to = prompt(t("sh.savePrompt"), path.split("/").pop() ?? "bản sao");
    if (to?.trim()) run(() => api.saveCopy(sel()!.owner_id, path, to.trim()));
  };
  const newFolder = () => {
    const n = prompt(t("f.newFolderPrompt"))?.trim();
    if (n) run(() => api.mkdir(`${abs()}/${n}`, sel()!.owner_id));
  };
  const up = (fs: FileList | null) => {
    if (!fs?.length || !sel()) return;
    run(async () => {
      for (const f of Array.from(fs)) await api.upload(`${abs()}/${f.name}`, f, undefined, sel()!.owner_id);
    });
  };
  let fileInput!: HTMLInputElement;
  const canEdit = () => sel()?.permission === "edit";
  const relName = (p: string) => (sel() && (p === sel()!.path || p.startsWith(sel()!.path + "/")) ? p.slice(sel()!.path.length + 1) || p.split("/").pop()! : p);
  return (
    <div class="kanban">
      <div class="toolbar kb-bar">
        <h2 class="tb-title"><Icon name="users" /> {t("nav.shared")}</h2>
        <div class="kb-boards">
          <For each={list()} fallback={!list.loading && <span class="muted sm">{t("sh.noShared")}</span>}>
            {(s) => (
              <button class="chip" classList={{ on: sel()?.id === s.id }} onClick={() => pick(s)} title={`${s.owner_name} · ${s.permission === "edit" ? t("sh.canEdit") : t("sh.viewOnly")}${s.via ? ` · ${t("sh.via", { name: s.via })}` : ""}`}>
                {s.owner_name}/{s.path.split("/").pop()}
              </button>
            )}
          </For>
        </div>
      </div>
      <Show when={sel()} fallback={<div class="empty"><Icon name="users" size={48} /><p>{t("sh.pickOne")}</p></div>}>
        <div class="kb-sub">
          <Show when={crumbs().length} fallback={<b>{sel()!.path}</b>}>
            <button class="link" onClick={() => go(0)}>{sel()!.path.split("/").pop()}</button>
            <For each={crumbs()}>{(c, i) => <><span class="muted">/</span><button class="link" onClick={() => go(i() + 1)}>{c}</button></>}</For>
          </Show>
          <span class="muted xs">{t("sh.ownerLine", { name: sel()!.owner_name, perm: sel()!.permission === "edit" ? t("sh.canEdit") : t("sh.viewOnly") })}{sel()!.via ? ` · ${t("sh.via", { name: sel()!.via! })}` : ""}</span>
          <Show when={!sel()!.is_dir}>
            <button class="btn ghost sm" onClick={() => setView(sel()!.path)}><Icon name="file" size={14} /> {t("c.open")}</button>
            <button class="btn ghost sm" onClick={async () => {
              try {
                const u = await api.sharedDirectUrl(sel()!.owner_id, sel()!.path);
                const a = document.createElement("a");
                a.href = u;
                a.download = sel()!.path.split("/").pop() ?? "";
                a.click();
              } catch (e) {
                props.onError(errText(e));
              }
            }}><Icon name="download" size={14} /> {t("sh.dl")}</button>
          </Show>
          <span class="grow" />
          <Show when={sel()!.is_dir}>
            <input class="sh-search" type="search" value={query()} onInput={(e) => onQuery(e.currentTarget.value)} placeholder={t("sh.search")} />
            <a class="btn ghost sm" href={api.zipDownloadUrl(abs(), sel()!.owner_id)} download="" title={t("f.zip")}><Icon name="download" size={15} /> zip</a>
          </Show>
          <button class="btn ghost sm" onClick={() => setShareFor(abs())} title={t("f.share")}><Icon name="users" size={15} /> {t("f.share")}</button>
          <Show when={canEdit()}>
            <button class="btn ghost sm" onClick={newFolder}><Icon name="plus" size={15} /> {t("sh.mkdir")}</button>
            <button class="btn ghost sm" onClick={() => fileInput.click()}><Icon name="upload" size={15} /> {t("c.upload")}</button>
            <input ref={fileInput} type="file" multiple hidden onChange={(e) => { up(e.currentTarget.files); e.currentTarget.value = ""; }} />
          </Show>
          <button class="btn ghost sm" onClick={() => saveCopy(abs())} title={t("sh.saveDirTip")}><Icon name="copy" size={15} /> {t("sh.saveCopy")}</button>
        </div>
        <Show when={!(term() ? hits.loading : page.loading)} fallback={<p class="muted pad">{t("c.loading")}</p>}>
          <div class="sh-list">
            <For each={rows()} fallback={<p class="muted pad">{term() ? t("sh.noHits") : t("sh.emptyList")}</p>}>
              {(e) => (
                <div class="tk-row">
                  <SharedIcon e={e} owner={sel()!.owner_id} />
                  <div class="tk-main">
                    <b>{relName(e.path)}</b>
                    <span class="muted xs">{e.is_dir ? t("f.dirLabel") : `${fmtNum(e.size)} B`}</span>
                  </div>
                  <Show when={e.is_dir}>
                    <button class="btn ghost sm" onClick={() => openDir(e)}>{t("c.open")}</button>
                    <a class="btn ghost sm" href={api.zipDownloadUrl(e.path, sel()!.owner_id)} download="" title={t("f.zip")}><Icon name="download" size={14} /></a>
                  </Show>
                  <Show when={!e.is_dir}>
                    <button class="btn ghost sm" onClick={() => setView(e.path)}><Icon name="file" size={14} /> {t("c.open")}</button>
                    <button class="btn ghost sm" onClick={() => dl(e)}><Icon name="download" size={14} /> {t("sh.dl")}</button>
                    <button class="btn ghost sm" onClick={() => saveCopy(e.path)} title={t("sh.saveTip")}><Icon name="copy" size={14} /></button>
                  </Show>
                  <button class="btn ghost sm" onClick={() => setShareFor(e.path)} title={t("f.share")}><Icon name="users" size={14} /></button>
                  <Show when={canEdit()}>
                    <button class="icon-btn sm danger" title={t("sh.delTip")} onClick={() => confirm(t("sh.delAsk", { name: e.name })) && run(() => api.remove(e.path, sel()!.owner_id))}><Icon name="trash" size={13} /></button>
                  </Show>
                </div>
              )}
            </For>
          </div>
        </Show>
      </Show>
      <Show when={view() && sel()}>
        <ViewerOverlay path={view()!} owner={sel()!.owner_id} onClose={() => setView(null)} onError={props.onError} />
      </Show>
      <Show when={shareFor() && sel()}>
        <ShareDialog path={shareFor()!} owner={sel()!.owner_id} myPerm={sel()!.permission} onClose={() => setShareFor(null)} onError={props.onError} />
      </Show>
    </div>
  );
}

/** Icon dòng: thumbnail ảnh/video (server) hoặc trang 1 pdf (pdf.js, file ≤ 20MB), lỗi thì về icon thường. */
function SharedIcon(props: { e: Entry; owner: string }) {
  const [bad, setBad] = createSignal(false);
  const icon = () => <Icon name={props.e.is_dir ? "folder" : "file"} size={16} class="ic muted" />;
  if (props.e.is_dir) return icon();
  if (extOfName(props.e.name) === "pdf" && props.e.size <= 20 * 1024 * 1024) {
    return (
      <Show when={!bad()} fallback={icon()}>
        <Suspense fallback={icon()}><PdfThumb url={api.streamUrl(props.e.path, props.owner)} size={40} class="sh-thumb" onFail={() => setBad(true)} /></Suspense>
      </Show>
    );
  }
  return (
    <Show when={hasThumb(props.e.name) && !bad()} fallback={icon()}>
      <img class="sh-thumb" loading="lazy" decoding="async" src={api.thumbUrl(props.e.path, 128, props.owner)} alt="" onError={() => setBad(true)} />
    </Show>
  );
}

/** Cổng link công khai: kiểm tra link, hỏi mật khẩu nếu cần; mở được thì App hiện giao diện duyệt file chỉ xem. */
export function PublicGate(props: { token: string; onOpen: (m: LinkMeta, pw: string) => void }) {
  const [pw, setPw] = createSignal("");
  const [tried, setTried] = createSignal("");
  const [gone, setGone] = createSignal("");
  // 401 = sai/thiếu mật khẩu -> hiện ô nhập; lỗi khác = link hỏng/hết hạn.
  const [meta] = createResource(
    () => ({ t: props.token, p: tried() }),
    async (v): Promise<LinkMeta | null> => {
      try {
        setGone("");
        const m = await api.linkMeta(v.t, v.p || undefined);
        if (!m.locked) props.onOpen(m, v.p);
        return m;
      } catch (e) {
        if (String((e as Error)?.message ?? e).includes("unauthorized")) return { title: "", is_dir: true, locked: true, expires_at: null };
        setGone(errText(e));
        return null;
      }
    },
  );
  return (
    <div class="auth">
      <div class="auth-card glass pub">
        <h2>{t("sh.pubH2")}</h2>
        <Show when={meta.loading}><p class="muted">{t("c.loading")}</p></Show>
        <Show when={!meta.loading && gone()}><p class="err">{gone()}</p></Show>
        <Show when={meta()?.locked}>
          <Show when={meta()!.title}><h3>{meta()!.title}</h3></Show>
          <p class="muted">{t("sh.needPw")}</p>
          <Show when={tried() && !meta.loading}><p class="err">{t("sh.badPw")}</p></Show>
          <div class="sh-add">
            <input type="password" value={pw()} onInput={(e) => setPw(e.currentTarget.value)} placeholder={t("sh.pwLinkPh")} onKeyDown={(e) => e.key === "Enter" && setTried(pw())} />
            <button class="btn primary sm" onClick={() => setTried(pw())}>{t("c.open")}</button>
          </div>
        </Show>
        <a class="link sm" href="#/">{t("login.go")}</a>
      </div>
    </div>
  );
}
