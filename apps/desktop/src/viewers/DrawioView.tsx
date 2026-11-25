import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { onEsc, inField } from "../esc";
import { api } from "../api";
import { loc, t } from "../i18n";
import { Icon } from "../icons";

/**
 * Sửa draw.io (.drawio) qua editor nhúng. Lưu ngược lại file trên NextKloud.
 * Cần mạng để tải editor từ embed.diagrams.net (lần đầu), file vẫn nằm 100% trên server nhà.
 */
export default function DrawioView(props: { path: string }) {
  const [err, setErr] = createSignal("");
  const [saved, setSaved] = createSignal<string | null>(null);
  // "Full màn hình" = lấp đầy khung trình duyệt (giữ chrome/nút điều hướng), KHÔNG dùng Fullscreen API.
  const [wide, setWide] = createSignal(false);
  let frame!: HTMLIFrameElement;
  let dead = false;
  let timer: any = null;

  const send = (msg: any) => frame?.contentWindow?.postMessage(JSON.stringify(msg), "*");

  const persist = (xml: string) => {
    setSaved(t("vw.drawSaving"));
    api.saveText(props.path, xml).then(
      () => !dead && setSaved(t("vw.drawSaved", { t: new Date().toLocaleTimeString(loc()) })),
      (e: any) => !dead && setErr(String(e?.message ?? e)),
    );
  };
  const persistSoon = (xml: string) => {
    clearTimeout(timer);
    timer = setTimeout(() => persist(xml), 2500);
  };

  const onMsg = (ev: MessageEvent) => {
    if (!ev.data || typeof ev.data !== "string" || ev.data[0] !== "{") return;
    let m: any;
    try {
      m = JSON.parse(ev.data);
    } catch {
      return;
    }
    if (m.event === "init") {
      api.readText(props.path).then(
        (xml) => !dead && send({ action: "load", xml: xml.trim() || undefined, autosave: 1, saveAndExit: "0" }),
        () => !dead && send({ action: "load", autosave: 1, saveAndExit: "0" }),
      );
    } else if (m.event === "save" || m.event === "autosave") {
      if (typeof m.xml === "string") {
        if (m.event === "save") {
          clearTimeout(timer);
          persist(m.xml);
        } else persistSoon(m.xml);
      }
    } else if (m.event === "exit") {
      // ở lại viewer, không thoát đi đâu
    }
  };

  onMount(() => {
    onEsc((e) => wide() && !inField(e) && (setWide(false), true));
    window.addEventListener("message", onMsg);
    onCleanup(() => {
      dead = true;
      clearTimeout(timer);
      window.removeEventListener("message", onMsg);
    });
  });

  return (
    <div class="drawio-wrap" classList={{ "vw-wide": wide() }}>
      <button class="fs-btn" onClick={() => setWide((w) => !w)} title={wide() ? t("vw.fsExit") : t("vw.fsEnter")}>
        <Icon name={wide() ? "compress" : "expand"} size={17} />
      </button>
      <Show when={err()}><p class="err">{err()}</p></Show>
      <Show when={saved()}><p class="muted sm">{saved()}</p></Show>
      <iframe
        ref={frame!}
        title="drawio"
        src="https://embed.diagrams.net/?embed=1&ui=atlas&spin=1&proto=json&saveAndExit=0"
        allow="clipboard-write"
      />
    </div>
  );
}
