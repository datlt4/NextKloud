"""NextKloud TTS: edge-tts (giọng Microsoft, miễn phí) + detect ngôn ngữ bằng LibreTranslate.

POST /synthesize {"text": "...", "voice"?: "vi-VN-HoaiMyNeural", "rate"?: "+0%"}
  -> mp3 (cache theo hash). Không truyền voice = tự detect EN/VI/ZH.
GET  /detect?q=...     -> {"language": "vi", "confidence": 0.9}
GET  /voices           -> map ngôn ngữ -> voice mặc định
GET  /health           -> {"status": "ok"}
"""
import hashlib
import json
import os
import re
import urllib.request

from fastapi import FastAPI, HTTPException
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import FileResponse
from pydantic import BaseModel

LIBRE = os.environ.get("LIBRETRANSLATE_URL", "http://host.docker.internal:8015")
DATA = os.environ.get("TTS_DATA", "/data")
DEFAULT_RATE = os.environ.get("TTS_RATE", "+0%")

# Giọng Neural mặc định cho 3 ngôn ngữ yêu cầu + giọng dự phòng (Microsoft
# thỉnh thoảng không trả audio với vài cụm từ — thử giọng khác thường qua được).
VOICES = {
    "en": "en-US-AriaNeural",
    "vi": "vi-VN-HoaiMyNeural",
    "zh": "zh-CN-XiaoxiaoNeural",
}
FALLBACK_VOICES = {
    "en": "en-US-GuyNeural",
    "vi": "vi-VN-NamMinhNeural",
    "zh": "zh-CN-YunjianNeural",
}

app = FastAPI()
# Web client gọi trực tiếp (browser/Tauri): cho phép mọi origin (LAN).
app.add_middleware(
    CORSMiddleware, allow_origins=["*"], allow_methods=["*"], allow_headers=["*"]
)


def detect_lang(text: str) -> str:
    """LibreTranslate trước, heuristic fallback (offline-safe)."""
    try:
        req = urllib.request.Request(
            LIBRE + "/detect",
            data=json.dumps({"q": text[:500]}).encode(),
            headers={"Content-Type": "application/json"},
        )
        r = json.load(urllib.request.urlopen(req, timeout=10))
        if r and r[0].get("language"):
            lang = r[0]["language"].lower()
            if lang.startswith("zh"):
                return "zh"
            return lang if lang in VOICES else "en"
    except Exception:
        pass
    if re.search(r"[\u4e00-\u9fff]", text):
        return "zh"
    if re.search(
        r"[àáạảãâầấậẩẫăằắặẳẵèéẹẻẽêềếệểễìíịỉĩòóọỏõôồốộổỗơờớợởỡùúụủũưừứựửữỳýỵỷỹđ]",
        text,
        re.I,
    ):
        return "vi"
    return "en"


def strip_md(t: str) -> str:
    """Bỏ cú pháp md, giữ chữ để đọc. Code/math do frontend loại trước khi gọi."""
    t = re.sub(r"```.*?```", " ", t, flags=re.S)  # code fence sót lại
    t = re.sub(r"`([^`]*)`", r"\1", t)
    t = re.sub(r"!\[\[[^\]]*\]\]", " ", t)  # embed wikilink (ảnh/file) -> bỏ
    t = re.sub(r"\[\[([^\]|]*\|)?([^\]|]*)\]\]", r"\2", t)  # [[a|b]] -> b
    t = re.sub(r"!\[([^\]]*)\]\([^)]*\)", r"\1", t)  # ảnh -> alt text
    t = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", t)  # link -> text
    t = re.sub(r"\$\$.*?\$\\$", " ", t, flags=re.S)  # math display sót lại
    t = re.sub(r"\$.+?\$", " ", t)  # math inline sót lại
    t = re.sub(r"[#>*_\-~]+", " ", t)
    return re.sub(r"\s+", " ", t).strip()

# Google Translate TTS (miễn phí, không key): dự phòng khi edge-tts bị throttle.
# Endpoint công khai của web translate.google.com, cần User-Agent trình duyệt.
GTTS_UA = (
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/126.0 Safari/537.36"
)
GTTS_LANG = {"en": "en", "vi": "vi", "zh": "zh-CN"}


