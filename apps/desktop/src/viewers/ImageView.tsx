import { Show, createEffect, createSignal, onCleanup } from "solid-js";
import { Portal } from "solid-js/web";
import Lightbox from "./Lightbox";
import { api } from "../api";
import { Icon } from "../icons";

export interface ImgItem {
  path: string;
  name: string;
}

/** Lightbox ảnh: vuốt/trái-phải + phím mũi tên trong cùng thư mục. */
export default function ImageView(props: { files: ImgItem[]; index: number }) {
  const [i, setI] = createSignal(props.index);
  const [err, setErr] = createSignal("");
  const [zoom, setZoom] = createSignal(false);
  const n = () => props.files.length;
  const cur = () => props.files[i()];
  const prev = () => setI((i() + n() - 1) % n());
  const next = () => setI((i() + 1) % n());
  let startX = 0;

  const onKey = (e: KeyboardEvent) => {
    if (zoom()) return;
    if (e.key === "ArrowLeft") prev();
    if (e.key === "ArrowRight") next();
  };
  createEffect(() => {
    setErr("");
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  return (
    <div class="lightbox" onClick={(e) => {
      // Click nền đen thì thôi (không đóng, viewer đã có nút Quay lại riêng).
      e.stopPropagation();
    }}>
      <Show when={n() > 1}>
        <button class="icon-btn lb-nav" onClick={(e) => { e.stopPropagation(); prev(); }} title="Ảnh trước"><Icon name="back" size={22} /></button>
      </Show>
      <figure
        class="lb-main"
        onTouchStart={(e) => { startX = e.touches[0].clientX; }}
        onTouchEnd={(e) => {
          const dx = e.changedTouches[0].clientX - startX;
          if (Math.abs(dx) > 40) (dx > 0 ? prev() : next());
        }}
      >
        <img src={api.streamUrl(cur().path)} alt={cur().name} draggable={false} title="Bấm để phóng to" style={{ cursor: "zoom-in" }} onClick={() => setZoom(true)} onError={() => setErr("Không tải được ảnh")} />
        <figcaption class="muted sm">{cur().name} · {i() + 1}/{n()}</figcaption>
        <Show when={err()}><p class="err">{err()}</p></Show>
      </figure>
      <Show when={n() > 1}>
        <button class="icon-btn lb-nav" onClick={(e) => { e.stopPropagation(); next(); }} title="Ảnh tiếp"><Icon name="next" size={22} /></button>
      </Show>
      <Show when={zoom()}>
        <Portal><Lightbox
          src={api.streamUrl(cur().path)}
          alt={cur().name}
          onClose={() => setZoom(false)}
          onPrev={n() > 1 ? prev : undefined}
          onNext={n() > 1 ? next : undefined}
          label={n() > 1 ? `${i() + 1}/${n()}` : undefined}
        /></Portal>
      </Show>
    </div>
  );
}
