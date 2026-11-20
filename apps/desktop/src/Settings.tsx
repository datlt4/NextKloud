import { For, Show, createResource, createSignal } from "solid-js";
import { api, type Me } from "./api";
import { Avatar, squareAvatar } from "./avatar";
import { fmtSize } from "./format";
import { ResetForm } from "./forms";
import { Icon } from "./icons";
import { LookPanel } from "./look";
import UsersAdmin from "./UsersAdmin";
import { GitPanel } from "./gitsync";

export type SettingsTab = "profile" | "security" | "look" | "git" | "users" | "system";

const TABS: { id: SettingsTab; label: string; icon: string; admin?: boolean }[] = [
  { id: "profile", label: "Hồ sơ", icon: "user" },
  { id: "security", label: "Mật khẩu", icon: "key" },
  { id: "look", label: "Giao diện & nền", icon: "palette" },
  { id: "git", label: "Đồng bộ Git", icon: "code" },
  { id: "users", label: "Người dùng", icon: "users", admin: true },
  { id: "system", label: "Hệ thống", icon: "server", admin: true },
];

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
        <div class="set-group">Cá nhân</div>
        <For each={tabs().filter((t) => !t.admin)}>
          {(t) => (
            <button class="nav" classList={{ active: props.tab === t.id }} onClick={() => props.onTab(t.id)}>
              <Icon name={t.icon} /> {t.label}
            </button>
          )}
        </For>
        <Show when={props.me.is_admin}>
          <div class="set-group">Quản trị</div>
          <For each={tabs().filter((t) => t.admin)}>
            {(t) => (
              <button class="nav" classList={{ active: props.tab === t.id }} onClick={() => props.onTab(t.id)}>
                <Icon name={t.icon} /> {t.label}
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
            <h2>Giao diện & nền</h2>
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
      props.onError("Không lưu được hồ sơ: " + String((e as Error).message));
      return false;
    } finally {
      setBusy(false);
    }
  };
  const pct = () => (props.me.quota_limit > 0 ? Math.min(100, (props.me.quota_used / props.me.quota_limit) * 100) : 0);
  return (
    <>
      <section class="set-card">
        <h2>Hồ sơ</h2>
        <div class="profile-head">
          <Avatar id={props.me.id} v={props.me.avatar_v} name={props.me.display_name || props.me.username} size={88} class="big" />
          <div class="profile-act">
            <button class="btn primary" disabled={busy()} onClick={() => pick.click()}>
              <Icon name="image" /> {props.me.avatar_v ? "Đổi ảnh" : "Tải ảnh lên"}
            </button>
            <Show when={props.me.avatar_v > 0}>
              <button class="btn ghost" disabled={busy()} onClick={() => run(() => api.deleteAvatar())}>
                <Icon name="trash" /> Xoá ảnh
              </button>
            </Show>
            <p class="muted sm">Ảnh được cắt vuông, thu về 256px.</p>
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
          <label>Tên đăng nhập</label>
          <input value={props.me.username} disabled />
          <label>Tên hiển thị</label>
          <div class="row-input">
            <input value={name()} maxLength={64} placeholder={props.me.username} onInput={(e) => { setName(e.currentTarget.value); setSaved(false); }} />
            <button type="submit" class="btn primary" disabled={busy() || name().trim() === (props.me.display_name ?? "")}>
              {saved() ? "Đã lưu" : "Lưu"}
            </button>
          </div>
        </form>
      </section>
      <section class="set-card">
        <h2>Dung lượng</h2>
        <p class="big-num">
          {fmtSize(props.me.quota_used)}
          <span class="muted"> / {props.me.quota_limit > 0 ? fmtSize(props.me.quota_limit) : "không giới hạn"}</span>
        </p>
        <Show when={props.me.quota_limit > 0}>
          <div class="bar thick"><i style={{ width: `${pct()}%` }} classList={{ hot: pct() > 90 }} /></div>
          <p class="muted sm">Còn trống {fmtSize(Math.max(0, props.me.quota_limit - props.me.quota_used))}. Muốn thêm dung lượng, liên hệ quản trị viên.</p>
        </Show>
        <Show when={props.me.is_admin}><p class="muted sm"><span class="tag">admin</span> Tài khoản quản trị.</p></Show>
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
      <h2>Đổi mật khẩu</h2>
      <p class="muted sm">Tối thiểu 8 ký tự. Các thiết bị đang đăng nhập vẫn giữ phiên.</p>
      <Show when={err()}><p class="err">{err()}</p></Show>
      <Show when={ok()}><p class="ok">Đã đổi mật khẩu.</p></Show>
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
        lines.push(`${u.username}: ${r.files.toLocaleString("vi-VN")} file · ${fmtSize(r.bytes)}`);
      }
      setResult(lines.join("\n"));
      refetch();
    } catch (e) {
      props.onError("Dựng lại chỉ mục lỗi: " + String((e as Error).message));
    } finally {
      setBusy(false);
    }
  };
  const cards = () => {
    const s = stats();
    if (!s) return [];
    return [
      ["Phiên bản", s.version],
      ["Lưu trữ", s.storage === "s3" ? "RustFS (S3)" : s.storage],
      ["Người dùng", s.users.toLocaleString("vi-VN")],
      ["File", s.files.toLocaleString("vi-VN")],
      ["Thư mục", s.dirs.toLocaleString("vi-VN")],
      ["Tổng dung lượng", fmtSize(s.bytes)],
    ];
  };
  return (
    <>
      <section class="set-card">
        <h2>Hệ thống</h2>
        <div class="stats">
          <For each={cards()}>{([k, v]) => <div class="stat"><span class="muted sm">{k}</span><b>{v}</b></div>}</For>
        </div>
      </section>
      <section class="set-card">
        <h2>Dựng lại chỉ mục</h2>
        <p class="muted sm">
          Quét lại toàn bộ file trên kho lưu trữ, cập nhật danh sách, ngày sửa và dung lượng đã dùng. Dùng khi chép dữ liệu thẳng vào RustFS hoặc thấy danh sách bị lệch.
        </p>
        <div class="row-input">
          <select value={target()} onChange={(e) => setTarget(e.currentTarget.value)}>
            <option value="">Tất cả người dùng</option>
            <For each={users() ?? []}>{(u) => <option value={u.id}>{u.username}</option>}</For>
          </select>
          <button class="btn primary" disabled={busy() || !users()} onClick={reindex}>
            {busy() ? "Đang quét…" : "Dựng lại"}
          </button>
        </div>
        <Show when={result()}><pre class="secret">{result()}</pre></Show>
      </section>
    </>
  );
}
