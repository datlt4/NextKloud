import { For, Show, createSignal, onCleanup } from "solid-js";
import { isTauri, serverOrigin, token } from "./api";
import { ago } from "./gitsync";
import { fmtNum, t } from "./i18n";
import { Icon } from "./icons";

/** 1 cặp thư mục máy <-> server + trạng thái (engine Rust trong app desktop). */
export interface SyncPair {
  id: string;
  local: string;
  remote: string;
  paused: boolean;
  last_sync: number | null;
  up: number;
  down: number;
  conflicts: number;
  errors: number;
  last_error: string | null;
  done: number;
  total: number;
  state: "idle" | "syncing" | "error" | "paused" | "auth" | "other";
}

const invoke = async <T,>(cmd: string, args?: Record<string, unknown>) => (await import("@tauri-apps/api/core")).invoke<T>(cmd, args);
const msg = (m: string) => (m.startsWith("sync.e.") ? t(m) : m);

/** Báo phiên đăng nhập cho engine đồng bộ (null = đăng xuất -> dừng). */
export function syncAuth(user: string | null) {
  if (!isTauri()) return;
  const tok = user ? token() : null;
  invoke("sync_auth", { server: serverOrigin(), token: tok, account: user, tz: -new Date().getTimezoneOffset() }).catch(() => {});
}

export function FolderSyncPanel(props: { onOpen: (folder: string) => void }) {
  const [pairs, setPairs] = createSignal<SyncPair[]>([]);
  const [local, setLocal] = createSignal("");
  const [remote, setRemote] = createSignal("");
  const [err, setErr] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [, setTick] = createSignal(0);
  let unlisten: (() => void) | undefined;
  let gone = false;
  invoke<SyncPair[]>("sync_get").then(setPairs).catch((e) => setErr(String(e)));
  import("@tauri-apps/api/event")
    .then((ev) => ev.listen<SyncPair[]>("nk-sync", (e) => setPairs(e.payload)))
    .then((u) => (gone ? u() : (unlisten = u)));
  // "x phút trước" tự cập nhật
  const timer = setInterval(() => setTick((x) => x + 1), 30_000);
  onCleanup(() => {
    gone = true;
    unlisten?.();
    clearInterval(timer);
  });
  const act = async (cmd: string, args?: Record<string, unknown>) => {
    setErr("");
    try {
      const r = await invoke<SyncPair[] | null>(cmd, args);
      if (r) setPairs(r);
      return true;
    } catch (e) {
      setErr(msg(String(e)));
      return false;
    }
  };
  const chip = (p: SyncPair): string => ({ idle: "good", syncing: "soft", error: "err", paused: "soft", auth: "warn", other: "warn" })[p.state];
  return (
    <>
      <section class="set-card">
        <h2>{t("set.sync")}</h2>
        <p class="muted sm">{t("sync.desc")}</p>
        <form
          class="set-form"
          onSubmit={async (e) => {
            e.preventDefault();
            setBusy(true);
            if (await act("sync_add", { local: local(), remote: remote() })) {
              setLocal("");
              setRemote("");
            }
            setBusy(false);
          }}
        >
          <label>{t("sync.local")}</label>
          <input value={local()} onInput={(e) => setLocal(e.currentTarget.value)} placeholder={t("sync.localPh")} spellcheck={false} autocomplete="off" />
          <label>{t("sync.remote")}</label>
          <div class="row-input">
            <input value={remote()} onInput={(e) => setRemote(e.currentTarget.value)} placeholder={t("sync.remotePh")} spellcheck={false} autocomplete="off" />
            <button type="submit" class="btn primary" disabled={busy() || !local().trim()}>
              <Icon name="plus" /> {t("sync.add")}
            </button>
          </div>
        </form>
        <Show when={err()}><p class="err">{err()}</p></Show>
      </section>
      <For each={pairs()} fallback={<section class="set-card"><p class="muted sm">{t("sync.empty")}</p></section>}>
        {(p) => (
          <section class="set-card repo">
            <div class="repo-head">
              <span class="fi repo-ic"><Icon name="folder" size={20} /></span>
              <div class="repo-title">
                <b class="repo-url" title={p.local}>{p.local}</b>
                <button class="link muted sm" onClick={() => props.onOpen(p.remote)}>
                  <Icon name="cloud" size={14} /> /{p.remote || t("sync.root")}
                </button>
              </div>
              <span class={`tag ${chip(p)}`}>{t(`sync.st.${p.state}`)}</span>
            </div>
            <div class="repo-meta muted sm">
              <span>{ago(p.last_sync)}</span>
              <span>{t("sync.stats", { up: fmtNum(p.up), down: fmtNum(p.down), c: fmtNum(p.conflicts) })}</span>
              <Show when={p.state === "syncing" && p.total > 0}>
                <span>{t("sync.progress", { done: fmtNum(p.done), total: fmtNum(p.total) })}</span>
              </Show>
            </div>
            <Show when={p.state === "other"}><p class="muted sm">{t("sync.other")}</p></Show>
            <Show when={p.last_error}><p class="err sm">{msg(p.last_error!)}</p></Show>
            <div class="repo-act">
              <button class="btn primary sm" disabled={p.paused || p.state === "syncing"} onClick={() => act("sync_now")}>
                <Icon name="cloud" /> {p.state === "syncing" ? t("sync.st.syncing") : t("sync.now")}
              </button>
              <button class="btn ghost sm" onClick={() => act("sync_pause", { id: p.id, paused: !p.paused })}>
                <Icon name={p.paused ? "play" : "pause"} /> {p.paused ? t("sync.resume") : t("sync.pause")}
              </button>
              <span class="grow" />
              <button class="btn danger sm" title={t("sync.remove")} onClick={() => confirm(t("sync.removeAsk", { local: p.local })) && act("sync_remove", { id: p.id })}>
                <Icon name="trash" />
              </button>
            </div>
          </section>
        )}
      </For>
    </>
  );
}
