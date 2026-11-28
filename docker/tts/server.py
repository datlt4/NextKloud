"""NextKloud TTS: edge-tts (giọng Microsoft, miễn phí) + detect ngôn ngữ bằng LibreTranslate.

POST /synthesize {"text": "...", "voice"?: "vi-VN-HoaiMyNeural", "rate"?: "+0%", "provider"?: "auto"}
  -> mp3/wav (cache theo hash). Không truyền voice = tự detect EN/VI/ZH.
  provider "auto" (mặc định): tiếng Việt -> VienEU (192.168.0.186:7861, giọng Ngọc Huyền),
    engine khác -> edge-tts trước, google khi edge lỗi/throttle.
  provider "vieneu" | "edge" | "google": ép dùng đúng 1 engine.
GET  /detect?q=...     -> {"language": "vi", "confidence": 0.9}
GET  /voices           -> map ngôn ngữ -> voice mặc định
GET  /health           -> {"status": "ok"}
"""
import asyncio
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
# Adapter VienEU (Gradio) trên mini PC: trong stack nextkloud gọi qua tên service,
# chạy lẻ ngoài docker thì dùng local của máy đó.
VIENEU_URL = os.environ.get("VIENEU_URL", "http://127.0.0.1:7861").rstrip("/")
VIENEU_VOICE = os.environ.get("VIENEU_VOICE", "Ngọc Huyền")
# Adapter xử lý tuần tự 1 job/lúc: khóa chung để request sau chờ request trước.
_vieneu_lock = asyncio.Lock()
# Cache mp3 giới hạn dung lượng: quá thì xoá file lâu không nghe nhất (LRU theo mtime).
CACHE_MAX = int(os.environ.get("TTS_CACHE_MAX_MB", "5120")) * 1024 * 1024
_new_since_prune = 0


def prune_cache() -> None:
    files = []
    total = 0
    with os.scandir(DATA) as it:
        for e in it:
            if e.name.endswith((".mp3", ".wav")) and e.is_file():
                st = e.stat()
                files.append((st.st_mtime, st.st_size, e.path))
                total += st.st_size
    if total <= CACHE_MAX:
        return
    for _, size, path in sorted(files):  # cũ nhất trước
        try:
            os.remove(path)
        except OSError:
            continue
        total -= size
        if total <= CACHE_MAX * 0.9:
            break


def _note_new() -> None:
    """Đếm file cache mới; mỗi 50 file quét prune 1 lần."""
    global _new_since_prune
    _new_since_prune += 1
    if _new_since_prune >= 50:
        _new_since_prune = 0
        prune_cache()

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


def _vieneu_cues(text: str, limit: int = 300) -> list[str]:
    """Chia text thành cue ngắn tại ranh giới câu (adapter đọc theo từng cue)."""
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
        while len(p) > limit:
            cut = p.rfind(" ", 0, limit)
            cut = cut if cut > 0 else limit
            chunks.append(p[:cut])
            p = p[cut:].strip()
        cur = p
    if cur:
        chunks.append(cur)
    return chunks or [text[:limit]]


