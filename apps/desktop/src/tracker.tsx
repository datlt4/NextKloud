import { For, Show, createMemo, createResource, createSignal } from "solid-js";
import { api, type Habit, type TaskItem, type TaskView } from "./api";
import { fmtNum, loc, t } from "./i18n";
import { errText } from "./fileops";
import { Icon } from "./icons";

const fmtDay = (s: number) => new Date(s * 1000).toLocaleDateString(loc(), { day: "2-digit", month: "2-digit" });
const dayStr = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
const todayStr = () => dayStr(new Date());

const VIEWS: TaskView[] = ["overdue", "today", "week", "all", "done"];

export function TrackerPage(props: { onOpenBoard: (id: string) => void; onError: (m: string) => void }) {
  const [tab, setTab] = createSignal<"tasks" | "habits">("tasks");
  return (
    <div class="tracker">
      <div class="toolbar kb-bar">
        <h2 class="tb-title"><Icon name="check" /> {t("tk.title")}</h2>
        <div class="kb-boards">
          <button class="chip" classList={{ on: tab() === "tasks" }} onClick={() => setTab("tasks")}>{t("tk.tasks")}</button>
          <button class="chip" classList={{ on: tab() === "habits" }} onClick={() => setTab("habits")}>{t("tk.habits")}</button>
        </div>
      </div>
      <Show when={tab() === "tasks"}><TasksView onOpenBoard={props.onOpenBoard} onError={props.onError} /></Show>
      <Show when={tab() === "habits"}><HabitsView onError={props.onError} /></Show>
    </div>
  );
}

function TasksView(props: { onOpenBoard: (id: string) => void; onError: (m: string) => void }) {
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
                      <button class="btn ghost sm" onClick={() => props.onOpenBoard(it.board_id)}>{t("tk.openBoard")}</button>
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

/** 12 tuần gần nhất, cột = tuần, ô = ngày (xanh = đã làm). Bấm ô để chấm/bỏ. */
function HabitCard(props: { h: Habit; onToggle: (day: string, on: boolean) => void; onPatch: (b: { title?: string; archived?: boolean }) => void; onDelete: () => void }) {
  const done = () => new Set(props.h.days);
  // 84 ngày: 12 cột × 7 hàng (hàng 0 = Thứ 2)
  const cells = createMemo(() => {
    const out: { day: string; label: string; future: boolean }[] = [];
    const t = new Date();
    t.setHours(0, 0, 0, 0);
    const monday = new Date(t);
    monday.setDate(t.getDate() - ((t.getDay() + 6) % 7) - 7 * 11);
    for (let i = 0; i < 84; i++) {
      const d = new Date(monday);
      d.setDate(monday.getDate() + i);
      out.push({ day: dayStr(d), label: d.toLocaleDateString(loc(), { day: "2-digit", month: "2-digit" }), future: d.getTime() > t.getTime() });
    }
    return out;
  });
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
      <div class="tk-grid" role="grid" aria-label={t("tk.gridLabel", { name: props.h.title })}>
        <For each={cells()}>
          {(c) => (
            <button
              class="tk-cell"
              classList={{ on: done().has(c.day), future: c.future }}
              title={`${c.label}${done().has(c.day) ? t("tk.cellDone") : ""}`}
              disabled={c.future}
              onClick={() => props.onToggle(c.day, !done().has(c.day))}
            />
          )}
        </For>
      </div>
    </section>
  );
}
