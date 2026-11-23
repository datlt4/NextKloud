import { For, Show, createResource, createSignal } from "solid-js";
import { api, serverOrigin, type Entry, type LinkMeta, type ShareRow, type SharedItem } from "./api";
import { fmtDate, fmtNum, t } from "./i18n";
import { errText } from "./fileops";
import { Icon } from "./icons";

const expText = (e: number | null) => (!e ? t("sh.forever") : e * 1000 < Date.now() ? t("sh.expired", { d: fmtDate(e) }) : t("sh.until", { d: fmtDate(e) }));

/** Dialog quản lý chia sẻ + link công khai cho 1 file/thư mục của mình. */
export function ShareDialog(props: { path: string; onClose: () => void; onError: (m: string) => void }) {
  const [rows, { refetch }] = createResource(() => props.path, (p) => api.shares(p).catch((e) => (props.onError(errText(e)), [] as ShareRow[])));
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
              <option value="edit">{t("sh.canEdit")}</option>
            </select>
            <button class="btn primary sm" onClick={() => user().trim() && run(() => api.shareWith(props.path, user().trim(), perm()).then(() => setUser("")))}>{t("c.add")}</button>
          </div>
          <For each={userShares()} fallback={<p class="muted xs">{t("sh.noShares")}</p>}>
            {(r) => (
              <div class="sh-row">
                <Icon name="user" size={15} class="ic muted" />
                <span>{r.target}</span>
                <span class="muted xs">{r.permission === "edit" ? t("sh.canEdit") : t("sh.canView")} · {expText(r.expires_at)}</span>
                <span class="grow" />
                <button class="icon-btn sm danger" title={t("sh.revoke")} onClick={() => confirm(t("sh.revokeAsk", { name: r.target ?? "" })) && run(() => api.unshare(r.id))}><Icon name="trash" size={13} /></button>
              </div>
            )}
          </For>
        </div>
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
        <p class="muted xs">{t("sh.note")}</p>
      </div>
    </div>
  );
}

/** Trang "Được chia sẻ": duyệt + tải + (sửa nếu được phép) nội dung người khác chia sẻ. */
export function SharedPage(props: { onError: (m: string) => void }) {
  const [list, { refetch }] = createResource(() => api.shared().catch((e) => (props.onError(errText(e)), [] as SharedItem[])), { initialValue: [] });
  const [sel, setSel] = createSignal<SharedItem | null>(null);
  const [prefix, setPrefix] = createSignal("");
  const abs = () => (prefix() ? `${sel()!.path}/${prefix()}` : sel()!.path);
  const [page, { refetch: refetchFiles }] = createResource(
    () => (sel() ? { o: sel()!.owner_id, p: abs() } : null),
    (v) => api.files(v.p, { sort: "name", desc: false, offset: 0, limit: 200 }, v.o).catch((e) => (props.onError(errText(e)), { entries: [], dirs: 0, files: 0, bytes: 0 })),
  );
  const crumbs = () => prefix().split("/").filter(Boolean);
  const go = (i: number) => setPrefix(crumbs().slice(0, i).join("/"));
  const pick = (s: SharedItem) => {
    setSel(s);
    setPrefix("");
  };
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
              <button class="chip" classList={{ on: sel()?.id === s.id }} onClick={() => pick(s)} title={`${s.owner_name} · ${s.permission === "edit" ? t("sh.canEdit") : t("sh.viewOnly")}`}>
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
          <span class="muted xs">{t("sh.ownerLine", { name: sel()!.owner_name, perm: sel()!.permission === "edit" ? t("sh.canEdit") : t("sh.viewOnly") })}</span>
          <span class="grow" />
          <Show when={canEdit()}>
            <button class="btn ghost sm" onClick={newFolder}><Icon name="plus" size={15} /> {t("sh.mkdir")}</button>
            <button class="btn ghost sm" onClick={() => fileInput.click()}><Icon name="upload" size={15} /> {t("c.upload")}</button>
            <input ref={fileInput} type="file" multiple hidden onChange={(e) => { up(e.currentTarget.files); e.currentTarget.value = ""; }} />
          </Show>
          <button class="btn ghost sm" onClick={() => saveCopy(abs())} title={t("sh.saveDirTip")}><Icon name="copy" size={15} /> {t("sh.saveCopy")}</button>
        </div>
        <Show when={!page.loading} fallback={<p class="muted pad">{t("c.loading")}</p>}>
          <div class="sh-list">
            <For each={page()?.entries ?? []} fallback={<p class="muted pad">{t("sh.emptyList")}</p>}>
              {(e) => (
                <div class="tk-row">
                  <Icon name={e.is_dir ? "folder" : "file"} size={16} class="ic muted" />
                  <div class="tk-main">
                    <b>{relName(e.path)}</b>
                    <span class="muted xs">{e.is_dir ? t("f.dirLabel") : `${fmtNum(e.size)} B`}</span>
                  </div>
                  <Show when={e.is_dir}>
                    <button class="btn ghost sm" onClick={() => setPrefix(prefix() ? `${prefix()}/${e.name}` : e.name)}>{t("c.open")}</button>
                  </Show>
                  <Show when={!e.is_dir}>
                    <button class="btn ghost sm" onClick={() => dl(e)}><Icon name="download" size={14} /> {t("sh.dl")}</button>
                    <button class="btn ghost sm" onClick={() => saveCopy(e.path)} title={t("sh.saveTip")}><Icon name="copy" size={14} /></button>
                  </Show>
                  <Show when={canEdit()}>
                    <button class="icon-btn sm danger" title={t("sh.delTip")} onClick={() => confirm(t("sh.delAsk", { name: e.name })) && run(() => api.remove(e.path, sel()!.owner_id))}><Icon name="trash" size={13} /></button>
                  </Show>
                </div>
              )}
            </For>
          </div>
        </Show>
      </Show>
    </div>
  );
}

