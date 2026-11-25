import { For, Show, createMemo, createResource, createSignal, onCleanup, onMount } from "solid-js";
import { api, type Habit, type TaskItem, type TaskView } from "./api";
import { fmtNum, loc, t } from "./i18n";
import { errText } from "./fileops";
import { Icon } from "./icons";
import { TaskDrawer } from "./kanban";
import { ViewerOverlay } from "./viewers/ViewerOverlay";

const fmtDay = (s: number) => new Date(s * 1000).toLocaleDateString(loc(), { day: "2-digit", month: "2-digit" });
const dayStr = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
const todayStr = () => dayStr(new Date());

const VIEWS: TaskView[] = ["overdue", "today", "week", "all", "done"];

export function TrackerPage(props: { tab: "tasks" | "habits"; onTab: (t: "tasks" | "habits") => void; cal: boolean; onCal: (on: boolean) => void; onError: (m: string) => void }) {
  const tab = () => props.tab;
  const setTab = props.onTab;
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
          <TasksView cal={props.cal} onCal={props.onCal} onError={props.onError} />
        </div>
        <div class="tk-col tk-col-habits">
          <h3 class="tk-col-title">{t("tk.habits")}</h3>
          <HabitsView onError={props.onError} />
        </div>
      </div>
    </div>
  );
}

function TasksView(props: { cal: boolean; onCal: (on: boolean) => void; onError: (m: string) => void }) {
  const [view, setView] = createSignal<TaskView>("all");
  // đang xem lịch tuần thì không tải list; ver tăng để lịch tải lại sau khi sửa trong drawer
  const [ver, setVer] = createSignal(0);
  const [items, { refetch }] = createResource(() => !props.cal && view(), (v) => api.tasks(v).catch((e) => (props.onError(errText(e)), [] as TaskItem[])), { initialValue: [] });
  // mở task ngay tại Tracker (không nhảy sang Deck) + xem file đính kèm tại chỗ
  const [openTask, setOpenTask] = createSignal<{ board: string; card: string } | null>(null);
  const [viewPath, setViewPath] = createSignal<string | null>(null);
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
          {(v) => <button class="chip" classList={{ on: !props.cal && view() === v }} onClick={() => (setView(v), props.onCal(false))}>{t(`tk.${v}`)}</button>}
        </For>
        <button class="chip" classList={{ on: props.cal }} onClick={() => props.onCal(true)}><Icon name="calendar" size={13} /> {t("tk.calView")}</button>
        <span class="grow" />
        <button class="btn ghost sm" onClick={() => (props.cal ? setVer(ver() + 1) : refetch())} title={t("tk.reload")}><Icon name="menu" size={14} /> {t("tk.reload")}</button>
      </div>
      <Show when={props.cal}>
        <WeekCal ver={ver()} onOpen={(it) => setOpenTask({ board: it.board_id, card: it.id })} onError={props.onError} />
      </Show>
      {/* giữ list khi refetch (không unmount/remount gây blink + mất scroll) */}
      <Show when={!props.cal}>
      <Show when={groups().length || !items.loading} fallback={<p class="muted pad">{t("c.loading")}</p>}>
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
                      <button class="btn ghost sm" onClick={() => setOpenTask({ board: it.board_id, card: it.id })}>{t("tk.openTask")}</button>
                    </div>
                  )}
                </For>
              </section>
            )}
          </For>
        </Show>
      </Show>
      </Show>
      <Show when={openTask()} keyed>
        {(o) => <TaskDrawer boardId={o.board} cardId={o.card} onClose={() => setOpenTask(null)} onChanged={() => (props.cal ? setVer(ver() + 1) : refetch())} onOpenFile={setViewPath} onError={props.onError} />}
      </Show>
      <Show when={viewPath()}>
        {(p) => <ViewerOverlay path={p()} onClose={() => setViewPath(null)} onError={props.onError} />}
      </Show>
    </div>
  );
}

