import { Show, createSignal } from "solid-js";

export function ResetForm(props: { current?: string; onDone: (o: string, n: string) => void; onError: (m: string) => void }) {
  const [o, setO] = createSignal("");
  const [n, setN] = createSignal("");
  const [n2, setN2] = createSignal("");
  const submit = () => {
    if (n().length < 8) return props.onError("Mật khẩu mới phải có ít nhất 8 ký tự");
    if (n() !== n2()) return props.onError("Hai lần nhập mật khẩu mới không khớp");
    props.onDone(props.current || o(), n());
  };
  return (
    <form onSubmit={(e) => { e.preventDefault(); submit(); }}>
      <Show when={!props.current}>
        <label>Mật khẩu hiện tại</label>
        <input type="password" value={o()} onInput={(e) => setO(e.currentTarget.value)} autocomplete="current-password" />
      </Show>
      <label>Mật khẩu mới</label>
      <input type="password" value={n()} onInput={(e) => setN(e.currentTarget.value)} autocomplete="new-password" />
      <label>Nhập lại mật khẩu mới</label>
      <input type="password" value={n2()} onInput={(e) => setN2(e.currentTarget.value)} autocomplete="new-password" />
      <button type="submit" class="btn primary block">Đổi mật khẩu</button>
    </form>
  );
}
