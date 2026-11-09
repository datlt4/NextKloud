import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

export default defineConfig({
  plugins: [solid()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: {
    target: "es2020",
    // Giữ bundle nhẹ cho máy yếu (Samsung A8 Star): code-split theo route/viewer.
    chunkSizeWarningLimit: 600,
  },
});
