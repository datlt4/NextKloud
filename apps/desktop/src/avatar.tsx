import { Show, createEffect, createSignal } from "solid-js";
import { api } from "./api";

// <img> không gửi được Bearer token -> tải blob qua api rồi dùng object URL; cache theo id+version.
const cache = new Map<string, Promise<string>>();
const srcOf = (id: string, v: number) => {
  const k = `${id}:${v}`;
  let p = cache.get(k);
  if (!p) {
    p = api.avatarBlob(id, v).then((b) => URL.createObjectURL(b));
    p.catch(() => cache.delete(k));
    cache.set(k, p);
  }
  return p;
};

export function Avatar(props: { id: string; v: number; name: string; size?: number; class?: string }) {
  const [src, setSrc] = createSignal<string | null>(null);
  createEffect(() => {
    const { id, v } = props;
    setSrc(null);
    if (v > 0) srcOf(id, v).then((u) => props.id === id && props.v === v && setSrc(u)).catch(() => {});
  });
  const size = () => props.size ?? 34;
  return (
    <span class={`av ${props.class ?? ""}`} style={{ width: `${size()}px`, height: `${size()}px`, "font-size": `${size() * 0.42}px` }}>
      <Show when={src()} fallback={(props.name || "?").slice(0, 1).toUpperCase()}>
        <img src={src()!} alt="" />
      </Show>
    </span>
  );
}

/** Cắt giữa thành hình vuông, thu về 256px webp trước khi gửi (server chỉ nhận ≤ 1MB). */
export async function squareAvatar(file: File): Promise<Blob> {
  const bmp = await createImageBitmap(file);
  const s = Math.min(bmp.width, bmp.height);
  const cv = document.createElement("canvas");
  cv.width = cv.height = 256;
  cv.getContext("2d")!.drawImage(bmp, (bmp.width - s) / 2, (bmp.height - s) / 2, s, s, 0, 0, 256, 256);
  bmp.close();
  return new Promise((res, rej) => cv.toBlob((b) => (b ? res(b) : rej(new Error("encode failed"))), "image/webp", 0.88));
}
