import { For, Show, createResource, createSignal, onMount } from "solid-js";
import { SECRET_ROOT, api, isSecret, type TrashItem } from "./api";
import { fmtSize } from "./format";
import { Icon } from "./icons";

/** Lấy phần `{"error": "..."}` từ lỗi `req` cho dễ đọc. */
export function errText(e: unknown): string {
  const m = String((e as Error)?.message ?? e);
  const j = m.match(/\{.*\}/);
  try {
    return j ? JSON.parse(j[0]).error : m;
  } catch {
    return m;
  }
}

export const joinPath = (dir: string, name: string) => (dir ? `${dir}/${name}` : name);
const baseName = (p: string) => p.split("/").pop() ?? p;
/** Đường dẫn để hiện (bỏ tiền tố nội bộ của khu Bí mật). */
const shown = (p: string) => (isSecret(p) ? p.slice(SECRET_ROOT.length + 1) : p);

function badName(n: string): string {
  if (!n) return "Tên không được để trống";
  if (n.includes("/") || n.includes("\\")) return "Tên không được chứa / hoặc \\";
  if (n === "." || n === "..") return "Tên không hợp lệ";
  if (n.startsWith(".")) return "Tên bắt đầu bằng dấu chấm sẽ bị ẩn";
  return "";
}

function Head(props: { title: string; onClose: () => void }) {
  return (
    <div class="modal-head">
      <h3>{props.title}</h3>
      <button type="button" class="icon-btn" onClick={props.onClose}><Icon name="close" /></button>
    </div>
  );
}

/** Hỏi 1 cái tên: thư mục mới, đổi tên. */
export function NameDialog(props: {
  title: string;
  label: string;
  initial?: string;
  ok: string;
  onSubmit: (name: string) => Promise<void>;
  onClose: () => void;
}) {
  const [name, setName] = createSignal(props.initial ?? "");
  const [err, setErr] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  let input!: HTMLInputElement;
  onMount(() => {
    input.focus();
    // Đổi tên: chọn phần tên, chừa đuôi file.
    const dot = (props.initial ?? "").lastIndexOf(".");
    input.setSelectionRange(0, dot > 0 ? dot : input.value.length);
  });
  return (
    <div class="modal-bg" onClick={() => !busy() && props.onClose()}>
      <form
        class="modal glass form"
        onClick={(e) => e.stopPropagation()}
        onSubmit={async (e) => {
          e.preventDefault();
          const n = name().trim();
          const bad = badName(n);
          if (bad) return setErr(bad);
          setBusy(true);
          try {
            await props.onSubmit(n);
          } catch (e) {
            setErr(errText(e));
          } finally {
            setBusy(false);
          }
        }}
      >
        <Head title={props.title} onClose={props.onClose} />
        <label>{props.label}</label>
        <input ref={input} value={name()} onInput={(e) => { setName(e.currentTarget.value); setErr(""); }} autocomplete="off" spellcheck={false} />
        <Show when={err()}><p class="err">{err()}</p></Show>
        <button type="submit" class="btn primary block" disabled={busy() || name().trim() === (props.initial ?? "")}>
          {busy() ? "Đang lưu…" : props.ok}
        </button>
      </form>
    </div>
  );
}

/** Chọn thư mục đích: duyệt cây từng cấp, không cho chọn chính mục đang chuyển. */
export function MoveDialog(props: {
  paths: string[];
  start: string;
  copy?: boolean;
  onSubmit: (dest: string) => Promise<void>;
  onClose: () => void;
}) {
  const [cur, setCur] = createSignal(props.start);
  const [err, setErr] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [dirs] = createResource(cur, async (p) => {
    try {
      const r = await api.files(p, { sort: "name", desc: false, offset: 0, limit: 1000 });
      setErr("");
      return r.entries.filter((e) => e.is_dir);
    } catch (e) {
      setErr(isSecret(p) ? "Khu Bí mật đang khoá: mở khoá (mục Bí mật) rồi thử lại" : errText(e));
      return [];
    }
  });
  const blocked = (p: string) => props.paths.some((m) => p === m || p.startsWith(m + "/"));
  const sameDir = () => !props.copy && props.paths.every((m) => m.split("/").slice(0, -1).join("/") === cur());
  const root = () => (isSecret(cur()) ? SECRET_ROOT : "");
  /** Chuyển từ file thường vào Bí mật: bản gốc bị xoá hẳn. */
  const intoSecret = () => isSecret(cur()) && !isSecret(props.paths[0] ?? "");
  const crumbs = () => cur().split("/").filter(Boolean).filter((c) => c !== SECRET_ROOT);
  const verb = props.copy ? "Sao chép" : "Di chuyển";
  const title = () => (props.paths.length === 1 ? `${verb} "${baseName(props.paths[0])}"` : `${verb} ${props.paths.length} mục`);
  return (
    <div class="modal-bg" onClick={() => !busy() && props.onClose()}>
      <div class="modal glass form" onClick={(e) => e.stopPropagation()}>
        <Head title={title()} onClose={props.onClose} />
        <div class="seg pick-root">
          <button classList={{ on: !root() }} onClick={() => setCur("")}><Icon name="home" size={15} /> Tất cả file</button>
          <button classList={{ on: !!root() }} onClick={() => setCur(SECRET_ROOT)}><Icon name="lock" size={15} /> Bí mật</button>
        </div>
        <nav class="crumbs pick-crumbs">
          <button class="crumb" onClick={() => setCur(root())}><Icon name={root() ? "lock" : "home"} size={16} /></button>
          <For each={crumbs()}>
            {(c, i) => (
              <>
                <Icon name="chevron" size={14} class="ic dim" />
                <button class="crumb" classList={{ cur: i() === crumbs().length - 1 }} onClick={() => setCur(joinPath(root(), crumbs().slice(0, i() + 1).join("/")))}>
                  {c}
                </button>
              </>
            )}
          </For>
        </nav>
        <div class="pick-list">
          <Show when={!dirs.loading} fallback={<p class="muted sm pad">Đang tải…</p>}>
            <For each={dirs() ?? []} fallback={<p class="muted sm pad">Không có thư mục con</p>}>
              {(d) => (
                <button class="pick-row" disabled={blocked(d.path)} onClick={() => setCur(d.path)}>
                  <Icon name="folder" /> <span class="fname">{d.name}</span> <Icon name="chevron" size={14} class="ic dim" />
                </button>
              )}
            </For>
          </Show>
        </div>
        <Show when={err()}><p class="err">{err()}</p></Show>
        <Show when={intoSecret() && !props.copy}>
          <p class="muted sm">Bản gốc sẽ bị <b>xoá hẳn</b> sau khi mã hoá vào Bí mật (không vào thùng rác).</p>
        </Show>
        <div class="pick-foot">
          <button type="button" class="btn ghost" onClick={async () => {
            const n = prompt("Tên thư mục mới")?.trim();
            if (!n) return;
            const bad = badName(n);
            if (bad) return setErr(bad);
            try {
              await api.mkdir(joinPath(cur(), n));
              setCur(joinPath(cur(), n));
            } catch (e) {
              setErr(errText(e));
            }
          }}>
            <Icon name="plus" /> Thư mục mới
          </button>
          <button class="btn primary" disabled={busy() || sameDir() || blocked(cur())} onClick={async () => {
            setBusy(true);
            setErr("");
            try {
              await props.onSubmit(cur());
            } catch (e) {
              setErr(errText(e));
            } finally {
              setBusy(false);
            }
          }}>
            {busy() ? (props.copy ? "Đang sao chép…" : "Đang chuyển…") : props.copy ? "Sao chép vào đây" : "Chuyển vào đây"}
          </button>
        </div>
      </div>
    </div>
  );
}

