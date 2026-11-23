import { marked } from "marked";
import { For, Show, createEffect, createMemo, createResource, createSignal, onCleanup } from "solid-js";
import { api, hashText, speak, type Backlink, type VaultOutline } from "../api";
import { fmtNum, t } from "../i18n";
import { Portal } from "solid-js/web";
import { Icon } from "../icons";
import Lightbox from "./Lightbox";
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
const escAttr = (s: string) => escapeHtml(s).replace(/"/g, "&quot;");

// Công thức $...$ / $$...$$: giữ nguyên TeX trong data-tex (marked không được đụng vào _ và *), KaTeX render sau (lazy).
marked.use({
  extensions: [
    {
      name: "mathBlock",
      level: "block",
      start: (src: string) => src.match(/\$\$/)?.index,
      tokenizer(src: string) {
        const m = /^\$\$([\s\S]+?)\$\$[^\n]*(?:\n|$)/.exec(src);
        if (m) return { type: "mathBlock", raw: m[0], text: m[1].trim() };
      },
      renderer: (t: any) => `<div class="math-block" data-tex="${escAttr(t.text)}">${escapeHtml(t.text)}</div>`,
    },
    {
      name: "mathInline",
      level: "inline",
      start: (src: string) => src.match(/\$/)?.index,
      tokenizer(src: string) {
        const m = /^\$(?!\s)([^$\n]+?)(?<!\s)\$(?!\d)/.exec(src);
        if (m) return { type: "mathInline", raw: m[0], text: m[1] };
      },
      renderer: (t: any) => `<span class="math-inline" data-tex="${escAttr(t.text)}">${escapeHtml(t.text)}</span>`,
    },
  ],
});

/** Frontmatter YAML đầu file Obsidian -> bảng thuộc tính nhỏ (không đọc TTS). */
function frontmatter(md: string): { html: string; rest: string } | null {
  const m = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(md);
  if (!m) return null;
  const rows = m[1]
    .split(/\r?\n/)
    .filter((l) => l.trim())
    .map((l) => {
      const i = l.indexOf(":");
      return i > 0 && !/^\s/.test(l)
        ? `<div class="fm-row"><span class="fm-k">${escapeHtml(l.slice(0, i))}</span><span class="fm-v">${escapeHtml(l.slice(i + 1).trim())}</span></div>`
        : `<div class="fm-row"><span class="fm-k"></span><span class="fm-v">${escapeHtml(l.trim())}</span></div>`;
    })
    .join("");
  return { html: `<div class="frontmatter">${rows}</div>`, rest: md.slice(m[0].length) };
}

/** Callout Obsidian: > [!type]± Tiêu đề. "-" = thu gọn sẵn, "+" = mở sẵn (dùng <details>). */
function callout(raw: string): { html: string; text: string } | null {
  const lines = raw.split("\n");
  const m = /^\s*>\s*\[!([\w-]+)\]([+-]?)[ \t]*(.*)$/.exec(lines[0]);
  if (!m) return null;
  const type = m[1].toLowerCase();
  const title = m[3].trim() || type.charAt(0).toUpperCase() + type.slice(1);
  const body = lines.slice(1).map((l) => l.replace(/^\s*> ?/, "")).join("\n");
  const inner = `<div class="callout-body">${linkifyWiki(marked.parse(body) as string)}</div>`;
  const head = linkifyWiki(marked.parseInline(title) as string);
  const html = m[2]
    ? `<details class="callout" data-callout="${escAttr(type)}"${m[2] === "+" ? " open" : ""}><summary class="callout-title">${head}</summary>${inner}</details>`
    : `<div class="callout" data-callout="${escAttr(type)}"><div class="callout-title">${head}</div>${inner}</div>`;
  return { html, text: `${title}. ${body}` };
}

/** Tách md thành block đọc được. Code fence + math ($$/$$...$$) -> skip (không có nút voice). */
export function blocks(md: string): Block[] {
  const out: Block[] = [];
  const fm = frontmatter(md);
  if (fm) {
    out.push({ kind: "skip", html: fm.html, text: "" });
    md = fm.rest;
  }
  for (const t of marked.lexer(md)) {
    if (t.type === "space") continue;
    switch (t.type) {
      case "code":
        out.push({ kind: "skip", html: marked.parse(t.raw) as string, text: "" });
        break;
      case "heading":
      case "paragraph":
        if (/^\s*\$[^$]+\$\s*$/.test(t.raw)) {
          // đoạn chỉ có 1 công thức: không có gì để đọc
          out.push({ kind: "skip", html: marked.parse(t.raw) as string, text: "" });
        } else if (/^\s*(?:(?:!\[\[[^\]]+\]\]|!\[[^\]]*\]\([^)]*\))\s*)+$/.test(t.raw)) {
          // đoạn chỉ có ảnh: không đọc
          out.push({ kind: "skip", html: linkifyWiki(marked.parse(t.raw) as string), text: "" });
        } else {
          const h = t.type === "heading" ? { level: t.depth as number, label: stripMd(t.text ?? "").replace(/[*_`]/g, "") } : {};
          out.push({ kind: "speak", html: linkifyWiki(marked.parse(t.raw) as string), text: t.text ?? t.raw, ...h });
        }
        break;
      case "blockquote": {
        // annotation/callout của Anh cũng cần TTS -> speak
        const c = callout(t.raw);
        out.push(c ? { kind: "speak", ...c } : { kind: "speak", html: linkifyWiki(marked.parse(t.raw) as string), text: stripMd(t.raw) });
        break;
      }
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
      p = p.replace(/\[\[([^\]|#]*)(?:#([^\]|]*))?(?:\|([^\]]*))?\]\]/g, (_m, t: string, h?: string, a?: string) => {
        const target = t.trim();
        const enc = encodeURIComponent(target);
        const anc = h ? ` data-anchor="${encodeURIComponent(h.trim())}"` : "";
        const label = escapeHtml(a !== undefined && a !== "" ? a : displayOf(target, h));
        return `<a href="#" class="wiki-link" data-wiki="${enc}"${anc}>${label}</a>`;
      });
      // 4) block id Obsidian "^fig-1" cuối đoạn: ẩn, giữ làm đích cho [[#^fig-1]]
      p = p.replace(/(\s|^)\^([A-Za-z0-9-]+)\s*(?=<\/(?:p|li|em|strong)>|$)/g, (_m, sp: string, id: string) => `${sp}<span class="block-id" data-block="${escAttr(id)}"></span>`);
      // 5) #tag Obsidian (chỉ trong chữ, không trong thuộc tính HTML)
      p = p.replace(/(^|>)([^<]*)/g, (_m, gt: string, text: string) =>
        gt + text.replace(/(^|[\s(])#([\p{L}_][\p{L}\p{N}_/-]*)/gu, (_x, pre: string, tag: string) => `${pre}<span class="md-tag">#${tag}</span>`),
      );
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
  return heading ? (name ? `${name} › ${heading}` : heading.replace(/^\^/, "")) : name;
}

function stripMd(s: string) {
  s = s.replace(/!\[\[[^\]]*\]\]/g, " ");
  s = s.replace(/\[\[([^\]|#\]]+)(?:#[^\]|]*)?(?:\|([^\]]*))?\]\]/g, (_m, t: string, a: string | undefined) => a ?? t);
  return s.replace(/^[>\s#]*/, "").trim();
}

const imgUrlCache = new Map<string, string>();
/** Lazy-load ảnh md: key đang tải + hàng đợi theo element (sống qua các lượt effect). */
const imgInflight = new Map<string, Promise<string>>();
const pendingGo = new WeakMap<Element, () => void>();
let imgObs: IntersectionObserver | null = null;
/** Mục lục vault dùng lại 2 phút khi chuyển giữa các file cùng vault. */
let outlineCache: { at: number; v: VaultOutline } | null = null;
async function vaultOutline(path: string): Promise<VaultOutline | null> {
  const c = outlineCache;
  if (c && Date.now() - c.at < 120_000 && c.v.files.some((f) => f.path === path)) return c.v;
  const v = await api.vaultOutline(path).catch(() => null);
  if (v) outlineCache = { at: Date.now(), v };
  return v;
}
/** Thư mục gốc đã tìm thấy ảnh lần trước (thường là gốc vault) -> thử trước, đỡ dò lại (chỉ dùng khi ngoài vault, không có mục lục). */
const imgBases: string[] = [];

/** Ảnh tĩnh: tải bản thu nhỏ theo màn hình (server resize + cache), giữ link gốc để xem phóng to. */
const origOf = new Map<string, string>();
function srcFor(p: string): Promise<string> | string {
  if (!/\.(jpe?g|png|webp|bmp|tiff?)$/i.test(p)) return api.streamUrl(p); // gif/svg giữ bản gốc (link trực tiếp, không blob RAM)
  const u = api.thumbUrl(p, 1920);
  origOf.set(u, api.streamUrl(p));
  return u;
}

/** Chỉ mục tra nhanh từ mục lục vault: đường dẫn đầy đủ + theo tên file. */
interface ImgIndex {
  root: string;
  set: Set<string>;
  byName: Map<string, string[]>;
}
function buildImgIndex(o: VaultOutline): ImgIndex {
  const set = new Set<string>();
  const byName = new Map<string, string[]>();
  for (const p of [...o.files.map((f) => f.path), ...(o.assets ?? [])]) {
    set.add(p);
    const n = p.split("/").pop() ?? p;
    const l = byName.get(n);
    if (l) l.push(p);
    else byName.set(n, [p]);
  }
  return { root: o.root, set, byName };
}

/** Tra đường dẫn thật của target Obsidian (ảnh/embed/link): trong vault tra mục lục, ngoài vault dò mạng. */
async function resolveTarget(mdPath: string, raw: string, idx?: ImgIndex | null): Promise<string | null> {
  const clean = raw.split("#")[0].split("?")[0] || raw;
  const dir = mdPath.includes("/") ? mdPath.slice(0, mdPath.lastIndexOf("/")) : "";
  const ups: string[] = [];
  for (let d = dir; ; d = d.includes("/") ? d.slice(0, d.lastIndexOf("/")) : "") {
    ups.push(d);
    if (!d) break;
  }
  if (idx) {
    // ref tuyệt đối từ gốc vault: /assets/x.png (Obsidian) -> <vaultRoot>/assets/x.png
    if (clean.startsWith("/")) {
      const rp = `${idx.root}/${clean.replace(/^\/+/, "")}`;
      if (idx.set.has(rp)) return rp;
    } else {
      for (let d = dir; ; d = d.includes("/") ? d.slice(0, d.lastIndexOf("/")) : "") {
        const p = joinRel(d, clean);
        if (idx.set.has(p)) return p;
        if (!d) break;
      }
    }
    // chỉ có tên file: tra theo tên trong cây thư mục gần nhất (không gọi /api/search)
    const name = clean.split("/").pop()!;
    const hits = (idx.byName.get(name) ?? []).filter((p) => ups.some((u) => !u || p.startsWith(u + "/")));
    hits.sort((a, b) => a.length - b.length);
    if (hits.length) return hits[0];
    // không có trong mục lục (có thể cache cũ) -> dò mạng như ngoài vault
  }
  const bases = [dir, ...imgBases.filter((b) => ups.includes(b)), ...ups];
  const tried = new Set<string>();
  for (const b of bases) {
    const p = joinRel(b, clean);
    if (tried.has(p)) continue;
    tried.add(p);
    try {
      await api.exists(p);
      if (b !== dir && !imgBases.includes(b)) imgBases.unshift(b);
      return p;
    } catch {
      /* thử chỗ khác */
    }
  }
  // chỉ có tên file: tìm theo tên trong cây thư mục gần nhất
  const name = clean.split("/").pop()!;
  const hit = (await api.search(name)).filter((e) => !e.is_dir && e.name === name && ups.some((u) => !u || e.path.startsWith(u + "/")));
  hit.sort((a, b) => a.path.length - b.path.length);
  return hit.length ? hit[0].path : null;
}

/** Ảnh Obsidian có thể ghi tương đối với file, với gốc vault, hoặc chỉ tên file.
 *  Trong vault: tra mục lục đã tải sẵn (0 request dò, hết loạt 404).
 *  Ngoài vault: dò mạng lần lượt như Obsidian (giữ hành vi cũ). */
async function loadImg(mdPath: string, raw: string, idx?: ImgIndex | null): Promise<string> {
  const t = await resolveTarget(mdPath, raw, idx);
  if (!t) throw new Error("404");
  return srcFor(t);
}

/** Embed ![[note]]: nội dung note khác hiện inline (1 cấp, tối đa 40 dòng). */
const embedHtmlCache = new Map<string, string>();
const embedInflight = new Map<string, Promise<string | null>>();
function embedTarget(raw: string): { file: string; head?: string } {
  const [file, head] = raw.split("|")[0].split("#");
  return { file: file.trim(), head: head?.trim() };
}
async function embedHtml(mdPath: string, raw: string, idx?: ImgIndex | null): Promise<string | null> {
  const { file, head } = embedTarget(raw);
  if (/\.(png|jpe?g|gif|webp|avif|bmp|svg)$/i.test(file)) return null; // ảnh do loader ảnh xử lý
  const key = `${dirOf2(mdPath)}${String.fromCharCode(0)}${raw}`;
  const hit = embedHtmlCache.get(key);
  if (hit !== undefined) return hit;
  let p = embedInflight.get(key);
  if (!p) {
    p = (async () => {
      const t = await resolveTarget(mdPath, file, idx);
      if (!t || !/\.(md|markdown|txt)$/i.test(t)) return null;
      let text: string;
      try {
        text = await api.readText(t);
      } catch {
        return null;
      }
      const lines = text.split("\n").filter((l) => l.trim()).slice(0, 40);
      if (!lines.length) return null;
      // ảnh tương đối trong note được nhúng: viết thành tuyệt đối từ gốc để loader ảnh dùng được
      const base = t.includes("/") ? t.slice(0, t.lastIndexOf("/")) : "";
      let html = linkifyWiki(marked.parse(lines.join("\n")) as string);
      html = html.replace(/data-wsrc="([^"]+)"/g, (_m, s: string) => {
        const d = decodeURIComponent(s);
        return /^(https?:|blob:|data:|#)/.test(d) ? `data-wsrc="${s}"` : `data-wsrc="${encodeURIComponent(joinRel(base, d))}"`;
      });
      const title = head ? `${t.split("/").pop()} › ${head}` : t.split("/").pop() ?? t;
      return `<div class="md-embed-head">◈ ${escapeHtml(title)}</div><div class="md-embed-body">${html}</div>`;
    })().then((h) => {
      embedInflight.delete(key);
      if (h) embedHtmlCache.set(key, h);
      return h;
    });
    embedInflight.set(key, p);
  }
  return p;
}
function dirOf2(p: string) {
  return p.includes("/") ? p.slice(0, p.lastIndexOf("/")) : "";
}

function VoiceButton(props: { text: string; onPlay: () => void }) {
  const [st, setSt] = createSignal<"idle" | "busy" | "play">("idle");
  let audio: HTMLAudioElement | null = null;
  return (
    <button
      class="voice-btn"
      title={t("vw.mdListen")}
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
  /** Gọi sau khi lưu bản sửa (parent tải lại nội dung). */
  onSaved?: () => void;
}) {
  const list = createMemo(() => blocks(props.text));
  const [lb, setLb] = createSignal<{ src: string; alt: string } | null>(null);
  // --- sửa inline: textarea + lưu ngược (cảnh báo khi nằm trong vault mirror từ git)
  const [editing, setEditing] = createSignal(false);
  const [draft, setDraft] = createSignal("");
  const [saving, setSaving] = createSignal(false);
  const [mirrored, setMirrored] = createSignal<string | null>(null);
  let reposLoaded: Promise<void> | null = null;
  const enterEdit = () => {
    setDraft(props.text);
    setMirrored(null);
    setEditing(true);
    reposLoaded ??= api.gitRepos().then((rs) => {
      const hit = rs.find((r) => props.path === r.folder || props.path.startsWith(r.folder + "/"));
      if (hit) setMirrored(hit.folder);
    }).catch(() => {});
  };
  const saveEdit = async () => {
    if (draft() === props.text) return setEditing(false);
    if (mirrored() && !confirm(t("vw.mdMirrorAsk", { dir: mirrored() ?? "" }))) return;
    setSaving(true);
    try {
      await api.saveText(props.path, draft());
      setEditing(false);
      props.onSaved?.();
    } catch (e) {
      (props.onError ?? console.error)(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };
  // đổi file: thoát chế độ sửa
  createEffect(() => {
    props.path;
    setEditing(false);
  });
  // --- backlinks: file nào nhắc tới file này
  const [links] = createResource(() => props.path, (p) => api.backlinks(p).catch((): Backlink[] => []));
  const [outline] = createResource(() => props.path, vaultOutline);
  /** Chỉ mục tra ảnh từ mục lục vault (null = ngoài vault -> dò mạng như cũ). */
  const imgIdx = createMemo(() => {
    const o = outline();
    return o ? buildImgIndex(o) : null;
  });
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
    if (anchor.startsWith("q:")) {
      const q = anchor.slice(2).toLowerCase();
      const i = plain().findIndex((t) => t.toLowerCase().includes(q));
      if (i < 0) return;
      anchor = `b${i}`;
    }
    const el = host?.querySelector<HTMLElement>(`[data-anchor="${anchor}"]`);
    if (!el) return;
    el.scrollIntoView({ block: "start", behavior: "smooth" });
    el.classList.add("flash");
    setTimeout(() => el.classList.remove("flash"), 1400);
  };
  // Mục đang đọc = heading cuối cùng đã cuộn qua đỉnh khung xem.
  const onScroll = () => {
    if (!host?.isConnected) return;
    const hs = host.querySelectorAll<HTMLElement>(".md-block[data-level]");
    const top = (scroller?.getBoundingClientRect().top ?? 0) + 80;
    let cur = "";
    for (const h of Array.from(hs)) {
      if (h.getBoundingClientRect().top > top) break;
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
  const [prepped, setPrepped] = createSignal(false);
  const curHash = createMemo(() => hashText(props.text));
  const stale = () => builtHash() !== null && builtHash() !== curHash();
  createEffect(() => {
    curHash();
    setPrepped(false);
  });
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

  // Tải trước toàn bài vào cache RAM (nghe liền mạch, không chờ từng đoạn).
  // Chưa phải lưu offline lâu dài: tắt/reload trang là mất, phải tải lại.
  const prefetchAll = async () => {
    const items = list().filter((b) => b.kind === "speak" && stripForSpeech(b.text).trim());
    setPrepping([0, items.length]);
    setPrepped(false);
    try {
      for (let i = 0; i < items.length; i++) {
        await speak(stripForSpeech(items[i].text)).catch(() => {});
        setPrepping([i + 1, items.length]);
      }
      setPrepped(true);
    } finally {
      setPrepping(null);
    }
  };

  const onArticleClick = (e: MouseEvent) => {
    const t = e.target as HTMLElement;
    // bấm ảnh: xem toàn màn hình (ảnh to chữ nhỏ)
    if (t instanceof HTMLImageElement && t.closest(".md-content") && !t.closest("a") && t.src && !t.src.startsWith("data:")) return setLb({ src: origOf.get(t.getAttribute("src") ?? "") ?? t.src, alt: t.alt });
    const a = t.closest?.("a.wiki-link") as HTMLElement | null;
    if (!a) return;
    e.preventDefault();
    // [[#^block]] / [[#Heading]]: nhảy trong chính bài này
    if (!a.getAttribute("data-wiki") && a.getAttribute("data-anchor")) return jumpTo(decodeURIComponent(a.getAttribute("data-anchor")!));
    if (!props.onOpenLink) return;
    props.onOpenLink(
      decodeURIComponent(a.getAttribute("data-wiki") ?? ""),
      a.getAttribute("data-anchor") ? decodeURIComponent(a.getAttribute("data-anchor")!) : undefined,
    );
  };

  createEffect(() => {
    // gắn lại sau mỗi lần render danh sách block
    list();
    // 1) embed ![[note]] hiện nội dung inline trước, rồi mới tải ảnh (kể cả ảnh trong note nhúng).
    // Đợi mục lục vault xong mới tra đường dẫn (khớp chính xác, không dò 404);
    // effect đăng ký outline() qua imgIdx() nên tự chạy lại khi mục lục về.
    // Lazy-load: chỉ tải ảnh trong/kề khung nhìn, còn lại đợi cuộn tới.
    const idx = imgIdx();
    const ready = !outline.loading;
    onCleanup(() => { imgObs?.disconnect(); imgObs = null; });
    if (ready) {
      const anchors = Array.from(host?.querySelectorAll<HTMLAnchorElement>("a.wiki-embed:not(.md-embed a):not([data-embed-done])") ?? []);
      anchors.forEach((a) => {
        a.setAttribute("data-embed-done", "1");
        const raw = decodeURIComponent(a.getAttribute("data-wiki") ?? "");
        void embedHtml(props.path, raw, idx).then((html) => {
          if (html && a.isConnected) {
            const div = document.createElement("div");
            div.className = "md-embed";
            div.innerHTML = html;
            a.replaceWith(div);
            hydrate();
          }
        });
      });
    }
    const hydrate = () => {
    const imgs = host?.querySelectorAll<HTMLImageElement>("img[data-wsrc]") ?? [];
    if (ready) {
      const root = host?.closest(".viewer") ?? null;
      const ensureObs = () => {
        if (!imgObs) {
          imgObs = new IntersectionObserver((es) => {
            for (const e of es) {
              if (!e.isIntersecting) continue;
              const t = e.target as HTMLImageElement;
              imgObs?.unobserve(t);
              pendingGo.get(t)?.();
            }
          }, { root, rootMargin: "600px 0px" });
        }
        return imgObs;
      };
      const rb = root instanceof Element ? root.getBoundingClientRect() : null;
      imgs.forEach((img) => {
        const raw = decodeURIComponent(img.getAttribute("data-wsrc") ?? "");
        const key = `${dirOf(props.path)}${String.fromCharCode(0)}${raw}`;
        const hit = imgUrlCache.get(key);
        if (hit) {
          img.src = hit;
          img.removeAttribute("data-wsrc");
          return;
        }
        const go = () => {
          pendingGo.delete(img);
          if (!img.isConnected || !img.hasAttribute("data-wsrc")) return;
          let p = imgInflight.get(key);
          if (!p) {
            p = loadImg(props.path, raw, idx).finally(() => imgInflight.delete(key));
            imgInflight.set(key, p);
          }
          p.then((url) => {
            imgUrlCache.set(key, url);
            if (img.isConnected) {
              // thumb lỗi (ảnh quá lớn / không decode được) -> dùng bản gốc
              img.onerror = () => { const o = origOf.get(url); if (o && img.src !== o) img.src = o; };
              img.src = url;
              img.removeAttribute("data-wsrc");
            }
          })
          .catch(() => {
            if (img.isConnected) {
              img.removeAttribute("data-wsrc");
              img.alt = raw;
            }
          });
        };
        const r = img.getBoundingClientRect();
        if (!rb || (r.top < rb.bottom + 800 && r.bottom > rb.top - 800)) {
          go(); // ảnh đầu trang: tải ngay, ưu tiên xong trước
          return;
        }
        pendingGo.set(img, go);
        ensureObs().observe(img);
      });
    }
    };
    if (ready) hydrate(); // ảnh trong note nhúng: hydrate() lại sau mỗi lần chèn embed (xem trên)
    // 2) công thức: chỉ tải KaTeX khi bài có công thức
    const maths = host?.querySelectorAll<HTMLElement>("[data-tex]:not([data-done])") ?? [];
    if (maths.length) {
      Promise.all([import("katex"), import("katex/dist/katex.min.css")]).then(([{ default: katex }]) => {
        maths.forEach((el) => {
          katex.render(el.dataset.tex ?? "", el, { displayMode: el.classList.contains("math-block"), throwOnError: false });
          el.dataset.done = "1";
        });
      });
    }
    // 3) cuộn tới heading / block (^id) nếu mở link có #section
    if (props.anchor) jumpTo(props.anchor, false);
  });

  /** Nhảy tới block "^id" hoặc heading chứa chữ (link Obsidian). */
  const jumpTo = (anchor: string, smooth = true) => {
    if (anchor.startsWith("^")) {
      const b = host?.querySelector(`.block-id[data-block="${CSS.escape(anchor.slice(1))}"]`)?.closest<HTMLElement>(".md-block");
      if (b?.dataset.anchor) jump(b.dataset.anchor);
      return;
    }
    const norm = (x: string) => x.toLowerCase().replace(/[-_*`]+/g, " ").replace(/\s+/g, " ").trim();
    const needle = norm(anchor);
    const hs = host?.querySelectorAll("h1,h2,h3,h4,h5,h6") ?? [];
    for (const h of Array.from(hs)) {
      if (norm(h.textContent ?? "").includes(needle)) {
        (h as HTMLElement).scrollIntoView({ block: "start", behavior: smooth ? "smooth" : "auto" });
        break;
      }
    }
  };

  const anchorOf = (n: Node) => {
    const el = (n instanceof Element ? n : n.parentElement)?.closest<HTMLElement>(".md-block");
    return el?.dataset.anchor ?? null;
  };
  return (
    <ReaderLayout
      r={reader}
      toc={toc()}
      current={current() || toc()[0]?.anchor || ""}
      onJump={jump}
      anchorOf={anchorOf}
      vault={outline() ? { outline: outline()!, path: props.path, onOpen: (p, h) => props.onOpenLink?.("/" + p, h) } : null}
    >
    <article
      class="md-body"
      ref={(el) => {
        host = el;
        queueMicrotask(() => {
          scroller = el.closest(".viewer");
          scroller?.addEventListener("scroll", onScroll, { passive: true });
          setTimeout(onScroll, 60); // đợi layout xong mới đo vị trí heading
          if (props.mark) setTimeout(() => jump(props.mark!), 50);
        });
      }}
      onClick={onArticleClick}
    >
      <div class="md-bar">
        <button onClick={readAll} disabled={playingAll() || editing()}>
          {playingAll() ? t("vw.mdReading") : (<><Icon name="listen" size={15} /> {t("vw.mdReadAll")}</>)}
        </button>
        <button class="ghost" onClick={prefetchAll} disabled={!!prepping() || playingAll() || editing()} title={t("vw.mdPrefetchTip")}>
          {prepping() ? t("vw.mdPrefetching", { a: prepping()![0], b: prepping()![1] }) : prepped() ? t("vw.mdPrefetched") : t("vw.mdPrefetch")}
        </button>
        <Show when={!editing()} fallback={
          <>
            <button class="primary" onClick={saveEdit} disabled={saving() || draft() === props.text}>{saving() ? t("vw.mdSaving") : t("vw.mdSave")}</button>
            <button class="ghost" onClick={() => setEditing(false)} disabled={saving()}>{t("c.cancel")}</button>
          </>
        }>
          <button class="ghost" onClick={enterEdit} title={t("vw.mdEditTip")}><Icon name="edit" size={15} /> {t("vw.mdEdit")}</button>
        </Show>
      </div>
      <Show when={editing() && mirrored()}>
        <p class="stale">{t("vw.mdMirrorWarn", { dir: mirrored() ?? "" })}</p>
      </Show>
      <Show when={stale()}>
        <p class="stale">{t("vw.mdStale")}</p>
      </Show>
      <Show when={editing()} fallback={
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
                  <button class="mk mk-bm" title={t("vw.mdUnbookmark")} onClick={() => reader.toggleBookmark(`b${i()}`, "")}><Icon name="bookmark" size={14} /></button>
                </Show>
                <Show when={st()}>
                  {(s) => (
                    <button class={`mk mk-note st-${s()}`} title={t("vw.mdViewNotes")} onClick={() => reader.showNotes(anns().find((a) => a.kind === "note")!.id)}>
                      <Icon name={STATE_UI[s()].icon} size={12} />
                      <Show when={anns().filter((a) => a.kind === "note").length > 1}>{anns().filter((a) => a.kind === "note").length}</Show>
                    </button>
                  )}
                </Show>
                <span class="md-tools">
                <Show when={!bm()}>
                  <button title={t("vw.mdBookmarkIt")} onClick={() => reader.toggleBookmark(`b${i()}`, quote())}><Icon name="bookmark" size={14} /></button>
                </Show>
                <button title={t("vw.mdAddNote")} onClick={() => reader.startNote(`b${i()}`, quote())}><Icon name="note" size={14} /></button>
                </span>
              </div>
            </div>
          );
        }}
      </For>
      }>
        <textarea class="md-edit" value={draft()} onInput={(e) => setDraft(e.currentTarget.value)} spellcheck={false} placeholder="Markdown…" />
      </Show>
      <Show when={!editing() && (links() ?? []).length}>
        <section class="md-links">
          <b>{t("vw.mdLinks", { n: fmtNum((links() ?? []).length) })}</b>
          <For each={links() ?? []}>
            {(l) => (
              <button class="md-linkref" onClick={() => props.onOpenLink?.("/" + l.path)} title={l.path}>
                <b>{l.name}</b>
                <span class="muted xs">{l.snippet.replace(/\s+/g, " ").slice(0, 140)}</span>
              </button>
            )}
          </For>
        </section>
      </Show>
      <Show when={lb()}>{(l) => <Portal><Lightbox src={l().src} alt={l().alt} onClose={() => setLb(null)} /></Portal>}</Show>
    </article>
    </ReaderLayout>
  );
}

function stripForSpeech(t: string): string {
  // dùng chung logic stripMd nhưng giữ nguyên text hiển thị cho TTS
  return t
    .replace(/\$\$[\s\S]*?\$\$/g, " ")
    .replace(/\$[^$\n]+\$/g, " ")
    .replace(/!\[\[[^\]]*\]\]/g, " ")
    .replace(/(^|\s)\^[A-Za-z0-9-]+\s*$/gm, "$1")
    .replace(/\[\[([^\]|#\]]*)(?:#[^\]|]*)?(?:\|([^\]]*))?\]\]/g, (_m, tt: string, a: string | undefined) => a ?? tt);
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
