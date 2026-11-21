import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import * as pdfjs from "pdfjs-dist";
import PdfWorker from "pdfjs-dist/build/pdf.worker.min.mjs?worker";
import { api } from "../api";
import { Icon } from "../icons";

pdfjs.GlobalWorkerOptions.workerPort = new PdfWorker();

/** PDF: render từng trang bằng pdfjs, nút trước/tiếp + zoom. Lazy import nên không phình bundle chính. */
export default function PdfView(props: { path: string }) {
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

  const go = (d: number) => {
    setPage((p) => Math.min(Math.max(1, p + d), pages() || 1));
    render();
  };
  const zoom = (d: number) => {
    setScale((s) => Math.min(3, Math.max(0.6, +(s + d).toFixed(1))));
    render();
  };

  return (
    <div class="pdf-wrap">
      <div class="pdf-bar">
        <button class="icon-btn" onClick={() => go(-1)} disabled={page() <= 1} title="Trang trước"><Icon name="back" /></button>
        <span class="muted sm">{page()}/{pages() || "…"}</span>
        <button class="icon-btn" onClick={() => go(1)} disabled={page() >= pages()} title="Trang tiếp"><Icon name="next" /></button>
        <span class="grow" />
        <button class="icon-btn" onClick={() => zoom(-0.2)} title="Thu nhỏ">−</button>
        <span class="muted sm">{Math.round(scale() * 100)}%</span>
        <button class="icon-btn" onClick={() => zoom(0.2)} title="Phóng to">+</button>
      </div>
      <Show when={err()}><p class="err">{err()}</p></Show>
      <div class="pdf-page"><canvas ref={canvas!} /></div>
    </div>
  );
}
