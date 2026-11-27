import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { Excalidraw, getSceneVersion, serializeAsJSON } from "@excalidraw/excalidraw";
import { onEsc, inField } from "../esc";
import { api } from "../api";
import { Icon } from "../icons";
import { t } from "../i18n";

/**
 * Sửa .excalidraw bằng editor chính chủ (component React, mount vào 1 root React riêng
 * vì app là Solid). Chỉ tự lưu khi nét vẽ đổi (không lưu lúc mới mở / chỉ đổi zoom), debounce 2s.
 */
/** readonly: link công khai — chỉ xem, không lưu. */
export default function ExcalidrawView(props: { path: string; readonly?: boolean }) {
  const [err, setErr] = createSignal("");
  const [ready, setReady] = createSignal(false);
  // "Full màn hình" = lấp đầy khung trình duyệt (giữ chrome/nút điều hướng), KHÔNG dùng Fullscreen API.
  const [wide, setWide] = createSignal(false);
  let host!: HTMLDivElement;
  let root: Root | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let dead = false;
  let lastVer = -1;

  const onChange = (elements: any, appState: any, files: any) => {
    const v = getSceneVersion(elements);
    if (v === lastVer || props.readonly) return;
    lastVer = v;
    clearTimeout(timer);
    timer = setTimeout(() => {
      api.saveText(props.path, serializeAsJSON(elements, appState, files, "local")).catch((e: any) => {
        if (!dead) setErr(String(e?.message ?? e));
      });
    }, 2000);
  };

  api.readText(props.path).then(
    (t) => {
      if (dead) return;
      let data: any;
      try {
        const j = t.trim() ? JSON.parse(t) : {};
        data = { elements: j.elements ?? [], appState: { ...(j.appState ?? {}), collaborators: new Map() }, files: j.files ?? {} };
      } catch {
        data = { elements: [], appState: {}, files: {} };
      }
      lastVer = getSceneVersion(data.elements);
      root = createRoot(host);
      root.render(createElement(Excalidraw as any, { initialData: data, onChange, viewModeEnabled: !!props.readonly }));
      setReady(true);
    },
    (e: any) => !dead && setErr(String(e?.message ?? e)),
  );
  onCleanup(() => {
    dead = true;
    clearTimeout(timer);
    root?.unmount();
  });
  onMount(() => {
    onEsc((e) => wide() && !inField(e) && (setWide(false), true));
  });

  return (
    <div class="excali-wrap" classList={{ "vw-wide": wide() }}>
      <button class="fs-btn excali-fs" onClick={() => setWide((w) => !w)} title={wide() ? t("vw.fsExit") : t("vw.fsEnter")}>
        <Icon name={wide() ? "compress" : "expand"} size={17} />
      </button>
      <Show when={err()}><p class="err">{err()}</p></Show>
      <Show when={!ready() && !err()}><p class="muted">Đang mở Excalidraw…</p></Show>
      <div ref={host} class="excali-host" />
    </div>
  );
}
