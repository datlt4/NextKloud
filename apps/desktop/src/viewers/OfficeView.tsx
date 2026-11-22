import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { api, serverOrigin } from "../api";

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
      const s = document.createElement("script");
      // DocumentServer đi qua proxy /oo của backend: cùng origin, https.
      s.src = `${serverOrigin()}/oo/web-apps/apps/api/documents/api.js`;
      s.onload = () => (window.DocsAPI ? res() : rej(new Error("không tải được trình soạn thảo OnlyOffice")));
      s.onerror = () => {
        scriptPromise = null; // lần sau thử lại
        rej(new Error("không kết nối được OnlyOffice (kiểm tra ONLYOFFICE_URL trên server)"));
      };
      document.head.appendChild(s);
    });
  }
  return scriptPromise;
}

/**
 * Mở/sửa office trực tiếp bằng OnlyOffice DocumentServer (qua proxy /oo).
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
      // DocEditor nhận id của placeholder (không nhận element); nó thay placeholder bằng iframe.
      const ph = document.createElement("div");
      ph.id = `oo-${Math.random().toString(36).slice(2)}`;
      box.replaceChildren(ph);
      editor = new window.DocsAPI.DocEditor(ph.id, {
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
