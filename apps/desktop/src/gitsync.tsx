import { For, Show, createResource, createSignal, onCleanup } from "solid-js";
import { api, type GitRepo } from "./api";
import { errText } from "./fileops";
import { Icon } from "./icons";

const repoName = (url: string) => {
  const last = url.replace(/\/+$/, "").split(/[/:]/).pop() ?? "repo";
  return last.replace(/\.git$/, "") || "repo";
};
const ago = (s: number | null) => {
  if (!s) return "chưa đồng bộ";
  const d = Math.max(0, Date.now() / 1000 - s);
  if (d < 60) return "vừa xong";
  if (d < 3600) return `${Math.floor(d / 60)} phút trước`;
  if (d < 86400) return `${Math.floor(d / 3600)} giờ trước`;
  return new Date(s * 1000).toLocaleString("vi-VN", { dateStyle: "short", timeStyle: "short" });
};
const INTERVALS = [1, 4, 8, 12, 24];

function Copy(props: { text: string }) {
  const [ok, setOk] = createSignal(false);
  return (
    <button type="button" class="btn ghost sm" onClick={async () => {
      await navigator.clipboard?.writeText(props.text).catch(() => {});
      setOk(true);
      setTimeout(() => setOk(false), 1500);
    }}>
      <Icon name={ok() ? "check" : "text"} /> {ok() ? "Đã chép" : "Chép"}
    </button>
  );
}

export function GitPanel(props: { onOpen: (folder: string) => void; onError: (m: string) => void }) {
  const [repos, { refetch }] = createResource(() => api.gitRepos());
  const [adding, setAdding] = createSignal(false);
  // Đang có repo đồng bộ -> hỏi lại trạng thái mỗi 2 giây.
  const timer = setInterval(() => {
    if (repos()?.some((r) => r.state === "syncing")) refetch();
  }, 2000);
  onCleanup(() => clearInterval(timer));
  const act = async (f: () => Promise<unknown>) => {
    try {
      await f();
    } catch (e) {
      props.onError(errText(e));
    }
    refetch();
  };
  return (
    <>
      <section class="set-card">
        <div class="git-top">
          <div>
            <h2>Đồng bộ Git</h2>
            <p class="muted sm">
              Mirror 1 chiều từ git repo (vd vault Obsidian) vào thư mục của bạn, tự cập nhật theo chu kỳ. Sửa file trong NextKloud sẽ bị ghi đè khi repo thay đổi.
            </p>
          </div>
          <button class="btn primary" onClick={() => setAdding(true)}><Icon name="plus" /> Thêm repo</button>
        </div>
      </section>
      <For each={repos() ?? []} fallback={
        <Show when={!repos.loading}>
          <section class="set-card"><p class="muted sm">Chưa có repo nào.</p></section>
        </Show>
      }>
        {(r) => <RepoCard r={r} onOpen={props.onOpen} act={act} />}
      </For>
      <Show when={adding()}>
        <AddDialog onClose={() => setAdding(false)} onDone={() => { setAdding(false); refetch(); }} />
      </Show>
    </>
  );
}

