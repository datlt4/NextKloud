import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { onEsc } from "../esc";
import { Icon } from "../icons";
import { t } from "../i18n";

/** Xem ảnh toàn màn hình: lăn chuột / nút / chụm 2 ngón để zoom, kéo để di chuyển, Esc hoặc bấm nền để đóng. */
export default function Lightbox(props: { src: string; alt?: string; onClose: () => void; onPrev?: () => void; onNext?: () => void; label?: string }) {
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
    if (pts.size >= 2) {
      // chụm 2 ngón: xử lý ở touchmove (bắt cả cử chỉ bắt đầu trước khi mở hộp này)
      pts.set(e.pointerId, { x: e.clientX, y: e.clientY });
      moved = true;
      return;
    }
    setX(x() + e.clientX - p.x);
    setY(y() + e.clientY - p.y);
    if (Math.hypot(e.clientX - p.x, e.clientY - p.y) > 2) moved = true;
    pts.set(e.pointerId, { x: e.clientX, y: e.clientY });
  };
  const up = (e: PointerEvent) => {
    pts.delete(e.pointerId);
    // bấm (không kéo) ra ngoài ảnh thì đóng
    if (!moved && pts.size === 0 && e.target === box) props.onClose();
  };
  let pd = 0; // khoảng cách 2 ngón lần trước
  const tmove = (e: TouchEvent) => {
    if (e.touches.length !== 2) return void (pd = 0);
    e.preventDefault();
    const [a, b] = [e.touches[0], e.touches[1]];
    const d = Math.hypot(a.clientX - b.clientX, a.clientY - b.clientY);
    if (pd > 0) zoomAt(d / pd, (a.clientX + b.clientX) / 2, (a.clientY + b.clientY) / 2);
    pd = d;
  };
  const tend = () => (pd = 0);
  const key = (e: KeyboardEvent) => {
    if ((e.ctrlKey || e.metaKey) && ["+", "=", "-", "0"].includes(e.key)) e.preventDefault(); // không zoom cả trang
    if (e.key === "ArrowLeft" && props.onPrev) props.onPrev();
    else if (e.key === "ArrowRight" && props.onNext) props.onNext();
    else if (e.key === "+" || e.key === "=") zoomAt(1.25);
    else if (e.key === "-") zoomAt(0.8);
    else if (e.key === "0") reset(fit);
  };
  onEsc(() => (props.onClose(), true));
  onMount(() => {
    document.addEventListener("keydown", key);
    // wheel cần passive:false để chặn cuộn trang
    const wheel = (e: WheelEvent) => {
      e.preventDefault();
      zoomAt(Math.exp(-e.deltaY * (e.ctrlKey ? 0.01 : 0.0015)), e.clientX, e.clientY);
    };
    box.addEventListener("wheel", wheel, { passive: false });
    document.addEventListener("touchmove", tmove, { passive: false });
    document.addEventListener("touchend", tend);
    document.addEventListener("touchstart", tend);
    onCleanup(() => {
      document.removeEventListener("touchmove", tmove);
      document.removeEventListener("touchend", tend);
      document.removeEventListener("touchstart", tend);
      document.removeEventListener("keydown", key);
      box.removeEventListener("wheel", wheel);
    });
  });
  return (
    <div class="zoombox" ref={box} onPointerDown={down} onPointerMove={move} onPointerUp={up} onPointerCancel={up}>
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
        <Show when={props.onPrev}>
          <button onClick={() => props.onPrev!()} title={t("vw.lbPrev")}><Icon name="back" /></button>
        </Show>
        <Show when={props.label}><span class="lb-label">{props.label}</span></Show>
        <Show when={props.onNext}>
          <button onClick={() => props.onNext!()} title={t("vw.lbNext")}><Icon name="next" /></button>
          <span class="lb-sep" />
        </Show>
        <button onClick={() => zoomAt(0.8)} title={t("vw.lbZout")}><Icon name="minus" /></button>
        <button class="lb-pct" onClick={() => reset(fit)} title={t("vw.lbFit")}>{Math.round(s() * 100)}%</button>
        <button onClick={() => zoomAt(1.25)} title={t("vw.lbZin")}><Icon name="plus" /></button>
        <button onClick={() => reset(1)} title={t("vw.lbReal")}>1:1</button>
        <button onClick={props.onClose} title={t("vw.lbClose")}><Icon name="close" /></button>
      </div>
    </div>
  );
}
