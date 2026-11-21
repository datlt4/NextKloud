import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { api } from "../api";

declare global {
  interface Window {
    DocsAPI?: any;
  }
}

let scriptPromise: Promise<void> | null = null;
function loadDocsApi(): Promise<void> {
  if (window.DocsAPI) return Promise.resolve();
  if (!scriptPromise) {
    scriptPromise = new Promise((res, rej) => {
      const host = window.location.hostname || "localhost";
      const s = document.createElement("script");
      s.src = `http://${host}:8027/web-apps/apps/api/documents/api.js`;
      s.onload = () => (window.DocsAPI ? res() : rej(new Error("không tải được DocsAPI từ :8027")));
      s.onerror = () => rej(new Error("không tải được DocsAPI từ :8027 (DocumentServer có chạy không?)"));
      document.head.appendChild(s);
    });
  }
  return scriptPromise;
}

/**
 * Mở/sửa office trực tiếp bằng OnlyOffice DocumentServer (:8027).
 * File lưu qua callback của server -> ghi đè lên store + cập nhật quota/chỉ mục.
 */
export default function OfficeView(props: { path: string }) {
  const [err, setErr] = createSignal("");
  let box!: HTMLDivElement;
  let editor: any = null;
  let dead = false;

  onMount(async () => {
    try {
      await loadDocsApi();
      if (dead) return;
      const { config, token } = await api.officeConfig(props.path);
      if (dead) return;
      editor = new window.DocsAPI.DocEditor(box, {
        ...config,
        token,
        width: "100%",
        height: "100%",
        events: {
          onError: (e: any) => !dead && setErr(`OnlyOffice: ${e?.data ?? "lỗi không rõ"}`),
        },
      });
    } catch (e: any) {
      if (!dead) setErr(String(e?.message ?? e));
    }
  });
  onCleanup(() => {
    dead = true;
    try {
      editor?.destroy();
    } catch {}
    editor = null;
  });

  return (
    <div class="office-wrap">
      <Show when={err()}><p class="err">{err()}</p></Show>
      <div class="office-box" ref={box!} />
    </div>
  );
}
