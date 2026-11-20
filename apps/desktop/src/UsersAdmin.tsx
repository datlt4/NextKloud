import { For, Match, Show, Switch, createSignal, onMount } from "solid-js";
import { api, type UserRow } from "./api";
import { fmtSize } from "./format";
import { Icon } from "./icons";

const GB = 1024 ** 3;

/** Mật khẩu mặc định: bỏ ký tự dễ nhầm (0/O, 1/l/I). */
export function genPassword(n = 12): string {
  const abc = "abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
  const r = crypto.getRandomValues(new Uint32Array(n));
  return Array.from(r, (x) => abc[x % abc.length]).join("");
}

type Dialog =
  | { kind: "create" }
  | { kind: "quota"; u: UserRow }
  | { kind: "reset"; u: UserRow }
  | { kind: "delete"; u: UserRow }
  | { kind: "secret"; title: string; username: string; password: string };

export default function UsersAdmin(props: { meId: string; onError: (m: string) => void; onChanged: () => void }) {
  const [users, setUsers] = createSignal<UserRow[]>([]);
  const [dlg, setDlg] = createSignal<Dialog | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [dlgErr, setDlgErr] = createSignal("");

  const reload = async () => {
    try {
      setUsers(await api.users());
    } catch (e) {
      props.onError("Không tải được danh sách người dùng: " + String((e as Error).message));
    }
  };
  onMount(reload);

  const open = (d: Dialog) => {
    setDlgErr("");
    setDlg(d);
  };
  /** Chạy thao tác trong dialog; lỗi hiện ngay trong dialog. */
  const run = async (f: () => Promise<Dialog | null>) => {
    setBusy(true);
    setDlgErr("");
    try {
      setDlg(await f());
      await reload();
      props.onChanged();
    } catch (e) {
      const m = String((e as Error).message);
      setDlgErr(
        m.includes("username taken") ? "Tên đăng nhập đã tồn tại" : m.includes("at least 8") ? "Mật khẩu tối thiểu 8 ký tự" : m,
      );
    } finally {
      setBusy(false);
    }
  };

  const pct = (u: UserRow) => (u.quota_limit > 0 ? Math.min(100, (u.quota_used / u.quota_limit) * 100) : 0);

  return (
    <div class="admin">
      <div class="admin-head">
        <div>
          <h2>Người dùng</h2>
          <p class="muted sm">{users().length} tài khoản · người mới phải đổi mật khẩu ở lần đăng nhập đầu</p>
        </div>
        <button class="btn primary" onClick={() => open({ kind: "create" })}>
          <Icon name="plus" /> <span class="hide-xs">Thêm người dùng</span>
        </button>
      </div>

      <div class="ulist">
        <For each={users()}>
          {(u) => (
            <div class="ucard">
              <div class="avatar static">{u.username.slice(0, 1).toUpperCase()}</div>
              <div class="ubody">
                <div class="uname">
                  <b>{u.username}</b>
                  <Show when={u.is_admin}><span class="tag">admin</span></Show>
                  <Show when={u.id === props.meId}><span class="tag soft">bạn</span></Show>
                  <Show when={u.must_reset}><span class="tag warn">chờ đổi mật khẩu</span></Show>
                </div>
                <div class="uquota">
                  <span>
                    {fmtSize(u.quota_used)} / {u.quota_limit > 0 ? fmtSize(u.quota_limit) : "không giới hạn"}
                  </span>
                  <Show when={u.quota_limit > 0}>
                    <div class="bar"><i style={{ width: `${pct(u)}%` }} classList={{ hot: u.quota_used > u.quota_limit * 0.9 }} /></div>
                  </Show>
                </div>
              </div>
              <div class="uact">
                <button class="btn ghost sm" onClick={() => open({ kind: "quota", u })} title="Dung lượng">
                  <Icon name="cloud" /> <span class="hide-sm">Quota</span>
                </button>
                <button class="btn ghost sm" onClick={() => open({ kind: "reset", u })} title="Đặt lại mật khẩu">
                  <Icon name="key" /> <span class="hide-sm">Reset mật khẩu</span>
                </button>
                <button
                  class="btn ghost sm danger"
                  disabled={u.id === props.meId}
                  onClick={() => open({ kind: "delete", u })}
                  title={u.id === props.meId ? "Không thể xoá chính mình" : "Xoá người dùng"}
                >
                  <Icon name="trash" />
                </button>
              </div>
            </div>
          )}
        </For>
      </div>

      <Show when={dlg()}>
        {(d) => (
          <div class="modal-bg" onClick={() => !busy() && setDlg(null)}>
            <div class="modal glass form" onClick={(e) => e.stopPropagation()}>
              <Switch>
                <Match when={d().kind === "create"}>
                  <CreateForm busy={busy()} err={dlgErr()} onClose={() => setDlg(null)} onSubmit={(b) =>
                    run(async () => {
                      await api.createUser(b);
                      return { kind: "secret", title: "Đã tạo người dùng", username: b.username, password: b.password };
                    })}
                  />
                </Match>
                <Match when={d().kind === "quota" && (d() as { u: UserRow }).u}>
                  {(u) => <QuotaForm u={u()} busy={busy()} err={dlgErr()} onClose={() => setDlg(null)} onSubmit={(bytes) =>
                    run(async () => {
                      await api.setQuota(u().id, bytes);
                      return null;
                    })}
                  />}
                </Match>
                <Match when={d().kind === "reset" && (d() as { u: UserRow }).u}>
                  {(u) => <ResetForm u={u()} busy={busy()} err={dlgErr()} onClose={() => setDlg(null)} onSubmit={(pw) =>
                    run(async () => {
                      await api.resetPassword(u().id, pw);
                      return { kind: "secret", title: "Đã đặt lại mật khẩu", username: u().username, password: pw };
                    })}
                  />}
                </Match>
                <Match when={d().kind === "delete" && (d() as { u: UserRow }).u}>
                  {(u) => <DeleteForm u={u()} busy={busy()} err={dlgErr()} onClose={() => setDlg(null)} onSubmit={() =>
                    run(async () => {
                      await api.deleteUser(u().id);
                      return null;
                    })}
                  />}
                </Match>
                <Match when={d().kind === "secret" && (d() as Extract<Dialog, { kind: "secret" }>)}>
                  {(s) => <SecretView s={s()} onClose={() => setDlg(null)} />}
                </Match>
              </Switch>
            </div>
          </div>
        )}
      </Show>
    </div>
  );
}

