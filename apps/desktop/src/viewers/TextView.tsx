import { Show, createResource } from "solid-js";
import { api } from "../api";

/** Xem nhanh file text/code (< 500KB, quá thì cắt + gợi ý tải). */
export default function TextView(props: { path: string }) {
  const [text] = createResource(() => props.path, (p) => api.readText(p));
  return (
    <div class="code-view">
      <Show when={text.error}><p class="err">Không đọc được: {String((text.error as any)?.message ?? text.error)}</p></Show>
      <Show when={text()}>
        {(t) => (
          <>
            <pre>{t().length > 500_000 ? t().slice(0, 500_000) : t()}</pre>
            <Show when={t().length > 500_000}>
              <p class="muted sm">File quá lớn, chỉ hiện 500KB đầu — hãy tải về để xem đầy đủ.</p>
            </Show>
          </>
        )}
      </Show>
      <Show when={text.loading}><p class="muted">Đang tải…</p></Show>
    </div>
  );
}
