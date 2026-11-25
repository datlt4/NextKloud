import { For, Show, createResource, createSignal, onCleanup } from "solid-js";
import { api, type GitRepo } from "./api";
import { errText } from "./fileops";
import { fmtDateTime, fmtNum, t } from "./i18n";
import { Icon } from "./icons";

const repoName = (url: string) => {
  const last = url.replace(/\/+$/, "").split(/[/:]/).pop() ?? "repo";
  return last.replace(/\.git$/, "") || "repo";
};
export const ago = (s: number | null) => {
  if (!s) return t("git.never");
  const d = Math.max(0, Date.now() / 1000 - s);
  if (d < 60) return t("git.justNow");
  if (d < 3600) return t("git.minAgo", { n: Math.floor(d / 60) });
  if (d < 86400) return t("git.hourAgo", { n: Math.floor(d / 3600) });
  return t("git.dayAgo", { d: fmtDateTime(s) });
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
      <Icon name={ok() ? "check" : "text"} /> {ok() ? t("git.copied") : t("git.copy")}
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
    // refetch vẽ lại list làm .set-body rớt về top -> giữ vị trí cuộn (chỉ trả lại nếu bị kéo lên trên,
    // user chủ động cuộn xuống sâu hơn thì tôn trọng)
    const sc = document.querySelector(".set-body") as HTMLElement | null;
    const top = sc ? sc.scrollTop : 0;
    try {
      await f();
    } catch (e) {
      props.onError(errText(e));
    }
    refetch();
    for (let i = 0; i < 200 && repos.loading; i++) await new Promise((r) => setTimeout(r, 25));
    for (let i = 0; i < 3; i++) await new Promise((r) => requestAnimationFrame(r));
    if (sc && sc.scrollTop < top) sc.scrollTop = top;
  };
  return (
    <>
      <section class="set-card">
        <div class="git-top">
          <div>
            <h2>{t("git.title")}</h2>
            <p class="muted sm">
              {t("git.desc")}
            </p>
          </div>
          <button class="btn primary" onClick={() => setAdding(true)}><Icon name="plus" /> {t("git.add")}</button>
        </div>
      </section>
      <For each={repos() ?? []} fallback={
        <Show when={!repos.loading}>
          <section class="set-card"><p class="muted sm">{t("git.empty")}</p></section>
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
  const chip = (): [string, string] =>
    ({ pending: [t("git.pending"), "warn"], idle: [t("git.synced"), "good"], syncing: [t("git.syncing"), "soft"], error: [t("git.error"), "err"] })[r().state] as [string, string];
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
        <span>{t("f.filesN", { n: fmtNum(r().files) })}</span>
        <span>{ago(r().last_sync)}</span>
        <Show when={r().last_commit}><span class="mono">{r().last_commit!.slice(0, 7)}</span></Show>
      </div>
      <Show when={r().state === "error" && r().last_error}>
        <p class="err sm">{r().last_error}</p>
      </Show>
      <Show when={r().ssh_pub && (r().state === "pending" || (r().state === "error" && !r().last_commit) || showKey())}>
        <div class="keybox">
          <p class="sm" innerHTML={t("git.deployNote")} />
          <pre class="secret">{r().ssh_pub}</pre>
          <Copy text={r().ssh_pub!} />
        </div>
      </Show>
      <div class="repo-act">
        <button class="btn primary sm" disabled={r().state === "syncing"} onClick={() => props.act(() => api.gitSync(r().id))}>
          <Icon name="cloud" /> {r().state === "syncing" ? t("git.syncing") : t("git.syncNow")}
        </button>
        <label class="muted sm interval">
          {t("git.every")}
          <select value={r().interval_h} onChange={(e) => props.act(() => api.gitUpdate(r().id, { interval_h: +e.currentTarget.value }))}>
            <For each={INTERVALS.includes(r().interval_h) ? INTERVALS : [...INTERVALS, r().interval_h].sort((a, b) => a - b)}>
              {(h) => <option value={h}>{t("git.intervalOpt", { h })}</option>}
            </For>
          </select>
        </label>
        <span class="grow" />
        <Show when={r().state !== "syncing"}>
          <button class="btn ghost sm" title={t("git.editTitle")} onClick={() => setEditing(true)}>
            <Icon name="edit" /> <span class="hide-xs">{t("git.editLink")}</span>
          </button>
        </Show>
        <Show when={r().ssh_pub && r().state !== "pending"}>
          <button class="btn ghost sm" onClick={() => setShowKey(!showKey())}><Icon name="key" /> <span class="hide-xs">{showKey() ? t("git.hideKey") : t("git.showKey")}</span></button>
        </Show>
        <button class="btn danger sm" disabled={r().state === "syncing"} title={t("git.remove")} onClick={() => {
          if (!confirm(t("git.removeAsk", { name: repoName(r().url) }))) return;
          const files = confirm(t("git.trashAsk", { folder: r().folder }));
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
  // URL để trống = giữ nguyên (server chỉ che *** khi gửi về, không bao giờ gửi token thật).
  const [url, setUrl] = createSignal("");
  const [branch, setBranch] = createSignal(props.r.branch ?? "");
  const [err, setErr] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const urlChanged = () => {
    const u = url().trim();
    return u !== "" && u !== props.r.url;
  };
  const changed = () => urlChanged() || branch().trim() !== (props.r.branch ?? "");
  return (
    <div class="modal-bg" onClick={() => !busy() && props.onClose()}>
      <form
        class="modal glass form"
        onClick={(e) => e.stopPropagation()}
        onSubmit={async (e) => {
          e.preventDefault();
          const u = url().trim();
          // Chặn gửi nhầm URL đã che: token thật không bao giờ rời server.
          if (u.includes("***")) {
            setErr(t("git.urlStars"));
            return;
          }
          setBusy(true);
          setErr("");
          try {
            const body: { url?: string; branch: string } = { branch: branch().trim() };
            if (urlChanged()) body.url = u;
            await api.gitUpdate(props.r.id, body);
            props.onDone();
          } catch (e) {
            setErr(errText(e));
          } finally {
            setBusy(false);
          }
        }}
      >
        <div class="modal-head">
          <h3>{t("git.editTitle")}</h3>
          <button type="button" class="icon-btn" onClick={props.onClose}><Icon name="close" /></button>
        </div>
        <label>URL repo</label>
        <input value={url()} onInput={(e) => setUrl(e.currentTarget.value)} placeholder={props.r.url} autocomplete="off" spellcheck={false} />
        <p class="muted sm">{t("git.urlKeep")}</p>
        <label>{t("git.branch")} <span class="muted">({t("git.branchDefault")})</span></label>
        <input value={branch()} onInput={(e) => setBranch(e.currentTarget.value)} placeholder={t("git.branchPh")} spellcheck={false} />
        <p class="muted sm">{t("git.branchWarn")}</p>
        <Show when={err()}><p class="err">{err()}</p></Show>
        <button type="submit" class="btn primary block" disabled={busy() || !changed()}>
          {busy() ? t("git.saving") : t("git.save")}
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
          <h3>{t("git.addTitle")}</h3>
          <button type="button" class="icon-btn" onClick={props.onClose}><Icon name="close" /></button>
        </div>
        <label>URL repo</label>
        <input value={url()} onInput={(e) => setUrl(e.currentTarget.value)} placeholder="git@github.com:ban/vault.git" autocomplete="off" spellcheck={false} />
        <p class="muted sm">{t("git.addPrivate")}</p>
        <div class="two">
          <div>
            <label>{t("git.branch")}</label>
            <input value={branch()} onInput={(e) => setBranch(e.currentTarget.value)} placeholder={t("git.branchPh")} spellcheck={false} />
          </div>
          <div>
            <label>{t("git.dest")}</label>
            <input value={folder()} onInput={(e) => setFolder(e.currentTarget.value)} placeholder={`Obsidian Vaults/${url().trim() ? repoName(url().trim()) : `<${t("c.name").toLowerCase()}>`}`} spellcheck={false} />
          </div>
        </div>
        <label>{t("git.key")}</label>
        <div class="seg wide">
          <button type="button" classList={{ on: key() === "generate" }} onClick={() => setKey("generate")}>{t("git.keyNew")}</button>
          <button type="button" classList={{ on: key() === "paste" }} onClick={() => setKey("paste")}>{t("git.keyPaste")}</button>
          <button type="button" classList={{ on: key() === "none" }} onClick={() => setKey("none")}>{t("git.keyNone")}</button>
        </div>
        <Show when={key() === "generate"}>
          <p class="muted sm">{t("git.keyNewNote")}</p>
        </Show>
        <Show when={key() === "paste"}>
          <textarea class="keyarea" value={priv()} onInput={(e) => setPriv(e.currentTarget.value)} placeholder="-----BEGIN OPENSSH PRIVATE KEY-----" spellcheck={false} />
          <p class="muted sm">{t("git.keyNote")}</p>
        </Show>
        <Show when={key() === "none" && !isHttps() && url().trim()}>
          <p class="err sm">{t("git.keyNeed")}</p>
        </Show>
        <label>{t("git.interval")}</label>
        <select value={interval()} onChange={(e) => setInterval_(+e.currentTarget.value)}>
          <For each={INTERVALS}>{(h) => <option value={h}>{t("git.hours", { h })}</option>}</For>
        </select>
        <Show when={err()}><p class="err">{err()}</p></Show>
        <button type="submit" class="btn primary block" disabled={busy() || !url().trim() || (key() === "paste" && !priv().trim())}>
          {busy() ? t("git.adding") : key() === "generate" ? t("git.addGoKey") : t("git.addGo")}
        </button>
      </form>
    </div>
  );
}
