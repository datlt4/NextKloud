import { Match, Show, Suspense, Switch, createEffect, createResource, createSignal, lazy, on, onCleanup } from "solid-js";
import { api } from "../api";
import { t } from "../i18n";
import { Icon } from "../icons";
import { viewerKind } from "./kinds";
// Mở file ngay tại trang hiện tại (Deck/Tracker) thay vì điều hướng sang All Files.
const TextView = lazy(() => import("./TextView"));
const ImageView = lazy(() => import("./ImageView"));
const VideoView = lazy(() => import("./VideoView"));
const PdfView = lazy(() => import("./PdfView"));
const ZipView = lazy(() => import("./ZipView"));
const EpubView = lazy(() => import("./EpubView"));
const DrawioView = lazy(() => import("./DrawioView"));
const ExcalidrawView = lazy(() => import("./ExcalidrawView"));
const OfficeView = lazy(() => import("./OfficeView"));
const MarkdownView = lazy(() => import("./MarkdownView"));

export function ViewerOverlay(props: { path: string; onClose: () => void; onError?: (m: string) => void }) {
  const [cur, setCur] = createSignal(props.path);
  createEffect(on(() => props.path, (p) => setCur(p)));
  const name = () => cur().split("/").pop() ?? cur();
  const kind = () => viewerKind(name());
  const [text, { refetch }] = createResource(() => (kind() === "markdown" ? cur() : null), (p) => api.readText(p));
  const close = () => props.onClose();
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape" && !(e.target as HTMLElement).closest("textarea, input")) close();
  };
  document.addEventListener("keydown", onKey);
  onCleanup(() => document.removeEventListener("keydown", onKey));
  const downloadOther = async () => {
    try {
      const a = document.createElement("a");
      a.href = await api.directDownloadUrl(cur());
      a.download = name();
      a.click();
      close();
    } catch (e) {
      props.onError?.(String((e as Error)?.message ?? e));
    }
  };
  return (
    <div class="viewer-bg" onClick={close}>
      <div class="viewer-sheet glass" onClick={(e) => e.stopPropagation()}>
        <div class="viewer-bar">
          <button class="btn ghost sm" onClick={close}><Icon name="back" size={15} /> {t("c.back")}</button>
          <b class="fname" title={cur()}>{name()}</b>
          <span class="grow" />
          <button class="icon-btn" onClick={close} title={t("c.close")}><Icon name="close" /></button>
        </div>
        <div class="viewer">
          <Suspense fallback={<p class="muted pad">{t("c.opening")}</p>}>
            <Show when={kind() === "markdown" && text() !== undefined} fallback={
              <Switch fallback={<p class="muted pad">{t("c.loading")}</p>}>
                <Match when={kind() === "text"}><TextView path={cur()} /></Match>
                <Match when={kind() === "image"}><ImageView files={[{ path: cur(), name: name() }]} index={0} /></Match>
                <Match when={kind() === "video" || kind() === "audio"}><VideoView path={cur()} /></Match>
                <Match when={kind() === "pdf"}><PdfView path={cur()} onError={props.onError} /></Match>
                <Match when={kind() === "zip"}><ZipView path={cur()} /></Match>
                <Match when={kind() === "epub"}><EpubView path={cur()} /></Match>
                <Match when={kind() === "drawio"}><DrawioView path={cur()} /></Match>
                <Match when={kind() === "excalidraw"}><ExcalidrawView path={cur()} /></Match>
                <Match when={kind() === "office"}><OfficeView path={cur()} /></Match>
                <Match when={kind() === "other"}>
                  <div class="empty"><Icon name="file" size={48} /><p class="muted">{name()}</p>
                    <button class="btn primary" onClick={downloadOther}><Icon name="download" /> {t("f.download")}</button>
                  </div>
                </Match>
              </Switch>
            }>
              <MarkdownView path={cur()} text={text()!} onOpenLink={(l) => setCur(l.replace(/^\//, ""))} onError={props.onError} onSaved={() => refetch()} />
            </Show>
          </Suspense>
        </div>
      </div>
    </div>
  );
}
