// Inline stroke icons (24x24) — không thêm dep/font icon.
const P: Record<string, string> = {
  folder: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z",
  file: "M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8zM14 3v5h5",
  image: "M5 4h14a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1zM4 16l5-5 4 4 3-3 4 4M15.5 8.5h.01",
  video: "M4 6h11a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1zM16 10l5-3v10l-5-3",
  music: "M9 18V5l11-2v13M9 18a3 3 0 1 1-6 0 3 3 0 0 1 6 0zM20 16a3 3 0 1 1-6 0 3 3 0 0 1 6 0z",
  archive: "M4 4h16v4H4zM5 8v11a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V8M10 12h4",
  book: "M4 19.5A2.5 2.5 0 0 1 6.5 17H20V3H6.5A2.5 2.5 0 0 0 4 5.5zM4 19.5A2.5 2.5 0 0 0 6.5 22H20v-5",
  text: "M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8zM14 3v5h5M9 13h6M9 17h6",
  code: "M8 8l-5 4 5 4M16 8l5 4-5 4M14 4l-4 16",
  home: "M3 11l9-8 9 8M5 10v10h14V10",
  upload: "M12 16V4M7 9l5-5 5 5M4 20h16",
  plus: "M12 5v14M5 12h14",
  grid: "M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z",
  list: "M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01",
  search: "M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14zM21 21l-5-5",
  menu: "M4 6h16M4 12h16M4 18h16",
  more: "M5 12h.01M12 12h.01M19 12h.01",
  chevron: "M9 6l6 6-6 6",
  down: "M6 9l6 6 6-6",
  palette: "M12 3a9 9 0 1 0 0 18c1 0 1.5-.7 1.5-1.5 0-.4-.2-.8-.4-1.1-.3-.3-.4-.6-.4-1 0-.8.7-1.4 1.5-1.4H16a5 5 0 0 0 5-5c0-4.4-4-8-9-8zM7.5 11h.01M10.5 7.5h.01M15 7.5h.01",
  logout: "M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4M16 17l5-5-5-5M21 12H9",
  trash: "M3 6h18M8 6V4h8v2M6 6l1 14h10l1-14",
  download: "M12 4v12M7 11l5 5 5-5M4 20h16",
  close: "M6 6l12 12M18 6L6 18",
  back: "M15 18l-6-6 6-6",
  cloud: "M7 18h10a4 4 0 0 0 .5-8A6 6 0 0 0 6 9.5 4.3 4.3 0 0 0 7 18z",
  check: "M5 12l5 5 9-10",
};

export function Icon(props: { name: string; size?: number; class?: string }) {
  return (
    <svg
      class={props.class ?? "ic"}
      width={props.size ?? 18}
      height={props.size ?? 18}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.8"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d={P[props.name] ?? P.file} />
    </svg>
  );
}

const KINDS: [RegExp, string, string][] = [
  [/\.(png|jpe?g|gif|webp|avif|bmp|svg|heic)$/i, "image", "#22c55e"],
  [/\.(mp4|mkv|webm|mov|avi|m4v)$/i, "video", "#f43f5e"],
  [/\.(mp3|flac|wav|ogg|m4a|opus)$/i, "music", "#a855f7"],
  [/\.(zip|rar|7z|tar|gz|xz|bz2)$/i, "archive", "#f59e0b"],
  [/\.(epub|mobi|azw3|fb2|cbz)$/i, "book", "#14b8a6"],
  [/\.pdf$/i, "text", "#ef4444"],
  [/\.(docx?|odt|rtf)$/i, "text", "#3b82f6"],
  [/\.(xlsx?|ods|csv)$/i, "text", "#16a34a"],
  [/\.(pptx?|odp)$/i, "text", "#ea580c"],
  [/\.(md|txt)$/i, "text", "#94a3b8"],
  [/\.(drawio|excalidraw)$/i, "image", "#f97316"],
  [/\.(rs|ts|tsx|js|py|c|cpp|h|go|java|json|ya?ml|toml|sh|html|css)$/i, "code", "#38bdf8"],
];

export function kindOf(name: string, isDir: boolean): { icon: string; color: string } {
  if (isDir) return { icon: "folder", color: "#e07a45" };
  for (const [re, icon, color] of KINDS) if (re.test(name)) return { icon, color };
  return { icon: "file", color: "#94a3b8" };
}

export function ext(name: string): string {
  const i = name.lastIndexOf(".");
  return i > 0 ? name.slice(i + 1).toUpperCase().slice(0, 4) : "";
}
