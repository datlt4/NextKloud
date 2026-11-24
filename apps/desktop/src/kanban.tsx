import { marked } from "marked";
import { For, Show, createEffect, createMemo, createResource, createSignal, on, onCleanup } from "solid-js";
import { api, type DeckPreview, type KBoard, type KCard, type KCardPatch, type KColumn } from "./api";
import { fmtNum, loc, t } from "./i18n";
import { errText } from "./fileops";
import { Icon, kindOf } from "./icons";

const DONE_LIMIT = 10;
const fmtDay = (s: number) => new Date(s * 1000).toLocaleDateString(loc(), { day: "2-digit", month: "2-digit", year: "2-digit" });
const today0 = () => {
  const d = new Date();
  d.setHours(0, 0, 0, 0);
  return d.getTime() / 1000;
};
/** Checklist markdown trong mô tả: [đã xong, tổng]. */
const checklist = (md: string): [number, number] => {
  const all = md.match(/^\s*[-*+] \[[ xX]\]/gm) ?? [];
  return [all.filter((l) => /\[[xX]\]/.test(l)).length, all.length];
};

export function KanbanPage(props: { boardId: string | null; onBoard: (id: string | null) => void; onError: (m: string) => void; onOpenFile: (path: string) => void; flashId?: string | null; onFlashed?: () => void }) {
  const [boards, { refetch }] = createResource(() => api.boards(), { initialValue: [] });
  const [importing, setImporting] = createSignal(false);
  // có file xuất Deck mà user được nhập (admin hoặc chủ dữ liệu) thì mới hiện nút
  const [canImport] = createResource(() => api.deckPreview().then((l) => l.length > 0, () => false));
  const active = () => boards().filter((b) => !b.archived);
  const archived = () => boards().filter((b) => b.archived);
  // Chưa chọn board: mở board đầu tiên.
  createEffect(() => {
    const l = boards();
    if (!boards.loading && l.length && (!props.boardId || !l.some((b) => b.id === props.boardId))) props.onBoard((active()[0] ?? l[0]).id);
  });
  const run = async (f: () => Promise<unknown>) => {
    try {
      await f();
    } catch (e) {
      props.onError(errText(e));
    }
  };
  const newBoard = () => {
    const nb = prompt(t("kb.boardPrompt"))?.trim();
    if (nb) run(async () => {
      const b = await api.createBoard(nb);
      await refetch();
      props.onBoard(b.id);
    });
  };
  return (
    <div class="kanban">
      <div class="toolbar kb-bar">
        <h2 class="tb-title"><Icon name="kanban" /> Deck</h2>
        <div class="kb-boards">
          <For each={active()}>
            {(b) => <button class="chip" classList={{ on: b.id === props.boardId }} onClick={() => props.onBoard(b.id)}>{b.title}</button>}
          </For>
          <Show when={archived().length}>
            <select class="kb-arch" value="" onChange={(e) => e.currentTarget.value && props.onBoard(e.currentTarget.value)}>
              <option value="">{t("kb.archivedBoards", { n: fmtNum(archived().length) })}</option>
              <For each={archived()}>{(b) => <option value={b.id}>{b.title}</option>}</For>
            </select>
          </Show>
          <button class="btn ghost sm" onClick={newBoard}><Icon name="plus" size={15} /> {t("kb.newBoard")}</button>
          <Show when={canImport()}>
            <button class="btn ghost sm" onClick={() => setImporting(true)}>{t("kb.importDeck")}</button>
          </Show>
        </div>
      </div>
      <Show when={props.boardId ? boards().find((b) => b.id === props.boardId) : undefined} keyed>
        {(b) => <BoardView board={b} flashId={props.flashId} onFlashed={props.onFlashed} onOpenFile={props.onOpenFile} onError={props.onError} onChanged={refetch} onDeleted={() => { refetch(); props.onBoard(null); }} />}
      </Show>
      <Show when={!boards.loading && !boards().length}>
        <div class="empty"><Icon name="kanban" size={48} /><p>{t("kb.emptyBoard")}</p></div>
      </Show>
      <Show when={importing()}>
        <DeckImport onClose={() => setImporting(false)} onError={props.onError} onDone={async (id) => { setImporting(false); await refetch(); props.onBoard(id); }} />
      </Show>
    </div>
  );
}

