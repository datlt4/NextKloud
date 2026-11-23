import { For, Show, createResource, createSignal } from "solid-js";
import { api } from "../api";
import { Icon } from "../icons";
import { fmtNum, t } from "../i18n";

/** Duyệt zip: liệt kê central directory (Range, không tải cả file) + tải 1 file con. */
export default function ZipView(props: { path: string }) {
  const [entries, { refetch }] = createResource(() => props.path, (p) => api.zipList(p));
  const [busy, setBusy] = createSignal<string | null>(null);

  const save = async (name: string) => {
    setBusy(name);
    try {
      const a = document.createElement("a");
      a.href = api.zipFileUrl(props.path, name);
      a.download = name.split("/").pop() || "file";
      // Token đã nằm trong URL (?t=) nên browser tải trực tiếp được.
      document.body.appendChild(a);
      a.click();
      a.remove();
    } finally {
      setBusy(null);
    }
  };

  return (
    <div class="zip-wrap">
      <Show when={entries.error}><p class="err">{t("vw.zipErr", { e: String((entries.error as any)?.message ?? entries.error) })}</p></Show>
      <Show when={entries()}>
        {(z) => (
          <>
            <p class="muted sm">{t("vw.zipTotal", { n: fmtNum(z().total) })}{z().truncated ? t("vw.zipShowing", { n: fmtNum(z().entries.length) }) : ""} · bấm để tải file con</p>
            <ul class="files">
              <For each={z().entries}>
                {(e) => (
                  <li>
                    <span>{e.is_dir ? "📁" : "📄"}</span>
                    <a href="#" title={e.name} onClick={(ev) => { ev.preventDefault(); if (!e.is_dir) save(e.name); }}>{e.name}</a>
                    {e.is_dir ? "" : <span class="size">{(e.size / 1024).toFixed(1)} KB</span>}
                    <Show when={!e.is_dir}>
                      <button class="icon-btn sm" title={t("vw.zipDl")} onClick={() => save(e.name)} disabled={busy() === e.name}>
                        <Icon name="download" size={14} />
                      </button>
                    </Show>
                  </li>
                )}
              </For>
            </ul>
          </>
        )}
      </Show>
      <Show when={entries.loading && !entries()}><p class="muted">{t("vw.zipLoading")}</p></Show>
    </div>
  );
}
