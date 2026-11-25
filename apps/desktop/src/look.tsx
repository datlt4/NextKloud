import { For, Show, createSignal } from "solid-js";
import { FONTS, PRESETS, previewDim, previewReadSize, setDim, setFonts, setImage, setLang, setPreset, setTheme, setTtsRate, previewTtsRate, ui } from "./background";
import { LANGS, lang, t } from "./i18n";
import { Icon } from "./icons";

export function LookPanel(props: { onError: (m: string) => void }) {
  const [busy, setBusy] = createSignal(false);
  let pick!: HTMLInputElement;
  const run = async (f: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await f();
    } catch (e) {
      props.onError(t("look.saveErr") + String((e as Error).message));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div class="look">
      <p class="muted sm">{t("look.synced")}</p>
      <div class="field-label">{t("look.lang")}</div>
      <div class="seg wide">
        <For each={LANGS}>
          {({ id, label }) => (
            <button classList={{ on: lang() === id }} disabled={busy()} onClick={() => run(() => setLang(id))}>
              {label}
            </button>
          )}
        </For>
      </div>
      <div class="field-label">{t("look.theme")}</div>
      <div class="seg wide">
        <For each={[["auto", t("look.auto")], ["light", t("look.light")], ["dark", t("look.dark")]] as const}>
          {([id, label]) => (
            <button classList={{ on: ui().theme === id }} disabled={busy()} onClick={() => run(() => setTheme(id))}>
              {label}
            </button>
          )}
        </For>
      </div>
      <div class="field-label">{t("look.font")}</div>
      <div class="font-grid">
        <label>
          <span class="muted sm">{t("look.uiFont")}</span>
          <select value={ui().font} disabled={busy()} onChange={(e) => run(() => setFonts({ font: e.currentTarget.value }))}>
            <For each={FONTS}>{(f) => <option value={f.id}>{f.name}</option>}</For>
          </select>
        </label>
        <label>
          <span class="muted sm">{t("look.readFont")}</span>
          <select value={ui().readFont} disabled={busy()} onChange={(e) => run(() => setFonts({ readFont: e.currentTarget.value }))}>
            <option value="">{t("look.sameFont")}</option>
            <For each={FONTS}>{(f) => <option value={f.id}>{f.name}</option>}</For>
          </select>
        </label>
      </div>
      <label class="slider">
        <span>{t("look.readSize", { size: ui().readSize })}</span>
        <input
          type="range"
          min="12"
          max="24"
          step="1"
          value={ui().readSize}
          onInput={(e) => previewReadSize(+e.currentTarget.value)}
          onChange={(e) => run(() => setFonts({ readSize: +e.currentTarget.value }))}
        />
      </label>
      <label class="slider">
        <span>{t("look.ttsRate", { rate: ui().ttsRate.toFixed(1) })}</span>
        <input
          type="range"
          min="0.1"
          max="2"
          step="0.1"
          value={ui().ttsRate}
          onInput={(e) => previewTtsRate(+e.currentTarget.value)}
          onChange={(e) => run(() => setTtsRate(+e.currentTarget.value))}
        />
      </label>
      <p class="font-sample">Tiếng Việt có dấu: Hệ thống 1 tư duy nhanh, trực giác — ABC xyz 0123.</p>
      <div class="field-label">{t("look.wallpaper")}</div>
      <div class="swatches">
        <button
          class="swatch upload"
          classList={{ on: ui().bg === "image" }}
          disabled={busy()}
          onClick={() => pick.click()}
          title={t("look.yourPhoto")}
        >
          <Icon name={busy() ? "cloud" : "image"} size={22} />
          <span>{busy() ? t("look.saving") : t("look.yourPhoto")}</span>
        </button>
        <For each={PRESETS}>
          {(p) => (
            <button
              class="swatch"
              classList={{ on: ui().bg === p.id }}
              style={{ background: p.css }}
              disabled={busy()}
              onClick={() => run(() => setPreset(p.id))}
            >
              <Show when={ui().bg === p.id}><Icon name="check" size={20} /></Show>
              <span>{p.name}</span>
            </button>
          )}
        </For>
      </div>
      <input
        ref={pick}
        type="file"
        accept="image/*"
        hidden
        onChange={(e) => {
          const f = e.currentTarget.files?.[0];
          if (f) run(() => setImage(f));
          e.currentTarget.value = "";
        }}
      />
      <Show when={ui().bg === "image"}>
        <label class="slider">
          <span>{t("look.dim")}</span>
          <input
            type="range"
            min="0"
            max="0.8"
            step="0.05"
            value={ui().dim}
            onInput={(e) => previewDim(+e.currentTarget.value)}
            onChange={(e) => run(() => setDim(+e.currentTarget.value))}
          />
        </label>
      </Show>
    </div>
  );
}

export function LookDialog(props: { onClose: () => void; onError: (m: string) => void }) {
  return (
    <div class="modal-bg" onClick={props.onClose}>
      <div class="modal glass" onClick={(e) => e.stopPropagation()}>
        <div class="modal-head">
          <h3>{t("look.title")}</h3>
          <button class="icon-btn" onClick={props.onClose}><Icon name="close" /></button>
        </div>
        <LookPanel onError={props.onError} />
      </div>
    </div>
  );
}
