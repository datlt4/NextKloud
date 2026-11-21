export type VKind = "image" | "video" | "audio" | "pdf" | "zip" | "epub" | "drawio" | "excalidraw" | "office" | "markdown" | "text" | "other";

const IMAGE = new Set(["jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff"]);
const VIDEO = new Set(["mp4", "m4v", "mov", "mkv", "webm", "avi", "3gp", "ts", "wmv", "flv", "mpg", "mpeg"]);
const AUDIO = new Set(["mp3", "m4a", "ogg", "oga", "opus", "wav", "flac"]);
const OFFICE = new Set(["doc", "docx", "odt", "rtf", "xls", "xlsx", "ods", "csv", "ppt", "pptx", "odp"]);
const TEXT = new Set(["txt", "log", "json", "yaml", "yml", "csv", "xml", "html", "htm", "sh", "py", "js", "ts", "tsx", "rs", "go", "java", "c", "h", "cpp", "ini", "toml", "conf", "sql", "css", "vue"]);

export function extOfName(name: string): string {
  const i = name.lastIndexOf(".");
  return i < 0 ? "" : name.slice(i + 1).toLowerCase();
}

export function viewerKind(name: string): VKind {
  const e = extOfName(name);
  if (e === "md" || e === "markdown") return "markdown";
  if (IMAGE.has(e)) return "image";
  if (VIDEO.has(e)) return "video";
  if (AUDIO.has(e)) return "audio";
  if (e === "pdf") return "pdf";
  if (e === "zip" || e === "cbz") return "zip";
  if (e === "epub") return "epub";
  if (e === "drawio" || e === "dio") return "drawio";
  if (e === "excalidraw") return "excalidraw";
  if (OFFICE.has(e)) return "office";
  if (TEXT.has(e)) return "text";
  return "other";
}

/** Có thumbnail server-side không (ảnh/video). */
export function hasThumb(name: string): boolean {
  const e = extOfName(name);
  return IMAGE.has(e) || VIDEO.has(e);
}
