import { For, Show, createResource, createSignal } from "solid-js";
import { api, isTauri, type Me } from "./api";
import { Avatar, squareAvatar } from "./avatar";
import { fmtSize } from "./format";
import { fmtNum, t } from "./i18n";
import { ResetForm } from "./forms";
import { Icon } from "./icons";
import { LookPanel } from "./look";
import UsersAdmin from "./UsersAdmin";
import { installUpdate, restartApp, checkUpdate, updateState } from "./updater";
import { GitPanel } from "./gitsync";

export type SettingsTab = "profile" | "security" | "look" | "git" | "users" | "system";

const TABS: { id: SettingsTab; icon: string; admin?: boolean }[] = [
  { id: "profile", icon: "user" },
  { id: "security", icon: "key" },
  { id: "look", icon: "palette" },
  { id: "git", icon: "code" },
  { id: "users", icon: "users", admin: true },
  { id: "system", icon: "server", admin: true },
];
const tabLabel = (id: SettingsTab) => t(`set.${id}`);

export default function Settings(props: {
  me: Me;
  tab: SettingsTab;
  onTab: (t: SettingsTab) => void;
  onMe: (m: Me) => void;
  onError: (m: string) => void;
  onOpen: (folder: string) => void;
}) {
  const tabs = () => TABS.filter((t) => !t.admin || props.me.is_admin);
  return (
    <div class="settings">
      <nav class="set-nav">
        <div class="set-group">{t("set.personal")}</div>
        <For each={tabs().filter((t) => !t.admin)}>
          {(t) => (
            <button class="nav" classList={{ active: props.tab === t.id }} onClick={() => props.onTab(t.id)}>
              <Icon name={t.icon} /> {tabLabel(t.id)}
            </button>
          )}
        </For>
        <Show when={props.me.is_admin}>
          <div class="set-group">{t("set.admin")}</div>
          <For each={tabs().filter((t) => t.admin)}>
            {(t) => (
              <button class="nav" classList={{ active: props.tab === t.id }} onClick={() => props.onTab(t.id)}>
                <Icon name={t.icon} /> {tabLabel(t.id)}
              </button>
            )}
          </For>
        </Show>
      </nav>
      <div class="set-body">
        <Show when={props.tab === "profile"}>
          <Profile me={props.me} onMe={props.onMe} onError={props.onError} />
        </Show>
        <Show when={props.tab === "security"}>
          <Security />
        </Show>
        <Show when={props.tab === "look"}>
          <section class="set-card">
            <h2>{t("set.look")}</h2>
            <LookPanel onError={props.onError} />
          </section>
        </Show>
        <Show when={props.tab === "git"}>
          <GitPanel onOpen={props.onOpen} onError={props.onError} />
        </Show>
        <Show when={props.tab === "users" && props.me.is_admin}>
          <UsersAdmin meId={props.me.id} onError={props.onError} onChanged={() => api.me().then(props.onMe).catch(() => {})} />
        </Show>
        <Show when={props.tab === "system" && props.me.is_admin}>
          <System onError={props.onError} />
        </Show>
      </div>
    </div>
  );
}

function Profile(props: { me: Me; onMe: (m: Me) => void; onError: (m: string) => void }) {
  const [name, setName] = createSignal(props.me.display_name ?? "");
  const [busy, setBusy] = createSignal(false);
  const [saved, setSaved] = createSignal(false);
  let pick!: HTMLInputElement;
  const run = async (f: () => Promise<Me>) => {
    setBusy(true);
    try {
      props.onMe(await f());
      return true;
    } catch (e) {
      props.onError(t("set.saveProfileErr") + String((e as Error).message));
      return false;
    } finally {
      setBusy(false);
    }
  };
  const pct = () => (props.me.quota_limit > 0 ? Math.min(100, (props.me.quota_used / props.me.quota_limit) * 100) : 0);
  return (
    <>
      <section class="set-card">
        <h2>{t("set.profile")}</h2>
        <div class="profile-head">
          <Avatar id={props.me.id} v={props.me.avatar_v} name={props.me.display_name || props.me.username} size={88} class="big" />
          <div class="profile-act">
            <button class="btn primary" disabled={busy()} onClick={() => pick.click()}>
              <Icon name="image" /> {props.me.avatar_v ? t("set.avatarChange") : t("set.avatarUpload")}
            </button>
            <Show when={props.me.avatar_v > 0}>
              <button class="btn ghost" disabled={busy()} onClick={() => run(() => api.deleteAvatar())}>
                <Icon name="trash" /> {t("set.avatarDel")}
              </button>
            </Show>
            <p class="muted sm">{t("set.avatarNote")}</p>
          </div>
          <input
            ref={pick}
            type="file"
            accept="image/*"
            hidden
            onChange={async (e) => {
              const f = e.currentTarget.files?.[0];
              e.currentTarget.value = "";
              if (f) await run(async () => api.putAvatar(await squareAvatar(f)));
            }}
          />
        </div>
        <form
          class="set-form"
          onSubmit={async (e) => {
            e.preventDefault();
            setSaved(false);
            if (await run(() => api.updateMe(name()))) setSaved(true);
          }}
        >
          <label>{t("set.username")}</label>
          <input value={props.me.username} disabled />
          <label>{t("set.displayName")}</label>
          <div class="row-input">
            <input value={name()} maxLength={64} placeholder={props.me.username} onInput={(e) => { setName(e.currentTarget.value); setSaved(false); }} />
            <button type="submit" class="btn primary" disabled={busy() || name().trim() === (props.me.display_name ?? "")}>
              {saved() ? t("set.saved") : t("set.save")}
            </button>
          </div>
        </form>
      </section>
      <section class="set-card">
        <h2>{t("set.quota")}</h2>
        <p class="big-num">
          {fmtSize(props.me.quota_used)}
          <span class="muted"> / {props.me.quota_limit > 0 ? fmtSize(props.me.quota_limit) : t("set.unlimited")}</span>
        </p>
        <Show when={props.me.quota_limit > 0}>
          <div class="bar thick"><i style={{ width: `${pct()}%` }} classList={{ hot: pct() > 90 }} /></div>
          <p class="muted sm">{t("set.free", { size: fmtSize(Math.max(0, props.me.quota_limit - props.me.quota_used)) })}</p>
        </Show>
        <Show when={props.me.is_admin}><p class="muted sm"><span class="tag">admin</span> {t("set.isAdmin")}</p></Show>
      </section>
    </>
  );
}

