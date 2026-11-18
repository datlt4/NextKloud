import { marked } from "marked";
import { For, Show, createMemo, createSignal } from "solid-js";
import { hashText, speak } from "../api";

export interface Block {
  kind: "speak" | "skip";
  html: string;
  text: string;
}

/** Tách md thành block đọc được. Code fence + math ($$/$$...$$) -> skip (không nút voice). */
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
          out.push({ kind: "speak", html: marked.parse(t.raw) as string, text: t.text ?? t.raw });
        }
        break;
      case "blockquote":
        // annotation/callout của Anh cũng cần TTS -> speak
        out.push({ kind: "speak", html: marked.parse(t.raw) as string, text: stripMd(t.raw) });
        break;
      case "list":
        for (const item of t.items) {
          out.push({ kind: "speak", html: marked.parse(item.text) as string, text: item.text });
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

function escapeHtml(s: string) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function stripMd(s: string) {
  return s.replace(/^[>\s#]*/, "").trim();
}

function VoiceButton(props: { text: string; onPlay: () => void }) {
  const [st, setSt] = createSignal<"idle" | "busy" | "play">("idle");
  let audio: HTMLAudioElement | null = null;
  return (
    <button
      class="voice-btn"
      title="Nghe đoạn này"
      disabled={st() === "busy"}
      onClick={async () => {
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
      {st() === "busy" ? "…" : st() === "play" ? "⏸" : "🔊"}
    </button>
  );
}

export default function MarkdownView(props: { path: string; text: string }) {
  const list = createMemo(() => blocks(props.text));
  // Stale: hash lúc mở vs hash lúc ấn "đọc cả bài".
  const [builtHash, setBuiltHash] = createSignal<string | null>(null);
  const [playingAll, setPlayingAll] = createSignal(false);
  const curHash = createMemo(() => hashText(props.text));
  const stale = () => builtHash() !== null && builtHash() !== curHash();

  const readAll = async () => {
    setBuiltHash(curHash());
    setPlayingAll(true);
    try {
      for (const b of list()) {
        if (b.kind !== "speak" || !b.text.trim()) continue;
        const url = await speak(b.text);
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

  return (
    <article class="md-body">
      <div class="md-bar">
        <b>{props.path}</b>
        <button onClick={readAll} disabled={playingAll()}>
          {playingAll() ? "Đang đọc…" : "🔊 Đọc cả bài"}
        </button>
      </div>
      <Show when={stale()}>
        <p class="stale">⚠️ Bài đã sửa sau khi bấm đọc — âm thanh đã cũ, bấm "Đọc cả bài" lại để render mới.</p>
      </Show>
      <For each={list()}>
        {(b) => (
          <div class="md-block">
            <Show when={b.kind === "speak"}>
              <VoiceButton text={b.text} onPlay={() => setBuiltHash(curHash())} />
            </Show>
            <div innerHTML={b.html} />
          </div>
        )}
      </For>
    </article>
  );
}
