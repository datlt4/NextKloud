import { Match, Show, Suspense, Switch, createEffect, createResource, createSignal, lazy, on, onCleanup } from "solid-js";
import { marked } from "marked";
import { api, setViewOwner, speak } from "../api";
import { ui } from "../background";
import { onEsc, inField } from "../esc";
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

/** owner: file của người khác chia sẻ -> chỉ xem (md render thuần, zip/drawio tải về); Office xem/sửa theo quyền share. */
export function ViewerOverlay(props: { path: string; owner?: string; onClose: () => void; onError?: (m: string) => void }) {
  const [cur, setCur] = createSignal(props.path);
  createEffect(on(() => props.path, (p) => setCur(p)));
  setViewOwner(props.owner ?? null);
  onCleanup(() => setViewOwner(null));
  const name = () => cur().split("/").pop() ?? cur();
  const SHARED_OK = ["markdown", "text", "image", "video", "audio", "pdf", "epub", "office"];
  const kind = () => {
    const k = viewerKind(name());
    return props.owner && !SHARED_OK.includes(k) ? "other" : k;
  };
  const [text, { refetch }] = createResource(() => (kind() === "markdown" ? cur() : null), (p) => api.readText(p));
  const [anchor, setAnchor] = createSignal<string | null>(null);
  // [[link]] trong note: tương đối theo thư mục note, thử thêm .md (giống openWikiLink ở App)
  const openLink = async (target: string, a?: string) => {
    const tt = target.trim();
    if (!tt) return a && setAnchor(a);
    const dir = cur().includes("/") ? cur().slice(0, cur().lastIndexOf("/")) : "";
    const norm = (segs: string[]) => {
      const out: string[] = [];
      for (const x of segs) x === ".." ? out.pop() : x && x !== "." && out.push(x);
      return out.join("/");
    };
    const base = tt.startsWith("/") ? norm(tt.slice(1).split("/")) : norm([...dir.split("/"), ...tt.split("/")]);
    const cands = /\.[a-zA-Z0-9]{1,5}$/.test(base) ? [base] : [base, `${base}.md`];
    for (const c of cands) {
      try {
        await api.exists(c);
        setAnchor(a ?? null);
        return setCur(c);
      } catch {
        /* thử ứng viên kế */
      }
    }
    props.onError?.(`${t("c.notFound")}: ${tt}`);
  };
  // md được chia sẻ: đọc lần lượt từng khối đã render (bấm lần nữa để dừng)
  const [reading, setReading] = createSignal(false);
  let mdEl: HTMLDivElement | undefined;
  let voice: HTMLAudioElement | null = null;
  const stopRead = () => {
    setReading(false);
    voice?.pause();
    voice = null;
  };
  onCleanup(stopRead);
  const readAll = async () => {
    if (reading()) return stopRead();
    setReading(true);
    try {
      for (const el of Array.from(mdEl?.children ?? [])) {
        const txt = el.textContent?.trim();
        if (!reading()) break;
        if (!txt) continue;
        const url = await speak(txt);
        if (!reading()) break;
        await new Promise<void>((res) => {
          voice = new Audio(url);
          voice.playbackRate = ui().ttsRate;
          voice.onended = voice.onerror = voice.onpause = () => res();
          voice.play().catch(() => res());
        });
      }
    } catch (e) {
      props.onError?.(String((e as Error)?.message ?? e));
    } finally {
      stopRead();
    }
  };
  const close = () => props.onClose();
  onEsc((e) => !inField(e) && (close(), true));
  const downloadOther = async () => {
    try {
      const a = document.createElement("a");
      a.href = props.owner ? await api.sharedDirectUrl(props.owner, cur()) : await api.directDownloadUrl(cur());
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
                <Match when={kind() === "image"}><ImageView files={[{ path: cur(), name: name() }]} index={0} startZoom /></Match>
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
              <Show when={!props.owner} fallback={
                <div class="md-shared">
                  <div class="md-bar">
                    <button onClick={() => void readAll()} title={reading() ? t("vw.mdStopRead") : undefined}>
                      {reading() ? t("vw.mdReading") : (<><Icon name="listen" size={15} /> {t("vw.mdReadAll")}</>)}
                    </button>
                  </div>
                  <div class="md-content" ref={mdEl} innerHTML={marked.parse(text()!) as string} />
                </div>
              }>
                <MarkdownView path={cur()} text={text()!} anchor={anchor()} onOpenLink={(l, a) => void openLink(l, a)} onError={props.onError} onSaved={() => refetch()} />
              </Show>
            </Show>
          </Suspense>
        </div>
      </div>
    </div>
  );
}
