import { For, Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";
import * as pdfjs from "pdfjs-dist";
import PdfWorker from "pdfjs-dist/build/pdf.worker.min.mjs?worker";
import { api } from "../api";
import { fmtNum, t } from "../i18n";
import { Icon } from "../icons";
import { ReaderLayout, STATE_UI, type TocItem, createReader, topState } from "../annot";

pdfjs.GlobalWorkerOptions.workerPort = new PdfWorker();

type Layout = "single" | "scroll" | "spread" | "spread-scroll";
const LAYOUTS: Layout[] = ["single", "scroll", "spread", "spread-scroll"];
const layoutLabel = (id: Layout) => t(`vw.pdfMode${id === "single" ? "Single" : id === "scroll" ? "Scroll" : id === "spread" ? "Spread" : "SpreadScroll"}`);
const layoutTitle = (id: Layout) => t(`vw.pdfT${id === "single" ? "Single" : id === "scroll" ? "Scroll" : id === "spread" ? "Spread" : "SpreadScroll"}`);
const savedLayout = (): Layout => {
  try {
    const v = localStorage.getItem("nk-pdf-layout") as Layout | null;
    return v && (LAYOUTS as string[]).includes(v) ? v : "scroll";
  } catch {
    return "scroll";
  }
};

/** Màu đọc đỡ mỏi mắt: đổi nền + chữ theo độ sáng, giữ nguyên vùng ảnh. */
type Tone = "normal" | "sepia" | "dark";
const TONES: { id: Tone; bg?: [number, number, number]; fg?: [number, number, number] }[] = [
  { id: "normal" },
  { id: "sepia", bg: [245, 225, 186], fg: [52, 36, 20] },
  { id: "dark", bg: [20, 17, 15], fg: [236, 176, 108] },
];
const toneLabel = (id: Tone) => t(`vw.tone${id === "normal" ? "Normal" : id === "sepia" ? "Sepia" : "Dark"}`);
const savedTone = (): Tone => {
  try {
    const v = localStorage.getItem("nk-pdf-tone") as Tone | null;
    return v && TONES.some((x) => x.id === v) ? v : "normal";
  } catch {
    return "normal";
  }
};

const toneBg = (tone: Tone) => {
  const bg = TONES.find((x) => x.id === tone)?.bg;
  return bg ? `rgb(${bg.join(",")})` : undefined;
};

/** Đổi màu canvas: sáng -> bg, tối -> fg (nội suy theo độ sáng); bỏ qua vùng ảnh (toạ độ chuẩn hoá từ pdf.js). */
function recolor(canvas: HTMLCanvasElement, tone: Tone, imgs: ArrayLike<number> | null) {
  const tx = TONES.find((x) => x.id === tone);
  if (!tx?.bg || !tx.fg) return;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const W = canvas.width;
  const H = canvas.height;
  const mask = new Uint8Array(W * H);
  for (let i = 0; imgs && i + 5 < imgs.length; i += 6) {
    const xs = [imgs[i], imgs[i + 2], imgs[i + 4], imgs[i + 2] + imgs[i + 4] - imgs[i]];
    const ys = [imgs[i + 1], imgs[i + 3], imgs[i + 5], imgs[i + 3] + imgs[i + 5] - imgs[i + 1]];
    const x0 = Math.max(0, Math.floor(Math.min(...xs) * W));
    const x1 = Math.min(W, Math.ceil(Math.max(...xs) * W));
    const y0 = Math.max(0, Math.floor(Math.min(...ys) * H));
    const y1 = Math.min(H, Math.ceil(Math.max(...ys) * H));
    for (let y = y0; y < y1; y++) mask.fill(1, y * W + x0, y * W + x1);
  }
  const lut = new Uint8Array(256 * 3);
  for (let l = 0; l < 256; l++) for (let c = 0; c < 3; c++) lut[l * 3 + c] = Math.round(tx.fg[c] + ((tx.bg[c] - tx.fg[c]) * l) / 255);
  const img = ctx.getImageData(0, 0, W, H);
  const d = img.data;
  for (let p = 0, k = 0; p < mask.length; p++, k += 4) {
    if (mask[p]) continue;
    const l = (d[k] * 77 + d[k + 1] * 150 + d[k + 2] * 29) >> 8;
    d[k] = lut[l * 3];
    d[k + 1] = lut[l * 3 + 1];
    d[k + 2] = lut[l * 3 + 2];
  }
  ctx.putImageData(img, 0, 0);
}

/** 1 trang PDF: chỉ render khi cuộn tới gần (lazy), nét theo devicePixelRatio, render lại khi đổi zoom.
 *  Có lớp chữ trong suốt phía trên để chọn / copy chữ. */
function PdfPage(props: { doc: pdfjs.PDFDocumentProxy; n: number; scale: number; ratio: number; w: number; tone: Tone; onErr: (m: string) => void }) {
  let box!: HTMLDivElement;
  let canvas!: HTMLCanvasElement;
  let textBox!: HTMLDivElement;
  let text: pdfjs.TextLayer | null = null;
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
    const tone = props.tone;
    if (!seen()) return;
    try {
      const p = await props.doc.getPage(props.n);
      const vp = p.getViewport({ scale: sc });
      const dpr = Math.min(window.devicePixelRatio || 1, 2);
      job?.cancel();
      text?.cancel();
      setRatio(vp.height / vp.width);
      setWid(vp.width);
      // vẽ + đổi màu trên canvas ẩn rồi mới chép sang: không loé trang trắng trước khi đổi màu
      const off = document.createElement("canvas");
      off.width = Math.floor(vp.width * dpr);
      off.height = Math.floor(vp.height * dpr);
      const t = p.render({ canvas: off, viewport: vp, transform: dpr !== 1 ? [dpr, 0, 0, dpr, 0, 0] : undefined, recordImages: tone !== "normal" } as any);
      job = t;
      await t.promise;
      if (tone !== "normal") recolor(off, tone, (t as any).imageCoordinates ?? (p as any).imageCoordinates ?? null);
      canvas.width = off.width;
      canvas.height = off.height;
      canvas.getContext("2d")?.drawImage(off, 0, 0);
      // lớp chữ (chọn / copy)
      textBox.replaceChildren();
      box.style.setProperty("--total-scale-factor", String(sc));
      text = new pdfjs.TextLayer({ textContentSource: p.streamTextContent(), container: textBox, viewport: vp });
      await text.render();
    } catch (e: any) {
      if (e?.name !== "RenderingCancelledException" && e?.name !== "AbortException") props.onErr(String(e?.message ?? e));
    }
  });
  onCleanup(() => {
    job?.cancel();
    text?.cancel();
  });
  return (
    <div class="pdf-pg" ref={box} data-page={props.n} style={{ width: `${wid() ?? props.w * props.scale}px`, "aspect-ratio": `1 / ${ratio()}`, background: toneBg(props.tone) }}>
      <canvas ref={canvas} />
      <div class="textLayer" ref={textBox} />
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
  // "Full màn hình" = lấp đầy khung trình duyệt (giữ chrome/nút điều hướng), KHÔNG dùng Fullscreen API.
  const [wide, setWide] = createSignal(false);
  const [layout, setLayoutSig] = createSignal<Layout>(savedLayout());
  const [tone, setToneSig] = createSignal<Tone>(savedTone());
  const setTone = (t: Tone) => {
    setToneSig(t);
    try {
      localStorage.setItem("nk-pdf-tone", t);
    } catch {}
  };
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

  /** Nhảy tới trang n. Trang phía trên chưa render có chiều cao ước lượng -> giữ trang n ở đầu khung
   *  thêm ~1.5s trong lúc chúng đổi kích thước (thôi giữ khi người dùng tự cuộn). */
  let pin: { n: number; until: number } | null = null;
  const toPage = (n: number) => host?.querySelector<HTMLElement>(`[data-page="${n}"]`)?.scrollIntoView({ block: "start", behavior: "auto" });
  const scrollToPage = (n: number) =>
    queueMicrotask(() => {
      toPage(n);
      pin = { n, until: Date.now() + 1500 };
    });
  const unpin = () => (pin = null);
  const setLayout = (l: Layout) => {
    setLayoutSig(l);
    try {
      localStorage.setItem("nk-pdf-layout", l);
    } catch {}
    if (scrolling()) scrollToPage(page());
  };

  // Màn cảm ứng: cuộn xuống thì ẩn thanh công cụ, chạm 1 cái (không vuốt) để hiện/ẩn.
  const touchUi = window.matchMedia?.("(pointer: coarse)").matches ?? false;
  const [barHidden, setBarHidden] = createSignal(false);
  let lastTop = 0;

  // Cuộn liên tục: trang hiện tại = trang đầu tiên còn thấy dưới thanh công cụ.
  const onScroll = () => {
    const sc = wide() ? host : scroller; // lấp đầy khung thì chính host cuộn
    if (touchUi && sc) {
      if (sc.scrollTop > lastTop + 12) setBarHidden(true);
      lastTop = sc.scrollTop;
    }
    if (!scrolling()) return;
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
    for (const t of [scroller, host]) {
      t?.addEventListener("wheel", unpin, { passive: true });
      t?.addEventListener("touchstart", unpin, { passive: true });
    }
    const ro = new ResizeObserver(() => {
      if (pin && Date.now() < pin.until) toPage(pin.n);
      else pin = null;
    });
    ro.observe(host);
    onCleanup(() => ro.disconnect());
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && wide() && !(e.target as HTMLElement).closest("input, textarea, select")) setWide(false);
    };
    document.addEventListener("keydown", onKey);
    onCleanup(() => document.removeEventListener("keydown", onKey));
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
    try {
      task?.destroy();
    } catch {}
    task = null;
  });

  const jump = (anchor: string) => {
    const n = Math.min(+anchor.slice(1) || 1, pages() || 1);
    setPage(n);
    if (scrolling()) scrollToPage(n);
  };
  const go = (d: number) => jump(`p${Math.min(Math.max(1, (spread() ? shown()[0] : page()) + d * step()), pages() || 1)}`);
  const toggleWide = () => setWide((w) => !w);
  const zoom = (d: number) => setScale((s) => Math.min(3, Math.max(0.4, +(s + d).toFixed(1))));

  // Chụm 2 ngón / Ctrl(⌘)+lăn chuột: xem trước bằng CSS scale, thả tay mới render lại ở zoom mới.
  let gf = 1; // hệ số đang xem trước
  let gp = { x: 0, y: 0 }; // điểm giữ đứng yên (toạ độ màn hình)
  let commitT = 0;
  const pagesEl = () => Array.from(host.querySelectorAll<HTMLElement>(":scope > .pdf-list, :scope > .pdf-row"));
  const preview = (f: number, cx: number, cy: number) => {
    gf = Math.min(3 / scale(), Math.max(0.4 / scale(), f));
    gp = { x: cx, y: cy };
    for (const el of pagesEl()) {
      const r = el.getBoundingClientRect();
      el.style.transformOrigin = `${cx - r.left}px ${cy - r.top}px`;
      el.style.transform = `scale(${gf})`;
    }
  };
  const commit = () => {
    clearTimeout(commitT);
    if (gf === 1) return;
    const sc = wide() ? host : scroller;
    const r = sc?.getBoundingClientRect();
    const k = gf;
    gf = 1;
    for (const el of pagesEl()) el.style.transform = "";
    setScale(+(scale() * k).toFixed(2));
    // giữ điểm dưới ngón tay/con trỏ đứng yên sau khi các trang đổi kích thước
    if (sc && r) requestAnimationFrame(() => {
      sc.scrollTop = (sc.scrollTop + gp.y - r.top) * k - (gp.y - r.top);
      sc.scrollLeft = (sc.scrollLeft + gp.x - r.left) * k - (gp.x - r.left);
    });
  };
  onMount(() => {
    let d0 = 0;
    const dist = (e: TouchEvent) => Math.hypot(e.touches[0].clientX - e.touches[1].clientX, e.touches[0].clientY - e.touches[1].clientY);
    const ts = (e: TouchEvent) => {
      if (e.touches.length === 2) {
        e.preventDefault();
        d0 = dist(e) / gf;
      }
    };
    const tm = (e: TouchEvent) => {
      if (e.touches.length !== 2 || !d0) return;
      e.preventDefault();
      preview(dist(e) / d0, (e.touches[0].clientX + e.touches[1].clientX) / 2, (e.touches[0].clientY + e.touches[1].clientY) / 2);
    };
    let tap: { x: number; y: number; t: number } | null = null;
    const te = (e: TouchEvent) => {
      if (e.touches.length < 2 && d0) {
        d0 = 0;
        commit();
        tap = null;
      }
      const c = e.changedTouches[0];
      if (tap && e.touches.length === 0 && c && Date.now() - tap.t < 300 && Math.hypot(c.clientX - tap.x, c.clientY - tap.y) < 10) {
        const el = e.target as HTMLElement;
        if (!el.closest(".pdf-bar, button, input, select, a") && !window.getSelection()?.toString()) setBarHidden((h) => !h);
      }
      tap = null;
    };
    const ts1 = (e: TouchEvent) => {
      tap = e.touches.length === 1 ? { x: e.touches[0].clientX, y: e.touches[0].clientY, t: Date.now() } : null;
    };
    host.addEventListener("touchstart", ts1, { passive: true });
    const wh = (e: WheelEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      e.preventDefault();
      preview(gf * Math.exp(-e.deltaY * 0.004), e.clientX, e.clientY);
      clearTimeout(commitT);
      commitT = window.setTimeout(commit, 250);
    };
    const key = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || !["+", "=", "-", "0"].includes(e.key)) return;
      e.preventDefault();
      if (e.key === "0") setScale(1.4);
      else zoom(e.key === "-" ? -0.2 : 0.2);
    };
    const gs = (e: Event) => e.preventDefault(); // Safari: chặn zoom trang
    host.addEventListener("touchstart", ts, { passive: false });
    host.addEventListener("touchmove", tm, { passive: false });
    host.addEventListener("touchend", te);
    host.addEventListener("touchcancel", te);
    host.addEventListener("wheel", wh, { passive: false });
    host.addEventListener("gesturestart", gs);
    document.addEventListener("keydown", key);
    onCleanup(() => {
      clearTimeout(commitT);
      document.removeEventListener("keydown", key);
    });
  });
  const onErr = (m: string) => !cancelled && setErr(m);

  return (
    <ReaderLayout r={reader} toc={toc()} current={toc().filter((t) => +t.anchor.slice(1) <= page()).pop()?.anchor ?? ""} onJump={jump} anchorOf={() => here()}>
    <div class="pdf-wrap" ref={host} classList={{ "vw-wide": wide() }}>
      <div class="pdf-bar" classList={{ hide: barHidden() }}>
        <button class="icon-btn" onClick={() => go(-1)} disabled={page() <= 1} title={t("vw.prevPage")}><Icon name="back" /></button>
        <span class="pdf-num muted sm">
          <input
            type="number"
            min="1"
            max={pages() || 1}
            value={page()}
            title={t("vw.gotoPh")}
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
        <button class="icon-btn" onClick={() => go(1)} disabled={(spread() && !scrolling() ? shown()[shown().length - 1] : page()) >= pages()} title={t("vw.nextPage")}><Icon name="next" /></button>
        <button class="icon-btn" classList={{ "bm-on": !!bm() }} onClick={() => reader.toggleBookmark(here(), t("vw.pageOf", { p: page() }))} title={bm() ? t("vw.bmDel") : t("vw.bmAdd")}>
          <Icon name="bookmark" />
        </button>
        <button class="icon-btn" onClick={() => reader.startNote(here(), "")} title={t("vw.notePage")}><Icon name="note" /></button>
        <Show when={st()}>
          {(s) => (
            <button class={`mk mk-note st-${s()}`} onClick={() => reader.showNotes(pageAnns().find((a) => a.kind === "note")!.id)} title={t("nt.viewNotes")}>
              <Icon name={STATE_UI[s()].icon} size={12} />
            </button>
          )}
        </Show>
        <span class="grow" />
        <select class="pdf-layout" value={tone()} onChange={(e) => setTone(e.currentTarget.value as Tone)} title={t("vw.toneTip")}>
          <For each={TONES}>{(o) => <option value={o.id}>{toneLabel(o.id)}</option>}</For>
        </select>
        <select class="pdf-layout" value={layout()} onChange={(e) => setLayout(e.currentTarget.value as Layout)} title={t("vw.layoutTip")}>
          <For each={LAYOUTS}>{(l) => <option value={l} title={layoutTitle(l)}>{layoutLabel(l)}</option>}</For>
        </select>
        <button class="icon-btn" onClick={() => zoom(-0.2)} title={t("vw.zout")}>−</button>
        <span class="pdf-num muted sm">
          <input
            type="number"
            min="40"
            max="300"
            step="10"
            value={Math.round(scale() * 100)}
            title={t("vw.zoomPh")}
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
        <button class="icon-btn" onClick={() => zoom(0.2)} title={t("vw.zin")}>+</button>
        <button class="icon-btn" onClick={toggleWide} title={wide() ? t("vw.fsExit") : t("vw.fsEnter")}>
          <Icon name={wide() ? "compress" : "expand"} size={17} />
        </button>
      </div>
      <Show when={err()}><p class="err">{err()}</p></Show>
      <Show when={doc()}>
        {(d) => (
          <Show
            when={scrolling()}
            fallback={
              <div class="pdf-row" classList={{ two: spread() }}>
                <For each={shown()}>{(n) => <PdfPage doc={d()} n={n} scale={scale()} ratio={ratio()} w={w1()} tone={tone()} onErr={onErr} />}</For>
              </div>
            }
          >
            <div class="pdf-list">
              <For each={spread() ? rows() : all().map((n) => [n])}>
                {(r) => (
                  <div class="pdf-row" classList={{ two: spread() }}>
                    <For each={r}>{(n) => <PdfPage doc={d()} n={n} scale={scale()} ratio={ratio()} w={w1()} tone={tone()} onErr={onErr} />}</For>
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
