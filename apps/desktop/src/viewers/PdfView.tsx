import { For, Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";
import * as pdfjs from "pdfjs-dist";
import PdfWorker from "pdfjs-dist/build/pdf.worker.min.mjs?worker";
import { api } from "../api";
import { Icon } from "../icons";
import { ReaderLayout, STATE_UI, type TocItem, createReader, topState } from "../annot";

pdfjs.GlobalWorkerOptions.workerPort = new PdfWorker();

type Layout = "single" | "scroll" | "spread" | "spread-scroll";
const LAYOUTS: { id: Layout; label: string; title: string }[] = [
  { id: "single", label: "1 trang", title: "Từng trang" },
  { id: "scroll", label: "Liên tục", title: "Cuộn liên tục" },
  { id: "spread", label: "2 trang", title: "2 trang cạnh nhau" },
  { id: "spread-scroll", label: "2 trang liên tục", title: "2 trang cạnh nhau, cuộn liên tục" },
];
const savedLayout = (): Layout => {
  try {
    const v = localStorage.getItem("nk-pdf-layout") as Layout | null;
    return v && LAYOUTS.some((l) => l.id === v) ? v : "scroll";
  } catch {
    return "scroll";
  }
};

/** 1 trang PDF: chỉ render khi cuộn tới gần (lazy), nét theo devicePixelRatio, render lại khi đổi zoom. */
function PdfPage(props: { doc: pdfjs.PDFDocumentProxy; n: number; scale: number; ratio: number; w: number; onErr: (m: string) => void }) {
  let box!: HTMLDivElement;
  let canvas!: HTMLCanvasElement;
  const [seen, setSeen] = createSignal(false);
  const [ratio, setRatio] = createSignal(props.ratio);
  const [wid, setWid] = createSignal<number | null>(null); // bề rộng thật sau khi đọc trang
  let job: { cancel: () => void } | null = null;
  onMount(() => {
    const io = new IntersectionObserver((es) => es.some((e) => e.isIntersecting) && setSeen(true), { rootMargin: "800px 0px" });
    io.observe(box);
    onCleanup(() => io.disconnect());
  });
  createEffect(async () => {
    const sc = props.scale;
    if (!seen()) return;
    try {
      const p = await props.doc.getPage(props.n);
      const vp = p.getViewport({ scale: sc });
      const dpr = Math.min(window.devicePixelRatio || 1, 2);
      job?.cancel();
      setRatio(vp.height / vp.width);
      setWid(vp.width);
      canvas.width = Math.floor(vp.width * dpr);
      canvas.height = Math.floor(vp.height * dpr);
      const t = p.render({ canvas, viewport: vp, transform: dpr !== 1 ? [dpr, 0, 0, dpr, 0, 0] : undefined });
      job = t;
      await t.promise;
    } catch (e: any) {
      if (e?.name !== "RenderingCancelledException") props.onErr(String(e?.message ?? e));
    }
  });
  onCleanup(() => job?.cancel());
  return (
    <div class="pdf-pg" ref={box} data-page={props.n} style={{ width: `${wid() ?? props.w * props.scale}px`, "aspect-ratio": `1 / ${ratio()}` }}>
      <canvas ref={canvas} />
    </div>
  );
}

/** PDF: 1 trang / cuộn liên tục / 2 trang / 2 trang liên tục + zoom. Lazy import nên không phình bundle chính. */
export default function PdfView(props: { path: string; onError?: (m: string) => void; mark?: string }) {
  const reader = createReader(() => props.path, (m) => (props.onError ?? console.error)(m));
  const [toc, setToc] = createSignal<TocItem[]>([]);
  const here = () => `p${page()}`;
  const pageAnns = () => reader.items().filter((a) => a.anchor === here());
  const bm = () => pageAnns().find((a) => a.kind === "bookmark");
  const st = () => topState(pageAnns());
  const [err, setErr] = createSignal("");
  const [doc, setDoc] = createSignal<pdfjs.PDFDocumentProxy | null>(null);
  const [pages, setPages] = createSignal(0);
  const [page, setPage] = createSignal(1);
  const [scale, setScale] = createSignal(1.4);
  const [ratio, setRatio] = createSignal(1.294); // tỉ lệ cao/rộng trang đầu, dùng cho khung chờ
  const [w1, setW1] = createSignal(612); // bề rộng trang đầu ở zoom 100%
  const [fs, setFs] = createSignal(false);
  const [layout, setLayoutSig] = createSignal<Layout>(savedLayout());
  const spread = () => layout() === "spread" || layout() === "spread-scroll";
  const scrolling = () => layout() === "scroll" || layout() === "spread-scroll";
  const step = () => (spread() ? 2 : 1);
  let task: pdfjs.PDFDocumentLoadingTask | null = null;
  let cancelled = false;
  let host!: HTMLDivElement;
  let scroller: HTMLElement | null = null;

  /** Trang (hoặc cặp trang) đang hiện ở chế độ không cuộn. */
  const shown = () => {
    if (!spread()) return [page()];
    const a = page() % 2 ? page() : page() - 1;
    return a + 1 <= pages() ? [a, a + 1] : [a];
  };
  const all = () => Array.from({ length: pages() }, (_, i) => i + 1);
  const rows = () => {
    const out: number[][] = [];
    for (let i = 1; i <= pages(); i += step()) out.push(step() === 2 && i + 1 <= pages() ? [i, i + 1] : [i]);
    return out;
  };

  const scrollToPage = (n: number, smooth = false) =>
    queueMicrotask(() => host?.querySelector<HTMLElement>(`[data-page="${n}"]`)?.scrollIntoView({ block: "start", behavior: smooth ? "smooth" : "auto" }));
  const setLayout = (l: Layout) => {
    setLayoutSig(l);
    try {
      localStorage.setItem("nk-pdf-layout", l);
    } catch {}
    if (scrolling()) scrollToPage(page());
  };

  // Cuộn liên tục: trang hiện tại = trang đầu tiên còn thấy dưới thanh công cụ.
  const onScroll = () => {
    if (!scrolling()) return;
    const sc = document.fullscreenElement === host ? host : scroller; // toàn màn hình thì chính host cuộn
    const top = (sc?.getBoundingClientRect().top ?? 0) + 120;
    let cur = 1;
    for (const el of Array.from(host.querySelectorAll<HTMLElement>(".pdf-row > .pdf-pg:first-child"))) {
      if (el.getBoundingClientRect().top > top) break;
      cur = +el.dataset.page!;
    }
    setPage(cur);
  };

  onMount(async () => {
    scroller = host.closest(".viewer");
    scroller?.addEventListener("scroll", onScroll, { passive: true });
    host.addEventListener("scroll", onScroll, { passive: true });
    document.addEventListener("fullscreenchange", onFs);
    try {
      const buf = await (await api.readBlob(props.path)).arrayBuffer();
      if (cancelled) return;
      task = pdfjs.getDocument({ data: buf });
      const d = await task.promise;
      if (cancelled) return;
      const vp = (await d.getPage(1)).getViewport({ scale: 1 });
      setRatio(vp.height / vp.width);
      setW1(vp.width);
      setPages(d.numPages);
      setDoc(d);
      if (props.mark) jump(props.mark);
      loadOutline(d).then((t) => !cancelled && setToc(t)).catch(() => {});
    } catch (e: any) {
      if (!cancelled) setErr(String(e?.message ?? e));
    }
  });
  onCleanup(() => {
    cancelled = true;
    scroller?.removeEventListener("scroll", onScroll);
    document.removeEventListener("fullscreenchange", onFs);
    try {
      task?.destroy();
    } catch {}
    task = null;
  });

  const jump = (anchor: string) => {
    const n = Math.min(+anchor.slice(1) || 1, pages() || 1);
    setPage(n);
    if (scrolling()) scrollToPage(n, true);
  };
  const go = (d: number) => jump(`p${Math.min(Math.max(1, (spread() ? shown()[0] : page()) + d * step()), pages() || 1)}`);
  const toggleFs = async () => {
    try {
      if (document.fullscreenElement) await document.exitFullscreen();
      else await host.requestFullscreen();
    } catch {}
  };
  const onFs = () => !cancelled && setFs(!!document.fullscreenElement);
  const zoom = (d: number) => setScale((s) => Math.min(3, Math.max(0.4, +(s + d).toFixed(1))));
  const onErr = (m: string) => !cancelled && setErr(m);

  return (
    <ReaderLayout r={reader} toc={toc()} current={toc().filter((t) => +t.anchor.slice(1) <= page()).pop()?.anchor ?? ""} onJump={jump} anchorOf={() => here()}>
    <div class="pdf-wrap" ref={host}>
      <div class="pdf-bar">
        <button class="icon-btn" onClick={() => go(-1)} disabled={page() <= 1} title="Trang trước"><Icon name="back" /></button>
        <span class="pdf-num muted sm">
          <input
            type="number"
            min="1"
            max={pages() || 1}
            value={page()}
            title="Nhập số trang rồi Enter"
            onFocus={(e) => e.currentTarget.select()}
            onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
            onChange={(e) => {
              const n = Math.round(+e.currentTarget.value);
              if (n >= 1) jump(`p${n}`);
              e.currentTarget.value = String(page());
            }}
          />
          /{pages() || "…"}
        </span>
        <button class="icon-btn" onClick={() => go(1)} disabled={(spread() && !scrolling() ? shown()[shown().length - 1] : page()) >= pages()} title="Trang tiếp"><Icon name="next" /></button>
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
        <select class="pdf-layout" value={layout()} onChange={(e) => setLayout(e.currentTarget.value as Layout)} title="Chế độ xem">
          <For each={LAYOUTS}>{(l) => <option value={l.id} title={l.title}>{l.label}</option>}</For>
        </select>
        <button class="icon-btn" onClick={() => zoom(-0.2)} title="Thu nhỏ">−</button>
        <span class="pdf-num muted sm">
          <input
            type="number"
            min="40"
            max="300"
            step="10"
            value={Math.round(scale() * 100)}
            title="Nhập % zoom rồi Enter"
            onFocus={(e) => e.currentTarget.select()}
            onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
            onChange={(e) => {
              const v = +e.currentTarget.value;
              if (v > 0) setScale(Math.min(3, Math.max(0.4, v / 100)));
              e.currentTarget.value = String(Math.round(scale() * 100));
            }}
          />
          %
        </span>
        <button class="icon-btn" onClick={() => zoom(0.2)} title="Phóng to">+</button>
        <button class="icon-btn" onClick={toggleFs} title={fs() ? "Thoát toàn màn hình (Esc)" : "Xem toàn màn hình"}>
          <Icon name="expand" size={17} />
        </button>
      </div>
      <Show when={err()}><p class="err">{err()}</p></Show>
      <Show when={doc()}>
        {(d) => (
          <Show
            when={scrolling()}
            fallback={
              <div class="pdf-row" classList={{ two: spread() }}>
                <For each={shown()}>{(n) => <PdfPage doc={d()} n={n} scale={scale()} ratio={ratio()} w={w1()} onErr={onErr} />}</For>
              </div>
            }
          >
            <div class="pdf-list">
              <For each={spread() ? rows() : all().map((n) => [n])}>
                {(r) => (
                  <div class="pdf-row" classList={{ two: spread() }}>
                    <For each={r}>{(n) => <PdfPage doc={d()} n={n} scale={scale()} ratio={ratio()} w={w1()} onErr={onErr} />}</For>
                  </div>
                )}
              </For>
            </div>
          </Show>
        )}
      </Show>
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
