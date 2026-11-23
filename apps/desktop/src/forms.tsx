import { Show, createSignal } from "solid-js";
import { t } from "./i18n";

export function ResetForm(props: { current?: string; onDone: (o: string, n: string) => void; onError: (m: string) => void }) {
  const [o, setO] = createSignal("");
  const [n, setN] = createSignal("");
  const [n2, setN2] = createSignal("");
  const submit = () => {
    if (n().length < 8) return props.onError(t("set.passShort"));
    if (n() !== n2()) return props.onError(t("set.passMismatch"));
    props.onDone(props.current || o(), n());
  };
  return (
    <form onSubmit={(e) => { e.preventDefault(); submit(); }}>
      <Show when={!props.current}>
        <label>{t("set.passCur")}</label>
        <input type="password" value={o()} onInput={(e) => setO(e.currentTarget.value)} autocomplete="current-password" />
      </Show>
      <label>{t("set.passNew")}</label>
      <input type="password" value={n()} onInput={(e) => setN(e.currentTarget.value)} autocomplete="new-password" />
      <label>{t("set.passNew2")}</label>
      <input type="password" value={n2()} onInput={(e) => setN2(e.currentTarget.value)} autocomplete="new-password" />
      <button type="submit" class="btn primary block">{t("set.passSubmit")}</button>
    </form>
  );
}
