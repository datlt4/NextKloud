import { For, Show, createMemo, createResource, createSignal, onCleanup, onMount } from "solid-js";
import { api, type Habit, type TaskItem, type TaskView } from "./api";
import { fmtNum, loc, t } from "./i18n";
import { errText } from "./fileops";
import { Icon } from "./icons";

const fmtDay = (s: number) => new Date(s * 1000).toLocaleDateString(loc(), { day: "2-digit", month: "2-digit" });
const dayStr = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
const todayStr = () => dayStr(new Date());

const VIEWS: TaskView[] = ["overdue", "today", "week", "all", "done"];

export function TrackerPage(props: { onOpenBoard: (boardId: string, cardId?: string) => void; onError: (m: string) => void }) {
  const [tab, setTab] = createSignal<"tasks" | "habits">("tasks");
  return (
    <div class="tracker" data-tab={tab()}>
      <div class="toolbar kb-bar">
        <h2 class="tb-title"><Icon name="check" /> {t("tk.title")}</h2>
        <div class="kb-boards">
          <button class="chip" classList={{ on: tab() === "tasks" }} onClick={() => setTab("tasks")}>{t("tk.tasks")}</button>
          <button class="chip" classList={{ on: tab() === "habits" }} onClick={() => setTab("habits")}>{t("tk.habits")}</button>
        </div>
      </div>
      <div class="tk-cols">
        <div class="tk-col tk-col-tasks">
          <h3 class="tk-col-title">{t("tk.tasks")}</h3>
          <TasksView onOpenBoard={props.onOpenBoard} onError={props.onError} />
        </div>
        <div class="tk-col tk-col-habits">
          <h3 class="tk-col-title">{t("tk.habits")}</h3>
          <HabitsView onError={props.onError} />
        </div>
      </div>
    </div>
  );
}

function TasksView(props: { onOpenBoard: (boardId: string, cardId?: string) => void; onError: (m: string) => void }) {
  const [view, setView] = createSignal<TaskView>("overdue");
  const [items, { refetch }] = createResource(view, (v) => api.tasks(v).catch((e) => (props.onError(errText(e)), [] as TaskItem[])), { initialValue: [] });
  const groups = createMemo(() => {
    const m = new Map<string, TaskItem[]>();
    for (const t of items()) {
      const k = t.board_title;
      if (!m.has(k)) m.set(k, []);
      m.get(k)!.push(t);
    }
    return [...m];
  });
  return (
    <div>
      <div class="tk-views">
        <For each={VIEWS}>
          {(v) => <button class="chip" classList={{ on: view() === v }} onClick={() => setView(v)}>{t(`tk.${v}`)}</button>}
        </For>
        <span class="grow" />
        <button class="btn ghost sm" onClick={() => refetch()} title={t("tk.reload")}><Icon name="menu" size={14} /> {t("tk.reload")}</button>
      </div>
      <Show when={!items.loading} fallback={<p class="muted pad">{t("c.loading")}</p>}>
        <Show when={items().length} fallback={<div class="empty"><Icon name="check" size={48} /><p>{view() === "done" ? t("tk.emptyDone") : t("tk.emptyOpen")}</p></div>}>
          <For each={groups()}>
            {([board, list]) => (
              <section class="tk-group">
                <h3 class="tk-board">{board} <span class="muted xs">{list.length}</span></h3>
                <For each={list}>
                  {(it: TaskItem) => (
                    <div class="tk-row" classList={{ done: !!it.done_at, late: !it.done_at && (it.due_at ?? 0) * 1000 < new Date().setHours(0, 0, 0, 0) }}>
                      <Show when={it.done_at} fallback={<Icon name="calendar" size={15} class="ic muted" />}><Icon name="check" size={15} class="ic kb-done-ic" /></Show>
                      <div class="tk-main">
                        <b>{it.title}</b>
                        <span class="muted xs">{it.column_title}{it.due_at ? ` · ${t("tk.dueAt", { d: fmtDay(it.due_at) })}` : ""}{it.done_at ? ` · ${t("tk.doneAt", { d: fmtDay(it.done_at) })}` : ""}</span>
                      </div>
                      <button class="btn ghost sm" onClick={() => props.onOpenBoard(it.board_id, it.id)}>{t("tk.openBoard")}</button>
                    </div>
                  )}
                </For>
              </section>
            )}
          </For>
        </Show>
      </Show>
    </div>
  );
}

