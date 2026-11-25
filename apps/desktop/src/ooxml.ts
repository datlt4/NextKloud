// Tạo file Office trống hợp lệ ngay trên client: zip kiểu stored (không nén),
// backend nhận qua API upload thường nên tự có quota/index/resume như mọi file khác.
export type OfficeKind = "docx" | "xlsx" | "pptx";

const CRC_TABLE: Uint32Array = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

function crc32(d: Uint8Array): number {
  let c = 0xffffffff;
  for (let i = 0; i < d.length; i++) c = CRC_TABLE[(c ^ d[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

const te = new TextEncoder();
const enc = (s: string) => te.encode(s);

function u16(v: number, out: number[], o: { at: number }): void {
  out[o.at++] = v & 0xff;
  out[o.at++] = (v >> 8) & 0xff;
}
function u32(v: number, out: number[], o: { at: number }): void {
  u16(v & 0xffff, out, o);
  u16((v >>> 16) & 0xffff, out, o);
}

export function zipStore(files: { name: string; data: Uint8Array }[]): Uint8Array<ArrayBuffer> {
  const names = files.map((f) => enc(f.name));
  let total = 22;
  for (let i = 0; i < files.length; i++) total += 30 + names[i].length + files[i].data.length + 46 + names[i].length;
  const out: number[] = new Array(total).fill(0);
  const o = { at: 0 };
  const offs: number[] = [];
  const wbytes = (d: Uint8Array) => {
    for (let i = 0; i < d.length; i++) out[o.at++] = d[i];
  };
  for (let i = 0; i < files.length; i++) {
    const f = files[i];
    const nm = names[i];
    const crc = crc32(f.data);
    offs.push(o.at);
    u32(0x04034b50, out, o);
    u16(20, out, o);
    u16(0x0800, out, o); // UTF-8
    u16(0, out, o); // stored
    u16(0, out, o); // time
    u16(0x21, out, o); // date 1980-01-01
    u32(crc, out, o);
    u32(f.data.length, out, o);
    u32(f.data.length, out, o);
    u16(nm.length, out, o);
    u16(0, out, o);
    wbytes(nm);
    wbytes(f.data);
    (f as { _crc?: number })._crc = crc;
  }
  const cdStart = o.at;
  let cdSize = 0;
  for (let i = 0; i < files.length; i++) {
    const f = files[i];
    const nm = names[i];
    const crc = (f as { _crc?: number })._crc ?? 0;
    const before = o.at;
    u32(0x02014b50, out, o);
    u16(63, out, o);
    u16(20, out, o);
    u16(0x0800, out, o);
    u16(0, out, o);
    u16(0, out, o);
    u16(0x21, out, o);
    u32(crc, out, o);
    u32(f.data.length, out, o);
    u32(f.data.length, out, o);
    u16(nm.length, out, o);
    u16(0, out, o);
    u16(0, out, o);
    u16(0, out, o);
    u16(0, out, o);
    u32(0, out, o);
    u32(offs[i], out, o);
    wbytes(nm);
    cdSize += o.at - before;
  }
  u32(0x06054b50, out, o);
  u16(0, out, o);
  u16(0, out, o);
  u16(files.length, out, o);
  u16(files.length, out, o);
  u32(cdSize, out, o);
  u32(cdStart, out, o);
  u16(0, out, o);
  return new Uint8Array(out);
}

const RELS = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="WORD_DOC_PLACEHOLDER"/></Relationships>`;

function docx(): Uint8Array<ArrayBuffer> {
  const ct = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>`;
  return zipStore([
    { name: "[Content_Types].xml", data: enc(ct) },
    { name: "_rels/.rels", data: enc(RELS.replace("WORD_DOC_PLACEHOLDER", "word/document.xml")) },
    {
      name: "word/document.xml",
      data: enc(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p/></w:body></w:document>`),
    },
  ]);
}

function xlsx(): Uint8Array<ArrayBuffer> {
  const ct = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>`;
  return zipStore([
    { name: "[Content_Types].xml", data: enc(ct) },
    { name: "_rels/.rels", data: enc(RELS.replace("WORD_DOC_PLACEHOLDER", "xl/workbook.xml")) },
    {
      name: "xl/workbook.xml",
      data: enc(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>`),
    },
    {
      name: "xl/_rels/workbook.xml.rels",
      data: enc(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>`),
    },
    {
      name: "xl/worksheets/sheet1.xml",
      data: enc(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>`),
    },
  ]);
}

function pptx(): Uint8Array<ArrayBuffer> {
  const ct = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/><Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/><Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/><Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/></Types>`;
  const rels = (target: string) =>
    `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="${target}"/></Relationships>`;
  return zipStore([
    { name: "[Content_Types].xml", data: enc(ct) },
    { name: "_rels/.rels", data: enc(rels("ppt/presentation.xml")) },
    {
      name: "ppt/presentation.xml",
      data: enc(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldMasterIdLst><p:sldMasterId r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst><p:sldSz cx="9144000" cy="5143500"/><p:notesSz cx="6858000" cy="9144000"/></p:presentation>`),
    },
    {
      name: "ppt/_rels/presentation.xml.rels",
      data: enc(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/></Relationships>`),
    },
    {
      name: "ppt/slides/slide1.xml",
      data: enc(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld><p:clrMapOvr><a:overrideClrMapping xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/></p:clrMapOvr></p:sld>`),
    },
    {
      name: "ppt/slides/_rels/slide1.xml.rels",
      data: enc(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/></Relationships>`),
    },
    {
      name: "ppt/slideLayouts/slideLayout1.xml",
      data: enc(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:sldLayout xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" type="blank"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/></p:sldLayout>`),
    },
    {
      name: "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
      data: enc(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/></Relationships>`),
    },
    {
      name: "ppt/slideMasters/slideMaster1.xml",
      data: enc(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:sldMaster xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld><p:txStyles/><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/></p:sldMaster>`),
    },
    {
      name: "ppt/slideMasters/_rels/slideMaster1.xml.rels",
      data: enc(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/></Relationships>`),
    },
    {
      name: "ppt/theme/theme1.xml",
      data: enc(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?><a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office"><a:themeElements><a:clrScheme name="Office"><a:dk1><a:srgbClr val="000000"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="1F497D"/></a:dk2><a:lt2><a:srgbClr val="EEECE1"/></a:lt2><a:accent1><a:srgbClr val="4F81BD"/></a:accent1><a:accent2><a:srgbClr val="C0504D"/></a:accent2><a:accent3><a:srgbClr val="9BBB59"/></a:accent3><a:accent4><a:srgbClr val="806000"/></a:accent4><a:accent5><a:srgbClr val="9B30FF"/></a:accent5><a:accent6><a:srgbClr val="00B0F0"/></a:accent6><a:hlink><a:srgbClr val="0000FF"/></a:hlink><a:folHlink><a:srgbClr val="800080"/></a:folHlink></a:clrScheme><a:fmtScheme name="Office"><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:fillStyleLst><a:lnStyleLst><a:ln><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>`),
    },
  ]);
}

export function buildOffice(kind: OfficeKind): Uint8Array<ArrayBuffer> {
  return kind === "docx" ? docx() : kind === "xlsx" ? xlsx() : pptx();
}
