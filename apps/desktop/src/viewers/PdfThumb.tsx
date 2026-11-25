import { Show, createSignal, onCleanup, onMount } from "solid-js";
import * as pdfjs from "pdfjs-dist";
import PdfWorker from "pdfjs-dist/build/pdf.worker.min.mjs?worker";

if (!pdfjs.GlobalWorkerOptions.workerPort) pdfjs.GlobalWorkerOptions.workerPort = new PdfWorker();

/** Ảnh trang 1 của pdf, chỉ vẽ khi cuộn tới; pdf.js đọc theo Range nên không tải cả file. */
export default function PdfThumb(props: { url: string; size: number; class?: string; onFail?: () => void }) {
  const [fail, setFail] = createSignal(false);
  let canvas!: HTMLCanvasElement;
  let task: pdfjs.PDFDocumentLoadingTask | null = null;
  let dead = false;
  const draw = async () => {
    try {
      task = pdfjs.getDocument({ url: props.url, disableAutoFetch: true, disableStream: true, rangeChunkSize: 128 * 1024 });
      const page = await (await task.promise).getPage(1);
      if (dead) return;
      const v1 = page.getViewport({ scale: 1 });
      const vp = page.getViewport({ scale: (props.size * (window.devicePixelRatio || 1)) / Math.max(v1.width, v1.height) });
      canvas.width = Math.ceil(vp.width);
      canvas.height = Math.ceil(vp.height);
      await page.render({ canvas, viewport: vp } as any).promise;
    } catch {
      if (!dead) (setFail(true), props.onFail?.());
    } finally {
      void task?.destroy();
      task = null;
    }
  };
  onMount(() => {
    const io = new IntersectionObserver((es) => {
      if (es.some((e) => e.isIntersecting)) {
        io.disconnect();
        void draw();
      }
    });
    io.observe(canvas);
    onCleanup(() => io.disconnect());
  });
  onCleanup(() => {
    dead = true;
    void task?.destroy();
  });
  return (
    <Show when={!fail()}>
      <canvas ref={canvas} class={props.class} />
    </Show>
  );
}