function Security() {
  const [err, setErr] = createSignal("");
  const [ok, setOk] = createSignal(false);
  const [k, setK] = createSignal(0); // đổi key để xoá trắng form sau khi đổi xong
  return (
    <section class="set-card">
      <h2>{t("set.changePass")}</h2>
      <p class="muted sm">{t("set.passHint")}</p>
      <Show when={err()}><p class="err">{err()}</p></Show>
      <Show when={ok()}><p class="ok">{t("set.passDone")}</p></Show>
      <div class="set-form">
        <Show when={k() + 1} keyed>
          <ResetForm
            onError={(m) => { setOk(false); setErr(m); }}
            onDone={async (o, n) => {
              setErr("");
              setOk(false);
              try {
                await api.changePassword(o, n);
                setOk(true);
                setK((x) => x + 1);
              } catch (e) {
                setErr((e as Error).message);
              }
            }}
          />
        </Show>
      </div>
    </section>
  );
}

function System(props: { onError: (m: string) => void }) {
  const [stats, { refetch }] = createResource(() => api.adminStats());
  const [users] = createResource(() => api.users());
  const [target, setTarget] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [result, setResult] = createSignal("");
  const reindex = async () => {
    const list = users() ?? [];
    const todo = target() ? list.filter((u) => u.id === target()) : list;
    setBusy(true);
    setResult("");
    try {
      const lines = [];
      for (const u of todo) {
        const r = await api.adminReindex(u.id);
        lines.push(`${u.username}: ${fmtNum(r.files)} file · ${fmtSize(r.bytes)}`);
      }
      setResult(lines.join("\n"));
      refetch();
    } catch (e) {
      props.onError(t("set.reindexErr") + String((e as Error).message));
    } finally {
      setBusy(false);
    }
  };
  const cards = () => {
    const s = stats();
    if (!s) return [];
    return [
      [t("set.version"), s.version],
      [t("set.storage"), s.storage === "s3" ? t("set.storageS3") : s.storage],
      [t("set.usersN"), fmtNum(s.users)],
      [t("set.filesN"), fmtNum(s.files)],
      [t("set.dirsN"), fmtNum(s.dirs)],
      [t("set.bytesN"), fmtSize(s.bytes)],
    ];
  };
  return (
    <>
      <section class="set-card">
        <h2>{t("set.system")}</h2>
        <div class="stats">
          <For each={cards()}>{([k, v]) => <div class="stat"><span class="muted sm">{k}</span><b>{v}</b></div>}</For>
        </div>
      </section>
      <section class="set-card">
        <h2>{t("up.title")}</h2>
        <Show when={isTauri()} fallback={<p class="muted sm">{t("up.web")}</p>}>
          <p class="muted sm">{t("up.desc")}</p>
          <div class="row-input">
            <button class="btn primary" disabled={updateState.checking() || updateState.progress() !== null} onClick={() => checkUpdate()}>
              {updateState.checking() ? t("up.checking") : t("up.check")}
            </button>
          </div>
          <Show when={updateState.error()}><p class={updateState.info() ? "ok" : "muted sm"}>{updateState.error()}</p></Show>
          <Show when={updateState.info()}>
            {(u) => (
              <>
                <p><b>{t("up.found", { v: u().version })}</b></p>
                <Show when={u().body}><p class="muted sm">{t("up.body")}: {u().body}</p></Show>
                <Show when={!updateState.done()} fallback={
                  <button class="btn primary" onClick={restartApp}>{t("up.restart")}</button>
                }>
                  <button class="btn primary" disabled={updateState.progress() !== null} onClick={installUpdate}>
                    {updateState.progress() !== null
                      ? updateState.progress()! < 0 ? t("up.downloading", { p: "…" }) : t("up.downloading", { p: `${updateState.progress()}%` })
                      : t("up.download")}
                  </button>
                </Show>
                <Show when={updateState.done()}><p class="ok">{t("up.ready")}</p></Show>
              </>
            )}
          </Show>
        </Show>
      </section>
      <section class="set-card">
        <h2>{t("set.reindex")}</h2>
        <p class="muted sm">
          {t("set.reindexDesc")}
        </p>
        <div class="row-input">
          <select value={target()} onChange={(e) => setTarget(e.currentTarget.value)}>
            <option value="">{t("set.allUsers")}</option>
            <For each={users() ?? []}>{(u) => <option value={u.id}>{u.username}</option>}</For>
          </select>
          <button class="btn primary" disabled={busy() || !users()} onClick={reindex}>
            {busy() ? t("set.scanning") : t("set.rebuild")}
          </button>
        </div>
        <Show when={result()}><pre class="secret">{result()}</pre></Show>
      </section>
    </>
  );
}