function Head(props: { title: string; onClose: () => void }) {
  return (
    <div class="modal-head">
      <h3>{props.title}</h3>
      <button type="button" class="icon-btn" onClick={props.onClose}><Icon name="close" /></button>
    </div>
  );
}

function PasswordField(props: { value: string; onInput: (v: string) => void }) {
  return (
    <div class="pw-row">
      <input value={props.value} onInput={(e) => props.onInput(e.currentTarget.value)} autocomplete="off" spellcheck={false} />
      <button type="button" class="btn ghost" onClick={() => props.onInput(genPassword())} title="Tạo ngẫu nhiên">
        Tạo mới
      </button>
    </div>
  );
}

// Thanh kéo phi tuyến: mỗi đoạn chiếm bề rộng bằng nhau dù khoảng GB khác nhau.
const STOPS = [0, 5, 10, 50, 100, 500, 1000, 2000];
const STEPS = [0.5, 1, 5, 10, 50, 100, 250];
const SEG = 100;
const MAX_POS = SEG * (STOPS.length - 1);

export function posToGb(pos: number): number {
  const i = Math.min(Math.floor(pos / SEG), STOPS.length - 2);
  const g = STOPS[i] + ((pos - i * SEG) / SEG) * (STOPS[i + 1] - STOPS[i]);
  return Math.round(g / STEPS[i]) * STEPS[i];
}

export function gbToPos(gb: number): number {
  if (gb >= STOPS[STOPS.length - 1]) return MAX_POS;
  const i = Math.max(0, STOPS.findIndex((s, k) => gb < STOPS[k + 1]));
  return i * SEG + ((gb - STOPS[i]) / (STOPS[i + 1] - STOPS[i])) * SEG;
}

const fmtGb = (g: number) => (g === 0 ? "Không giới hạn" : g >= 1000 ? `${+(g / 1000).toFixed(2)} TB` : `${g} GB`);

function QuotaSlider(props: { gb: number; onChange: (gb: number) => void; usedBytes?: number }) {
  const pct = (gb: number) => `${(gbToPos(gb) / MAX_POS) * 100}%`;
  return (
    <div class="qslider">
      <div class="qs-top">
        <b>{fmtGb(props.gb)}</b>
        <input
          type="number"
          min="0"
          step="any"
          value={props.gb}
          onInput={(e) => props.onChange(Math.max(0, +e.currentTarget.value || 0))}
          aria-label="GB"
        />
        <span class="muted sm">GB</span>
      </div>
      <div class="qs-track" style={{ "--p": pct(props.gb) }}>
        <input
          type="range"
          min="0"
          max={MAX_POS}
          step="1"
          value={gbToPos(props.gb)}
          onInput={(e) => props.onChange(posToGb(+e.currentTarget.value))}
        />
        <Show when={props.usedBytes}>
          <i class="qs-used" style={{ left: pct(props.usedBytes! / GB) }} title="Đang dùng" />
        </Show>
      </div>
      <div class="qs-ticks">
        <For each={STOPS}>
          {(g) => (
            <button type="button" style={{ left: pct(g) }} classList={{ on: props.gb === g }} onClick={() => props.onChange(g)}>
              {g === 0 ? "∞" : g >= 1000 ? `${g / 1000}TB` : g}
            </button>
          )}
        </For>
      </div>
    </div>
  );
}

type FormProps<T> = { busy: boolean; err: string; onClose: () => void; onSubmit: (v: T) => void };

