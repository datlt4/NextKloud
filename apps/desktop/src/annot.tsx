import { For, type JSX, Show, createMemo, createResource, createSignal, onCleanup, onMount } from "solid-js";
import { api, type Annotation, type NoteState } from "./api";
import { errText } from "./fileops";
import { Icon } from "./icons";

/** 1 dòng mục lục: anchor dùng chung với annotation ("b12" = block 12 của md, "p3" = trang 3 của pdf). */
export interface TocItem {
  anchor: string;
  label: string;
  level: number;
}

export const STATE_UI: Record<NoteState, { icon: string; label: string; next: NoteState }> = {
  todo: { icon: "pause", label: "Chưa làm", next: "doing" },
  doing: { icon: "play", label: "Đang làm", next: "done" },
  done: { icon: "check", label: "Xong", next: "todo" },
};

/** Trạng thái chung của 1 trình đọc (md/pdf): danh sách annotation + sidebar. */
export function createReader(path: () => string, onError: (m: string) => void) {
  const [items, { mutate, refetch }] = createResource(path, (p) => api.annotations(p), { initialValue: [] });
  const [tab, setTab] = createSignal<"toc" | "notes">("toc");
  const [sheet, setSheet] = createSignal(false); // mobile: bottom sheet đang mở
  const [focus, setFocus] = createSignal<string | null>(null); // annotation đang được trỏ tới
  const [draft, setDraft] = createSignal<{ anchor: string; quote: string } | null>(null);
  const run = async <T,>(f: () => Promise<T>) => {
    try {
      return await f();
    } catch (e) {
      onError(errText(e));
      refetch();
    }
  };
  const showNotes = (id?: string) => {
    setTab("notes");
    setSheet(true);
    if (id) setFocus(id);
  };
  return {
    items: () => items() ?? [],
    tab,
    setTab,
    sheet,
    setSheet,
    focus,
    setFocus,
    draft,
    setDraft,
    showNotes,
    /** Bookmark bật/tắt tại 1 vị trí. */
    toggleBookmark: (anchor: string, quote: string) =>
      run(async () => {
        const hit = (items() ?? []).find((a) => a.kind === "bookmark" && a.anchor === anchor);
        if (hit) {
          await api.removeAnnotation(hit.id);
          mutate((l) => l.filter((a) => a.id !== hit.id));
        } else {
          const a = await api.addAnnotation({ path: path(), kind: "bookmark", anchor, quote: quote.slice(0, 300) });
          mutate((l) => [...l, a]);
        }
      }),
    startNote: (anchor: string, quote: string) => {
      setDraft({ anchor, quote: quote.slice(0, 1000) });
      showNotes();
    },
    saveDraft: (body: string) =>
      run(async () => {
        const d = draft();
        if (!d) return;
        const a = await api.addAnnotation({ path: path(), kind: "note", anchor: d.anchor, quote: d.quote, body });
        mutate((l) => [...l, a]);
        setDraft(null);
        setFocus(a.id);
      }),
    update: (id: string, b: { body?: string; state?: NoteState }) =>
      run(async () => {
        const a = await api.updateAnnotation(id, b);
        mutate((l) => l.map((x) => (x.id === id ? a : x)));
      }),
    remove: (id: string) =>
      run(async () => {
        await api.removeAnnotation(id);
        mutate((l) => l.filter((x) => x.id !== id));
      }),
  };
}
export type Reader = ReturnType<typeof createReader>;

/** Thứ tự ưu tiên khi 1 đoạn có nhiều ghi chú: việc chưa xong nổi bật hơn. */
export function topState(list: Annotation[]): NoteState | null {
  const notes = list.filter((a) => a.kind === "note");
  if (notes.some((a) => a.state === "doing")) return "doing";
  if (notes.some((a) => a.state === "todo")) return "todo";
  return notes.length ? "done" : null;
}

const ago = (s: number) => new Date(s * 1000).toLocaleString("vi-VN", { dateStyle: "short", timeStyle: "short" });

