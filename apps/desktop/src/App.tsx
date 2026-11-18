import { For, Match, Show, Switch, createSignal, onMount } from "solid-js";
import { api, serverOrigin } from "./api";
import MarkdownView from "./viewers/MarkdownView";

function useForceReset() {
  const [need, setNeed] = createSignal(false);
  return { need, setNeed };
}

export default function App() {
  const [server, setServer] = createSignal(serverOrigin());
  const [username, setUsername] = createSignal("");
  const [password, setPassword] = createSignal("");
  const [authed, setAuthed] = createSignal(!!localStorage.getItem("nk-token"));
  const [err, setErr] = createSignal("");
  const fr = useForceReset();
  const [prefix, setPrefix] = createSignal("");
  const [files, setFiles] = createSignal<{ name: string; is_dir: boolean; size: number }[]>([]);
  const [open, setOpen] = createSignal<{ path: string; text: string } | null>(null);

  const load = async (p: string) => {
    setErr("");
    try {
      setFiles(await api.files(p));
      setPrefix(p);
    } catch (e: any) {
      if (String(e.message).includes("unauthorized")) setAuthed(false);
      else setErr(String(e.message));
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
      if (r.must_reset) fr.setNeed(true);
      else {
        setAuthed(true);
        load("");
      }
    } catch {
      setErr("Đăng nhập thất bại");
    }
  };

  const doReset = async (oldPw: string, next: string) => {
    setErr("");
    try {
      await api.changePassword(oldPw, next);
      fr.setNeed(false);
      setAuthed(true);
      load("");
    } catch (e: any) {
      setErr(String(e.message));
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
      setErr(String(e.message));
    }
  };

  const up = () => {
    const p = prefix().split("/").filter(Boolean);
    p.pop();
    setOpen(null);
    load(p.join("/"));
  };

  return (
    <main style={{ padding: "16px", "font-family": "system-ui", "max-width": "900px", margin: "0 auto" }}>
      <h1>NextKloud</h1>
      <Show when={err()}>
        <p style={{ color: "red" }}>{err()}</p>
      </Show>
      <Switch>
        <Match when={fr.need()}>
          <ResetForm onDone={doReset} />
        </Match>
        <Match when={!authed()}>
          <div>
            <input placeholder="Server (vd http://mini:8011)" value={server()} onInput={(e) => setServer(e.currentTarget.value)} style={{ width: "100%" }} />
            <input placeholder="Username" value={username()} onInput={(e) => setUsername(e.currentTarget.value)} />
            <input placeholder="Password" type="password" value={password()} onInput={(e) => setPassword(e.currentTarget.value)} />
            <button onClick={doLogin}>Đăng nhập</button>
          </div>
        </Match>
        <Match when={true}>
          <div>
            <button onClick={() => { localStorage.removeItem("nk-token"); setAuthed(false); }}>Đăng xuất</button>
            <Show when={prefix()}>
              <button onClick={up}>⬆ Lên</button>
            </Show>
            <Show when={!open()}>
              <ul>
                <For each={files()}>{(f) => <li>{f.is_dir ? "📁" : "📄"} <a href="#" onClick={(e) => { e.preventDefault(); f.is_dir ? (setOpen(null), load(prefix() ? `${prefix()}/${f.name}` : f.name)) : openFile(f.name); }}>{f.name}</a> {f.is_dir ? "" : `(${(f.size / 1024).toFixed(1)}KB)`}</li>}</For>
              </ul>
            </Show>
            <Show when={open()}>
              <button onClick={() => setOpen(null)}>← Đóng</button>
              <MarkdownView path={open()!.path} text={open()!.text} />
            </Show>
          </div>
        </Match>
      </Switch>
    </main>
  );
}

function ResetForm(props: { onDone: (o: string, n: string) => void }) {
  const [o, setO] = createSignal("");
  const [n, setN] = createSignal("");
  return (
    <div>
      <p>Đổi mật khẩu lần đầu (tối thiểu 8 ký tự):</p>
      <input placeholder="Mật khẩu hiện tại" type="password" value={o()} onInput={(e) => setO(e.currentTarget.value)} />
      <input placeholder="Mật khẩu mới" type="password" value={n()} onInput={(e) => setN(e.currentTarget.value)} />
      <button onClick={() => props.onDone(o(), n())}>Đổi</button>
    </div>
  );
}