def _vieneu_srt(text: str) -> bytes:
    """Text thường -> file SRT (timing giả, adapter đọc theo cue)."""
    out: list[str] = []
    t = 0
    for i, cue in enumerate(_vieneu_cues(text), 1):
        dur = max(3, min(15, len(cue) // 15))

        def ts(s: int) -> str:
            return f"{s // 3600:02d}:{(s // 60) % 60:02d}:{s % 60:02d},000"

        out += [str(i), f"{ts(t)} --> {ts(t + dur)}", cue, ""]
        t += dur + 1
    return ("\n".join(out) + "\n").encode("utf-8")


def _http(method: str, url: str, body: bytes | None = None, headers: dict | None = None, timeout: int = 60) -> tuple[int, bytes]:
    req = urllib.request.Request(url, data=body, headers=headers or {}, method=method)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()


def _vieneu_wav(text: str, voice: str) -> bytes:
    """Tổng hợp qua adapter VienEU (Gradio): upload SRT -> call wrapper_2 -> đợi SSE -> tải wav."""
    import uuid as _uuid

    base = VIENEU_URL
    # 1. upload SRT
    bound = "----nk" + _uuid.uuid4().hex
    srt = _vieneu_srt(text)
    body = (
        f"--{bound}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"nk.srt\"\r\n"
        f"Content-Type: application/x-subrip\r\n\r\n".encode() + srt + f"\r\n--{bound}--\r\n".encode()
    )
    st, raw = _http("POST", f"{base}/gradio_api/upload?upload_id={_uuid.uuid4().hex}", body,
                    {"Content-Type": f"multipart/form-data; boundary={bound}"}, 60)
    if st != 200:
        raise RuntimeError(f"vieneu upload srt lỗi {st}: {raw[:200]!r}")
    try:
        srv_path = json.loads(raw)[0]
    except Exception:
        raise RuntimeError(f"vieneu upload trả lạ: {raw[:200]!r}")
    # 2. gọi tổng hợp (tham số đúng thứ tự adapter wrapper_2)
    payload = {"data": [{"path": srv_path, "orig_name": "nk.srt", "meta": {"_type": "gradio.FileData"}},
                        voice, True, "wav", True, 32]}
    st, raw = _http("POST", f"{base}/gradio_api/call/wrapper_2", json.dumps(payload).encode(),
                    {"Content-Type": "application/json"}, 60)
    if st != 200:
        raise RuntimeError(f"vieneu call lỗi {st}: {raw[:200]!r}")
    try:
        event_id = json.loads(raw)["event_id"]
    except Exception:
        raise RuntimeError(f"vieneu call trả lạ: {raw[:200]!r}")
    # 3. đợi SSE tới event complete (phim dài vài phút: timeout rộng)
    req = urllib.request.Request(f"{base}/gradio_api/call/wrapper_2/{event_id}", method="GET")
    event, file_info = "", None
    try:
        with urllib.request.urlopen(req, timeout=900) as r:
            buf = b""
            while True:
                chunk = r.read(65536)
                if not chunk:
                    break
                buf += chunk
                while b"\n" in buf:
                    line, buf = buf.split(b"\n", 1)
                    line = line.strip().decode("utf-8", "replace")
                    if line.startswith("event:"):
                        event = line[6:].strip()
                    elif line.startswith("data:") and event == "complete":
                        try:
                            file_info = json.loads(line[5:].strip())[0]
                        except Exception:
                            pass
                        break
                if file_info:
                    break
    except Exception as e:
        raise RuntimeError(f"vieneu SSE lỗi: {e}")
    if not isinstance(file_info, dict) or not file_info.get("url"):
        raise RuntimeError("vieneu không trả file wav")
    url = file_info["url"]
    if url.startswith("/"):
        url = base + url
    # 4. tải wav (24kHz mono 16-bit)
    st, data = _http("GET", url, timeout=300)
    if st != 200 or len(data) < 1000:
        raise RuntimeError(f"vieneu tải wav lỗi {st} ({len(data)} bytes)")
    return data


class SynthBody(BaseModel):
    text: str
    voice: str = ""
    rate: str = ""
    # "auto" (mặc định): tiếng Việt -> VienEU trước, rớt mới edge/google; engine khác -> edge rồi google.
    # "vieneu" | "edge" | "google": ép dùng đúng 1 engine.
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
    os.makedirs(DATA, exist_ok=True)
    # Tiếng Việt: VienEU (giọng Ngọc Huyền) trước, rớt mới edge/google làm backup.
    # Engine xử lý tuần tự 1 job/lúc nên khóa chung (request sau chờ request trước).
    if provider == "vieneu" or (provider == "auto" and lang == "vi"):
        vkey = hashlib.sha256(f"vieneu|{VIENEU_VOICE}|{text}".encode()).hexdigest()
        out = os.path.join(DATA, f"{vkey}.wav")
        if os.path.exists(out) and os.path.getsize(out) == 0:
            os.remove(out)
        if not os.path.exists(out):
            async with _vieneu_lock:
                if not os.path.exists(out):
                    try:
                        data = await asyncio.to_thread(_vieneu_wav, text, VIENEU_VOICE)
                        with open(out + ".tmp", "wb") as f:
                            f.write(data)
                        os.replace(out + ".tmp", out)
                        _note_new()
                    except Exception as e:
                        if provider == "vieneu":
                            raise HTTPException(502, f"vieneu failed: {e}")
                        # auto: im lặng rớt xuống edge/google bên dưới
        if os.path.exists(out):
            os.utime(out)
            return FileResponse(out, media_type="audio/wav", filename=f"{vkey}.wav")
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
        global _new_since_prune
        _new_since_prune += 1
        if _new_since_prune >= 50:  # quét thư mục mỗi 50 file mới, không phải mỗi lần
            _new_since_prune = 0
            await asyncio.to_thread(prune_cache)
    else:
        os.utime(out)  # vừa được nghe lại -> "mới dùng" cho LRU
    return FileResponse(out, media_type="audio/mpeg", filename=f"{key}.mp3")