/**
 * Bố cục trình đọc: nội dung + sidebar (màn rộng: luôn hiện bên phải; mobile: ẩn, mở bằng nút nổi thành bottom sheet).
 * Chọn chữ trong nội dung -> hiện nút "Bookmark / Ghi chú" ngay cạnh vùng chọn.
 */
export function ReaderLayout(props: {
  r: Reader;
  toc: TocItem[];
  current: string;
  onJump: (anchor: string) => void;
  /** Vị trí (anchor) của 1 node trong nội dung, null nếu không thuộc nội dung đọc. */
  anchorOf: (n: Node) => string | null;
  children: JSX.Element;
}) {
  const r = props.r;
  let main!: HTMLDivElement;
  const [pop, setPop] = createSignal<{ x: number; y: number; anchor: string; quote: string } | null>(null);
  const onSel = () => {
    const sel = document.getSelection();
    const text = sel?.toString().trim() ?? "";
    if (!sel || !text || sel.rangeCount === 0 || !main.contains(sel.anchorNode)) return setPop(null);
    const anchor = props.anchorOf(sel.anchorNode!);
    if (!anchor) return setPop(null);
    const rect = sel.getRangeAt(0).getBoundingClientRect();
    const box = main.getBoundingClientRect();
    setPop({ x: Math.min(Math.max(rect.left + rect.width / 2 - box.left, 90), box.width - 90), y: rect.top - box.top + main.scrollTop, anchor, quote: text });
  };
  let t: number | undefined;
  const deb = () => {
    clearTimeout(t);
    t = window.setTimeout(onSel, 250);
  };
  onMount(() => document.addEventListener("selectionchange", deb));
  onCleanup(() => {
    document.removeEventListener("selectionchange", deb);
    clearTimeout(t);
  });
  const act = (f: (a: string, q: string) => void) => {
    const p = pop();
    if (!p) return;
    f(p.anchor, p.quote);
    setPop(null);
    document.getSelection()?.removeAllRanges();
  };
  const count = () => r.items().length;
  return (
    <div class="reader">
      <div class="reader-main" ref={main}>
        {props.children}
        <Show when={pop()}>
          {(p) => (
            <div class="sel-pop glass" style={{ left: `${p().x}px`, top: `${p().y}px` }} onMouseDown={(e) => e.preventDefault()}>
              <button onClick={() => act(r.toggleBookmark)}><Icon name="bookmark" size={15} /> Bookmark</button>
              <button onClick={() => act(r.startNote)}><Icon name="note" size={15} /> Ghi chú</button>
            </div>
          )}
        </Show>
      </div>
      <Show when={r.sheet()}>
        <div class="sheet-scrim" onClick={() => r.setSheet(false)} />
      </Show>
      <aside class="reader-side" classList={{ open: r.sheet() }}>
        <div class="rs-tabs">
          <button classList={{ on: r.tab() === "toc" }} onClick={() => r.setTab("toc")}><Icon name="toc" size={16} /> Mục lục</button>
          <button classList={{ on: r.tab() === "notes" }} onClick={() => r.setTab("notes")}>
            <Icon name="note" size={16} /> Bookmark & ghi chú <Show when={count()}><span class="rs-count">{count()}</span></Show>
          </button>
          <button class="icon-btn sm rs-close" onClick={() => r.setSheet(false)} title="Đóng"><Icon name="close" size={16} /></button>
        </div>
        <div class="rs-body">
          <Show when={r.tab() === "toc"}>
            <Toc items={props.toc} current={props.current} onJump={(a) => { props.onJump(a); r.setSheet(false); }} />
          </Show>
          <Show when={r.tab() === "notes"}>
            <NotesTab r={r} toc={props.toc} onJump={(a) => { props.onJump(a); r.setSheet(false); }} />
          </Show>
        </div>
      </aside>
      <button class="reader-fab" onClick={() => r.setSheet(true)} title="Mục lục, bookmark, ghi chú">
        <Icon name="toc" size={20} />
        <Show when={count()}><span class="rs-count">{count()}</span></Show>
      </button>
    </div>
  );
}

