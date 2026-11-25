import { For, Show, createResource, createSignal } from "solid-js";
import { api } from "./api";
import { errText } from "./fileops";
import { fmtSize } from "./format";
import { fmtDateTime, t } from "./i18n";
import { Icon } from "./icons";

/** Lịch sử phiên bản 1 file: tải về hoặc khôi phục bản cũ (bản hiện tại thành 1 phiên bản mới). */
export function VersionsDialog(props: { path: string; onClose: () => void; onRestored: () => void; onError: (m: string) => void }) {
  const [rows, { refetch }] = createResource(() => props.path, (p) => api.versions(p).catch((e) => (props.onError(errText(e)), [])));
  const [busy, setBusy] = createSignal(false);
  const name = () => props.path.split("/").pop() ?? props.path;
  const download = async (ts: number) => {
    try {
      const url = URL.createObjectURL(await api.versionBlob(props.path, ts));
      const a = document.createElement("a");
      const dot = name().lastIndexOf(".");
      a.href = url;
      a.download = dot > 0 ? `${name().slice(0, dot)} (${fmtDateTime(ts).replace(/[/:]/g, "-")})${name().slice(dot)}` : name();
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 10_000);
    } catch (e) {
      props.onError(errText(e));
    }
  };
  const restore = async (ts: number) => {
    if (!confirm(t("ver.restoreAsk", { d: fmtDateTime(ts) }))) return;
    setBusy(true);
    try {
      await api.restoreVersion(props.path, ts);
      refetch();
      props.onRestored();
    } catch (e) {
      props.onError(errText(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div class="modal-bg" onClick={props.onClose}>
      <div class="modal glass" onClick={(e) => e.stopPropagation()}>
        <div class="modal-head">
          <h3>{t("ver.title", { name: name() })}</h3>
          <button class="icon-btn" onClick={props.onClose}><Icon name="close" /></button>
        </div>
        <p class="muted xs">{t("ver.note")}</p>
        <Show when={!rows.loading} fallback={<p class="muted">{t("c.loading")}</p>}>
          <For each={rows()} fallback={<p class="muted">{t("ver.none")}</p>}>
            {(r) => (
              <div class="sh-row">
                <Icon name="history" size={15} class="ic muted" />
                <span>{fmtDateTime(r.ts)}</span>
                <span class="muted xs">{fmtSize(r.size)}</span>
                <span class="grow" />
                <button class="icon-btn sm" title={t("f.download")} onClick={() => download(r.ts)}><Icon name="download" size={14} /></button>
                <button class="btn sm" disabled={busy()} onClick={() => restore(r.ts)}>{t("ver.restore")}</button>
              </div>
            )}
          </For>
        </Show>
      </div>
    </div>
  );
}
