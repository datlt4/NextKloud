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
    t = re.sub(r"!\[([^\]]*)\]\([^)]*\)", r"\1", t)  # ảnh -> alt text
    t = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", t)  # link -> text
    t = re.sub(r"\$\$.*?\$\\$", " ", t, flags=re.S)  # math display sót lại
    t = re.sub(r"\$.+?\$", " ", t)  # math inline sót lại
    t = re.sub(r"[#>*_\-~]+", " ", t)
    return re.sub(r"\s+", " ", t).strip()


class SynthBody(BaseModel):
    text: str
    voice: str = ""
    rate: str = ""


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
    import edge_tts

    text = strip_md(b.text)
    if not text:
        raise HTTPException(400, "empty text after stripping")
    if len(text) > 5000:
        raise HTTPException(400, "text too long (max 5000 chars per paragraph)")
    voice = b.voice or VOICES[detect_lang(text)]
    rate = b.rate or DEFAULT_RATE
    tried = [voice] + ([FALLBACK_VOICES.get(detect_lang(text))] if not b.voice else [])
    key = hashlib.sha256(f"{voice}|{rate}|{text}".encode()).hexdigest()
    os.makedirs(DATA, exist_ok=True)
    out = os.path.join(DATA, f"{key}.mp3")
    # Tự heal: file rỗng (edge-tts fail giữa chừng) thì render lại.
    if os.path.exists(out) and os.path.getsize(out) == 0:
        os.remove(out)
    if not os.path.exists(out):
        tmp = out + ".tmp"
        err: Exception | None = None
        for v in dict.fromkeys(tried):  # giữ thứ tự, bỏ trùng
            try:
                await edge_tts.Communicate(text, v, rate=rate).save(tmp)
                if os.path.getsize(tmp) == 0:
                    raise RuntimeError(f"empty audio from {v}")
                err = None
                break
            except Exception as e:
                err = e
                continue
        if err is not None:
            if os.path.exists(tmp):
                os.remove(tmp)
            raise HTTPException(502, f"edge-tts failed: {err}")
        os.replace(tmp, out)  # atomic, tránh file nửa vời
    return FileResponse(out, media_type="audio/mpeg", filename=f"{key}.mp3")