function Toc(props: { items: TocItem[]; current: string; onJump: (a: string) => void }) {
  const min = () => Math.min(...props.items.map((i) => i.level), 9);
  return (
    <Show when={props.items.length} fallback={<p class="muted sm pad">Tài liệu không có mục lục.</p>}>
      <nav class="toc">
        <For each={props.items}>
          {(i) => (
            <button classList={{ cur: i.anchor === props.current }} style={{ "padding-left": `${10 + (i.level - min()) * 14}px` }} onClick={() => props.onJump(i.anchor)}>
              {i.label}
            </button>
          )}
        </For>
      </nav>
    </Show>
  );
}

/** Tên mục chứa 1 anchor (để ghi chú hiện "§ Tên mục"). */
function sectionOf(toc: TocItem[], anchor: string): string {
  const n = (a: string) => +a.slice(1) || 0;
  let label = "";
  for (const t of toc) if (n(t.anchor) <= n(anchor)) label = t.label;
  return label;
}

function NotesTab(props: { r: Reader; toc: TocItem[]; onJump: (a: string) => void }) {
  const r = props.r;
  const [filter, setFilter] = createSignal<NoteState | "all">("all");
  const byPos = (a: Annotation, b: Annotation) => (+a.anchor.slice(1) || 0) - (+b.anchor.slice(1) || 0) || a.created_at - b.created_at;
  const bookmarks = createMemo(() => r.items().filter((a) => a.kind === "bookmark").sort(byPos));
  const notes = createMemo(() => r.items().filter((a) => a.kind === "note" && (filter() === "all" || a.state === filter())).sort(byPos));
  const n = (s: NoteState) => r.items().filter((a) => a.kind === "note" && a.state === s).length;
  return (
    <div class="notes-tab">
      <Show when={r.draft()}>{(d) => <Composer quote={d().quote} onCancel={() => r.setDraft(null)} onSave={r.saveDraft} />}</Show>
      <Show when={bookmarks().length}>
        <div class="rs-section">Bookmark</div>
        <For each={bookmarks()}>
          {(b) => (
            <div class="bm-row" onClick={() => props.onJump(b.anchor)}>
              <Icon name="bookmark" size={15} class="ic bm-ic" />
              <span class="bm-text">{b.quote || sectionOf(props.toc, b.anchor) || b.anchor}</span>
              <button class="icon-btn sm" title="Bỏ bookmark" onClick={(e) => { e.stopPropagation(); r.remove(b.id); }}><Icon name="close" size={14} /></button>
            </div>
          )}
        </For>
      </Show>
      <div class="rs-section">Ghi chú</div>
      <div class="chips">
        <button classList={{ on: filter() === "all" }} onClick={() => setFilter("all")}>Tất cả</button>
        <For each={["todo", "doing", "done"] as NoteState[]}>
          {(s) => (
            <button classList={{ on: filter() === s }} class={`st-${s}`} onClick={() => setFilter(s)}>
              <Icon name={STATE_UI[s].icon} size={13} /> {n(s)}
            </button>
          )}
        </For>
      </div>
      <For each={notes()} fallback={<p class="muted sm pad">Chưa có ghi chú. Bôi đen một đoạn rồi bấm “Ghi chú”.</p>}>
        {(a) => <NoteCard a={a} r={r} section={sectionOf(props.toc, a.anchor)} onJump={() => props.onJump(a.anchor)} />}
      </For>
    </div>
  );
}

export function StateButton(props: { state: NoteState; onChange: (s: NoteState) => void }) {
  const ui = () => STATE_UI[props.state];
  return (
    <button class={`st-btn st-${props.state}`} title={`${ui().label} (bấm để chuyển)`} onClick={(e) => { e.stopPropagation(); props.onChange(ui().next); }}>
      <Icon name={ui().icon} size={14} />
    </button>
  );
}

