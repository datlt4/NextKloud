# NextKloud — kiến trúc MVP

## Stack
- App: Tauri v2 + SolidJS + TypeScript (1 codebase -> mac/linux/win + Android/iOS + web).
- Backend: Rust Axum (`crates/backend`), Postgres 16, Redis 7, MinIO (S3 API).
- Office: OnlyOffice DocumentServer self-host (`:8015`), nhúng iframe.
- Không PHP. Không `trusted_domains` — CORS `*` mặc định, siết ở reverse proxy khi cần.

## Câu hỏi của bạn
1. **S3/MinIO hay filesystem?** -> S3 abstraction (OpenDAL). Lợi: 1 API cho local + MinIO,
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

## Mini PC deploy
```bash
mkdir -p ~/Software/NextKloud
cp docker/server/docker-compose.yml ~/Software/NextKloud/
cd ~/Software/NextKloud && docker compose up -d
# backend :8014, minio :9000/:9001, onlyoffice :8015
```

## Test
```bash
cargo test -p nextkloud-backend   # quota gate
```