function RepoCard(props: { r: GitRepo; onOpen: (f: string) => void; act: (f: () => Promise<unknown>) => void }) {
  const r = () => props.r;
  const [showKey, setShowKey] = createSignal(false);
  const [editing, setEditing] = createSignal(false);
  const chip = () =>
    ({ pending: ["Chờ thêm khoá", "warn"], idle: ["Đã đồng bộ", "good"], syncing: ["Đang đồng bộ…", "soft"], error: ["Lỗi", "err"] })[r().state];
  return (
    <section class="set-card repo">
      <div class="repo-head">
        <span class="fi repo-ic"><Icon name="code" size={20} /></span>
        <div class="repo-title">
          <b>{repoName(r().url)}</b>
          <span class="muted sm repo-url" title={r().url}>{r().url}{r().branch ? ` · ${r().branch}` : ""}</span>
        </div>
        <span class={`tag ${chip()[1]}`}>{chip()[0]}</span>
      </div>
      <div class="repo-meta muted sm">
        <button class="link" onClick={() => props.onOpen(r().folder)}><Icon name="folder" size={14} /> {r().folder}</button>
        <span>{r().files.toLocaleString("vi-VN")} file</span>
        <span>{ago(r().last_sync)}</span>
        <Show when={r().last_commit}><span class="mono">{r().last_commit!.slice(0, 7)}</span></Show>
      </div>
      <Show when={r().state === "error" && r().last_error}>
        <p class="err sm">{r().last_error}</p>
      </Show>
      <Show when={r().ssh_pub && (r().state === "pending" || (r().state === "error" && !r().last_commit) || showKey())}>
        <div class="keybox">
          <p class="sm">
            Thêm khoá này vào <b>Deploy keys</b> của repo (GitHub/Gitea: Settings → Deploy keys, chỉ cần quyền đọc), rồi bấm <b>Đồng bộ ngay</b>.
          </p>
          <pre class="secret">{r().ssh_pub}</pre>
          <Copy text={r().ssh_pub!} />
        </div>
      </Show>
      <div class="repo-act">
        <button class="btn primary sm" disabled={r().state === "syncing"} onClick={() => props.act(() => api.gitSync(r().id))}>
          <Icon name="cloud" /> {r().state === "syncing" ? "Đang đồng bộ…" : "Đồng bộ ngay"}
        </button>
        <label class="muted sm interval">
          Mỗi
          <select value={r().interval_h} onChange={(e) => props.act(() => api.gitUpdate(r().id, { interval_h: +e.currentTarget.value }))}>
            <For each={INTERVALS.includes(r().interval_h) ? INTERVALS : [...INTERVALS, r().interval_h].sort((a, b) => a - b)}>
              {(h) => <option value={h}>{h} giờ</option>}
            </For>
          </select>
        </label>
        <span class="grow" />
        <Show when={r().state !== "syncing"}>
          <button class="btn ghost sm" title="Sửa link remote / nhánh" onClick={() => setEditing(true)}>
            <Icon name="edit" /> <span class="hide-xs">Sửa link</span>
          </button>
        </Show>
        <Show when={r().ssh_pub && r().state !== "pending"}>
          <button class="btn ghost sm" onClick={() => setShowKey(!showKey())}><Icon name="key" /> <span class="hide-xs">Khoá</span></button>
        </Show>
        <button class="btn danger sm" disabled={r().state === "syncing"} title="Gỡ repo" onClick={() => {
          if (!confirm(`Gỡ mirror "${repoName(r().url)}"? Sẽ ngừng đồng bộ.`)) return;
          const files = confirm(`Chuyển luôn thư mục "${r().folder}" vào thùng rác?\nOK = chuyển vào thùng rác · Huỷ = giữ lại file`);
          props.act(() => api.gitRemove(r().id, files));
        }}>
          <Icon name="trash" />
        </button>
      </div>
      <Show when={editing()}>
        <EditDialog r={r()} onClose={() => setEditing(false)} onDone={() => { setEditing(false); props.act(async () => {}); }} />
      </Show>
    </section>
  );
}

function EditDialog(props: { r: GitRepo; onClose: () => void; onDone: () => void }) {
  const [url, setUrl] = createSignal(props.r.url);
  const [branch, setBranch] = createSignal(props.r.branch ?? "");
  const [err, setErr] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const changed = () => url().trim() !== props.r.url || branch().trim() !== (props.r.branch ?? "");
  return (
    <div class="modal-bg" onClick={() => !busy() && props.onClose()}>
      <form
        class="modal glass form"
        onClick={(e) => e.stopPropagation()}
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setErr("");
          try {
            await api.gitUpdate(props.r.id, { url: url().trim(), branch: branch().trim() });
            props.onDone();
          } catch (e) {
            setErr(errText(e));
          } finally {
            setBusy(false);
          }
        }}
      >
        <div class="modal-head">
          <h3>Sửa link remote</h3>
          <button type="button" class="icon-btn" onClick={props.onClose}><Icon name="close" /></button>
        </div>
        <label>URL repo</label>
        <input value={url()} onInput={(e) => setUrl(e.currentTarget.value)} placeholder="git@github.com:ban/vault.git" autocomplete="off" spellcheck={false} />
        <p class="muted sm">Đổi URL áp dụng ngay lần đồng bộ kế tiếp, giữ nguyên deploy key. Repo riêng tư qua https: dùng dạng https://user:token@host/ban/vault.git.</p>
        <label>Nhánh <span class="muted">(trống = mặc định của repo)</span></label>
        <input value={branch()} onInput={(e) => setBranch(e.currentTarget.value)} placeholder="mặc định của repo" spellcheck={false} />
        <p class="muted sm">Đổi nhánh sẽ clone lại từ đầu.</p>
        <Show when={err()}><p class="err">{err()}</p></Show>
        <button type="submit" class="btn primary block" disabled={busy() || !url().trim() || !changed()}>
          {busy() ? "Đang lưu…" : "Lưu"}
        </button>
      </form>
    </div>
  );
}