function NoteCard(props: { a: Annotation; r: Reader; section: string; onJump: () => void }) {
  const [editing, setEditing] = createSignal(false);
  let el!: HTMLDivElement;
  // Được trỏ tới (bấm dấu ghi chú trong bài) -> cuộn tới + nháy sáng.
  const focused = () => props.r.focus() === props.a.id;
  const scrollIfFocused = () => focused() && el?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  onMount(scrollIfFocused);
  return (
    <div class="note-card" classList={{ focus: focused(), [`st-${props.a.state}`]: true }} ref={(e) => { el = e; queueMicrotask(scrollIfFocused); }}>
      <div class="nc-head">
        <StateButton state={props.a.state} onChange={(s) => props.r.update(props.a.id, { state: s })} />
        <button class="nc-where" onClick={props.onJump} title="Tới vị trí trong bài">{props.section || props.a.anchor}</button>
        <span class="muted xs">{ago(props.a.updated_at)}</span>
        <button class="icon-btn sm" title="Sửa" onClick={() => setEditing(!editing())}><Icon name="edit" size={14} /></button>
        <button class="icon-btn sm" title="Xoá" onClick={() => confirm("Xoá ghi chú này?") && props.r.remove(props.a.id)}><Icon name="trash" size={14} /></button>
      </div>
      <Show when={props.a.quote}>
        <blockquote class="nc-quote" onClick={props.onJump}>{props.a.quote}</blockquote>
      </Show>
      <Show when={editing()} fallback={<Show when={props.a.body}><p class="nc-body">{props.a.body}</p></Show>}>
        <Composer body={props.a.body} onCancel={() => setEditing(false)} onSave={async (b) => { await props.r.update(props.a.id, { body: b }); setEditing(false); }} />
      </Show>
    </div>
  );
}

function Composer(props: { quote?: string; body?: string; onSave: (b: string) => Promise<unknown> | void; onCancel: () => void }) {
  const [body, setBody] = createSignal(props.body ?? "");
  const [busy, setBusy] = createSignal(false);
  let ta!: HTMLTextAreaElement;
  onMount(() => ta.focus());
  const save = async () => {
    setBusy(true);
    try {
      await props.onSave(body().trim());
    } finally {
      setBusy(false);
    }
  };
  return (
    <div class="composer">
      <Show when={props.quote}><blockquote class="nc-quote">{props.quote}</blockquote></Show>
      <textarea
        ref={ta}
        value={body()}
        placeholder="Cần sửa gì ở đoạn này…"
        onInput={(e) => setBody(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) save();
          if (e.key === "Escape") props.onCancel();
        }}
      />
      <div class="composer-act">
        <span class="muted xs hide-xs">Ctrl+Enter để lưu</span>
        <span class="grow" />
        <button class="btn ghost sm" onClick={props.onCancel}>Huỷ</button>
        <button class="btn primary sm" disabled={busy()} onClick={save}>Lưu</button>
      </div>
    </div>
  );
}

const whereLabel = (a: Annotation) => (a.anchor.startsWith("p") ? `Trang ${a.anchor.slice(1)}` : `Đoạn ${+a.anchor.slice(1) + 1}`);

