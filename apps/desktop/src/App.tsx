import { For, Match, Show, Switch, createSignal, onMount } from "solid-js";
import { api, isSelfHostedWeb, serverOrigin } from "./api";
import MarkdownView from "./viewers/MarkdownView";

export default function App() {
  const [server, setServer] = createSignal(serverOrigin());
  const [username, setUsername] = createSignal("");
  const [password, setPassword] = createSignal("");
  const [authed, setAuthed] = createSignal(!!localStorage.getItem("nk-token"));
  const [isAdmin, setIsAdmin] = createSignal(false);
  const [needReset, setNeedReset] = createSignal(false);
  const [err, setErr] = createSignal("");
  const [prefix, setPrefix] = createSignal("");
  const [files, setFiles] = createSignal<{ name: string; is_dir: boolean; size: number }[]>([]);
  const [open, setOpen] = createSignal<{ path: string; text: string } | null>(null);

  const load = async (p: string) => {
    setErr("");
    try {
      setFiles(await api.files(p));
      setPrefix(p);
    } catch (e: any) {
      if (String(e.message).includes("unauthorized")) {
        localStorage.removeItem("nk-token");
        setAuthed(false);
      } else setErr("Không tải được danh sách file: " + String(e.message));
    }
  };

  onMount(() => {
    if (authed()) load("");
  });

  const doLogin = async () => {
    setErr("");
    try {
      localStorage.setItem("nk-server", server());
      const r = await api.login(username(), password());
      localStorage.setItem("nk-token", r.token);
      setIsAdmin(r.is_admin);
      if (r.must_reset) setNeedReset(true);
      else {
        setAuthed(true);
        load("");
      }
    } catch {
      setErr("Đăng nhập thất bại — kiểm tra server/username/password");
    }
  };

  const openFile = async (name: string) => {
    const path = prefix() ? `${prefix()}/${name}` : name;
    setErr("");
    try {
      if (/\.md$/i.test(name)) setOpen({ path, text: await api.readText(path) });
      else {
        const url = await api.downloadUrl(path);
        const a = document.createElement("a");
        a.href = url;
        a.download = name;
        a.click();
      }
    } catch (e: any) {
      setErr("Không mở được file: " + String(e.message));
    }
  };

  const up = () => {
    const p = prefix().split("/").filter(Boolean);
    p.pop();
    setOpen(null);
    load(p.join("/"));
  };

  return (
    <div class="wrap">
      <div class="topbar">
        <div class="brand">
          Next<span>Kloud</span>
        </div>
        <Show when={authed()}>
          <button class="btn-ghost" onClick={() => { localStorage.removeItem("nk-token"); setAuthed(false); setOpen(null); }}>
            Đăng xuất{isAdmin() ? " (admin)" : ""}
          </button>
        </Show>
      </div>

      <Show when={err()}>
        <p class="err">{err()}</p>
      </Show>

      <Switch>
        <Match when={needReset()}>
          <div class="card login-card">
            <h2>Đổi mật khẩu lần đầu</h2>
            <p class="muted">Tài khoản mới cấp — đặt mật khẩu mới (tối thiểu 8 ký tự).</p>
            <ResetForm
              onDone={async (o, n) => {
                setErr("");
                try {
                  await api.changePassword(o, n);
                  setNeedReset(false);
                  setAuthed(true);
                  load("");
                } catch (e: any) {
                  setErr("Đổi mật khẩu thất bại: " + String(e.message));
                }
              }}
            />
          </div>
        </Match>
        <Match when={!authed()}>
          <div class="card login-card">
            <h2>Đăng nhập</h2>
            <Show when={isSelfHostedWeb()} fallback={<p class="muted">Server backend NextKloud của bạn (vd http://mini:8011).</p>}>
              <p class="muted">Đang kết nối tới server này.</p>
            </Show>
            <Show when={!isSelfHostedWeb()}>
              <input value={server()} onInput={(e) => setServer(e.currentTarget.value)} placeholder="Server" />
            </Show>
            <input value={username()} onInput={(e) => setUsername(e.currentTarget.value)} placeholder="Username" autocomplete="username" />
            <input type="password" value={password()} onInput={(e) => setPassword(e.currentTarget.value)} placeholder="Password" autocomplete="current-password"
              onKeyDown={(e) => { if (e.key === "Enter") doLogin(); }} />
            <div class="btn-row">
              <button onClick={doLogin}>Đăng nhập</button>
            </div>
          </div>
        </Match>
        <Match when={true}>
          <div class="card">
            <div class="crumb">
              <Show when={prefix()}>
                <button class="btn-ghost" onClick={up}>⬆ Lên</button>
              </Show>
              <span>/{prefix()}</span>
            </div>
            <Show when={!open()}>
              <ul class="files">
                <For each={files()} fallback={<li class="muted">Thư mục trống</li>}>
                  {(f) => (
                    <li>
                      <span>{f.is_dir ? "📁" : "📄"}</span>
                      <a
                        href="#"
                        onClick={(e) => {
                          e.preventDefault();
                          if (f.is_dir) {
                            setOpen(null);
                            load(prefix() ? `${prefix()}/${f.name}` : f.name);
                          } else openFile(f.name);
                        }}
                      >
                        {f.name}
                      </a>
                      {f.is_dir ? "" : <span class="size">{(f.size / 1024).toFixed(1)} KB</span>}
                    </li>
                  )}
                </For>
              </ul>
            </Show>
            <Show when={open()}>
              <div class="btn-row" style={{ "margin-bottom": "12px" }}>
                <button class="btn-ghost" onClick={() => setOpen(null)}>← Đóng</button>
              </div>
              <MarkdownView path={open()!.path} text={open()!.text} />
            </Show>
          </div>
        </Match>
      </Switch>
    </div>
  );
}

function ResetForm(props: { onDone: (o: string, n: string) => void }) {
  const [o, setO] = createSignal("");
  const [n, setN] = createSignal("");
  return (
    <div>
      <input type="password" placeholder="Mật khẩu hiện tại" value={o()} onInput={(e) => setO(e.currentTarget.value)} />
      <input type="password" placeholder="Mật khẩu mới" value={n()} onInput={(e) => setN(e.currentTarget.value)} />
      <div class="btn-row">
        <button onClick={() => props.onDone(o(), n())}>Đổi mật khẩu</button>
      </div>
    </div>
  );
}
