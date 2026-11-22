import { For, Show, createSignal } from "solid-js";
import { FONTS, PRESETS, previewDim, previewReadSize, setDim, setFonts, setImage, setPreset, setTheme, ui } from "./background";
import { Icon } from "./icons";

export function LookPanel(props: { onError: (m: string) => void }) {
  const [busy, setBusy] = createSignal(false);
  let pick!: HTMLInputElement;
  const run = async (f: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await f();
    } catch (e) {
      props.onError("Không lưu được giao diện: " + String((e as Error).message));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div class="look">
      <p class="muted sm">Lưu trên server — đồng bộ mọi thiết bị.</p>
      <div class="field-label">Chế độ màu</div>
      <div class="seg wide">
        <For each={[["auto", "Theo hệ thống"], ["light", "Sáng"], ["dark", "Tối"]] as const}>
          {([id, label]) => (
            <button classList={{ on: ui().theme === id }} disabled={busy()} onClick={() => run(() => setTheme(id))}>
              {label}
            </button>
          )}
        </For>
      </div>
      <div class="field-label">Phông chữ</div>
      <div class="font-grid">
        <label>
          <span class="muted sm">Giao diện</span>
          <select value={ui().font} disabled={busy()} onChange={(e) => run(() => setFonts({ font: e.currentTarget.value }))}>
            <For each={FONTS}>{(f) => <option value={f.id}>{f.name}</option>}</For>
          </select>
        </label>
        <label>
          <span class="muted sm">Khi đọc tài liệu (.md)</span>
          <select value={ui().readFont} disabled={busy()} onChange={(e) => run(() => setFonts({ readFont: e.currentTarget.value }))}>
            <option value="">Giống giao diện</option>
            <For each={FONTS}>{(f) => <option value={f.id}>{f.name}</option>}</For>
          </select>
        </label>
      </div>
      <label class="slider">
        <span>Cỡ chữ khi đọc: {ui().readSize}px</span>
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
      <p class="font-sample">Tiếng Việt có dấu: Hệ thống 1 tư duy nhanh, trực giác — ABC xyz 0123.</p>
      <div class="field-label">Hình nền</div>
      <div class="swatches">
        <button
          class="swatch upload"
          classList={{ on: ui().bg === "image" }}
          disabled={busy()}
          onClick={() => pick.click()}
          title="Ảnh của bạn"
        >
          <Icon name={busy() ? "cloud" : "image"} size={22} />
          <span>{busy() ? "Đang lưu…" : "Ảnh của bạn"}</span>
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
          <span>Làm tối ảnh nền</span>
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
          <h3>Giao diện & nền</h3>
          <button class="icon-btn" onClick={props.onClose}><Icon name="close" /></button>
        </div>
        <LookPanel onError={props.onError} />
      </div>
    </div>
  );
}