function HabitsView(props: { onError: (m: string) => void }) {
  const [list, { refetch, mutate }] = createResource(() => api.habits().catch((e) => (props.onError(errText(e)), [] as Habit[])), { initialValue: [] });
  const [name, setName] = createSignal("");
  const [showArch, setShowArch] = createSignal(false);
  const visible = () => list().filter((h) => showArch() || !h.archived);
  const run = async (f: () => Promise<unknown>) => {
    try {
      await f();
    } catch (e) {
      props.onError(errText(e));
    }
    refetch();
  };
  const add = () => {
    const t = name().trim();
    if (!t) return;
    setName("");
    run(() => api.createHabit(t));
  };
  const toggle = (h: Habit, day: string, on: boolean) =>
    run(async () => {
      mutate((l) => l.map((x) => (x.id === h.id ? { ...x, days: on ? [...x.days, day] : x.days.filter((d) => d !== day), done_today: day === todayStr() ? on : x.done_today } : x)));
      await api.habitCheck(h.id, day, on);
    });
  return (
    <div>
      <div class="tk-views">
        <input class="kb-new tk-add" placeholder={t("tk.addPh")} value={name()} onInput={(e) => setName(e.currentTarget.value)} onKeyDown={(e) => e.key === "Enter" && add()} />
        <label class="kb-toggle"><input type="checkbox" checked={showArch()} onChange={(e) => setShowArch(e.currentTarget.checked)} /> {t("tk.showArch")}</label>
      </div>
      <Show when={!list.loading} fallback={<p class="muted pad">{t("c.loading")}</p>}>
        <Show when={visible().length} fallback={<div class="empty"><Icon name="check" size={48} /><p>{t("tk.emptyHabit")}</p></div>}>
          <For each={visible()}>{(h) => <HabitCard h={h} onToggle={(d, on) => toggle(h, d, on)} onPatch={(b) => run(() => api.updateHabit(h.id, b))} onDelete={() => confirm(t("tk.delAsk", { name: h.title })) && run(() => api.deleteHabit(h.id))} />}</For>
        </Show>
      </Show>
    </div>
  );
}

