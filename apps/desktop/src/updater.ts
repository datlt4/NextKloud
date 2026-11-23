import { createSignal } from "solid-js";
import { isTauri } from "./api";
import { t } from "./i18n";

export type UpdateInfo = { version: string; body?: string | null; apply: (onProgress: (pct: number) => void) => Promise<void> };

const [info, setInfo] = createSignal<UpdateInfo | null>(null);
const [checking, setChecking] = createSignal(false);
const [progress, setProgress] = createSignal<number | null>(null);
const [error, setError] = createSignal("");
const [done, setDone] = createSignal(false);
export const updateState = { info, checking, progress, error, done };

/** Kiểm tra bản mới (chỉ chạy trong app Tauri; web trả về null). */
export async function checkUpdate(silent = false): Promise<UpdateInfo | null> {
  // updater chỉ có trên desktop; app iOS/Android cập nhật qua bản cài mới
  if (!isTauri() || /iPhone|iPad|Android/i.test(navigator.userAgent)) return null;
  if (checking()) return info();
  setChecking(true);
  if (!silent) setError("");
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    const u = await check();
    if (!u) {
      if (!silent) setError(t("up.latest"));
      setInfo(null);
      return null;
    }
    const ver = u.version;
    let downloaded = 0;
    const inf: UpdateInfo = {
      version: ver,
      body: u.body,
      apply: async (onProgress) => {
        await u.downloadAndInstall((e) => {
          if (e.event === "Started") onProgress(0);
          else if (e.event === "Progress") {
            const c = (e.data as { chunkLength: number }).chunkLength;
            downloaded += c;
            const total = (u as unknown as { contentLength?: number }).contentLength ?? 0;
            onProgress(total > 0 ? Math.round((downloaded / total) * 100) : -1);
          } else if (e.event === "Finished") onProgress(100);
        });
      },
    };
    setInfo(inf);
    return inf;
  } catch (e) {
    if (!silent) setError(String((e as Error)?.message ?? e));
    return null;
  } finally {
    setChecking(false);
  }
}

/** Tải + cài bản mới, xong báo done (người dùng bấm khởi động lại). */
export async function installUpdate() {
  const u = info();
  if (!u) return;
  setError("");
  setProgress(0);
  try {
    await u.apply((p) => setProgress(p));
    setDone(true);
  } catch (e) {
    setError(String((e as Error)?.message ?? e));
  } finally {
    if (!done()) setProgress(null);
  }
}

export async function restartApp() {
  const { relaunch } = await import("@tauri-apps/plugin-process");
  await relaunch();
}
