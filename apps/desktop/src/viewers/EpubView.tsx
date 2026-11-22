import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { api, isSecret } from "../api";
import { Icon } from "../icons";

// epubjs không có type đầy đủ cho cách dùng này.
/** Đọc epub (sách điện tử): lật trang, giữ vị trí. */
export default function EpubView(props: { path: string }) {
  const [err, setErr] = createSignal("");
  const [loc, setLoc] = createSignal("");
  let host!: HTMLDivElement;
  let rendition: any = null;
  let book: any = null;
  let dead = false;

  onMount(async () => {
    try {
      const mod = await import("epubjs");
      const ePub = (mod as any).default ?? mod;
      const buf = await (await api.readBlob(props.path)).arrayBuffer();
      if (dead) return;
      book = ePub(buf);
      rendition = book.renderTo(host, { width: "100%", height: "100%" });
      // file bí mật: không ghi tên vào localStorage
      const key = isSecret(props.path) ? "" : `nk-epub-loc:${props.path}`;
      const saved = key ? localStorage.getItem(key) : null;
      await rendition.display(saved || undefined);
      rendition.on("relocated", (l: any) => {
        setLoc(`${l.start.cfi}`);
        try {
          if (key) localStorage.setItem(key, l.start.cfi);
        } catch {}
      });
    } catch (e: any) {
      if (!dead) setErr(String(e?.message ?? e));
    }
  });
  onCleanup(() => {
    dead = true;
    try {
      rendition?.destroy();
    } catch {}
    rendition = book = null;
  });

  return (
    <div class="epub-wrap">
      <div class="pdf-bar">
        <button class="icon-btn" onClick={() => rendition?.prev()} title="Trang trước"><Icon name="back" /></button>
        <button class="icon-btn" onClick={() => rendition?.next()} title="Trang tiếp"><Icon name="next" /></button>
        <span class="grow" />
        <span class="muted sm">{loc() ? "đang đọc…" : props.path.split("/").pop()}</span>
      </div>
      <Show when={err()}><p class="err">{err()}</p></Show>
      <div class="epub-book" ref={host!} />
    </div>
  );
}