function CreateForm(props: FormProps<{ username: string; password: string; quota_limit: number; is_admin: boolean }>) {
  const [name, setName] = createSignal("");
  const [pw, setPw] = createSignal(genPassword());
  const [gb, setGb] = createSignal(10);
  const [admin, setAdmin] = createSignal(false);
  return (
    <form onSubmit={(e) => {
      e.preventDefault();
      props.onSubmit({ username: name().trim(), password: pw(), quota_limit: Math.round(gb() * GB), is_admin: admin() });
    }}>
      <Head title="Thêm người dùng" onClose={props.onClose} />
      <Show when={props.err}><p class="err">{props.err}</p></Show>
      <label>Tên đăng nhập</label>
      <input value={name()} onInput={(e) => setName(e.currentTarget.value)} autocomplete="off" required maxLength={64} />
      <label>Mật khẩu mặc định</label>
      <PasswordField value={pw()} onInput={setPw} />
      <p class="muted sm">Người dùng sẽ phải đổi mật khẩu ở lần đăng nhập đầu.</p>
      <label>Dung lượng</label>
      <QuotaSlider gb={gb()} onChange={setGb} />
      <label class="check">
        <input type="checkbox" checked={admin()} onChange={(e) => setAdmin(e.currentTarget.checked)} /> Quyền quản trị
      </label>
      <button type="submit" class="btn primary block" disabled={props.busy || !name().trim()}>
        {props.busy ? "Đang tạo…" : "Tạo người dùng"}
      </button>
    </form>
  );
}

function QuotaForm(props: FormProps<number> & { u: UserRow }) {
  const [gb, setGb] = createSignal(+(props.u.quota_limit / GB).toFixed(2));
  const bytes = () => Math.round(gb() * GB);
  return (
    <form onSubmit={(e) => { e.preventDefault(); props.onSubmit(bytes()); }}>
      <Head title={`Dung lượng · ${props.u.username}`} onClose={props.onClose} />
      <Show when={props.err}><p class="err">{props.err}</p></Show>
      <p class="muted sm">Đang dùng {fmtSize(props.u.quota_used)}.</p>
      <label>Giới hạn</label>
      <QuotaSlider gb={gb()} onChange={setGb} usedBytes={props.u.quota_used} />
      <Show when={bytes() > 0 && bytes() < props.u.quota_used}>
        <p class="err">Thấp hơn dung lượng đang dùng: người này sẽ không upload được cho tới khi xoá bớt file.</p>
      </Show>
      <button type="submit" class="btn primary block" disabled={props.busy || !(gb() >= 0)}>
        {props.busy ? "Đang lưu…" : "Lưu"}
      </button>
    </form>
  );
}

function ResetForm(props: FormProps<string> & { u: UserRow }) {
  const [pw, setPw] = createSignal(genPassword());
  return (
    <form onSubmit={(e) => { e.preventDefault(); props.onSubmit(pw()); }}>
      <Head title={`Đặt lại mật khẩu · ${props.u.username}`} onClose={props.onClose} />
      <Show when={props.err}><p class="err">{props.err}</p></Show>
      <label>Mật khẩu mặc định mới</label>
      <PasswordField value={pw()} onInput={setPw} />
      <p class="muted sm">Người dùng sẽ phải đổi mật khẩu ở lần đăng nhập tới.</p>
      <button type="submit" class="btn primary block" disabled={props.busy}>
        {props.busy ? "Đang lưu…" : "Đặt lại"}
      </button>
    </form>
  );
}

function DeleteForm(props: FormProps<void> & { u: UserRow }) {
  const [typed, setTyped] = createSignal("");
  return (
    <form onSubmit={(e) => { e.preventDefault(); props.onSubmit(); }}>
      <Head title="Xoá người dùng" onClose={props.onClose} />
      <Show when={props.err}><p class="err">{props.err}</p></Show>
      <p class="err">
        Xoá <b>{props.u.username}</b> và <b>toàn bộ {fmtSize(props.u.quota_used)}</b> dữ liệu của người này. Không thể hoàn tác.
      </p>
      <label>Gõ lại tên đăng nhập để xác nhận</label>
      <input value={typed()} onInput={(e) => setTyped(e.currentTarget.value)} autocomplete="off" />
      <button type="submit" class="btn danger block" disabled={props.busy || typed() !== props.u.username}>
        {props.busy ? "Đang xoá…" : "Xoá vĩnh viễn"}
      </button>
    </form>
  );
}

function SecretView(props: { s: { title: string; username: string; password: string }; onClose: () => void }) {
  const [copied, setCopied] = createSignal(false);
  const text = () => `Tên đăng nhập: ${props.s.username}\nMật khẩu: ${props.s.password}`;
  return (
    <div>
      <Head title={props.s.title} onClose={props.onClose} />
      <p class="muted sm">Gửi thông tin này cho người dùng. Mật khẩu chỉ hiện một lần.</p>
      <pre class="secret">{text()}</pre>
      <button class="btn primary block" onClick={async () => {
        await navigator.clipboard?.writeText(text()).catch(() => {});
        setCopied(true);
      }}>
        <Icon name={copied() ? "check" : "text"} /> {copied() ? "Đã chép" : "Chép"}
      </button>
    </div>
  );
}
