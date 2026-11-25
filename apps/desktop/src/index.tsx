import { render } from "solid-js/web";
import "./style.css";
import "./sanitize";
import App from "./App";

// Không bao giờ zoom cả trang web: Ctrl/⌘+lăn chuột, Ctrl/⌘ +/-/0, chụm 2 ngón.
// Viewer ảnh/PDF tự bắt các cử chỉ này để zoom riêng đối tượng đang xem.
document.addEventListener("wheel", (e) => (e.ctrlKey || e.metaKey) && e.preventDefault(), { passive: false });
document.addEventListener("keydown", (e) => (e.ctrlKey || e.metaKey) && ["+", "=", "-", "0"].includes(e.key) && e.preventDefault());
document.addEventListener("touchmove", (e) => e.touches.length > 1 && e.preventDefault(), { passive: false });
for (const g of ["gesturestart", "gesturechange"]) document.addEventListener(g, (e) => e.preventDefault());

render(() => <App />, document.getElementById("root")!);