/** Trang "Ghi chú": mọi ghi chú của user, gom theo file, lọc theo trạng thái — danh sách việc cần sửa vault. */
export function NotesPage(props: { onOpen: (path: string, mark: string) => void; onError: (m: string) => void }) {
  const [all, { mutate, refetch }] = createResource(() => api.allNotes(), { initialValue: [] });
  const [filter, setFilter] = createSignal<NoteState | "open" | "all">("open");
  const [q, setQ] = createSignal("");
  const match = (a: Annotation) => {
    const f = filter();
    if (f === "open" ? a.state === "done" : f !== "all" && a.state !== f) return false;
    const s = q().trim().toLowerCase();
    return !s || `${a.path} ${a.quote} ${a.body}`.toLowerCase().includes(s);
  };
  const groups = createMemo(() => {
    const m = new Map<string, Annotation[]>();
    for (const a of all().filter(match)) m.set(a.path, [...(m.get(a.path) ?? []), a]);
    const byPos = (a: Annotation, b: Annotation) => (+a.anchor.slice(1) || 0) - (+b.anchor.slice(1) || 0);
    return [...m.entries()].map(([path, l]) => ({ path, list: l.sort(byPos) })).sort((a, b) => a.path.localeCompare(b.path, undefined, { numeric: true }));
  });
  const n = (s: NoteState) => all().filter((a) => a.state === s).length;
  const run = async (f: () => Promise<void>) => {
    try {
      await f();
    } catch (e) {
      props.onError(errText(e));
      refetch();
    }
  };
  const setState = (a: Annotation, s: NoteState) =>
    run(async () => {
      const u = await api.updateAnnotation(a.id, { state: s });
      mutate((l) => l.map((x) => (x.id === a.id ? u : x)));
    });
  const remove = (a: Annotation) =>
    confirm("Xoá ghi chú này?") &&
    run(async () => {
      await api.removeAnnotation(a.id);
      mutate((l) => l.filter((x) => x.id !== a.id));
    });
  return (
    <div class="notes-page">
      <div class="toolbar np-bar">
        <h2 class="tb-title"><Icon name="note" /> Ghi chú</h2>
        <div class="chips">
          <button classList={{ on: filter() === "open" }} onClick={() => setFilter("open")}>Chưa xong {n("todo") + n("doing")}</button>
          <For each={["todo", "doing", "done"] as NoteState[]}>
            {(s) => (
              <button class={`st-${s}`} classList={{ on: filter() === s }} onClick={() => setFilter(s)} title={STATE_UI[s].label}>
                <Icon name={STATE_UI[s].icon} size={13} /> {n(s)}
              </button>
            )}
          </For>
          <button classList={{ on: filter() === "all" }} onClick={() => setFilter("all")}>Tất cả {all().length}</button>
        </div>
        <input class="np-search" placeholder="Lọc theo file, đoạn trích, nội dung…" value={q()} onInput={(e) => setQ(e.currentTarget.value)} />
      </div>
      <div class="scroll np-list">
        <For each={groups()} fallback={
          <Show when={!all.loading}>
            <div class="empty"><Icon name="note" size={48} /><p>{all().length ? "Không có ghi chú khớp bộ lọc" : "Chưa có ghi chú. Mở 1 file .md hoặc .pdf, bôi đen rồi bấm “Ghi chú”."}</p></div>
          </Show>
        }>
          {(g) => (
            <section class="np-group">
              <div class="np-file">
                <Icon name="text" size={16} />
                <b title={g.path}>{g.path.split("/").pop()}</b>
                <span class="muted xs np-dir">{g.path.split("/").slice(0, -1).join(" / ")}</span>
                <span class="rs-count">{g.list.length}</span>
              </div>
              <For each={g.list}>
                {(a) => (
                  <div class={`note-card st-${a.state}`}>
                    <div class="nc-head">
                      <StateButton state={a.state} onChange={(s) => setState(a, s)} />
                      <button class="nc-where" onClick={() => props.onOpen(a.path, a.anchor)} title="Mở tới vị trí này">{whereLabel(a)} →</button>
                      <span class="muted xs">{ago(a.updated_at)}</span>
                      <button class="icon-btn sm" title="Xoá" onClick={() => remove(a)}><Icon name="trash" size={14} /></button>
                    </div>
                    <Show when={a.quote}><blockquote class="nc-quote" onClick={() => props.onOpen(a.path, a.anchor)}>{a.quote}</blockquote></Show>
                    <Show when={a.body}><p class="nc-body">{a.body}</p></Show>
                  </div>
                )}
              </For>
            </section>
          )}
        </For>
      </div>
    </div>
  );
}
