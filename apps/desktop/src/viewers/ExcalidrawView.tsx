import { Show, createSignal, onCleanup } from "solid-js";
import { Excalidraw } from "@excalidraw/excalidraw";
import { api } from "../api";

/**
 * Sửa .excalidraw bằng editor chính chủ (lazy chunk, chỉ tải khi mở file).
 * Tự lưu ngược lại file sau mỗi lần sửa (debounce 2s).
 */
export default function ExcalidrawView(props: { path: string }) {
  const [err, setErr] = createSignal("");
  const [data, setData] = createSignal<any>(null);
  const [ready, setReady] = createSignal(false);
  let timer: any = null;
  let dead = false;

  api.readText(props.path).then(
    (t) => {
      if (dead) return;
      try {
        const j = t.trim() ? JSON.parse(t) : {};
        setData({ elements: j.elements ?? [], appState: { ...(j.appState ?? {}), currentItemFontFamily: 1 }, files: j.files ?? {} });
      } catch {
        setData({ elements: [], appState: {}, files: {} });
      }
      setReady(true);
    },
    (e: any) => !dead && setErr(String(e?.message ?? e)),
  );
  onCleanup(() => {
    dead = true;
    clearTimeout(timer);
  });

  const onChange = (elements: any, appState: any, files: any) => {
    clearTimeout(timer);
    timer = setTimeout(() => {
      api.saveText(props.path, JSON.stringify({ type: "excalidraw", version: 2, source: "nextkloud", elements, appState, files })).catch((e: any) => {
        if (!dead) setErr(String(e?.message ?? e));
      });
    }, 2000);
  };

  return (
    <div class="excali-wrap">
      <Show when={err()}><p class="err">{err()}</p></Show>
      <Show when={ready() && data()} fallback={<p class="muted">Đang mở Excalidraw…</p>}>
        <Excalidraw initialData={data()} onChange={onChange} />
      </Show>
    </div>
  );
}