function BoardView(props: { board: KBoard; flashId?: string | null; onFlashed?: () => void; onOpenFile: (path: string) => void; onError: (m: string) => void; onChanged: () => void; onDeleted: () => void }) {
  const [cols, setCols] = createSignal<KColumn[]>([]);
  const [cards, setCards] = createSignal<KCard[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [openId, setOpenId] = createSignal<string | null>(null);
  const [hideSub, setHideSub] = createSignal(localStorage.getItem("nk-kb-hidesub") === "1");
  const load = async () => {
    try {
      const v = await api.board(props.board.id, DONE_LIMIT);
      setCols(v.columns);
      setCards(v.cards);
    } catch (e) {
      props.onError(errText(e));
    } finally {
      setLoading(false);
    }
  };
  load();
  const run = async (f: () => Promise<unknown>) => {
    try {
      await f();
    } catch (e) {
      props.onError(errText(e));
      load();
    }
  };
  const byId = createMemo(() => new Map(cards().map((c) => [c.id, c])));
  // Mở từ Due tasks / link #id: mở thẳng drawer của task. Task nằm ngoài đợt tải thì tải thêm
  // từng đợt 50 tới khi thấy hoặc hết (dò mọi cột còn ẩn, không chỉ cột done).
  const moreTried = new Set<string>();
  const openById = (id: string) => {
    const c = cards().find((x) => x.id === id || x.id.startsWith(id));
    if (c) {
      setOpenId(c.id);
      props.onFlashed?.();
      return true;
    }
    return false;
  };
  createEffect(() => {
    const id = props.flashId;
    const list = cards();
    if (!id || loading() || !list.length) return;
    if (openById(id)) return;
    const dc = cols().find((c) => c.total - loadedIn(c) > 0);
    const key = `${id}:${dc?.id ?? ""}:${dc ? loadedIn(dc) : 0}`;
    if (dc && !moreTried.has(key)) {
      moreTried.add(key);
      more(dc);
    } else props.onFlashed?.();
  });
  // Drawer mở/đóng thì URL theo đúng task để copy link, dán link là mở lại đúng task đó.
  createEffect(() => {
    const id = openId();
    const h = id ? `#/kanban/${props.board.id}#${id.slice(0, 8)}` : `#/kanban/${props.board.id}`;
    if (location.hash !== h) history.replaceState(null, "", h);
  });
  const colCards = (c: KColumn) => {
    const l = cards().filter((k) => k.column_id === c.id && (!hideSub() || !k.parent_id));
    if (c.is_done) return l.sort((a, b) => (b.done_at ?? 0) - (a.done_at ?? 0));
    return c.is_cancel ? l.sort((a, b) => b.updated_at - a.updated_at) : l.sort((a, b) => a.position - b.position);
  };
  const loadedIn = (c: KColumn) => cards().filter((k) => k.column_id === c.id).length;
  const labelColor = (t: string) => props.board.labels.find((l) => l.title === t)?.color ?? "#8b8f98";
  const firstOpen = () => cols().find((c) => !c.is_done && !c.is_cancel);
  const cancelCol = (id: string) => !!cols().find((c) => c.id === id)?.is_cancel;
  /** Card đã huỷ: nằm trong cột huỷ, hoặc task cha nằm trong cột huỷ. */
  const cancelled = (k: KCard) => cancelCol(k.column_id) || (k.parent_id ? (byId().get(k.parent_id) ? cancelCol(byId().get(k.parent_id)!.column_id) : !!k.parent_cancelled) : false);
  const [colMenuFor, setColMenuFor] = createSignal<string | null>(null);
  const closeColMenu = (e: MouseEvent) => !(e.target as Element).closest?.(".kb-colmenu") && setColMenuFor(null);
  document.addEventListener("click", closeColMenu);
  onCleanup(() => document.removeEventListener("click", closeColMenu));
  const firstDone = () => cols().find((c) => c.is_done);

  /** Cập nhật 1 card ở local + tổng số của cột khi đổi cột. */
  const put = (c: KCard) => {
    const old = byId().get(c.id);
    if (old && old.column_id !== c.column_id) setCols((l) => l.map((k) => (k.id === old.column_id ? { ...k, total: k.total - 1 } : k.id === c.column_id ? { ...k, total: k.total + 1 } : k)));
    setCards((l) => (old ? l.map((x) => (x.id === c.id ? c : x)) : [...l, c]));
    // tiến độ subtask của task cha
    const pid = c.parent_id ?? old?.parent_id;
    if (pid && (old?.done_at ?? null) !== c.done_at) {
      const d = (c.done_at ? 1 : 0) - (old?.done_at ? 1 : 0);
      setCards((l) => l.map((x) => (x.id === pid ? { ...x, sub_done: x.sub_done + d } : x)));
    }
  };
  const patch = (id: string, b: KCardPatch) =>
    run(async () => {
      const cur = byId().get(id);
      if (cur) put({ ...cur, ...b } as KCard);
      put(await api.updateCard(id, b));
    });
  const addCard = (col: KColumn, title: string) =>
    run(async () => {
      const c = await api.createCard(col.id, title);
      setCols((l) => l.map((k) => (k.id === col.id ? { ...k, total: k.total + 1 } : k)));
      setCards((l) => [...l, c]);
    });
  const more = (c: KColumn) => run(async () => {
    const got = await api.columnCards(c.id, loadedIn(c), 50);
    const have = new Set(cards().map((x) => x.id));
    setCards((l) => [...l, ...got.filter((x) => !have.has(x.id))]);
  });
  const colMenu = (c: KColumn, act: string) =>
    run(async () => {
      if (act === "rename") {
        const cb = prompt(t("kb.colPrompt"), c.title)?.trim();
        if (!cb) return;
        await api.updateColumn(c.id, { title: cb });
      } else if (act === "done") {
        await api.updateColumn(c.id, { is_done: !c.is_done, ...(c.is_done ? {} : { is_cancel: false }) });
      } else if (act === "cancel") {
        await api.updateColumn(c.id, { is_cancel: !c.is_cancel, ...(c.is_cancel ? {} : { is_done: false }) });
      } else if (act === "left" || act === "right") {
        const l = cols();
        const i = l.findIndex((x) => x.id === c.id);
        const j = act === "left" ? i - 1 : i + 1;
        if (j < 0 || j >= l.length) return;
        await api.updateColumn(c.id, { position: l[j].position });
        await api.updateColumn(l[j].id, { position: c.position });
      } else if (act === "delete") {
        if (!confirm(t("kb.delColAsk", { title: c.title, n: c.total }))) return;
        await api.deleteColumn(c.id);
      }
      await load();
    });
  const boardMenu = (act: string) =>
    run(async () => {
      const b = props.board;
      if (act === "rename") {
        const bb = prompt(t("kb.boardPrompt"), b.title)?.trim();
        if (bb) await api.updateBoard(b.id, { title: bb });
      } else if (act === "archive") await api.updateBoard(b.id, { archived: !b.archived });
      else if (act === "delete") {
        if (!confirm(t("kb.delBoardAsk", { name: b.title }))) return;
        await api.deleteBoard(b.id);
        return props.onDeleted();
      } else if (act === "column") {
        const nc = prompt(t("kb.colPrompt"))?.trim();
        if (nc) await api.createColumn(b.id, nc);
        await load();
      }
      props.onChanged();
    });

  // ---------- kéo thả (pointer events: chuột kéo ngay, cảm ứng nhấn giữ) ----------
  const [dragId, setDragId] = createSignal<string | null>(null);
  const [drop, setDrop] = createSignal<{ col: string; index: number } | null>(null);
  let boardEl!: HTMLDivElement;
  let suppressClick = false;
  const startDrag = (e: PointerEvent, card: KCard, el: HTMLElement) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest("button, input, a, select, textarea")) return;
    const sx = e.clientX, sy = e.clientY;
    let ghost: HTMLElement | null = null;
    let timer: number | undefined;
    let x = sx, y = sy, scrollTick: number | undefined;
    const rect = el.getBoundingClientRect();
    const begin = () => {
      ghost = el.cloneNode(true) as HTMLElement;
      ghost.classList.add("kb-ghost");
      ghost.style.width = `${rect.width}px`;
      document.body.appendChild(ghost);
      setDragId(card.id);
      navigator.vibrate?.(15);
      place();
      scrollTick = window.setInterval(autoScroll, 30);
    };
    const place = () => {
      if (!ghost) return;
      ghost.style.transform = `translate(${x - (sx - rect.left)}px, ${y - (sy - rect.top)}px) rotate(2deg)`;
      const body = (document.elementFromPoint(x, y) as HTMLElement | null)?.closest<HTMLElement>(".kb-col-body");
      if (!body) return setDrop(null);
      const items = Array.from(body.querySelectorAll<HTMLElement>(".kb-card")).filter((n) => n.dataset.id !== card.id);
      const index = items.findIndex((n) => {
        const r = n.getBoundingClientRect();
        return y < r.top + r.height / 2;
      });
      setDrop({ col: body.dataset.col!, index: index < 0 ? items.length : index });
    };
    const autoScroll = () => {
      const r = boardEl.getBoundingClientRect();
      if (x < r.left + 60) boardEl.scrollLeft -= 14;
      else if (x > r.right - 60) boardEl.scrollLeft += 14;
      const body = (document.elementFromPoint(x, y) as HTMLElement | null)?.closest<HTMLElement>(".kb-col-body");
      if (body) {
        const b = body.getBoundingClientRect();
        if (y < b.top + 40) body.scrollTop -= 12;
        else if (y > b.bottom - 40) body.scrollTop += 12;
      }
    };
    const blockTouch = (ev: TouchEvent) => ghost && ev.preventDefault();
    const move = (ev: PointerEvent) => {
      x = ev.clientX;
      y = ev.clientY;
      const far = Math.hypot(x - sx, y - sy);
      if (!ghost) {
        if (e.pointerType === "touch") {
          if (far > 8) cleanup(); // đang cuộn, không phải kéo
        } else if (far > 5) begin();
        return;
      }
      place();
    };
    const up = () => {
      const target = drop();
      const was = !!ghost;
      cleanup();
      if (!was) return;
      suppressClick = true;
      setTimeout(() => (suppressClick = false), 50);
      if (target) commit(card, target.col, target.index);
    };
    const cleanup = () => {
      clearTimeout(timer);
      clearInterval(scrollTick);
      ghost?.remove();
      ghost = null;
      setDragId(null);
      setDrop(null);
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      document.removeEventListener("touchmove", blockTouch);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
    document.addEventListener("touchmove", blockTouch, { passive: false });
    if (e.pointerType === "touch") timer = window.setTimeout(begin, 350);
  };
  const commit = (card: KCard, colId: string, index: number) => {
    const col = cols().find((c) => c.id === colId);
    if (!col) return;
    const list = colCards(col).filter((c) => c.id !== card.id);
    if (col.is_done) {
      if (card.column_id !== colId) patch(card.id, { column_id: colId });
      return;
    }
    const prev = list[index - 1], next = list[index];
    const position = prev && next ? (prev.position + next.position) / 2 : prev ? prev.position + 1 : next ? next.position - 1 : 1;
    patch(card.id, { column_id: colId, position });
  };

  const [adding, setAdding] = createSignal<string | null>(null);
  return (
    <>
      <div class="kb-sub">
        <label class="kb-toggle"><input type="checkbox" checked={hideSub()} onChange={(e) => { setHideSub(e.currentTarget.checked); localStorage.setItem("nk-kb-hidesub", e.currentTarget.checked ? "1" : "0"); }} /> {t("kb.hideSub")}</label>
        <span class="grow" />
        <select class="kb-menu" value="" onChange={(e) => { const v = e.currentTarget.value; e.currentTarget.value = ""; boardMenu(v); }}>
          <option value="">{t("kb.boardMenu")} ▾</option>
          <option value="column">+ {t("kb.addColumn")}</option>
          <option value="rename">{t("kb.renameBoard")}</option>
          <option value="archive">{props.board.archived ? t("kb.unarchiveBoard") : t("kb.archiveBoard")}</option>
          <option value="delete">{t("kb.delBoard")}</option>
        </select>
      </div>
      <Show when={!loading()} fallback={<p class="muted pad">{t("c.loading")}</p>}>
        <div class="kb-board" ref={boardEl}>
          <For each={cols()}>
            {(c) => {
              const list = () => colCards(c);
              const hidden = () => (c.is_done || c.is_cancel ? c.total - loadedIn(c) : 0);
              const act = (a: string) => {
                setColMenuFor(null);
                colMenu(c, a);
              };
              return (
                <section class="kb-col" classList={{ done: c.is_done, cancel: c.is_cancel, over: drop()?.col === c.id }}>
                  <header class="kb-col-head">
                    <Show when={c.is_done}><Icon name="check" size={15} class="ic kb-done-ic" /></Show>
                    <Show when={c.is_cancel}><Icon name="close" size={15} class="ic kb-cancel-ic" /></Show>
                    <b title={c.title}>{c.title}</b>
                    <span class="kb-count">{c.total}</span>
                    <span class="grow" />
                    <div class="kb-colmenu menu-wrap">
                      <button class="icon-btn sm" title={t("kb.colMenu")} onClick={() => setColMenuFor(colMenuFor() === c.id ? null : c.id)}><Icon name="more" size={16} /></button>
                      <Show when={colMenuFor() === c.id}>
                        <div class="menu">
                          <button onClick={() => act("rename")}><Icon name="edit" size={15} /> {t("c.rename")}</button>
                          <button onClick={() => act("done")}><Icon name="check" size={15} /> {c.is_done ? t("kb.unsetDone") : t("kb.setDone")}</button>
                          <button onClick={() => act("cancel")}><Icon name="close" size={15} /> {c.is_cancel ? t("kb.unsetCancel") : t("kb.setCancel")}</button>
                          <div class="menu-sep" />
                          <button onClick={() => act("left")}><Icon name="back" size={15} /> {t("kb.moveLeft")}</button>
                          <button onClick={() => act("right")}><Icon name="next" size={15} /> {t("kb.moveRight")}</button>
                          <div class="menu-sep" />
                          <button class="danger" onClick={() => act("delete")}><Icon name="trash" size={15} /> {t("kb.delCol")}</button>
                        </div>
                      </Show>
                    </div>
                  </header>
                  <div class="kb-col-body" data-col={c.id}>
                    <For each={list()}>
                      {(k, i) => (
                        <>
                          <Show when={drop()?.col === c.id && drop()!.index === i() && dragId() !== k.id}><div class="kb-drop" /></Show>
                          <CardItem
                            card={k}
                            parent={k.parent_id ? byId().get(k.parent_id) : undefined}
                            cancelled={cancelled(k)}
                            labelColor={labelColor}
                            dragging={dragId() === k.id}
                            onDown={(e, el) => startDrag(e, k, el)}
                            onOpen={() => !suppressClick && setOpenId(k.id)}
                          />
                        </>
                      )}
                    </For>
                    <Show when={drop()?.col === c.id && drop()!.index >= list().filter((k) => k.id !== dragId()).length}><div class="kb-drop" /></Show>
                    <Show when={hidden() > 0 && !hideSub()}>
                      <button class="kb-more" onClick={() => more(c)}>{c.is_done ? t("kb.olderDone") : t("kb.older")} ({fmtNum(hidden())}) — {t("kb.loadMore")}</button>
                    </Show>
                  </div>
                  <Show when={!c.is_done}>
                    <Show when={adding() === c.id} fallback={<button class="kb-add" onClick={() => setAdding(c.id)}><Icon name="plus" size={15} /> {t("kb.addCard")}</button>}>
                      <textarea
                        class="kb-new"
                        placeholder={t("kb.cardPh")}
                        ref={(el) => queueMicrotask(() => el.focus())}
                        onKeyDown={(e) => {
                          if (e.key === "Enter" && !e.shiftKey) {
                            e.preventDefault();
                            const t = e.currentTarget.value.trim();
                            if (t) addCard(c, t);
                            e.currentTarget.value = "";
                          }
                          if (e.key === "Escape") setAdding(null);
                        }}
                        onBlur={() => setAdding(null)}
                      />
                    </Show>
                  </Show>
                </section>
              );
            }}
          </For>
        </div>
      </Show>
      <Show when={openId() ? byId().get(openId()!) : undefined} keyed>
        {(k) => (
          <CardDrawer
            card={k}
            board={props.board}
            cols={cols()}
            cards={cards()}
            labelColor={labelColor}
            onClose={() => setOpenId(null)}
            onOpen={(id) => setOpenId(id)}
            onPatch={(b) => patch(k.id, b)}
            onPut={put}
            onDelete={() =>
              run(async () => {
                await api.deleteCard(k.id);
                setOpenId(null);
                await load();
              })
            }
            firstOpen={firstOpen()}
            firstDone={firstDone()}
            cancelled={cancelCol(k.column_id)}
            onOpenFile={props.onOpenFile}
            onError={props.onError}
          />
        )}
      </Show>
    </>
  );
}

