import { onCleanup } from "solid-js";

// Esc chỉ đóng lớp trên cùng (viewer full khung > lightbox > overlay > drawer), không đóng cả chồng 1 lần.
const stack: ((e: KeyboardEvent) => boolean)[] = [];
window.addEventListener(
  "keydown",
  (e) => {
    if (e.key !== "Escape") return;
    for (let i = stack.length - 1; i >= 0; i--) {
      if (stack[i](e)) {
        e.stopPropagation();
        e.preventDefault();
        return;
      }
    }
  },
  true,
);

/** Đăng ký xử lý Esc (gọi trong component); trả true = đã xử lý, chặn các lớp dưới. Lớp mount sau ở trên. */
export function onEsc(fn: (e: KeyboardEvent) => boolean) {
  stack.push(fn);
  onCleanup(() => {
    const i = stack.indexOf(fn);
    if (i >= 0) stack.splice(i, 1);
  });
}

export const inField = (e: KeyboardEvent) => !!(e.target as HTMLElement).closest?.("input, textarea, select");