function HabitDayCell(props: { ds: string; days: () => Set<string>; onToggle: (day: string, on: boolean) => void }) {
  const label = () => {
    const [y, m, d] = props.ds.split("-").map(Number);
    return new Date(y, m - 1, d).toLocaleDateString(loc(), { day: "2-digit", month: "2-digit" });
  };
  const fut = () => props.ds > todayStr();
  const on = () => props.days().has(props.ds);
  return (
    <button
      class="hb-cell"
      role="gridcell"
      classList={{ on: on(), future: fut() }}
      title={`${label()}${on() ? t("tk.cellDone") : ""}`}
      disabled={fut()}
      onClick={() => props.onToggle(props.ds, !on())}
    />
  );
}
const HB_CELL = 12, HB_GAP = 3, HB_DAYW = 30, HB_WEEKS_MIN = 13, HB_WEEKS_MAX = 260;
function HabitCard(props: { h: Habit; onToggle: (day: string, on: boolean) => void; onPatch: (b: { title?: string; archived?: boolean }) => void; onDelete: () => void }) {
  const done = () => new Set(props.h.days);
  const [wrapW, setWrapW] = createSignal(0);
  let wrapEl!: HTMLDivElement;
  onMount(() => {
    const ro = new ResizeObserver(() => setWrapW(wrapEl.clientWidth));
    ro.observe(wrapEl);
    setWrapW(wrapEl.clientWidth);
    onCleanup(() => ro.disconnect());
  });
  const todayMs = () => {
    const d = new Date();
    d.setHours(0, 0, 0, 0);
    return d.getTime();
  };
  // số tuần vừa khung hiện tại
  const weeks = () => Math.max(HB_WEEKS_MIN, Math.min(HB_WEEKS_MAX, Math.floor(((wrapW() || 1200) - HB_DAYW) / (HB_CELL + HB_GAP))));
  // Thứ 2 của tuần đầu (cột trái nhất)
  const startMon = createMemo(() => {
    const m = new Date(todayMs());
    m.setDate(m.getDate() - ((m.getDay() + 6) % 7) - 7 * (weeks() - 1));
    return m;
  });
  const at = (w: number, r: number) => {
    const d = new Date(startMon());
    d.setDate(startMon().getDate() + w * 7 + r);
    return d;
  };
  // nhãn tháng: cột đầu tiên mang tháng mới; tháng Một hiện số năm (phân biệt các năm).
  // 2 nhãn cách nhau < 3 cột (vd Nov/Dec 2025 đầu chart) thì bỏ nhãn cũ, giữ tháng sau.
  const monthCells = createMemo(() => {
    const out: string[] = [];
    let prev = -1, lastW = -99;
    for (let w = 0; w < weeks(); w++) {
      const d = at(w, 0);
      const mo = d.getMonth();
      if (mo === prev) {
        out.push("");
        continue;
      }
      prev = mo;
      const label = mo === 0 ? String(d.getFullYear()) : d.toLocaleDateString(loc(), { month: "short" });
      if (w - lastW < 3) out[lastW] = "";
      out.push(label);
      lastW = w;
    }
    return out;
  });
  // chuỗi ngày (YYYY-MM-DD) từng ô theo hàng — item của For nên đổi là vẽ lại đúng vị trí sau resize
  const rowDays = (r: number) => {
    const n = weeks();
    const s = startMon();
    const out: string[] = [];
    for (let w = 0; w < n; w++) {
      const d = new Date(s);
      d.setDate(s.getDate() + w * 7 + r);
      out.push(dayStr(d));
    }
    return out;
  };
  // nhãn Mon/Wed/Fri theo locale (01/01/2024 là Thứ 2)
  const dayName = (r: number) => new Date(2024, 0, 1 + r).toLocaleDateString(loc(), { weekday: "short" });
  const rangeCount = createMemo(() => [...done()].filter((d) => Date.parse(d) >= startMon().getTime()).length);
  const rangeMonths = createMemo(() => Math.max(1, Math.round((weeks() * 7) / 30.44)));
  const rename = () => {
    const nn = prompt(t("tk.namePrompt"), props.h.title)?.trim();
    if (nn) props.onPatch({ title: nn });
  };
  return (
    <section class="tk-habit" classList={{ arch: props.h.archived }}>
      <div class="tk-habit-head">
        <b>{props.h.title}</b>
        <span class="tk-streak" title={t("tk.streakTip")}>🔥 {fmtNum(props.h.streak)}</span>
        <span class="muted xs">{t("tk.totalN", { n: fmtNum(props.h.total) })}</span>
        <span class="grow" />
        <button class={props.h.done_today ? "btn primary sm" : "btn ghost sm"} onClick={() => props.onToggle(todayStr(), !props.h.done_today)}>
          {props.h.done_today ? t("tk.doneToday") : t("tk.checkToday")}
        </button>
        <button class="icon-btn sm" title={t("c.edit")} onClick={rename}><Icon name="edit" size={14} /></button>
        <button class="icon-btn sm" title={props.h.archived ? t("tk.unarchive") : t("tk.archive")} onClick={() => props.onPatch({ archived: !props.h.archived })}><Icon name="archive" size={14} /></button>
        <button class="icon-btn sm danger" title={t("c.delete")} onClick={props.onDelete}><Icon name="trash" size={14} /></button>
      </div>
      <div class="hb" ref={wrapEl}>
        <div class="hb-chart" role="grid" aria-label={t("tk.gridLabel", { name: props.h.title })}
          style={{ "grid-template-columns": `${HB_DAYW}px repeat(${weeks()}, ${HB_CELL}px)` }}>
          <span />
          <For each={monthCells()}>{(m) => <span class="hb-mon">{m}</span>}</For>
          <For each={[0, 1, 2, 3, 4, 5, 6]}>
            {(r) => (<>
              <span class="hb-day">{[0, 2, 4].includes(r) ? dayName(r) : ""}</span>
              <For each={rowDays(r)}>
                {(ds) => <HabitDayCell ds={ds} days={done} onToggle={props.onToggle} />}
              </For>
            </>)}
          </For>
        </div>
        <div class="hb-foot">
          <span>{t("tk.yearN", { n: fmtNum(rangeCount()), m: rangeMonths() })}</span>
          <span class="hb-legend">{t("tk.less")}<i class="hb-sw" /><i class="hb-sw on" />{t("tk.more")}</span>
        </div>
      </div>
    </section>
  );
}