function CardItem(props: { card: KCard; parent?: KCard; cancelled?: boolean; labelColor: (t: string) => string; dragging: boolean; onDown: (e: PointerEvent, el: HTMLElement) => void; onOpen: () => void }) {
  const k = () => props.card;
  const cl = () => checklist(k().description);
  const overdue = () => !k().done_at && k().due_at != null && k().due_at! < today0();
  let el!: HTMLDivElement;
  return (
    <div class="kb-card" ref={el} data-id={k().id} classList={{ dragging: props.dragging, done: !!k().done_at, cancel: !!props.cancelled }} style={k().color ? { "border-left-color": k().color! } : undefined} onPointerDown={(e) => props.onDown(e, el)} onClick={props.onOpen}>
      <Show when={k().labels.length}>
        <div class="kb-labels">
          <For each={k().labels}>{(l) => <span class="kb-label" style={{ background: props.labelColor(l) }} title={l}>{l}</span>}</For>
        </div>
      </Show>
      <Show when={props.parent?.title ?? k().parent_title}>{(label) => <div class="kb-parent" title={t("kb.parentTip")}><Icon name="sub" size={12} /> {label()}</div>}</Show>
      <div class="kb-title"><Show when={props.cancelled}><Icon name="close" size={13} class="ic kb-x" /></Show>{k().title}</div>
      <Show when={k().due_at || k().sub_total || cl()[1] || k().description || k().attachments.length}>
        <div class="kb-meta">
          <Show when={k().due_at}><span classList={{ "kb-due": true, late: overdue() }}><Icon name="calendar" size={12} /> {fmtDay(k().due_at!)}</span></Show>
          <Show when={k().sub_total}>
            <span class="kb-prog" title={t("kb.subDoneTip")}>
              <Icon name="check" size={12} /> {k().sub_done}/{k().sub_total}
              <i style={{ width: `${(k().sub_done / k().sub_total) * 100}%` }} />
            </span>
          </Show>
          <Show when={cl()[1]}><span title={t("kb.checklistTip")}>☑ {fmtNum(cl()[0])}/{fmtNum(cl()[1])}</span></Show>
          <Show when={k().description.trim() && !cl()[1]}><Icon name="text" size={12} class="ic muted" /></Show>
          <Show when={k().attachments.length}><span title={t("kb.atchTip")}><Icon name="clip" size={12} /> {fmtNum(k().attachments.length)}</span></Show>
        </div>
      </Show>
    </div>
  );
}

