import { marked } from "marked";
import { For, Show, createEffect, createMemo, createSignal, onCleanup } from "solid-js";
import { api, hashText, speak } from "../api";
import { Icon } from "../icons";
import { ReaderLayout, STATE_UI, type TocItem, createReader, topState } from "../annot";

export interface Block {
  kind: "speak" | "skip";
  html: string;
  text: string;
  /** Heading: cấp (1-6) + chữ, dùng cho mục lục. */
  level?: number;
  label?: string;
}

function escapeHtml(s: string) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

/** Tách md thành block đọc được. Code fence + math ($$/$$...$$) -> skip (không có nút voice). */
export function blocks(md: string): Block[] {
  const out: Block[] = [];
  for (const t of marked.lexer(md)) {
    if (t.type === "space") continue;
    switch (t.type) {
      case "code":
        out.push({ kind: "skip", html: marked.parse(t.raw) as string, text: "" });
        break;
      case "heading":
      case "paragraph":
        if (/^\s*\$\$[\s\S]*\$\$\s*$/.test(t.raw) || /^\s*\$[^$]+\$\s*$/.test(t.raw)) {
          out.push({ kind: "skip", html: `<pre class="math">${escapeHtml(t.raw)}</pre>`, text: "" });
        } else {
          const h = t.type === "heading" ? { level: t.depth as number, label: stripMd(t.text ?? "").replace(/[*_`]/g, "") } : {};
          out.push({ kind: "speak", html: linkifyWiki(marked.parse(t.raw) as string), text: t.text ?? t.raw, ...h });
        }
        break;
      case "blockquote":
        // annotation/callout của Anh cũng cần TTS -> speak
        out.push({ kind: "speak", html: linkifyWiki(marked.parse(t.raw) as string), text: stripMd(t.raw) });
        break;
      case "list":
        for (const item of t.items) {
          out.push({ kind: "speak", html: linkifyWiki(marked.parse(item.text) as string), text: item.text });
        }
        break;
      case "table":
        out.push({ kind: "skip", html: marked.parse(t.raw) as string, text: "" });
        break;
      default:
        out.push({ kind: "skip", html: marked.parse(t.raw) as string, text: "" });
    }
  }
  return out;
}

/** Chuyển [[link]] Obsidian thành <a class="wiki-link">, ![[]] ảnh thành <img data-wsrc>.
 *  Bỏ qua nội dung trong <pre>/<code>/<a>/<!-- --> để không phá code và link có sẵn. */
export function linkifyWiki(html: string): string {
  const parts = html.split(/(<pre[\s>][\s\S]*?<\/pre>|<code[\s>][\s\S]*?<\/code>|<a[\s>][\s\S]*?<\/a>|<img[^>]*>|<!--[\s\S]*?-->)/g);
  return parts
    .map((p, i) => {
      // 1) <img> nằm trong nhóm được bảo vệ: src tương đối -> đánh dấu để nạp blob sau
      if (i % 2 === 1) return p.startsWith("<img") ? relImg(p) : p;
      // 2) embed ![[...]]
      p = p.replace(/!\[\[([^\]]+)\]\]/g, (_m, inside: string) => {
        const target = inside.trim();
        // ![[a.png|300]] / ![[note#head]]: bỏ hậu tố kích thước/anchor khỏi đường dẫn
        const [file, head] = target.split("|")[0].split("#");
        const enc = encodeURIComponent(file.trim());
        if (/\.(png|jpe?g|gif|webp|avif|bmp|svg)$/i.test(file.trim())) return `<img data-wsrc="${enc}" alt="${escapeHtml(file.trim())}">`;
        const anc = head ? ` data-anchor="${encodeURIComponent(head.trim())}"` : "";
        const label = escapeHtml(displayOf(file.trim(), head?.trim()));
        return `<a href="#" class="wiki-link wiki-embed" data-wiki="${enc}"${anc}>${label}</a>`;
      });
      // 3) link [[target]], [[target|alias]], [[target#head]], [[target#head|alias]]
      p = p.replace(/\[\[([^\]|#]+)(?:#([^\]|]*))?(?:\|([^\]]*))?\]\]/g, (_m, t: string, h?: string, a?: string) => {
        const target = t.trim();
        const enc = encodeURIComponent(target);
        const anc = h ? ` data-anchor="${encodeURIComponent(h.trim())}"` : "";
        const label = escapeHtml(a !== undefined && a !== "" ? a : displayOf(target, h));
        return `<a href="#" class="wiki-link" data-wiki="${enc}"${anc}>${label}</a>`;
      });
      return p;
    })
    .join("");
}

function relImg(tag: string): string {
  return tag.replace(/<img([^>]*?)\ssrc="([^"]+)"([^>]*)>/, (m, a, src, b) => {
    if (/^(https?:|blob:|data:)/.test(src) || src.startsWith("#")) return m;
    return `<img${a} data-wsrc="${encodeURIComponent(src)}" src="data:,"${b}>`;
  });
}

function displayOf(target: string, heading?: string): string {
  const base = target.replace(/\.md$/i, "");
  const name = base.split("/").pop() ?? base;
  return heading ? `${name} › ${heading}` : name;
}

function stripMd(s: string) {
  s = s.replace(/!\[\[[^\]]*\]\]/g, " ");
  s = s.replace(/\[\[([^\]|#\]]+)(?:#[^\]|]*)?(?:\|([^\]]*))?\]\]/g, (_m, t: string, a: string | undefined) => a ?? t);
  return s.replace(/^[>\s#]*/, "").trim();
}

const imgUrlCache = new Map<string, string>();

function VoiceButton(props: { text: string; onPlay: () => void }) {
  const [st, setSt] = createSignal<"idle" | "busy" | "play">("idle");
  let audio: HTMLAudioElement | null = null;
  return (
    <button
      class="voice-btn"
      title="Nghe đoạn này"
      disabled={st() === "busy"}
      onClick={async (e) => {
        e.stopPropagation();
        if (st() === "play" && audio) {
          audio.pause();
          setSt("idle");
          return;
        }
        setSt("busy");
        try {
          const url = await speak(props.text);
          audio = new Audio(url);
          props.onPlay();
          setSt("play");
          audio.onended = () => setSt("idle");
          await audio.play();
        } catch {
          setSt("idle");
        }
      }}
    >
      {st() === "busy" ? <Icon name="listen" size={15} class="pulse" /> : st() === "play" ? <Icon name="pause" size={15} /> : <Icon name="listen" size={15} />}
    </button>
  );
}

export default function MarkdownView(props: {
  path: string;
  text: string;
  anchor?: string | null;
  onOpenLink?: (target: string, anchor?: string) => void;
  onError?: (m: string) => void;
  /** Mở sẵn tại vị trí annotation ("b12"). */
  mark?: string;
}) {
  const list = createMemo(() => blocks(props.text));
  const reader = createReader(() => props.path, (m) => (props.onError ?? console.error)(m));
  const plain = createMemo(() => list().map((b) => b.html.replace(/<[^>]+>/g, "").replace(/\s+/g, " ")));
  // Annotation -> block: ưu tiên đoạn chứa đúng câu đã trích (bài có thể đã sửa), không thì theo số thứ tự.
  const blockOf = (anchor: string, quote: string) => {
    const i = +anchor.slice(1) || 0;
    const q = quote.replace(/\s+/g, " ").trim().slice(0, 80);
    if (!q || plain()[i]?.includes(q)) return i;
    const j = plain().findIndex((t) => t.includes(q));
    return j >= 0 ? j : i;
  };
  const byBlock = createMemo(() => {
    const m = new Map<number, typeof reader.items extends () => infer T ? T : never>();
    for (const a of reader.items()) {
      const i = blockOf(a.anchor, a.quote);
      m.set(i, [...(m.get(i) ?? []), a]);
    }
    return m;
  });
  const toc = createMemo<TocItem[]>(() =>
    list().flatMap((b, i) => (b.level ? [{ anchor: `b${i}`, label: b.label || "…", level: b.level }] : [])),
  );
  const [current, setCurrent] = createSignal("");
  const jump = (anchor: string) => {
    const el = host?.querySelector<HTMLElement>(`[data-anchor="${anchor}"]`);
    if (!el) return;
    el.scrollIntoView({ block: "start", behavior: "smooth" });
    el.classList.add("flash");
    setTimeout(() => el.classList.remove("flash"), 1400);
  };
  // Mục đang đọc = heading cuối cùng đã cuộn qua đỉnh khung xem.
  const onScroll = () => {
    const hs = host?.querySelectorAll<HTMLElement>(".md-block[data-level]") ?? [];
    let cur = "";
    for (const h of Array.from(hs)) {
      if (h.getBoundingClientRect().top > 140) break;
      cur = h.dataset.anchor ?? "";
    }
    setCurrent(cur || toc()[0]?.anchor || "");
  };
  let scroller: HTMLElement | null = null;
  onCleanup(() => scroller?.removeEventListener("scroll", onScroll));
  // Stale: hash lúc mở vs hash lúc ấn "đọc cả bài".
  const [builtHash, setBuiltHash] = createSignal<string | null>(null);
  const [playingAll, setPlayingAll] = createSignal(false);
  const [prepping, setPrepping] = createSignal<[number, number] | null>(null);
  const curHash = createMemo(() => hashText(props.text));
  const stale = () => builtHash() !== null && builtHash() !== curHash();
  let host: HTMLElement | undefined;
  const dirOf = (p: string) => (p.includes("/") ? p.slice(0, p.lastIndexOf("/")) : "");

  const readAll = async () => {
    setBuiltHash(curHash());
    setPlayingAll(true);
    try {
      for (const b of list()) {
        if (!playingAll()) break;
        if (b.kind !== "speak" || !b.text.trim()) continue;
        const url = await speak(stripForSpeech(b.text));
        await new Promise<void>((res) => {
          const a = new Audio(url);
          a.onended = () => res();
          a.onerror = () => res();
          a.play().catch(() => res());
        });
      }
    } finally {
      setPlayingAll(false);
    }
  };

  // Tải trước toàn bài vào cache (để offline/nghe mượt), không phát tiếng.
  const prefetchAll = async () => {
    const items = list().filter((b) => b.kind === "speak" && stripForSpeech(b.text).trim());
    setPrepping([0, items.length]);
    try {
      for (let i = 0; i < items.length; i++) {
        await speak(stripForSpeech(items[i].text)).catch(() => {});
        setPrepping([i + 1, items.length]);
      }
    } finally {
      setPrepping(null);
    }
  };

  const onArticleClick = (e: MouseEvent) => {
    const a = (e.target as HTMLElement).closest?.("a.wiki-link") as HTMLElement | null;
    if (!a || !props.onOpenLink) return;
    e.preventDefault();
    props.onOpenLink(
      decodeURIComponent(a.getAttribute("data-wiki") ?? ""),
      a.getAttribute("data-anchor") ? decodeURIComponent(a.getAttribute("data-anchor")!) : undefined,
    );
  };

  createEffect(() => {
    // gắn lại sau mỗi lần render danh sách block
    list();
    // 1) ảnh tương đối / embed -> blob URL có auth
    const imgs = host?.querySelectorAll<HTMLImageElement>("img[data-wsrc]") ?? [];
    imgs.forEach((img) => {
      const raw = decodeURIComponent(img.getAttribute("data-wsrc") ?? "");
      const key = `${dirOf(props.path)}${String.fromCharCode(0)}${raw}`;
      const hit = imgUrlCache.get(key);
      if (hit) {
        img.src = hit;
        img.removeAttribute("data-wsrc");
        return;
      }
      api.downloadUrl(joinRel(dirOf(props.path), raw))
      .then((url) => {
        if (img.isConnected) {
          imgUrlCache.set(key, url);
          img.src = url;
          img.removeAttribute("data-wsrc");
        } else URL.revokeObjectURL(url);
      })
      .catch(() => {
        img.removeAttribute("data-wsrc");
        img.alt = raw;
      });
    });
    // 2) cuộn tới heading nếu mở link có #section
    if (props.anchor) {
      const needle = props.anchor.toLowerCase().replace(/[-_]+/g, " ");
      const hs = host?.querySelectorAll("h1,h2,h3,h4,h5,h6") ?? [];
      for (const h of Array.from(hs)) {
        if ((h.textContent ?? "").toLowerCase().includes(needle)) {
          (h as HTMLElement).scrollIntoView({ block: "start" });
          break;
        }
      }
    }
  });

  const anchorOf = (n: Node) => {
    const el = (n instanceof Element ? n : n.parentElement)?.closest<HTMLElement>(".md-block");
    return el?.dataset.anchor ?? null;
  };
  return (
    <ReaderLayout r={reader} toc={toc()} current={current()} onJump={jump} anchorOf={anchorOf}>
    <article
      class="md-body"
      ref={(el) => {
        host = el;
        queueMicrotask(() => {
          scroller = el.closest(".viewer");
          scroller?.addEventListener("scroll", onScroll, { passive: true });
          onScroll();
          if (props.mark) setTimeout(() => jump(props.mark!), 50);
        });
      }}
      onClick={onArticleClick}
    >
      <div class="md-bar">
        <b>{props.path}</b>
        <button onClick={readAll} disabled={playingAll()}>
          {playingAll() ? "Đang đọc…" : (<><Icon name="listen" size={15} /> Đọc cả bài</>)}
        </button>
        <button class="ghost" onClick={prefetchAll} disabled={!!prepping() || playingAll()} title="Tải trước audio toàn bài vào cache (nghe offline/mượt)">
          {prepping() ? `Chuẩn bị ${prepping()![0]}/${prepping()![1]}…` : "Chuẩn bị offline"}
        </button>
      </div>
      <Show when={stale()}>
        <p class="stale">⚠️ Bài đã sửa sau khi bấm đọc — âm thanh đã cũ, bấm "Đọc cả bài" lại để render mới.</p>
      </Show>
      <For each={list()}>
        {(b, i) => {
          const anns = () => byBlock().get(i()) ?? [];
          const bm = () => anns().find((a) => a.kind === "bookmark");
          const st = () => topState(anns());
          const quote = () => plain()[i()].slice(0, 160);
          return (
            <div class="md-block" data-anchor={`b${i()}`} data-level={b.level} classList={{ [`has-${st()}`]: !!st() }}>
              <Show when={b.kind === "speak"}>
                <VoiceButton text={stripForSpeech(b.text)} onPlay={() => setBuiltHash(curHash())} />
              </Show>
              <div class="md-content" innerHTML={b.html} />
              <div class="md-rail">
                <Show when={bm()}>
                  <button class="mk mk-bm" title="Bỏ bookmark" onClick={() => reader.toggleBookmark(`b${i()}`, "")}><Icon name="bookmark" size={14} /></button>
                </Show>
                <Show when={st()}>
                  {(s) => (
                    <button class={`mk mk-note st-${s()}`} title="Xem ghi chú" onClick={() => reader.showNotes(anns().find((a) => a.kind === "note")!.id)}>
                      <Icon name={STATE_UI[s()].icon} size={12} />
                      <Show when={anns().filter((a) => a.kind === "note").length > 1}>{anns().filter((a) => a.kind === "note").length}</Show>
                    </button>
                  )}
                </Show>
                <span class="md-tools">
                <Show when={!bm()}>
                  <button title="Bookmark đoạn này" onClick={() => reader.toggleBookmark(`b${i()}`, quote())}><Icon name="bookmark" size={14} /></button>
                </Show>
                <button title="Ghi chú cho đoạn này" onClick={() => reader.startNote(`b${i()}`, quote())}><Icon name="note" size={14} /></button>
                </span>
              </div>
            </div>
          );
        }}
      </For>
    </article>
    </ReaderLayout>
  );
}

function stripForSpeech(t: string): string {
  // dùng chung logic stripMd nhưng giữ nguyên text hiển thị cho TTS
  return t
    .replace(/!\[\[[^\]]*\]\]/g, " ")
    .replace(/\[\[([^\]|#\]]+)(?:#[^\]|]*)?(?:\|([^\]]*))?\]\]/g, (_m, tt: string, a: string | undefined) => a ?? tt);
}

function joinRel(dir: string, rel: string): string {
  if (!rel || rel.startsWith("/")) return rel.replace(/^\/+/, "");
  const parts: string[] = dir ? dir.split("/") : [];
  for (const seg of rel.split("/")) {
    if (seg === "" || seg === ".") continue;
    if (seg === "..") parts.pop();
    else parts.push(seg);
  }
  return parts.join("/");
}
