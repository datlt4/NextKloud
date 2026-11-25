import { marked } from "marked";

// Lọc HTML sinh từ markdown (file có thể do người khác ghi qua thư mục chia sẻ): bỏ thẻ chạy được
// script/nhúng trang, thuộc tính on*, và URL javascript:/vbscript:/data: (trừ data:image raster).
const BAD_TAGS = "script,iframe,frame,frameset,object,embed,applet,base,meta,link,style,form,template,noscript,svg,math";
const URL_ATTRS = new Set(["href", "src", "xlink:href", "action", "formaction", "poster", "background", "srcset", "data"]);
const badUrl = (v: string) => {
  const s = v.replace(/[\u0000- ]/g, "").toLowerCase();
  return /^(javascript|vbscript):/.test(s) || (s.startsWith("data:") && !/^data:image\/(png|jpe?g|gif|webp|avif|bmp);/.test(s));
};

export function sanitizeHtml(html: string): string {
  const doc = new DOMParser().parseFromString(`<body>${html}</body>`, "text/html");
  doc.body.querySelectorAll(BAD_TAGS).forEach((n) => n.remove());
  doc.body.querySelectorAll("*").forEach((el) => {
    for (const a of Array.from(el.attributes)) {
      const n = a.name.toLowerCase();
      if (n.startsWith("on") || n === "style" && /expression|url\s*\(/i.test(a.value) || (URL_ATTRS.has(n) && badUrl(a.value))) el.removeAttribute(a.name);
    }
  });
  return doc.body.innerHTML;
}

// Mọi marked.parse/parseInline trong app (markdown viewer, mô tả card) đều đi qua bộ lọc.
marked.use({ hooks: { postprocess: sanitizeHtml } });