function CardDrawer(props: {
  card: KCard;
  board: KBoard;
  cols: KColumn[];
  cards: KCard[];
  labelColor: (t: string) => string;
  onClose: () => void;
  onOpen: (id: string) => void;
  onPatch: (b: KCardPatch) => void;
  onPut: (c: KCard) => void;
  onDelete: () => void;
  firstOpen?: KColumn;
  firstDone?: KColumn;
  /** Card đang nằm trong cột huỷ -> subtask hiện là đã huỷ. */
  cancelled: boolean;
  onOpenFile: (path: string) => void;
  onError: (m: string) => void;
}) {
  const k = () => props.card;
  const [attaching, setAttaching] = createSignal(false);
  let attachInput!: HTMLInputElement;
  /** Tải file lên thư mục Deck/ (trùng tên thì thêm số) rồi gắn vào card. */
  const attach = async (list: FileList | null) => {
    if (!list?.length) return;
    setAttaching(true);
    try {
      const added: string[] = [];
      for (const f of Array.from(list)) {
        const dot = f.name.lastIndexOf(".");
        const [base, ext] = dot > 0 ? [f.name.slice(0, dot), f.name.slice(dot)] : [f.name, ""];
        let p = `Deck/${f.name}`;
        for (let i = 2; await api.exists(p).then(() => true, () => false); i++) p = `Deck/${base} (${i})${ext}`;
        await api.upload(p, f);
        added.push(p);
      }
      props.onPatch({ attachments: [...k().attachments, ...added] });
    } catch (e) {
      props.onError(errText(e));
    } finally {
      setAttaching(false);
      attachInput.value = "";
    }
  };
  const [editDesc, setEditDesc] = createSignal(false);
  const [subs, { mutate: setSubs, refetch: refetchSubs }] = createResource(() => k().id, (id) => api.subtasks(id), { initialValue: [] });
  // ô chọn task cha có tìm kiếm (hỏi server: gồm cả task chưa tải, vd sâu trong cột xong)
  const [pq, setPq] = createSignal("");
  const [pickOpen, setPickOpen] = createSignal(false);
  const [hits] = createResource(() => (pickOpen() ? pq() : null), (q) => api.parentSearch(props.board.id, q).catch(() => []), { initialValue: [] });
  const parentTitle = () => (k().parent_id ? props.cards.find((c) => c.id === k().parent_id)?.title ?? k().parent_title ?? "…" : "");
  const urls = createMemo(() => [...new Set(k().description.match(/https?:\/\/[^\s<>()\[\]"']+/g) ?? [])].map((u) => u.replace(/[.,;:!?]+$/, "")).slice(0, 6));
  const html = createMemo(() => (marked.parse(k().description || "") as string).replace(/<input (checked="" )?disabled="" type="checkbox">/g, (_m, on) => `<input ${on ? "checked " : ""}type="checkbox">`));
  // Bấm checkbox trong mô tả -> đổi đúng dòng thứ n "[ ]" <-> "[x]".
  const toggleCheck = (n: number) => {
    let i = -1;
    const d = k().description.replace(/^(\s*[-*+] )\[([ xX])\]/gm, (m, pre, v) => (++i === n ? `${pre}[${v === " " ? "x" : " "}]` : m));
    props.onPatch({ description: d });
  };
  const run = async (f: () => Promise<unknown>) => {
    try {
      await f();
    } catch (e) {
      props.onError(errText(e));
    }
  };
  const setSubDone = (s: KCard, done: boolean) =>
    run(async () => {
      const col = done ? props.firstDone : props.firstOpen;
      if (!col) return props.onError(done ? t("kb.noDoneCol") : t("kb.noOpenCol"));
      const u = await api.updateCard(s.id, { column_id: col.id });
      setSubs((l) => l.map((x) => (x.id === s.id ? u : x)));
      props.onPut(u);
      props.onPut({ ...k(), sub_done: k().sub_done + (done ? 1 : -1) });
    });
  const addSub = (title: string) =>
    run(async () => {
      if (!props.firstOpen) return;
      const c = await api.createCard(props.firstOpen.id, title, k().id);
      props.onPut(c);
      props.onPut({ ...k(), sub_total: k().sub_total + 1 });
      refetchSubs();
    });
  /** Subtask đã huỷ: task cha (card này) ở cột huỷ, hoặc chính subtask ở cột huỷ. */
  const subX = (s: KCard) => props.cancelled || !!props.cols.find((c) => c.id === s.column_id)?.is_cancel;
  const onKey = (e: KeyboardEvent) => e.key === "Escape" && !(e.target as HTMLElement).closest("textarea, input") && props.onClose();
  document.addEventListener("keydown", onKey);
  onCleanup(() => document.removeEventListener("keydown", onKey));
  createEffect(on(() => k().id, () => setEditDesc(false)));
  return (
    <div class="kb-drawer-bg" onClick={props.onClose}>
      <aside class="kb-drawer glass" onClick={(e) => e.stopPropagation()}>
        <div class="kb-dr-head">
          <Show when={k().parent_id}>
            {(pid) => <button class="kb-parent link" onClick={() => props.onOpen(pid())}><Icon name="sub" size={13} /> {parentTitle()}</button>}
          </Show>
          <span class="grow" />
          <button class="link mono muted sm" title={`${t("kb.copyId")}: ${k().id}`} onClick={() => navigator.clipboard?.writeText(k().id).catch(() => {})}>#{k().id.slice(0, 8)}</button>
          <button class="icon-btn" onClick={props.onClose} title={t("c.close")}><Icon name="close" /></button>
        </div>
        <textarea class="kb-dr-title" value={k().title} rows={1} onChange={(e) => e.currentTarget.value.trim() && props.onPatch({ title: e.currentTarget.value.trim() })} onKeyDown={(e) => e.key === "Enter" && (e.preventDefault(), e.currentTarget.blur())} />
        <div class="kb-fields">
          <label>
            <span>{t("kb.column")}</span>
            <select value={k().column_id} onChange={(e) => props.onPatch({ column_id: e.currentTarget.value })}>
              <For each={props.cols}>{(c) => <option value={c.id}>{c.title}</option>}</For>
            </select>
          </label>
          <label>
            <span>{t("kb.due")}</span>
            <input
              type="date"
              value={k().due_at ? new Date(k().due_at! * 1000).toISOString().slice(0, 10) : ""}
              onChange={(e) => props.onPatch({ due_at: e.currentTarget.value ? Math.floor(new Date(e.currentTarget.value + "T12:00:00").getTime() / 1000) : null })}
            />
          </label>
          <Show when={!k().sub_total}>
            <div class="wide kb-pick">
              <span>{t("kb.parent")}</span>
              <input
                value={pickOpen() ? pq() : parentTitle()}
                placeholder={t("kb.noParentPh")}
                onFocus={(e) => { setPq(""); setPickOpen(true); e.currentTarget.select(); }}
                onInput={(e) => setPq(e.currentTarget.value)}
                onBlur={() => setTimeout(() => setPickOpen(false), 150)}
                onKeyDown={(e) => e.key === "Escape" && e.currentTarget.blur()}
              />
              <Show when={pickOpen()}>
                <div class="kb-pick-list">
                  <button onMouseDown={(e) => { e.preventDefault(); props.onPatch({ parent_id: null }); setPickOpen(false); }}>{t("kb.noParentBtn")}</button>
                  <For each={hits().filter((h) => h.id !== k().id)} fallback={<p class="muted xs pad">{hits.loading ? t("c.loading") : t("kb.noHit")}</p>}>
                    {(h) => (
                      <button classList={{ on: h.id === k().parent_id, done: h.done }} onMouseDown={(e) => { e.preventDefault(); props.onPatch({ parent_id: h.id }); setPickOpen(false); }}>
                        {h.title}
                      </button>
                    )}
                  </For>
                </div>
              </Show>
            </div>
          </Show>
        </div>
        <Show when={props.board.labels.length}>
          <div class="kb-dr-labels">
            <For each={props.board.labels}>
              {(l) => {
                const on = () => k().labels.includes(l.title);
                return (
                  <button class="kb-label pick" classList={{ off: !on() }} style={{ background: l.color }} onClick={() => props.onPatch({ labels: on() ? k().labels.filter((x) => x !== l.title) : [...k().labels, l.title] })}>
                    {l.title}
                  </button>
                );
              }}
            </For>
          </div>
        </Show>
        <div class="kb-sec"><b>{t("kb.desc")}</b><button class="btn ghost sm" onClick={() => setEditDesc(!editDesc())}>{editDesc() ? t("kb.viewBtn") : t("kb.editBtn")}</button></div>
        <Show
          when={editDesc()}
          fallback={
            <div
              class="kb-desc md-body"
              innerHTML={html() || `<p class="muted">${t("kb.noDesc")}</p>`}
              onClick={(e) => {
                const t = e.target as HTMLElement;
                if (t instanceof HTMLInputElement && t.type === "checkbox") {
                  e.preventDefault();
                  toggleCheck(Array.from(e.currentTarget.querySelectorAll("input[type=checkbox]")).indexOf(t));
                } else if (!t.closest("a")) setEditDesc(true);
              }}
            />
          }
        >
          <textarea class="kb-desc-edit" value={k().description} placeholder={t("kb.editDescPh")} ref={(el) => queueMicrotask(() => el.focus())} onBlur={(e) => e.currentTarget.value !== k().description && props.onPatch({ description: e.currentTarget.value })} />
        </Show>
        <Show when={urls().length && !editDesc()}>
          <div class="kb-links">
            <For each={urls()}>{(u) => <LinkCard url={u} />}</For>
          </div>
        </Show>
        <div class="kb-sec">
          <b>{t("kb.atchTitle", { n: fmtNum(k().attachments.length) })}</b>
          <span class="grow" />
          <button class="btn ghost sm" disabled={attaching()} onClick={() => attachInput.click()}><Icon name="clip" size={14} /> {attaching() ? t("kb.uploading") : t("kb.addFile")}</button>
          <input ref={attachInput} type="file" multiple hidden onChange={(e) => attach(e.currentTarget.files)} />
        </div>
        <Show when={k().attachments.length}>
          <div class="kb-atts">
            <For each={k().attachments}>
              {(p) => {
                const name = p.split("/").pop() ?? p;
                const img = /\.(png|jpe?g|gif|webp|bmp)$/i.test(name);
                return (
                  <div class="kb-att">
                    <button class="kb-att-open" onClick={() => props.onOpenFile(p)} title={p}>
                      <Show when={img} fallback={<span class="kb-att-ic" style={{ color: kindOf(name, false).color }}><Icon name={kindOf(name, false).icon} size={18} /></span>}>
                        <img src={api.thumbUrl(p, 128)} alt="" loading="lazy" />
                      </Show>
                      <span class="kb-att-name">{name}</span>
                    </button>
                    <button class="icon-btn sm" title={t("kb.unattach")} onClick={() => props.onPatch({ attachments: k().attachments.filter((x) => x !== p) })}>
                      <Icon name="close" size={13} />
                    </button>
                  </div>
                );
              }}
            </For>
          </div>
        </Show>
        <Show when={!k().parent_id}>
          <div class="kb-sec"><b>{t("kb.subtasks")} {k().sub_total ? `${fmtNum(k().sub_done)}/${fmtNum(k().sub_total)}` : ""}</b></div>
          <div class="kb-subs">
            <For each={subs()}>
              {(s) => (
                <div class="kb-subrow" classList={{ done: !!s.done_at && !subX(s), cancel: subX(s) }}>
                  <Show when={!subX(s)} fallback={<span class="kb-x-box" title={t("kb.cancelledTip")}><Icon name="close" size={13} /></span>}>
                    <input type="checkbox" checked={!!s.done_at} onChange={(e) => setSubDone(s, e.currentTarget.checked)} />
                  </Show>
                  <button class="link" onClick={() => props.onOpen(s.id)}>{s.title}</button>
                  <span class="muted xs">{props.cols.find((c) => c.id === s.column_id)?.title}</span>
                </div>
              )}
            </For>
            <input
              class="kb-subnew"
              placeholder={t("kb.subPh")}
              onKeyDown={(e) => {
                if (e.key === "Enter" && e.currentTarget.value.trim()) {
                  addSub(e.currentTarget.value.trim());
                  e.currentTarget.value = "";
                }
              }}
            />
          </div>
        </Show>
        <div class="kb-dr-foot">
          <span class="muted xs">{t("kb.createdAt", { d: fmtDay(k().created_at) })}{k().done_at ? ` · ${t("tk.doneAt", { d: fmtDay(k().done_at!) })}` : ""}</span>
          <span class="grow" />
          <button class="btn ghost sm danger" onClick={() => confirm(t("kb.delCardAsk")) && props.onDelete()}><Icon name="trash" size={14} /> {t("c.delete")}</button>
        </div>
      </aside>
    </div>
  );
}

/** Xem trước link (ảnh, tiêu đề, tên trang) như Google Keep. */
function LinkCard(props: { url: string }) {
  const [p] = createResource(() => props.url, (u) => api.linkPreview(u).catch(() => null));
  const host = () => {
    try {
      return new URL(props.url).host.replace(/^www\./, "");
    } catch {
      return props.url;
    }
  };
  const [imgOk, setImgOk] = createSignal(true);
  return (
    <a class="kb-link" href={props.url} target="_blank" rel="noopener noreferrer">
      <Show when={p()?.image && imgOk()} fallback={<span class="kb-link-ph">{(p()?.site ?? host()).slice(0, 1).toUpperCase()}</span>}>
        <img src={p()!.image!} alt="" loading="lazy" referrerpolicy="no-referrer" onError={() => setImgOk(false)} />
      </Show>
      <span class="kb-link-txt">
        <b>{p.loading ? t("c.loading") : p()?.title ?? props.url}</b>
        <span class="muted xs">{p()?.site ?? host()}</span>
      </span>
    </a>
  );
}

/** Nhập Deck: xem đề xuất gom subtask, sửa task cha từng card, bỏ cột không cần, rồi mới ghi. */
function DeckImport(props: { onClose: () => void; onDone: (boardId: string) => void; onError: (m: string) => void }) {
  const [data] = createResource(() => api.deckPreview().catch((e) => (props.onError(errText(e)), [] as DeckPreview[])));
  const [sel, setSel] = createSignal<number | null>(null);
  const [parents, setParents] = createSignal<Record<number, number | null>>({});
  const [skip, setSkip] = createSignal<Set<number>>(new Set());
  const [busy, setBusy] = createSignal(false);
  const board = () => data()?.find((b) => b.id === sel());
  createEffect(() => {
    const l = data();
    if (l?.length && sel() == null) setSel((l.find((b) => !b.imported) ?? l[0]).id);
  });
  createEffect(
    on(board, (b) => {
      if (!b) return;
      setParents(Object.fromEntries(b.cards.filter((c) => !c.big).map((c) => [c.id, c.parent])));
      setSkip(new Set<number>());
    }),
  );
  const stackOf = (id: number) => board()?.stacks.find((s) => s.id === id);
  const bigs = () => board()?.cards.filter((c) => c.big) ?? [];
  const kids = (pid: number) => board()?.cards.filter((c) => !c.big && parents()[c.id] === pid) ?? [];
  const orphans = () => board()?.cards.filter((c) => !c.big && parents()[c.id] == null && stackOf(c.stack_id)?.merge_into) ?? [];
  const setP = (id: number, v: string) => setParents({ ...parents(), [id]: v ? +v : null });
  const apply = async () => {
    const b = board();
    if (!b) return;
    setBusy(true);
    try {
      const r = await api.deckApply(b.id, parents(), [...skip()]);
      alert(t("kb.importedOk", { c: r.cards, s: r.subtasks }));
      props.onDone(r.board);
    } catch (e) {
      props.onError(errText(e));
    } finally {
      setBusy(false);
    }
  };
  const Row = (p: { c: DeckPreview["cards"][number] }) => (
    <div class="dk-row">
      <span class="dk-t" title={p.c.title}>{p.c.title}</span>
      <span class="muted xs">{stackOf(p.c.stack_id)?.title}</span>
      <select value={parents()[p.c.id] ?? ""} onChange={(e) => setP(p.c.id, e.currentTarget.value)}>
        <option value="">{t("kb.noParentOpt")}</option>
        <For each={bigs()}>{(b) => <option value={b.id}>{b.title}</option>}</For>
      </select>
    </div>
  );
  return (
    <div class="modal-bg" onClick={props.onClose}>
      <div class="modal glass dk" onClick={(e) => e.stopPropagation()}>
        <div class="modal-head">
          <h3>{t("kb.impTitle")}</h3>
          <button class="icon-btn" onClick={props.onClose}><Icon name="close" /></button>
        </div>
        <Show when={!data.loading} fallback={<p class="muted">{t("c.loading")}</p>}>
          <div class="chips">
            <For each={data()}>
              {(b) => (
                <button classList={{ on: sel() === b.id }} onClick={() => setSel(b.id)}>
                  {b.title} #{b.id}{b.archived ? t("kb.archTag") : ""} · {fmtNum(b.cards.length)}{b.imported ? t("kb.importedTag") : ""}
                </button>
              )}
            </For>
          </div>
          <Show when={board()}>
            {(b) => (
              <>
                <p class="muted sm">{t("kb.impHelp", { n: DONE_LIMIT })}</p>
                <div class="dk-stacks">
                  <For each={b().stacks}>
                    {(s) => (
                      <label>
                        <input type="checkbox" checked={!skip().has(s.id)} onChange={(e) => { const n = new Set(skip()); e.currentTarget.checked ? n.delete(s.id) : n.add(s.id); setSkip(n); }} />
                        {s.title} ({s.count}){s.merge_into ? ` → ${s.merge_into}` : ""}{s.is_done ? " ✓" : ""}
                      </label>
                    )}
                  </For>
                </div>
                <div class="dk-groups">
                  <For each={bigs()}>
                    {(g) => (
                      <details class="dk-group" open={kids(g.id).length > 0 && kids(g.id).length <= 6}>
                        <summary><b>{g.title}</b> <span class="muted xs">{kids(g.id).length} subtask</span></summary>
                        <For each={kids(g.id)}>{(c) => <Row c={c} />}</For>
                      </details>
                    )}
                  </For>
                  <Show when={orphans().length}>
                    <details class="dk-group" open>
                      <summary><b>{t("kb.orphans")}</b> <span class="muted xs">{fmtNum(orphans().length)}</span></summary>
                      <For each={orphans()}>{(c) => <Row c={c} />}</For>
                    </details>
                  </Show>
                </div>
                <div class="modal-foot">
                  <span class="muted sm">{t("kb.willImport", { n: fmtNum(b().cards.filter((c) => !skip().has(c.stack_id)).length) })}</span>
                  <span class="grow" />
                  <button class="btn primary" disabled={busy() || b().imported} onClick={apply}>{b().imported ? t("kb.imported") : busy() ? t("kb.importing") : t("kb.doImport")}</button>
                </div>
              </>
            )}
          </Show>
        </Show>
      </div>
    </div>
  );
}
