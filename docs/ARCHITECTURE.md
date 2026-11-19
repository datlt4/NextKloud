# NextKloud — kiến trúc MVP

## Stack
- App: Tauri v2 + SolidJS + TypeScript (1 codebase -> mac/linux/win + Android/iOS + web).
- Backend: Rust Axum (`crates/backend`), Postgres 16, Redis 7, RustFS 1.0.x (S3 API, ghim version).
- Office: OnlyOffice DocumentServer self-host (`:8015`), nhúng iframe.
- Không PHP. Không `trusted_domains` — CORS `*` mặc định, siết ở reverse proxy khi cần.

## Câu hỏi của bạn
1. **S3/RustFS hay filesystem?** -> S3 abstraction (OpenDAL). Lợi: 1 API cho local + RustFS,
   presigned URL cho upload chunk lớn, thumbnail cache, sau này scale nhiều disk.
   `STORAGE_DRIVER=fs|s3` đổi bằng env, metadata vẫn ở Postgres.
2. **Redis?** -> Có. Immich nhanh nhờ cache; Nextcloud ì ạch vì PHP + stat filesystem
   mỗi request. Redis cache session/JWT, file-list trang đầu, thumbnail, TTS status.
   Mini PC 30GB RAM thoải mái (giới hạn 512MB LRU).
3. **Máy yếu (A8 Star)?** -> SolidJS bundle nhỏ (~1/3 React), lazy viewer theo loại file,
   list virtualized (`@tanstack/solid-virtual`), thumbnail 256px webp render server-side,
   upload chunk 8MB resumable trong Web Worker, graph Obsidian tắt mặc định.

## Milestones
- M0 (xong khung): workspace, quota unit tests, docker-compose, CI, i18n stub, resign script.
- M1: auth JWT + force-reset, users/quota API, files chunk upload/download, web file browser.
- M2: OnlyOffice/PDF/zip/ebook viewers, Kanban + subtask + Deck migrate, link unfurl thumbnail.
- M3: sync engine desktop (selective, custom path, free-up, auto-release), mobile browse-only,
  TTS pre-render + stale flag, notifications + auto-update, Tauri build matrix.

## Quyền file — không khóa app như Nextcloud
- Object data nằm trong RustFS (S3 API), chạy bằng PUID/PGID, không phải file host chia owner với webserver.
- Backend chạy bằng UID/GID của bạn (`user: "${UID:-1000}:${GID:-1000}"`), không đòi owner
  cụ thể. `chown`/`chmod` folder disk thoải mái, app vẫn chạy.
- `nextkloud-migrate` chỉ mở thư mục Nextcloud gốc ở chế độ read-only, không bao giờ
  chown/chmod nguồn. Muốn chắc: mount source với `:ro`.

## Mini PC deploy
```bash
mkdir -p ~/Software/NextKloud
cp docker/server/docker-compose.yml ~/Software/NextKloud/
cd ~/Software/NextKloud && docker compose up -d
# backend :8011 (host), rustfs :9000/:9001, onlyoffice dùng sẵn :8027, tts :8016
# .env cần S3_ACCESS_KEY / S3_SECRET_KEY; bucket-init tự tạo bucket `nextkloud`.
```

## TTS giọng đọc (Obsidian)
- Service `tts` (`docker/tts/server.py`): edge-tts Microsoft Neural, cache mp3 theo hash.
- Detect ngôn ngữ EN/VI/ZH bằng LibreTranslate `:8015`, fallback heuristic offline.
- Giọng mặc định: en-US-AriaNeural / vi-VN-HoaiMyNeural / zh-CN-XiaoxiaoNeural.
- Quy tắc viewer (M2): mỗi đoạn văn 1 nút voice, bỏ qua code fence + math.
  Sửa md xong ấn TTS mới render lại, chưa ấn hiện banner "âm thanh đã cũ".

## Test
```bash
cargo test -p nextkloud-backend   # quota gate
```
