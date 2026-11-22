import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import * as pdfjs from "pdfjs-dist";
import PdfWorker from "pdfjs-dist/build/pdf.worker.min.mjs?worker";
import { api } from "../api";
import { Icon } from "../icons";
import { ReaderLayout, STATE_UI, type TocItem, createReader, topState } from "../annot";

pdfjs.GlobalWorkerOptions.workerPort = new PdfWorker();

/** PDF: render từng trang bằng pdfjs, nút trước/tiếp + zoom. Lazy import nên không phình bundle chính. */
export default function PdfView(props: { path: string; onError?: (m: string) => void }) {
  const reader = createReader(() => props.path, (m) => (props.onError ?? console.error)(m));
  const [toc, setToc] = createSignal<TocItem[]>([]);
  const here = () => `p${page()}`;
  const pageAnns = () => reader.items().filter((a) => a.anchor === here());
  const bm = () => pageAnns().find((a) => a.kind === "bookmark");
  const st = () => topState(pageAnns());
  const [err, setErr] = createSignal("");
  const [pages, setPages] = createSignal(0);
  const [page, setPage] = createSignal(1);
  const [scale, setScale] = createSignal(1.4);
  let canvas!: HTMLCanvasElement;
  let doc: pdfjs.PDFDocumentProxy | null = null;
  let task: pdfjs.PDFDocumentLoadingTask | null = null;
  let cancelled = false;

  const render = async () => {
    if (!doc || !canvas) return;
    try {
      const p = await doc.getPage(Math.min(Math.max(1, page()), pages() || 1));
      const vp = p.getViewport({ scale: scale() });
      canvas.width = vp.width;
      canvas.height = vp.height;
      await p.render({ canvas, viewport: vp }).promise;
    } catch (e: any) {
      if (!cancelled) setErr(String(e?.message ?? e));
    }
  };

  onMount(async () => {
    try {
      const buf = await (await api.readBlob(props.path)).arrayBuffer();
      if (cancelled) return;
      task = pdfjs.getDocument({ data: buf });
      doc = await task.promise;
      if (cancelled) return;
      setPages(doc.numPages);
      await render();
      loadOutline(doc).then((t) => !cancelled && setToc(t)).catch(() => {});
    } catch (e: any) {
      if (!cancelled) setErr(String(e?.message ?? e));
    }
  });
  onCleanup(() => {
    cancelled = true;
    try {
      task?.destroy();
    } catch {}
    task = doc = null;
  });

  const jump = (anchor: string) => {
    const n = +anchor.slice(1);
    if (n >= 1) {
      setPage(Math.min(n, pages() || 1));
      render();
    }
  };
  const go = (d: number) => {
    setPage((p) => Math.min(Math.max(1, p + d), pages() || 1));
    render();
  };
  const zoom = (d: number) => {
    setScale((s) => Math.min(3, Math.max(0.6, +(s + d).toFixed(1))));
    render();
  };

  return (
    <ReaderLayout r={reader} toc={toc()} current={toc().filter((t) => +t.anchor.slice(1) <= page()).pop()?.anchor ?? ""} onJump={jump} anchorOf={() => here()}>
    <div class="pdf-wrap">
      <div class="pdf-bar">
        <button class="icon-btn" onClick={() => go(-1)} disabled={page() <= 1} title="Trang trước"><Icon name="back" /></button>
        <span class="muted sm">{page()}/{pages() || "…"}</span>
        <button class="icon-btn" onClick={() => go(1)} disabled={page() >= pages()} title="Trang tiếp"><Icon name="next" /></button>
        <button class="icon-btn" classList={{ "bm-on": !!bm() }} onClick={() => reader.toggleBookmark(here(), `Trang ${page()}`)} title={bm() ? "Bỏ bookmark trang này" : "Bookmark trang này"}>
          <Icon name="bookmark" />
        </button>
        <button class="icon-btn" onClick={() => reader.startNote(here(), "")} title="Ghi chú cho trang này"><Icon name="note" /></button>
        <Show when={st()}>
          {(s) => (
            <button class={`mk mk-note st-${s()}`} onClick={() => reader.showNotes(pageAnns().find((a) => a.kind === "note")!.id)} title="Ghi chú của trang">
              <Icon name={STATE_UI[s()].icon} size={12} />
            </button>
          )}
        </Show>
        <span class="grow" />
        <button class="icon-btn" onClick={() => zoom(-0.2)} title="Thu nhỏ">−</button>
        <span class="muted sm">{Math.round(scale() * 100)}%</span>
        <button class="icon-btn" onClick={() => zoom(0.2)} title="Phóng to">+</button>
      </div>
      <Show when={err()}><p class="err">{err()}</p></Show>
      <div class="pdf-page"><canvas ref={canvas!} /></div>
    </div>
    </ReaderLayout>
  );
}

/** Mục lục (outline) của PDF -> danh sách phẳng có cấp, anchor theo số trang. */
async function loadOutline(doc: pdfjs.PDFDocumentProxy): Promise<TocItem[]> {
  const out: TocItem[] = [];
  const walk = async (items: any[] | null, level: number) => {
    for (const it of items ?? []) {
      try {
        const dest = typeof it.dest === "string" ? await doc.getDestination(it.dest) : it.dest;
        if (dest?.[0]) {
          const idx = typeof dest[0] === "number" ? dest[0] : await doc.getPageIndex(dest[0]);
          out.push({ anchor: `p${idx + 1}`, label: it.title || "…", level });
        }
      } catch {
        /* mục hỏng: bỏ qua */
      }
      await walk(it.items, level + 1);
    }
  };
  await walk(await doc.getOutline(), 1);
  return out;
}