// thứ 2 đầu tuần (YYYY-MM-DD) chứa ngày d
const monOf = (d: Date) => {
  const m = new Date(d.getFullYear(), d.getMonth(), d.getDate());
  m.setDate(m.getDate() - ((m.getDay() + 6) % 7));
  return dayStr(m);
};
const addDays = (ds: string, n: number) => {
  const d = parseDS(ds);
  d.setDate(d.getDate() + n);
  return dayStr(d);
};
// giờ hạn; 00:00 và 12:00 (ô chọn ngày trong drawer lưu trưa) coi như không có giờ
const fmtTime = (s: number) => {
  const d = new Date(s * 1000);
  const hm = d.getHours() * 60 + d.getMinutes();
  return hm === 0 || (hm === 720 && !d.getSeconds()) ? "" : d.toLocaleTimeString(loc(), { hour: "2-digit", minute: "2-digit" });
};

/** Lịch tuần T2–CN: card theo ngày hạn, kéo sang ngày khác để đổi hạn (giữ giờ). */
function WeekCal(props: { ver: number; onOpen: (it: TaskItem) => void; onError: (m: string) => void }) {
  const [start, setStart] = createSignal(monOf(new Date()));
  const days = createMemo(() => [0, 1, 2, 3, 4, 5, 6].map((i) => addDays(start(), i)));
  const [items, { mutate, refetch }] = createResource(
    () => [start(), props.ver] as const,
    ([s]) => api.tasksRange(parseDS(s).getTime() / 1000, parseDS(addDays(s, 7)).getTime() / 1000).catch((e) => (props.onError(errText(e)), [] as TaskItem[])),
    { initialValue: [] },
  );
  const byDay = createMemo(() => {
    const m = new Map<string, TaskItem[]>();
    for (const it of items()) {
      const k = dayStr(new Date(it.due_at! * 1000));
      m.set(k, [...(m.get(k) ?? []), it]);
    }
    return m;
  });
  const move = async (it: TaskItem, ds: string) => {
    const old = new Date(it.due_at! * 1000);
    const nd = parseDS(ds);
    nd.setHours(old.getHours(), old.getMinutes(), old.getSeconds());
    const due = Math.floor(nd.getTime() / 1000);
    if (due === it.due_at) return;
    mutate((l) => l.map((x) => (x.id === it.id ? { ...x, due_at: due } : x)));
    try {
      await api.updateCard(it.id, { due_at: due });
    } catch (e) {
      props.onError(errText(e));
    }
    refetch();
  };
  // kéo thả như Deck: chuột kéo ngay, cảm ứng nhấn giữ
  const [dragId, setDragId] = createSignal<string | null>(null);
  const [over, setOver] = createSignal<string | null>(null);
  let suppressClick = false;
  const startDrag = (e: PointerEvent, it: TaskItem, el: HTMLElement) => {
    if (e.button !== 0) return;
    const sx = e.clientX, sy = e.clientY, rect = el.getBoundingClientRect();
    const sc = [el.closest<HTMLElement>(".tk-col"), el.closest<HTMLElement>(".tk-cols")].find((n) => n && n.scrollHeight > n.clientHeight + 1);
    let ghost: HTMLElement | null = null, timer: number | undefined, tick: number | undefined;
    let x = sx, y = sy;
    const dayAt = () => (document.elementFromPoint(x, y) as HTMLElement | null)?.closest<HTMLElement>(".tk-wk-day")?.dataset.day ?? null;
    const place = () => {
      if (!ghost) return;
      ghost.style.transform = `translate(${x - (sx - rect.left)}px, ${y - (sy - rect.top)}px) rotate(2deg)`;
      setOver(dayAt());
    };
    const begin = () => {
      ghost = el.cloneNode(true) as HTMLElement;
      ghost.classList.add("kb-ghost");
      ghost.style.width = `${rect.width}px`;
      document.body.appendChild(ghost);
      setDragId(it.id);
      navigator.vibrate?.(15);
      place();
      tick = window.setInterval(() => {
        if (!sc) return;
        const r = sc.getBoundingClientRect();
        if (y < r.top + 40) sc.scrollTop -= 12;
        else if (y > r.bottom - 40) sc.scrollTop += 12;
        else return;
        setOver(dayAt());
      }, 30);
    };
    const blockTouch = (ev: TouchEvent) => ghost && ev.preventDefault();
    const mv = (ev: PointerEvent) => {
      x = ev.clientX;
      y = ev.clientY;
      if (!ghost) {
        const far = Math.hypot(x - sx, y - sy);
        if (e.pointerType === "touch") {
          if (far > 8) cleanup(); // đang cuộn, không phải kéo
        } else if (far > 5) begin();
        return;
      }
      place();
    };
    const up = () => {
      const target = over();
      const was = !!ghost;
      cleanup();
      if (!was) return;
      suppressClick = true;
      setTimeout(() => (suppressClick = false), 50);
      if (target) void move(it, target);
    };
    const cleanup = () => {
      clearTimeout(timer);
      clearInterval(tick);
      ghost?.remove();
      ghost = null;
      setDragId(null);
      setOver(null);
      window.removeEventListener("pointermove", mv);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      document.removeEventListener("touchmove", blockTouch);
    };
    window.addEventListener("pointermove", mv);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
    document.addEventListener("touchmove", blockTouch, { passive: false });
    if (e.pointerType === "touch") timer = window.setTimeout(begin, 350);
  };
  const dm = (ds: string) => `${Number(ds.slice(8, 10))}/${Number(ds.slice(5, 7))}`;
  const label = () => `${dm(start())} – ${dm(addDays(start(), 6))}/${addDays(start(), 6).slice(0, 4)}`;
  const late = (it: TaskItem) => !it.done_at && (it.due_at ?? 0) * 1000 < new Date().setHours(0, 0, 0, 0);
  return (
    <div class="tk-wk-wrap">
      <div class="tk-wk-bar">
        <button class="icon-btn sm" onClick={() => setStart(addDays(start(), -7))} title={t("tk.prevW")}>‹</button>
        <b>{label()}</b>
        <button class="icon-btn sm" onClick={() => setStart(addDays(start(), 7))} title={t("tk.nextW")}>›</button>
        <Show when={start() !== monOf(new Date())}>
          <button class="btn ghost sm" onClick={() => setStart(monOf(new Date()))}>{t("tk.thisWeek")}</button>
        </Show>
        <span class="muted xs">{t("tk.dragTip")}</span>
      </div>
      <div class="tk-wk">
        <For each={days()}>
          {(ds) => (
            <div class="tk-wk-day" data-day={ds} classList={{ today: ds === todayStr(), over: over() === ds }}>
              <div class="tk-wk-head">{parseDS(ds).toLocaleDateString(loc(), { weekday: "short" })} <span>{dm(ds)}</span></div>
              <For each={byDay().get(ds) ?? []}>
                {(it) => {
                  let el!: HTMLDivElement;
                  return (
                    <div
                      ref={el}
                      class="tk-wk-card"
                      classList={{ done: !!it.done_at, late: late(it), dragging: dragId() === it.id }}
                      title={`${it.title} · ${it.board_title} · ${it.column_title}`}
                      onPointerDown={(e) => startDrag(e, it, el)}
                      onClick={() => !suppressClick && props.onOpen(it)}
                    >
                      <span class="tk-wk-t">{it.title}</span>
                      <span class="tk-wk-meta muted xs">
                        <Show when={it.done_at}><Icon name="check" size={11} class="ic kb-done-ic" /></Show>
                        <Show when={fmtTime(it.due_at!)}>{(tm) => <b class="tk-wk-time">{tm()}</b>}</Show>
                        <span>{it.board_title}</span>
                      </span>
                    </div>
                  );
                }}
              </For>
            </div>
          )}
        </For>
      </div>
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
  const patch = (h: Habit, b: { title?: string; archived?: boolean }) => run(() => api.updateHabit(h.id, b));
  const del = (h: Habit) => {
    if (confirm(t("tk.delAsk", { name: h.title }))) run(() => api.deleteHabit(h.id));
  };
  const rename = (h: Habit) => {
    const nn = prompt(t("tk.namePrompt"), h.title)?.trim();
    if (nn) patch(h, { title: nn });
  };
  return (
    <div>
      <div class="tk-views">
        <input class="kb-new tk-add" placeholder={t("tk.addPh")} value={name()} onInput={(e) => setName(e.currentTarget.value)} onKeyDown={(e) => e.key === "Enter" && add()} />
        <label class="kb-toggle"><input type="checkbox" checked={showArch()} onChange={(e) => setShowArch(e.currentTarget.checked)} /> {t("tk.showArch")}</label>
      </div>
      {/* giữ UI khi refetch (tick tháng cũ không mất tháng đang xem): chỉ loading khi chưa có data */}
      <Show when={visible().length || !list.loading} fallback={<p class="muted pad">{t("c.loading")}</p>}>
        <Show when={visible().length} fallback={<div class="empty"><Icon name="check" size={48} /><p>{t("tk.emptyHabit")}</p></div>}>
          <HabitMonth habits={visible()} onToggle={toggle} onRename={rename} onPatch={patch} onDelete={del} />
          <HabitYear habits={visible()} />
        </Show>
      </Show>
    </div>
  );
}

// Ngày tạo habit (YYYY-MM-DD): các ô trước ngày này bị disable, thêm giữa chừng không ảnh hưởng số liệu cũ.
const createdDayOf = (h: Habit) => dayStr(new Date(h.created_at * 1000));

// tone cam chủ đề: nhạt -> đậm, tuần bên phải (gần hiện tại) luôn đậm nhất
const ORANGE5 = ["#f3c39d", "#eda46f", "#e07f45", "#cf5c26", "#a8431f"];
// tuần chuẩn Thứ 2 -> CN: số thứ tự tuần trong tháng (1-based), tuần đầu có thể cụt
const weekOfMonth = (d: Date) => {
  const firstDowMon = (new Date(d.getFullYear(), d.getMonth(), 1).getDay() + 6) % 7;
  const shift = (7 - firstDowMon) % 7; // số ngày của tuần đầu
  const date = d.getDate();
  if (shift === 0) return Math.ceil(date / 7);
  return date <= shift ? 1 : 1 + Math.ceil((date - shift) / 7);
};
const parseDS = (ds: string) => new Date(Number(ds.slice(0, 4)), Number(ds.slice(5, 7)) - 1, Number(ds.slice(8, 10)));
// tên dài cắt giữa: "Learn ... Agent", giữ title đầy đủ ở tooltip
const truncMiddle = (s: string, max = 22) => (s.length <= max ? s : `${s.slice(0, 13).trimEnd()}...${s.slice(-6).trimStart()}`);

function HabitMonth(props: {
  habits: Habit[];
  onToggle: (h: Habit, day: string, on: boolean) => void;
  onRename: (h: Habit) => void;
  onPatch: (h: Habit, b: { archived?: boolean }) => void;
  onDelete: (h: Habit) => void;
}) {
  const nowD = new Date();
  const curY = nowD.getFullYear(), curM = nowD.getMonth();
  // neo cuối cửa sổ (ngày cuối đang xem); tháng suy từ neo
  const [endDS, setEndDS] = createSignal(dayStr(nowD));
  const endD = createMemo(() => parseDS(endDS()));
  const vy = () => endD().getFullYear();
  const vm = () => endD().getMonth();
  const [wrapW, setWrapW] = createSignal(0);
  let wrapEl!: HTMLDivElement;
  onMount(() => {
    const ro = new ResizeObserver(() => setWrapW(wrapEl.clientWidth));
    ro.observe(wrapEl);
    setWrapW(wrapEl.clientWidth);
    onCleanup(() => ro.disconnect());
  });
  const days = createMemo(() => {
    const y = vy(), m = vm();
    const n = new Date(y, m + 1, 0).getDate();
    const out: string[] = [];
    for (let d = 1; d <= n; d++) out.push(`${y}-${String(m + 1).padStart(2, "0")}-${String(d).padStart(2, "0")}`);
    return out;
  });
  // cửa sổ trượt: ô 30px cố định, hiện vừa khít khung (neo ở cuối, tràn sang tháng trước nếu rộng);
  // màn bé co tới tối thiểu 1 ngày để không bao giờ tràn ngang
  const per = () => Math.max(1, Math.floor(((wrapW() || 1200) - 200) / 34));
  const vis = createMemo(() => {
    const out: string[] = [];
    const e = endD();
    const s = new Date(e);
    s.setDate(e.getDate() - (per() - 1));
    for (let d = new Date(s); d <= e; d.setDate(d.getDate() + 1)) out.push(dayStr(d));
    return out;
  });
  const isCur = () => vy() === curY && vm() === curM;
  const goMonth = (d: number) => {
    const nd = new Date(vy(), vm() + d, 1);
    // không qua tháng hiện tại (ô tương lai không chấm được)
    if (nd.getFullYear() > curY || (nd.getFullYear() === curY && nd.getMonth() > curM)) return;
    const back = nd.getFullYear() === curY && nd.getMonth() === curM;
    // tháng hiện tại neo ở hôm nay, tháng cũ neo cuối tháng
    setEndDS(back ? todayStr() : dayStr(new Date(nd.getFullYear(), nd.getMonth() + 1, 0)));
  };
  // prev/next: dịch neo cuối đi per ngày (tự tràn tháng), chặn tương lai
  const shiftWin = (d: number) => {
    const ne = new Date(endD());
    ne.setDate(ne.getDate() + d * per());
    if (ne > new Date(curY, curM + 1, 0)) return;
    setEndDS(dayStr(ne));
  };
  const goToday = () => setEndDS(todayStr());
  const sets = createMemo(() => new Map(props.habits.map((h) => [h.id, new Set(h.days)])));
  const doneOn = (h: Habit, ds: string) => sets().get(h.id)?.has(ds) ?? false;
  const stats = createMemo(() => {
    let done = 0, poss = 0;
    for (const h of props.habits) {
      const cd = createdDayOf(h);
      for (const ds of days()) {
        if (ds < cd || ds > todayStr()) continue;
        poss++;
        if (doneOn(h, ds)) done++;
      }
    }
    return { n: props.habits.length, done, poss, pct: poss ? (done / poss) * 100 : 0 };
  });
  const monthLabel = () => new Date(vy(), vm(), 1).toLocaleDateString(loc(), { month: "long", year: "numeric" });
  // gộp ngày hiển thị thành nhóm (năm, tháng, tuần); màu theo vị trí: phải cùng đậm nhất
  const groups = createMemo(() => {
    const g: { y: number; m: number; w: number; count: number }[] = [];
    for (const ds of vis()) {
      const d = parseDS(ds);
      const k: [number, number, number] = [d.getFullYear(), d.getMonth(), weekOfMonth(d)];
      const lastG = g[g.length - 1];
      if (lastG && lastG.y === k[0] && lastG.m === k[1] && lastG.w === k[2]) lastG.count++;
      else g.push({ y: k[0], m: k[1], w: k[2], count: 1 });
    }
    return g;
  });
  const weekColor = (gi: number) => ORANGE5[Math.max(0, 5 - groups().length + Math.max(0, gi))];
  const gIdx = createMemo(() => {
    const m = new Map<string, number>();
    let gi = -1, pk = "";
    for (const ds of vis()) {
      const d = parseDS(ds);
      const k = `${d.getFullYear()}-${d.getMonth()}-${weekOfMonth(d)}`;
      if (k !== pk) {
        gi++;
        pk = k;
      }
      m.set(ds, gi);
    }
    return m;
  });
  const bandLb = (gi: number) => {
    const g = groups()[gi];
    const wn = `W${g.w}`;
    return g.y === vy() && g.m === vm() ? wn : `T${g.m + 1}·${wn}`;
  };
  const rangeLb = () => {
    const v = vis();
    if (!v.length) return "";
    const a = parseDS(v[0]), b = parseDS(v[v.length - 1]);
    return `${a.getDate()}/${a.getMonth() + 1} – ${b.getDate()}/${b.getMonth() + 1}`;
  };
  const dwName = (ds: string) => {
    const [y, m, d] = ds.split("-").map(Number);
    return new Date(y, m - 1, d).toLocaleDateString(loc(), { weekday: "short" });
  };
  const cellTitle = (h: Habit, ds: string) => {
    const [y, m, d] = ds.split("-").map(Number);
    const lb = new Date(y, m - 1, d).toLocaleDateString(loc(), { day: "2-digit", month: "2-digit" });
    return `${h.title} · ${lb}${doneOn(h, ds) ? t("tk.cellDone") : ""}`;
  };
  return (
    <section class="tk-habit hm">
      <div class="hm-top">
        <div class="hm-title">
          <button class="icon-btn sm" onClick={() => goMonth(-1)} title={t("tk.prevM")}>‹</button>
          <b>{monthLabel()}</b>
          <button class="icon-btn sm" disabled={isCur()} onClick={() => goMonth(1)} title={t("tk.nextM")}>›</button>
          <Show when={endDS() !== todayStr()}>
            <button class="btn ghost sm" onClick={goToday}>{t("tk.todayBtn")}</button>
          </Show>
        </div>
        <div class="hm-pager">
          <button class="icon-btn sm" onClick={() => shiftWin(-1)}>‹</button>
          <span class="muted xs">{rangeLb()}</span>
          <button class="icon-btn sm" disabled={endD() >= new Date(curY, curM + 1, 0)} onClick={() => shiftWin(1)}>›</button>
        </div>
        <div class="hm-stats">
          <span class="muted xs">{t("tk.completedM")}: <b>{fmtNum(stats().done)}</b></span>
          <span class="hm-bar"><i style={{ width: `${Math.min(100, stats().pct)}%` }} /></span>
          <span class="muted xs"><b>{stats().pct.toFixed(2)}%</b></span>
        </div>
      </div>
      <div class="hm-wrap" ref={wrapEl}>
        <div class="hm-grid" style={{ "grid-template-columns": `minmax(110px, 170px) repeat(${vis().length}, 30px)` }}>
          <span class="hm-head-name">{t("tk.myHabits")} ({fmtNum(stats().n)})</span>
          <For each={groups()}>
            {(g, i) => (
              <span class="hm-wk" style={{ "grid-column": `span ${g.count}`, background: weekColor(i()) }} title={bandLb(i())}>
                {bandLb(i())}
              </span>
            )}
          </For>
          <span />
          <For each={vis()}>
            {(ds) => <span class="hm-dw" classList={{ today: ds === todayStr() }}>{dwName(ds)}</span>}
          </For>
          <span />
          <For each={vis()}>
            {(ds) => <span class="hm-dn" classList={{ today: ds === todayStr() }}>{Number(ds.slice(8, 10))}{ds === todayStr() ? " ●" : ""}</span>}
          </For>
          <For each={props.habits}>
            {(h) => (
              <>
                <span class="hm-name" title={h.title}>
                  <span class="hm-name-t">{truncMiddle(h.title)}</span>
                  <button class="icon-btn sm hm-more" title={t("f.menu")}><Icon name="more" size={12} /></button>
                  <span class="hm-acts">
                    <button class="icon-btn sm" title={t("c.edit")} onClick={() => props.onRename(h)}><Icon name="edit" size={12} /></button>
                    <button class="icon-btn sm" title={h.archived ? t("tk.unarchive") : t("tk.archive")} onClick={() => props.onPatch(h, { archived: !h.archived })}><Icon name="archive" size={12} /></button>
                    <button class="icon-btn sm danger" title={t("c.delete")} onClick={() => props.onDelete(h)}><Icon name="trash" size={12} /></button>
                  </span>
                </span>
                <For each={vis()}>
                  {(ds) => {
                    const dis = ds > todayStr() || ds < createdDayOf(h);
                    const on = doneOn(h, ds);
                    const wc = weekColor(gIdx().get(ds) ?? 0);
                    return (
                      <button
                        class="hm-cell"
                        classList={{ on, today: ds === todayStr() }}
                        style={on ? { background: wc, "border-color": wc } : {}}
                        title={cellTitle(h, ds)}
                        disabled={dis}
                        onClick={() => props.onToggle(h, ds, !on)}
                      >{on ? "✓" : ""}</button>
                    );
                  }}
                </For>
              </>
            )}
          </For>
        </div>
      </div>
    </section>
  );
}

function HabitYear(props: { habits: Habit[] }) {
  const [wrapW, setWrapW] = createSignal(0);
  let wrapEl!: HTMLDivElement;
  onMount(() => {
    const ro = new ResizeObserver(() => setWrapW(wrapEl.clientWidth));
    ro.observe(wrapEl);
    setWrapW(wrapEl.clientWidth);
    onCleanup(() => ro.disconnect());
  });
  const CELL = 12, GAP = 3, DAYW = 30;
  // rộng bao nhiêu hiện bấy nhiêu tuần (tràn sang năm trước), hẹp thì ẩn tuần cũ đi
  const weeks = () => Math.max(13, Math.min(260, Math.floor(((wrapW() || 1200) - DAYW) / (CELL + GAP))));
  const todayMs = () => {
    const d = new Date();
    d.setHours(0, 0, 0, 0);
    return d.getTime();
  };
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
  const dayName = (r: number) => new Date(2024, 0, 1 + r).toLocaleDateString(loc(), { weekday: "short" });
  // tổng hợp cả năm: mỗi ô 1 ngày, đậm dần theo % habit đã chấm (0–100% chia 4 nấc)
  const info = (ds: string) => {
    let done = 0, act = 0;
    for (const h of props.habits) {
      if (h.archived || ds < createdDayOf(h) || ds > todayStr()) continue;
      act++;
      if (h.days.includes(ds)) done++;
    }
    const pct = act ? done / act : 0;
    const lv = pct <= 0 ? 0 : pct <= 0.25 ? 1 : pct <= 0.5 ? 2 : pct <= 0.75 ? 3 : 4;
    return { done, act, lv };
  };
  const cellTitle = (ds: string) => {
    const [y, m, d] = ds.split("-").map(Number);
    const lb = new Date(y, m - 1, d).toLocaleDateString(loc(), { day: "2-digit", month: "2-digit" });
    const { done, act } = info(ds);
    return `${lb}: ${done}/${act}`;
  };
  const total = createMemo(() => {
    const s = startMon().getTime();
    let n = 0;
    for (const h of props.habits) for (const d of h.days) if (Date.parse(d) >= s) n++;
    return n;
  });
  const rangeMonths = createMemo(() => Math.max(1, Math.round((weeks() * 7) / 30.44)));
  return (
    <section class="tk-habit">
      <div class="tk-habit-head">
        <b>{t("tk.yearStats")}</b>
        <span class="grow" />
      </div>
      <div class="hb" ref={wrapEl}>
        <div class="hb-chart" role="grid" aria-label={t("tk.yearStats")}
          style={{ "grid-template-columns": `${DAYW}px repeat(${weeks()}, ${CELL}px)` }}>
          <span />
          <For each={monthCells()}>{(m) => <span class="hb-mon">{m}</span>}</For>
          <For each={[0, 1, 2, 3, 4, 5, 6]}>
            {(r) => (<>
              <span class="hb-day">{[0, 2, 4].includes(r) ? dayName(r) : ""}</span>
              <For each={rowDays(r)}>
                {(ds) => {
                  const lv = () => info(ds).lv; // đọc trong binding để đổi màu ngay khi tick
                  const fut = ds > todayStr();
                  return (
                    <span
                      class="hb-cell hy"
                      role="img"
                      classList={{ lv1: lv() === 1, lv2: lv() === 2, lv3: lv() === 3, lv4: lv() === 4, future: fut }}
                      title={cellTitle(ds)}
                    />
                  );
                }}
              </For>
            </>)}
          </For>
        </div>
        <div class="hb-foot">
          <span>{t("tk.yearN", { n: fmtNum(total()), m: rangeMonths() })}</span>
          <span class="hb-legend">{t("tk.less")}<i class="hb-sw hy" /><i class="hb-sw hy lv1" /><i class="hb-sw hy lv2" /><i class="hb-sw hy lv3" /><i class="hb-sw hy lv4" />{t("tk.more")}</span>
        </div>
      </div>
    </section>
  );
}