function AddDialog(props: { onClose: () => void; onDone: () => void }) {
  const [url, setUrl] = createSignal("");
  const [branch, setBranch] = createSignal("");
  const [folder, setFolder] = createSignal("");
  const [key, setKey] = createSignal<"generate" | "paste" | "none">("generate");
  const [priv, setPriv] = createSignal("");
  const [interval, setInterval_] = createSignal(8);
  const [err, setErr] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const isHttps = () => /^https?:\/\//.test(url().trim());
  return (
    <div class="modal-bg" onClick={() => !busy() && props.onClose()}>
      <form
        class="modal glass form"
        onClick={(e) => e.stopPropagation()}
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setErr("");
          try {
            await api.gitAdd({ url: url().trim(), branch: branch().trim(), folder: folder().trim(), key: key(), private_key: key() === "paste" ? priv() : undefined, interval_h: interval() });
            props.onDone();
          } catch (e) {
            setErr(errText(e));
          } finally {
            setBusy(false);
          }
        }}
      >
        <div class="modal-head">
          <h3>Thêm git repo</h3>
          <button type="button" class="icon-btn" onClick={props.onClose}><Icon name="close" /></button>
        </div>
        <label>URL repo</label>
        <input value={url()} onInput={(e) => setUrl(e.currentTarget.value)} placeholder="git@github.com:ban/vault.git" autocomplete="off" spellcheck={false} />
        <p class="muted sm">Repo riêng tư qua https: dùng dạng https://user:token@host/ban/vault.git và chọn “Không cần khoá”.</p>
        <div class="two">
          <div>
            <label>Nhánh</label>
            <input value={branch()} onInput={(e) => setBranch(e.currentTarget.value)} placeholder="mặc định của repo" spellcheck={false} />
          </div>
          <div>
            <label>Thư mục đích</label>
            <input value={folder()} onInput={(e) => setFolder(e.currentTarget.value)} placeholder={`Vaults/${url().trim() ? repoName(url().trim()) : "<tên repo>"}`} spellcheck={false} />
          </div>
        </div>
        <label>Khoá truy cập</label>
        <div class="seg wide">
          <button type="button" classList={{ on: key() === "generate" }} onClick={() => setKey("generate")}>Tạo khoá mới</button>
          <button type="button" classList={{ on: key() === "paste" }} onClick={() => setKey("paste")}>Dán private key</button>
          <button type="button" classList={{ on: key() === "none" }} onClick={() => setKey("none")}>Không cần khoá</button>
        </div>
        <Show when={key() === "generate"}>
          <p class="muted sm">NextKloud tạo khoá ed25519 riêng cho repo này. Sau khi thêm, chép khoá công khai vào Deploy keys của repo.</p>
        </Show>
        <Show when={key() === "paste"}>
          <textarea class="keyarea" value={priv()} onInput={(e) => setPriv(e.currentTarget.value)} placeholder="-----BEGIN OPENSSH PRIVATE KEY-----" spellcheck={false} />
          <p class="muted sm">Khoá không được có passphrase. Lưu trên server, không bao giờ gửi lại về trình duyệt.</p>
        </Show>
        <Show when={key() === "none" && !isHttps() && url().trim()}>
          <p class="err sm">URL ssh cần khoá.</p>
        </Show>
        <label>Tự đồng bộ</label>
        <select value={interval()} onChange={(e) => setInterval_(+e.currentTarget.value)}>
          <For each={INTERVALS}>{(h) => <option value={h}>Mỗi {h} giờ</option>}</For>
        </select>
        <Show when={err()}><p class="err">{err()}</p></Show>
        <button type="submit" class="btn primary block" disabled={busy() || !url().trim() || (key() === "paste" && !priv().trim())}>
          {busy() ? "Đang thêm…" : key() === "generate" ? "Thêm & tạo khoá" : "Thêm & đồng bộ"}
        </button>
      </form>
    </div>
  );
}
