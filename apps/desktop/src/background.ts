// Nền tuỳ chỉnh: preset CSS (0 byte) hoặc ảnh người dùng.
// Lưu trong storage của user (.nextkloud/) -> sync mọi thiết bị.
import { createSignal } from "solid-js";
import { api } from "./api";

export const PRESETS: { id: string; name: string; css: string }[] = [
  { id: "cream", name: "Kem", css: "radial-gradient(at 0% 0%, #fbe1cc 0, transparent 45%), radial-gradient(at 100% 100%, #f7d3bc 0, transparent 45%), #f5f0e8" },
  { id: "peach", name: "Đào", css: "linear-gradient(135deg, #fdeee2 0%, #f9cfb1 55%, #f2a77f 100%)" },
  { id: "sunset", name: "Hoàng hôn", css: "linear-gradient(170deg, #fff6ee 0%, #f6b48c 60%, #d9653b 115%)" },
  { id: "clay", name: "Đất nung", css: "radial-gradient(at 85% 10%, #e9875f 0, transparent 50%), radial-gradient(at 10% 95%, #c4552d 0, transparent 50%), #efd8c4" },
  { id: "plain", name: "Trơn", css: "#f5f0e8" },
  { id: "night", name: "Đêm ấm", css: "radial-gradient(at 15% 15%, #6b3520 0, transparent 50%), radial-gradient(at 90% 90%, #4a2a1a 0, transparent 45%), #1f1b17" },
];

/** Phông chữ đóng gói sẵn (chạy offline, không gọi Google Fonts); chỉ tải khi được chọn. */
export const FONTS: { id: string; name: string; family: string; load?: () => Promise<unknown> }[] = [
  { id: "system", name: "Mặc định hệ thống", family: 'system-ui, -apple-system, "Segoe UI", Roboto, sans-serif' },
  { id: "be-vietnam-pro", name: "Be Vietnam Pro", family: '"Be Vietnam Pro", sans-serif', load: () => Promise.all([import("@fontsource/be-vietnam-pro/400.css"), import("@fontsource/be-vietnam-pro/600.css"), import("@fontsource/be-vietnam-pro/700.css")]) },
  { id: "inter", name: "Inter", family: '"Inter", sans-serif', load: () => Promise.all([import("@fontsource/inter/400.css"), import("@fontsource/inter/600.css"), import("@fontsource/inter/700.css")]) },
  { id: "noto-sans", name: "Noto Sans", family: '"Noto Sans", sans-serif', load: () => Promise.all([import("@fontsource/noto-sans/400.css"), import("@fontsource/noto-sans/600.css"), import("@fontsource/noto-sans/700.css")]) },
  { id: "lexend", name: "Lexend", family: '"Lexend", sans-serif', load: () => Promise.all([import("@fontsource/lexend/400.css"), import("@fontsource/lexend/600.css"), import("@fontsource/lexend/700.css")]) },
  { id: "nunito", name: "Nunito", family: '"Nunito", sans-serif', load: () => Promise.all([import("@fontsource/nunito/400.css"), import("@fontsource/nunito/600.css"), import("@fontsource/nunito/700.css")]) },
  { id: "noto-serif", name: "Noto Serif", family: '"Noto Serif", serif', load: () => Promise.all([import("@fontsource/noto-serif/400.css"), import("@fontsource/noto-serif/600.css"), import("@fontsource/noto-serif/700.css")]) },
  { id: "literata", name: "Literata", family: '"Literata", serif', load: () => Promise.all([import("@fontsource/literata/400.css"), import("@fontsource/literata/600.css"), import("@fontsource/literata/700.css")]) },
  { id: "lora", name: "Lora", family: '"Lora", serif', load: () => Promise.all([import("@fontsource/lora/400.css"), import("@fontsource/lora/600.css"), import("@fontsource/lora/700.css")]) },
  { id: "merriweather", name: "Merriweather", family: '"Merriweather", serif', load: () => Promise.all([import("@fontsource/merriweather/400.css"), import("@fontsource/merriweather/600.css"), import("@fontsource/merriweather/700.css")]) },
];

export interface UiSettings {
  bg: string; // preset id | "image"
  dim: number; // 0..0.8 lớp tối phủ lên ảnh
  theme: "auto" | "light" | "dark";
  v: number; // version, đổi khi ảnh mới -> bust cache
  font: string; // phông giao diện (FONTS id)
  readFont: string; // phông khi đọc md: "" = giống giao diện
  readSize: number; // cỡ chữ khi đọc (px)
}

const SETTINGS = ".nextkloud/settings.json";
const IMAGE = ".nextkloud/background.webp";
const DEFAULT: UiSettings = { bg: "cream", dim: 0.2, v: 0, theme: "auto", font: "system", readFont: "", readSize: 15 };

function cached(): UiSettings {
  try {
    return { ...DEFAULT, ...JSON.parse(localStorage.getItem("nk-ui") || "{}") };
  } catch {
    return DEFAULT;
  }
}

const [ui, setUi] = createSignal<UiSettings>(cached());
// Áp theme + phông ngay khi load (trước đăng nhập), tránh nháy.
if (cached().theme !== "auto") document.documentElement.dataset.theme = cached().theme;
applyFonts(cached());

function applyFonts(s: UiSettings) {
  const f = (id: string) => FONTS.find((x) => x.id === id) ?? FONTS[0];
  const ui = f(s.font);
  const rd = s.readFont ? f(s.readFont) : ui;
  ui.load?.().catch(() => {});
  rd.load?.().catch(() => {});
  const st = document.documentElement.style;
  st.setProperty("--font-ui", ui.family);
  st.setProperty("--font-read", rd.family);
  st.setProperty("--read-size", `${s.readSize}px`);
}
const [imageUrl, setImageUrl] = createSignal<string | null>(null);
export { ui };

export function wallCss(): string {
  const s = ui();
  if (s.bg === "image" && imageUrl()) return `center / cover no-repeat url("${imageUrl()}"), #f5f0e8`;
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
  applyFonts(s);
  if (s.theme === "auto") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = s.theme;
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

export const setTheme = (theme: UiSettings["theme"]) => save({ ...ui(), theme });
export const setPreset = (id: string) => save({ ...ui(), bg: id });
/** Kéo slider: chỉ preview; thả tay mới lưu (tránh upload mỗi pixel). */
export const previewDim = (dim: number) => setUi({ ...ui(), dim });
export const setDim = (dim: number) => save({ ...ui(), dim });
export const setFonts = (f: Partial<Pick<UiSettings, "font" | "readFont" | "readSize">>) => save({ ...ui(), ...f });
export const previewReadSize = (readSize: number) => {
  setUi({ ...ui(), readSize });
  applyFonts(ui());
};

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
