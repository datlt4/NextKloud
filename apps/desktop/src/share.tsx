import { For, Show, createResource, createSignal } from "solid-js";
import { api, serverOrigin, type Entry, type LinkMeta, type ShareRow, type SharedItem } from "./api";
import { errText } from "./fileops";
import { Icon } from "./icons";

const fmtDay = (s: number) => new Date(s * 1000).toLocaleDateString("vi-VN", { day: "2-digit", month: "2-digit", year: "2-digit" });
const expText = (e: number | null) => (!e ? "vĩnh viễn" : e * 1000 < Date.now() ? `hết hạn ${fmtDay(e)}` : `tới ${fmtDay(e)}`);

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
          <h3>Chia sẻ “{name()}”</h3>
          <button class="icon-btn" onClick={props.onClose}><Icon name="close" /></button>
        </div>
        <Show when={rows.loading}><p class="muted">Đang tải…</p></Show>
        <div class="sh-sec">
          <b>Chia sẻ cho người dùng</b>
          <div class="sh-add">
            <input value={user()} onInput={(e) => setUser(e.currentTarget.value)} placeholder="Tên đăng nhập" />
            <select value={perm()} onChange={(e) => setPerm(e.currentTarget.value)} title="Quyền">
              <option value="view">Được xem</option>
              <option value="edit">Được sửa</option>
            </select>
            <button class="btn primary sm" onClick={() => user().trim() && run(() => api.shareWith(props.path, user().trim(), perm()).then(() => setUser("")))}>Thêm</button>
          </div>
          <For each={userShares()} fallback={<p class="muted xs">Chưa chia sẻ cho ai.</p>}>
            {(r) => (
              <div class="sh-row">
                <Icon name="user" size={15} class="ic muted" />
                <span>{r.target}</span>
                <span class="muted xs">{r.permission === "edit" ? "được sửa" : "được xem"} · {expText(r.expires_at)}</span>
                <span class="grow" />
                <button class="icon-btn sm danger" title="Thu hồi" onClick={() => confirm(`Thu hồi quyền của ${r.target}?`) && run(() => api.unshare(r.id))}><Icon name="trash" size={13} /></button>
              </div>
            )}
          </For>
        </div>
        <div class="sh-sec">
          <b>Link công khai (chỉ xem + tải)</b>
          <div class="sh-add">
            <input type="password" value={pw()} onInput={(e) => setPw(e.currentTarget.value)} placeholder="Mật khẩu (tối thiểu 8 ký tự, trống = không)" autocomplete="new-password" />
            <select value={days()} onChange={(e) => setDays(e.currentTarget.value)} title="Hạn dùng">
              <option value="">Vĩnh viễn</option>
              <option value="1">1 ngày</option>
              <option value="7">7 ngày</option>
              <option value="30">30 ngày</option>
            </select>
            <button class="btn primary sm" onClick={() => run(() => api.createLink(props.path, pw(), days() ? +days() : null))}>Tạo link</button>
          </div>
          <For each={links()} fallback={<p class="muted xs">Chưa có link nào.</p>}>
            {(r) => (
              <div class="sh-row">
                <Icon name="copy" size={15} class="ic muted" />
                <input class="sh-link" readonly value={linkUrl(r.token!)} onFocus={(e) => e.currentTarget.select()} title="Bôi đen để chép tay nếu nút Chép không chạy" />
                <span class="muted xs">{r.has_password ? "có MK" : "không MK"} · {expText(r.expires_at)}</span>
                <span class="grow" />
                <button class="btn ghost sm" onClick={() => copyLink(r.token!)}>Chép</button>
                <button class="icon-btn sm danger" title="Xoá link" onClick={() => confirm("Xoá link này?") && run(() => api.unshare(r.id))}><Icon name="trash" size={13} /></button>
              </div>
            )}
          </For>
        </div>
        <p class="muted xs">Thu hồi/xoá link có hiệu lực ngay. File trong Bí mật không chia sẻ được.</p>
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
    const to = prompt("Lưu bản sao về đường dẫn (trong file của bạn)", path.split("/").pop() ?? "bản sao");
    if (to?.trim()) run(() => api.saveCopy(sel()!.owner_id, path, to.trim()));
  };
  const newFolder = () => {
    const n = prompt("Tên thư mục mới")?.trim();
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
        <h2 class="tb-title"><Icon name="users" /> Được chia sẻ</h2>
        <div class="kb-boards">
          <For each={list()} fallback={!list.loading && <span class="muted sm">Chưa ai chia sẻ gì với bạn.</span>}>
            {(s) => (
              <button class="chip" classList={{ on: sel()?.id === s.id }} onClick={() => pick(s)} title={`${s.owner_name} · ${s.permission === "edit" ? "được sửa" : "được xem"}`}>
                {s.owner_name}/{s.path.split("/").pop()}
              </button>
            )}
          </For>
        </div>
      </div>
      <Show when={sel()} fallback={<div class="empty"><Icon name="users" size={48} /><p>Chọn một mục bên trên để duyệt.</p></div>}>
        <div class="kb-sub">
          <Show when={crumbs().length} fallback={<b>{sel()!.path}</b>}>
            <button class="link" onClick={() => go(0)}>{sel()!.path.split("/").pop()}</button>
            <For each={crumbs()}>{(c, i) => <><span class="muted">/</span><button class="link" onClick={() => go(i() + 1)}>{c}</button></>}</For>
          </Show>
          <span class="muted xs">chủ: {sel()!.owner_name} · {sel()!.permission === "edit" ? "được sửa" : "chỉ xem"}</span>
          <span class="grow" />
          <Show when={canEdit()}>
            <button class="btn ghost sm" onClick={newFolder}><Icon name="plus" size={15} /> Thư mục</button>
            <button class="btn ghost sm" onClick={() => fileInput.click()}><Icon name="upload" size={15} /> Tải lên</button>
            <input ref={fileInput} type="file" multiple hidden onChange={(e) => { up(e.currentTarget.files); e.currentTarget.value = ""; }} />
          </Show>
          <button class="btn ghost sm" onClick={() => saveCopy(abs())} title="Chép cả thư mục này về file của bạn"><Icon name="copy" size={15} /> Lưu bản sao</button>
        </div>
        <Show when={!page.loading} fallback={<p class="muted pad">Đang tải…</p>}>
          <div class="sh-list">
            <For each={page()?.entries ?? []} fallback={<p class="muted pad">Trống.</p>}>
              {(e) => (
                <div class="tk-row">
                  <Icon name={e.is_dir ? "folder" : "file"} size={16} class="ic muted" />
                  <div class="tk-main">
                    <b>{relName(e.path)}</b>
                    <span class="muted xs">{e.is_dir ? "thư mục" : `${e.size.toLocaleString("vi-VN")} B`}</span>
                  </div>
                  <Show when={e.is_dir}>
                    <button class="btn ghost sm" onClick={() => setPrefix(prefix() ? `${prefix()}/${e.name}` : e.name)}>Mở</button>
                  </Show>
                  <Show when={!e.is_dir}>
                    <button class="btn ghost sm" onClick={() => dl(e)}><Icon name="download" size={14} /> Tải</button>
                    <button class="btn ghost sm" onClick={() => saveCopy(e.path)} title="Lưu về file của bạn"><Icon name="copy" size={14} /></button>
                  </Show>
                  <Show when={canEdit()}>
                    <button class="icon-btn sm danger" title="Xoá (vào thùng rác của chủ file)" onClick={() => confirm(`Xoá "${e.name}"?`) && run(() => api.remove(e.path, sel()!.owner_id))}><Icon name="trash" size={13} /></button>
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
        <h2>NextKloud — file được chia sẻ</h2>
        <Show when={!token()}><p class="err">Link không hợp lệ.</p></Show>
        <Show when={token() && meta.loading}><p class="muted">Đang tải…</p></Show>
        <Show when={token() && !meta.loading && gone()}><p class="err">{gone()}</p></Show>
        <Show when={token() && meta()}>
          {(md) => (
            <>
              <Show when={md().locked}>
                <p class="muted">Link này cần mật khẩu.</p>
                <div class="sh-add">
                  <input type="password" value={pw()} onInput={(e) => setPw(e.currentTarget.value)} placeholder="Mật khẩu link" onKeyDown={(e) => e.key === "Enter" && setTried(pw())} />
                  <button class="btn primary sm" onClick={() => setTried(pw())}>Mở</button>
                </div>
              </Show>
              <Show when={!md().locked && md().title}>
                <h3>{md().title}</h3>
                <Show when={md().is_dir} fallback={<a class="btn primary block" href={api.linkFileUrl(token(), "", tried() || undefined)} download={md().title}>Tải xuống</a>}>
                  <Show when={prefix()}><button class="link" onClick={() => setPrefix(prefix().split("/").slice(0, -1).join("/"))}>← {prefix().split("/").slice(0, -1).pop() || "Gốc"}</button></Show>
                  <For each={page()?.entries ?? []}>
                    {(e) => {
                      const rel = prefix() ? `${prefix()}/${e.name}` : e.name;
                      return (
                        <div class="sh-row">
                          <Icon name={e.is_dir ? "folder" : "file"} size={15} class="ic muted" />
                          <span>{e.name}</span>
                          <span class="grow" />
                          <Show when={e.is_dir} fallback={<a class="btn ghost sm" href={api.linkFileUrl(token(), rel, tried() || undefined)} download={e.name}>Tải</a>}>
                            <button class="btn ghost sm" onClick={() => setPrefix(rel)}>Mở</button>
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
