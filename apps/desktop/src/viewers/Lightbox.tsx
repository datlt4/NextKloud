import { createSignal, onCleanup, onMount } from "solid-js";
import { Icon } from "../icons";

/** Xem ảnh toàn màn hình: lăn chuột / nút / chụm 2 ngón để zoom, kéo để di chuyển, Esc hoặc bấm nền để đóng. */
export default function Lightbox(props: { src: string; alt?: string; onClose: () => void }) {
  const [s, setS] = createSignal(1);
  const [x, setX] = createSignal(0);
  const [y, setY] = createSignal(0);
  let box!: HTMLDivElement;
  let img!: HTMLImageElement;
  let fit = 1;
  const clamp = (v: number) => Math.min(8, Math.max(0.1, v));
  /** Zoom quanh điểm (cx, cy) trên màn hình, giữ điểm đó đứng yên. */
  const zoomAt = (k: number, cx = innerWidth / 2, cy = innerHeight / 2) => {
    const ns = clamp(s() * k);
    const r = ns / s();
    setX(cx - (cx - x()) * r);
    setY(cy - (cy - y()) * r);
    setS(ns);
  };
  const reset = (scale: number) => {
    setS(scale);
    setX((innerWidth - img.naturalWidth * scale) / 2);
    setY((innerHeight - img.naturalHeight * scale) / 2);
  };
  const onLoad = () => {
    fit = Math.min(1, (innerWidth - 40) / img.naturalWidth, (innerHeight - 90) / img.naturalHeight);
    reset(fit);
  };
  const pts = new Map<number, { x: number; y: number }>();
  let moved = false;
  const down = (e: PointerEvent) => {
    if ((e.target as HTMLElement).closest("button")) return;
    box.setPointerCapture(e.pointerId);
    pts.set(e.pointerId, { x: e.clientX, y: e.clientY });
    moved = false;
  };
  const move = (e: PointerEvent) => {
    const p = pts.get(e.pointerId);
    if (!p) return;
    if (pts.size === 2) {
      const [a, b] = [...pts.entries()];
      const other = a[0] === e.pointerId ? b[1] : a[1];
      const d0 = Math.hypot(p.x - other.x, p.y - other.y);
      const d1 = Math.hypot(e.clientX - other.x, e.clientY - other.y);
      if (d0 > 0) zoomAt(d1 / d0, (e.clientX + other.x) / 2, (e.clientY + other.y) / 2);
    } else {
      setX(x() + e.clientX - p.x);
      setY(y() + e.clientY - p.y);
    }
    if (Math.hypot(e.clientX - p.x, e.clientY - p.y) > 2) moved = true;
    pts.set(e.pointerId, { x: e.clientX, y: e.clientY });
  };
  const up = (e: PointerEvent) => {
    pts.delete(e.pointerId);
    // bấm (không kéo) ra ngoài ảnh thì đóng
    if (!moved && pts.size === 0 && e.target === box) props.onClose();
  };
  const key = (e: KeyboardEvent) => {
    if (e.key === "Escape") props.onClose();
    else if (e.key === "+" || e.key === "=") zoomAt(1.25);
    else if (e.key === "-") zoomAt(0.8);
    else if (e.key === "0") reset(fit);
  };
  onMount(() => {
    document.addEventListener("keydown", key);
    // wheel cần passive:false để chặn cuộn trang
    const wheel = (e: WheelEvent) => {
      e.preventDefault();
      zoomAt(Math.exp(-e.deltaY * (e.ctrlKey ? 0.01 : 0.0015)), e.clientX, e.clientY);
    };
    box.addEventListener("wheel", wheel, { passive: false });
    onCleanup(() => {
      document.removeEventListener("keydown", key);
      box.removeEventListener("wheel", wheel);
    });
  });
  return (
    <div class="lightbox" ref={box} onPointerDown={down} onPointerMove={move} onPointerUp={up} onPointerCancel={up}>
      <img
        ref={img}
        src={props.src}
        alt={props.alt ?? ""}
        draggable={false}
        onLoad={onLoad}
        onDblClick={(e) => (s() < fit * 1.5 ? zoomAt(2.5 / s() * fit, e.clientX, e.clientY) : reset(fit))}
        style={{ transform: `translate(${x()}px, ${y()}px) scale(${s()})` }}
      />
      <div class="lb-bar">
        <button onClick={() => zoomAt(0.8)} title="Thu nhỏ (-)"><Icon name="minus" /></button>
        <button class="lb-pct" onClick={() => reset(fit)} title="Vừa màn hình (0)">{Math.round(s() * 100)}%</button>
        <button onClick={() => zoomAt(1.25)} title="Phóng to (+)"><Icon name="plus" /></button>
        <button onClick={() => reset(1)} title="Kích thước thật">1:1</button>
        <button onClick={props.onClose} title="Đóng (Esc)"><Icon name="close" /></button>
      </div>
    </div>
  );
}