/** Trang link công khai — dùng được khi chưa đăng nhập. */
export function PublicPage() {
  const m = /^#\/s\/([A-Za-z0-9]+)/.exec(location.hash);
  const token = () => m?.[1] ?? "";
  const [pw, setPw] = createSignal("");
  const [tried, setTried] = createSignal("");
  const [gone, setGone] = createSignal("");
  // 401 = sai/thiếu mật khẩu -> hiện ô nhập; lỗi khác = link hỏng/hết hạn.
  const [meta] = createResource(
    () => ({ t: token(), p: tried() }),
    async (v): Promise<LinkMeta | null> => {
      if (!v.t) return null;
      try {
        setGone("");
        return await api.linkMeta(v.t, v.p || undefined);
      } catch (e) {
        if (String((e as Error)?.message ?? e).includes("unauthorized")) {
          return { title: "", is_dir: true, locked: true, expires_at: null };
        }
        setGone(errText(e));
        return null;
      }
    },
  );
  const [prefix, setPrefix] = createSignal("");
  const [page] = createResource(
    () => (meta() && !meta().locked && meta().is_dir ? { t: token(), p: prefix(), pw: tried() } : null),
    (v) => api.linkList(v.t, v.p, v.pw || undefined).catch(() => ({ entries: [], dirs: 0, files: 0, bytes: 0 })),
  );
  return (
    <div class="auth">
      <div class="auth-card glass pub">
        <h2>{t("sh.pubH2")}</h2>
        <Show when={!token()}><p class="err">{t("sh.badLink")}</p></Show>
        <Show when={token() && meta.loading}><p class="muted">{t("c.loading")}</p></Show>
        <Show when={token() && !meta.loading && gone()}><p class="err">{gone()}</p></Show>
        <Show when={token() && meta()}>
          {(md) => (
            <>
              <Show when={md().locked}>
                <p class="muted">{t("sh.needPw")}</p>
                <div class="sh-add">
                  <input type="password" value={pw()} onInput={(e) => setPw(e.currentTarget.value)} placeholder={t("sh.pwLinkPh")} onKeyDown={(e) => e.key === "Enter" && setTried(pw())} />
                  <button class="btn primary sm" onClick={() => setTried(pw())}>{t("c.open")}</button>
                </div>
              </Show>
              <Show when={!md().locked && md().title}>
                <h3>{md().title}</h3>
                <Show when={md().is_dir} fallback={<a class="btn primary block" href={api.linkFileUrl(token(), "", tried() || undefined)} download={md().title}>{t("c.download")}</a>}>
                  <Show when={prefix()}><button class="link" onClick={() => setPrefix(prefix().split("/").slice(0, -1).join("/"))}>← {prefix().split("/").slice(0, -1).pop() || t("sh.rootDir")}</button></Show>
                  <For each={page()?.entries ?? []}>
                    {(e) => {
                      const rel = prefix() ? `${prefix()}/${e.name}` : e.name;
                      return (
                        <div class="sh-row">
                          <Icon name={e.is_dir ? "folder" : "file"} size={15} class="ic muted" />
                          <span>{e.name}</span>
                          <span class="grow" />
                          <Show when={e.is_dir} fallback={<a class="btn ghost sm" href={api.linkFileUrl(token(), rel, tried() || undefined)} download={e.name}>{t("sh.dl")}</a>}>
                            <button class="btn ghost sm" onClick={() => setPrefix(rel)}>{t("c.open")}</button>
                          </Show>
                        </div>
                      );
                    }}
                  </For>
                </Show>
              </Show>
            </>
          )}
        </Show>
      </div>
    </div>
  );
}
