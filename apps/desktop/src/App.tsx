import { createSignal, For, lazy, Suspense } from "solid-js";

// Lazy viewers -> bundle đầu nhẹ cho máy yếu; chỉ tải khi mở đúng loại file.
const MarkdownView = lazy(() => import("./viewers/MarkdownView"));

const FILES = [
  { name: "Bai-giang-01.md", kind: "markdown" },
  { name: "Hop-dong.pdf", kind: "pdf" },
  { name: "Video.mp4", kind: "video" },
  { name: "Archive.zip", kind: "zip" },
];

export default function App() {
  const [open, setOpen] = createSignal<string | null>(null);
  return (
    <main style={{ padding: "24px", "font-family": "system-ui" }}>
      <h1>NextKloud (MVP)</h1>
      <p>File browser virtualized + chunked upload ở milestone M1. Viewer lazy-load.</p>
      <ul>
        <For each={FILES}>
          {(f) => (
            <li>
              <button onClick={() => setOpen(f.name)}>{f.name}</button>
            </li>
          )}
        </For>
      </ul>
      <Suspense fallback={<p>Loading viewer…</p>}>
        {open() === "Bai-giang-01.md" && <MarkdownView />}
      </Suspense>
    </main>
  );
}