const fmtWhen = (s: number) => new Date(s * 1000).toLocaleString("vi-VN", { dateStyle: "short", timeStyle: "short" });

export function TrashPage(props: { onError: (m: string) => void; onChanged: () => void; secret?: boolean }) {
  const [items, { refetch }] = createResource(() => api.trash(props.secret));
  const [cfg] = createResource(() => api.config().catch(() => ({ trash_days: 30 })));
  const [busy, setBusy] = createSignal<string | null>(null);
  const total = () => (items() ?? []).reduce((a, t) => a + t.size, 0);
  const run = async (key: string, f: () => Promise<void>) => {
    setBusy(key);
    try {
      await f();
      props.onChanged();
    } catch (e) {
      props.onError(errText(e));
    } finally {
      setBusy(null);
      refetch();
    }
  };
  const purge = (t: TrashItem) => {
    if (confirm(`Xoá vĩnh viễn "${baseName(t.orig_path)}"? Không thể hoàn tác.`)) run(t.id, () => api.purge(t.id, props.secret));
  };
  return (
    <div class="trash">
      <div class="toolbar">
        <h2 class="tb-title"><Icon name={props.secret ? "lock" : "trash"} /> {props.secret ? "Thùng rác Bí mật" : "Thùng rác"}</h2>
        <span class="muted sm hide-xs">Tự xoá sau {cfg()?.trash_days ?? 30} ngày · vẫn tính vào dung lượng</span>
        <button class="btn danger sm" style={{ "margin-left": "auto" }} disabled={!items()?.length || !!busy()} onClick={() => {
          if (confirm(`Xoá vĩnh viễn ${items()!.length} mục (${fmtSize(total())})? Không thể hoàn tác.`)) run("all", () => api.emptyTrash(props.secret));
        }}>
          {busy() === "all" ? "Đang dọn…" : "Dọn sạch"}
        </button>
      </div>
      <div class="scroll">
        <For each={items() ?? []} fallback={
          <Show when={!items.loading}>
            <div class="empty"><Icon name="trash" size={48} /><p>Thùng rác trống</p></div>
          </Show>
        }>
          {(t) => (
            <div class="row trow">
              <span class="c-name">
                <span class="fi"><Icon name={t.is_dir ? "folder" : "file"} size={20} /></span>
                <span class="fcol">
                  <span class="fname" title={shown(t.orig_path)}>{baseName(t.orig_path)}</span>
                  <span class="fpath">
                    {shown(t.orig_path).includes("/") ? shown(t.orig_path).split("/").slice(0, -1).join("/") + " · " : ""}
                    {t.is_dir ? `${t.files.toLocaleString("vi-VN")} file · ` : ""}{fmtSize(t.size)} · xoá {fmtWhen(t.deleted_at)}
                  </span>
                </span>
              </span>
              <span class="tact">
                <button class="btn sm ghost" disabled={!!busy()} onClick={() => run(t.id, () => api.restore(t.id, props.secret))}>
                  <Icon name="back" /> <span class="hide-xs">Khôi phục</span>
                </button>
                <button class="btn sm danger" disabled={!!busy()} title="Xoá vĩnh viễn" onClick={() => purge(t)}>
                  <Icon name="trash" />
                </button>
              </span>
            </div>
          )}
        </For>
      </div>
      <div class="foot">
        <span>{(items() ?? []).length} mục</span>
        <span>{fmtSize(total())}</span>
      </div>
    </div>
  );
}