def _split_gtts(text: str, limit: int = 180) -> list[str]:
    """Chia text thành chunk ngắn tại ranh giới câu/từ (URL query giới hạn độ dài)."""
    parts = re.split(r"(?<=[.!?…。！？])\s+|\n+", text)
    chunks: list[str] = []
    cur = ""
    for p in parts:
        p = p.strip()
        if not p:
            continue
        if len(cur) + len(p) + 1 <= limit:
            cur = f"{cur} {p}".strip()
            continue
        if cur:
            chunks.append(cur)
        while len(p) > limit:  # câu quá dài: cắt cứng theo từ
            cut = p.rfind(" ", 0, limit)
            cut = cut if cut > 0 else limit
            chunks.append(p[:cut])
            p = p[cut:].strip()
        cur = p
    if cur:
        chunks.append(cur)
    return chunks or [text[:limit]]


def gtts_synthesize(text: str, lang: str) -> bytes:
    """Đọc bằng Google Translate, nối các chunk mp3 lại (cùng codec nên play liền mạch)."""
    tl = GTTS_LANG.get(lang, "en")
    out = bytearray()
    for ch in _split_gtts(text):
        req = urllib.request.Request(
            "https://translate.google.com/translate_tts?ie=UTF-8&client=tw-ob"
            + f"&tl={tl}&q={urllib.parse.quote(ch)}",
            headers={"User-Agent": GTTS_UA, "Referer": "https://translate.google.com/"},
        )
        with urllib.request.urlopen(req, timeout=20) as r:
            data = r.read()
        if not data:
            raise RuntimeError(f"google tts trả rỗng (lang={tl})")
        out.extend(data)
    if not out:
        raise RuntimeError("google tts không trả audio")
    return bytes(out)




class SynthBody(BaseModel):
    text: str
    voice: str = ""
    rate: str = ""
    # "auto" (mặc định): edge-tts trước, google khi edge lỗi/throttle.
    # "edge" | "google": ép dùng đúng 1 provider.
    provider: str = "auto"


@app.get("/health")
def health():
    return {"status": "ok", "service": "nextkloud-tts"}


@app.get("/voices")
def voices():
    return VOICES


@app.get("/detect")
def detect(q: str):
    return {"language": detect_lang(q)}


@app.post("/synthesize")
async def synthesize(b: SynthBody):
    import asyncio

    import edge_tts

    text = strip_md(b.text)
    if not text:
        raise HTTPException(400, "empty text after stripping")
    if len(text) > 5000:
        raise HTTPException(400, "text too long (max 5000 chars per paragraph)")
    voice = b.voice or VOICES[detect_lang(text)]
    rate = b.rate or DEFAULT_RATE
    provider = (b.provider or "auto").lower()
    lang = detect_lang(text)
    # Microsoft throttle khi gọi dồn: retry cùng giọng + giọng phụ, giãn cách.
    tried = [voice] + ([] if b.voice else [FALLBACK_VOICES.get(lang, voice)])
    waits = [0, 8, 20]
    pkey = provider if provider in ("edge", "google") else ""
    key = hashlib.sha256(f"{voice}|{rate}|{text}|{pkey}".encode()).hexdigest()
    os.makedirs(DATA, exist_ok=True)
    out = os.path.join(DATA, f"{key}.mp3")
    # Tự heal: file rỗng (fail giữa chừng) thì render lại.
    if os.path.exists(out) and os.path.getsize(out) == 0:
        os.remove(out)
    if not os.path.exists(out):
        tmp = out + ".tmp"
        err: Exception | None = RuntimeError("no attempt made")
        done = False
        if provider in ("auto", "edge"):
            for v in dict.fromkeys(tried):  # giữ thứ tự, bỏ trùng
                for w in waits:
                    if w:
                        await asyncio.sleep(w)
                    try:
                        await edge_tts.Communicate(text, v, rate=rate).save(tmp)
                        if os.path.getsize(tmp) == 0:
                            raise RuntimeError(f"empty audio from {v} (likely throttled)")
                        err = None
                        done = True
                        break
                    except Exception as e:
                        err = e
                        continue
                if done:
                    break
        if not done and provider in ("auto", "google"):
            # Dự phòng Google Translate TTS (miễn phí, không key).
            try:
                data = await asyncio.to_thread(gtts_synthesize, text, lang)
                with open(tmp, "wb") as f:
                    f.write(data)
                err = None
                done = True
            except Exception as e:
                err = e
        if not done:
            if os.path.exists(tmp):
                os.remove(tmp)
            raise HTTPException(502, f"tts failed (edge+google): {err}")
        os.replace(tmp, out)  # atomic, tránh file nửa vời
    return FileResponse(out, media_type="audio/mpeg", filename=f"{key}.mp3")
