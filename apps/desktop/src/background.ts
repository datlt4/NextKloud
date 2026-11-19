// Nền tuỳ chỉnh: preset CSS (0 byte) hoặc ảnh người dùng.
// Lưu trong storage của user (.nextkloud/) -> sync mọi thiết bị.
import { createSignal } from "solid-js";
import { api } from "./api";

export const PRESETS: { id: string; name: string; css: string }[] = [
  { id: "aurora", name: "Aurora", css: "radial-gradient(at 15% 20%, #4338ca 0, transparent 50%), radial-gradient(at 85% 15%, #0e7490 0, transparent 45%), radial-gradient(at 70% 90%, #9d174d 0, transparent 50%), #0f1222" },
  { id: "sunset", name: "Hoàng hôn", css: "linear-gradient(160deg, #1e1b4b 0%, #7c2d12 55%, #f59e0b 120%)" },
  { id: "forest", name: "Rừng", css: "radial-gradient(at 20% 80%, #166534 0, transparent 55%), radial-gradient(at 80% 20%, #134e4a 0, transparent 50%), #0b1410" },
  { id: "ocean", name: "Đại dương", css: "linear-gradient(180deg, #0c4a6e 0%, #082f49 50%, #020617 100%)" },
  { id: "sakura", name: "Sakura", css: "radial-gradient(at 75% 25%, #be185d 0, transparent 50%), radial-gradient(at 20% 75%, #6d28d9 0, transparent 50%), #1a1020" },
  { id: "plain", name: "Trơn", css: "#14161f" },
];

export interface UiSettings {
  bg: string; // preset id | "image"
  dim: number; // 0..0.8 lớp tối phủ lên ảnh
  v: number; // version, đổi khi ảnh mới -> bust cache
}

const SETTINGS = ".nextkloud/settings.json";
const IMAGE = ".nextkloud/background.webp";
const DEFAULT: UiSettings = { bg: "aurora", dim: 0.35, v: 0 };

function cached(): UiSettings {
  try {
    return { ...DEFAULT, ...JSON.parse(localStorage.getItem("nk-ui") || "{}") };
  } catch {
    return DEFAULT;
  }
}

const [ui, setUi] = createSignal<UiSettings>(cached());
const [imageUrl, setImageUrl] = createSignal<string | null>(null);
export { ui };

export function wallCss(): string {
  const s = ui();
  if (s.bg === "image" && imageUrl()) return `center / cover no-repeat url("${imageUrl()}"), #0f1222`;
  return (PRESETS.find((p) => p.id === s.bg) ?? PRESETS[0]).css;
}

const bgKey = (v: number) => `/__nk_bg?v=${v}`;

async function cachePut(v: number, blob: Blob) {
  try {
    const c = await caches.open("nk-bg");
    for (const k of await c.keys()) await c.delete(k);
    await c.put(bgKey(v), new Response(blob));
  } catch {
    /* không có Cache API */
  }
}

let loadedV = -1;

async function loadImage(v: number) {
  if (v === loadedV && imageUrl()) return;
  loadedV = v;
  const key = bgKey(v);
  try {
    const c = await caches.open("nk-bg");
    const hit = await c.match(key);
    if (hit) return setImageUrl(URL.createObjectURL(await hit.blob()));
    const blob = await api.readBlob(IMAGE);
    await cachePut(v, blob);
    setImageUrl(URL.createObjectURL(blob));
  } catch {
    // Cache API không có (http không secure) -> tải thẳng.
    try {
      setImageUrl(URL.createObjectURL(await api.readBlob(IMAGE)));
    } catch {
      /* ảnh mất -> preset */
    }
  }
}

function apply(s: UiSettings) {
  setUi(s);
  localStorage.setItem("nk-ui", JSON.stringify(s));
  if (s.bg === "image") loadImage(s.v);
}

/** Gọi sau đăng nhập: lấy cài đặt từ server (nguồn chuẩn). */
export async function syncFromServer() {
  apply(cached());
  try {
    apply({ ...DEFAULT, ...JSON.parse(await api.readText(SETTINGS)) });
  } catch {
    /* chưa có file -> giữ cache */
  }
}

async function save(s: UiSettings) {
  apply(s);
  await api.upload(SETTINGS, new Blob([JSON.stringify(s)], { type: "application/json" }));
}

export const setPreset = (id: string) => save({ ...ui(), bg: id });
/** Kéo slider: chỉ preview; thả tay mới lưu (tránh upload mỗi pixel). */
export const previewDim = (dim: number) => setUi({ ...ui(), dim });
export const setDim = (dim: number) => save({ ...ui(), dim });

/** Thu nhỏ về 2560px webp trước khi upload — nhẹ cho máy yếu. */
export async function setImage(file: File) {
  const bmp = await createImageBitmap(file);
  const k = Math.min(1, 2560 / Math.max(bmp.width, bmp.height));
  const cv = document.createElement("canvas");
  cv.width = Math.round(bmp.width * k);
  cv.height = Math.round(bmp.height * k);
  cv.getContext("2d")!.drawImage(bmp, 0, 0, cv.width, cv.height);
  bmp.close();
  const blob = await new Promise<Blob>((res, rej) =>
    cv.toBlob((b) => (b ? res(b) : rej(new Error("encode failed"))), "image/webp", 0.85),
  );
  await api.upload(IMAGE, blob);
  const v = Date.now();
  await cachePut(v, blob);
  loadedV = v;
  setImageUrl(URL.createObjectURL(blob));
  await save({ ...ui(), bg: "image", v });
}
