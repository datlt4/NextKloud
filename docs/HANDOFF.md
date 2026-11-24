# NextKloud — Handoff (chi tiết)

Tài liệu bàn giao cho agent kế tiếp (review hoặc tiếp quản). Đọc hết file này trước khi sửa code.
Cập nhật: 2026-11-23 · version 0.1.14 · việc còn lại tổng hợp ở mục 9.0.

> Repo **public**. Không ghi secret, mật khẩu, nội dung file hay nội dung card Kanban của người dùng vào repo (kể cả trong tài liệu này).

## Mục lục
1. [Mục tiêu sản phẩm](#1-mục-tiêu-sản-phẩm)
2. [Quy tắc làm việc](#2-quy-tắc-làm-việc)
3. [Hạ tầng, deploy, test](#3-hạ-tầng-deploy-test)
4. [Bản đồ code](#4-bản-đồ-code)
5. [API hiện có](#5-api-hiện-có)
6. [Mô hình dữ liệu](#6-mô-hình-dữ-liệu)
7. [Quyết định đã chốt và lý do](#7-quyết-định-đã-chốt-và-lý-do)
8. [Đã xong (lịch sử)](#8-đã-xong-lịch-sử)
9. [Backlog chi tiết](#9-backlog-chi-tiết)
10. [Nợ kỹ thuật và rủi ro](#10-nợ-kỹ-thuật-và-rủi-ro)
11. [Bẫy đã gặp](#11-bẫy-đã-gặp)
12. [Checklist review](#12-checklist-review)

---

## 1. Mục tiêu sản phẩm

NextKloud là bản thay thế Nextcloud **không dùng PHP**: Tauri v2 + Rust (Axum) + TypeScript + SolidJS. License MIT.

| Mảng | Yêu cầu của người dùng |
|---|---|
| File | Kiểu Google Drive. **Thư mục 1 triệu file, upload/download hàng trăm GB không được treo** (đây là lý do bỏ Nextcloud). |
| Kanban | Thay Deck, kiểu JIRA: subtask, chia sẻ, thumbnail cho URL. Kèm Task tracker và Habit tracker. |
| Viewer | OnlyOffice (có sẵn ở :8027), media, zip, ebook, drawio, excalidraw, Markdown kiểu Obsidian (math) + chế độ TTS có banner "âm thanh đã cũ". |
| Hiệu năng | Mượt trên **Samsung A8 Star** (máy yếu). |
| Multi-user | Admin tạo user với mật khẩu mặc định, bắt đổi khi đăng nhập lần đầu. Admin nới hoặc thu hẹp quota. **Bắt buộc có unit test.** |
| Bảo mật | **Không có trusted_domains**. |
| i18n | EN / JA / ZH / VI, lựa chọn đồng bộ giữa các thiết bị. |
| Desktop | Sync kiểu OneDrive: chọn thư mục cục bộ, free-up space, tự giải phóng sau N ngày. |
| Mobile | Chỉ duyệt, upload, download và Kanban. |
| Khác | Notifications, auto-update. Giao diện như Nextcloud nhưng đẹp hơn: logo, favicon, ảnh nền tuỳ chọn, theme cam/kem/cam kiểu Claude. |
| CI | Theo mẫu Kermius/Kellyfin (chỉ desktop, không mobile). |
| Host | Docker trên mini PC (Ryzen 5 3400GE, không có GPU NVIDIA). |

## 2. Quy tắc làm việc

1. **Trả lời người dùng bằng tiếng Việt.** Mặc định trả lời ngắn; chỉ giải thích dài khi người dùng hỏi "giải thích".
2. **Sửa đúng chỗ.** Mỗi dòng thay đổi phải gắn với yêu cầu. Không refactor phần không hỏng. Giữ nguyên style hiện có. Comment tối đa 1–2 dòng.
3. **Tự commit và push lên `main` sau mỗi thay đổi đã xong, không cần hỏi.** Commit message viết như của developer, **không** có `Co-Authored-By` hay bất kỳ dòng ghi tác giả AI nào.
4. **Không commit secret.** `.env` chỉ có trên mini PC. Không đổi mật khẩu admin thật của người dùng.
5. **Không nói suy đoán như sự thật.** Giá trị nào kiểm chứng được (version, path, config) thì kiểm tra trước, bằng SSH hoặc đọc file.
6. **Thao tác không đảo ngược** (xoá dữ liệu trên server, purge bucket) phải hỏi trước, kiểm tra đúng đích (UUID đủ 36 ký tự) rồi mới chạy.
7. **Version luôn tự tăng (yêu cầu người dùng 23/11):** mỗi lần commit lên `main` đều bump patch trong cùng commit: chạy `scripts/set-version.sh patch` trước `git commit` (script tự +1 số cuối và cập nhật package.json, tauri.conf.json, các Cargo.toml + Cargo.lock).
   - `release.yml` chỉ tạo release khi version trong `apps/desktop/package.json` khác tag đã có, nên bump mỗi commit = mỗi push đều ra installer mới; version hiện ở chân sidebar (server) khớp với code đang chạy.
   - Bản lớn: `scripts/set-version.sh X.Y.0` (người dùng muốn **bump major** khi ra bản lớn).
   - Gộp nhiều thay đổi vào 1 push khi sửa liên tục: push dày làm CI huỷ build lẫn nhau và Docker Hub trả `429 Too Many Requests` (gặp 23/11).
   - **Quy trình mới (yêu cầu 24/11): có bug là fix + push ngay, không chờ image build xong.** `build.yml` đã bật `cancel-in-progress` nên push mới tự huỷ run cũ — chỉ bản mới nhất mới ra image/tag, các bản giữa không bao giờ có image (bình thường, khỏi chờ). Theo dõi deploy bằng `last_updated` của tag `latest` trên Docker Hub thay vì gọi GitHub API (giới hạn 60 lần/giờ khi không đăng nhập).
   - Release chạy ngầm (đã chuyển `workflow_dispatch`), không cản việc fix bug khác.
   - **Luôn bump patch mỗi commit (quy định lại 24/11):** chạy `scripts/set-version.sh patch` trước `git commit` để version hiện trên web FE (sidebar, từ `/api/config` trong image docker) luôn khớp bản đang chạy — nhìn version là biết mini đã update chưa. `release.yml` đang ở chế độ tay (`workflow_dispatch`) nên bump version **không** trigger build installer, chỉ chạy `build.yml` (web + backend + Docker).
   - **TẠM DỪNG release (24/11, yêu cầu người dùng):** `release.yml` đã chuyển sang `workflow_dispatch` (chạy tay) — push main **không** tạo branch/tag/installer nữa, dù version có đổi. `build.yml` vẫn chạy tự động mỗi push. Muốn mở lại: trả `on:` trong `release.yml` về `push: branches: [main]` + `workflow_dispatch`.

## 3. Hạ tầng, deploy, test

### 3.1 Mini PC (production)

- **SSH:** `ssh -o BatchMode=yes mini-wg` (đi qua WireGuard).
- **Compose:** `/home/mini/Software/NextKloud/` (`docker-compose.yml` + `.env`). **Không in `.env`.** Muốn dùng biến thì chạy `set -a; . ./.env; set +a` rồi tham chiếu `$S3_ACCESS_KEY`.
- **Container:**

  | Container | Image | Port (host) | Ghi chú |
  |---|---|---|---|
  | nextkloud-backend-1 | `hienanh/nextkloud:latest` (CI build) | 8011 → 8014 | chạy bằng PUID/PGID, serve luôn web từ `/web` |
  | nextkloud-db-1 | postgres:16-alpine | (nội bộ) | user/db `nextkloud`, có ICU và pg_trgm |
  | nextkloud-cache-1 | redis:7-alpine | (nội bộ) | **backend chưa dùng** |
  | nextkloud-tts | `hienanh/nextkloud-tts` | 8016 | edge-tts; nhận diện ngôn ngữ qua LibreTranslate :8015 |
  | rustfs | rustfs/rustfs:1.0.4 | 9000 (S3), 9001 (console) | bucket `nextkloud` |
  | bucket-init | rclone/rclone:1.75.2 | — | chạy một lần, `rclone mkdir nk:nextkloud` |
  | (có sẵn, ngoài compose) | onlyoffice-documentserver | 8027 | backend gọi qua `host.docker.internal:8027` |

- **Dữ liệu:** `/mnt/das_8tb/nextkloud_volume/`

  | Thư mục | Nội dung |
  |---|---|
  | `app/` | mount `/data` của backend; `app/staging/<uuid>/<upload-id>.part` là chunk đang upload |
  | `pgdata/` | Postgres |
  | `rustfs/` | object S3 (key = `<user_uuid>/<path>`) |
  | `redis/`, `tts/` | dữ liệu của Redis và TTS |
  | `s3/` | thư mục MinIO cũ, rỗng (120K), chưa xoá |
  | `migration/nextcloud-owncloud-2026-11-20.db` | backup SQLite của Nextcloud, dùng để nhập Deck |
  | `migration/datlt4-temp-password.txt` | mật khẩu tạm của user `datlt4` (umask 077) |

- **User thật:** `datlt4` (5.815 file, 15,39 GiB, quota_used = 16525284221) và `admin`. Chỉ mục của cả hai đã được backend tự dựng lúc khởi động.
- **Deploy:**
  1. Push lên `main`.
  2. CI `build.yml` build image amd64 + arm64 rồi tag `latest` và `<version>`.
  3. Trên mini PC:
     ```bash
     cd /home/mini/Software/NextKloud
     docker compose pull -q backend && docker compose up -d backend
     docker compose logs --tail 20 backend
     curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8011/api/health   # 200
     ```
  - Push commit mới khi build cũ chưa xong thì build cũ bị huỷ (concurrency). Phải chờ build của commit mới nhất.
- **Theo dõi CI** (máy dev không có `gh`):
  `curl -s "https://api.github.com/repos/datlt4/NextKloud/actions/runs?per_page=5&head_sha=<SHA đầy đủ>"`, lọc `name == "build"`, chờ `status == completed` và `conclusion == success`.
- **Dựng lại chỉ mục:**
  `docker compose exec backend nextkloud reindex [username]`. Lệnh xoá rồi dựng lại các dòng trong `files` từ S3, có HEAD từng object để lấy mtime gốc, và ghi lại `quota_used`.
- **Đọc metadata object bằng rclone** (debug):
  ```bash
  docker run --rm --network nextkloud_default \
    -e RCLONE_CONFIG_NK_TYPE=s3 -e RCLONE_CONFIG_NK_PROVIDER=Other \
    -e RCLONE_CONFIG_NK_ENDPOINT=http://rustfs:9000 \
    -e RCLONE_CONFIG_NK_ACCESS_KEY_ID=$S3_ACCESS_KEY -e RCLONE_CONFIG_NK_SECRET_ACCESS_KEY=$S3_SECRET_KEY \
    -e RCLONE_CONFIG_NK_FORCE_PATH_STYLE=true rclone/rclone:1.75.2 lsjson -R --metadata nk:nextkloud/<uuid>/
  ```
- **Đọc backup Deck:**
  ```bash
  docker run --rm -v <db>:/db.sqlite:ro keinos/sqlite3 sqlite3 "file:/db.sqlite?mode=ro&immutable=1" "<sql>"
  ```
  Bắt buộc có `immutable=1`; thiếu nó sqlite báo lỗi "readonly database".

### 3.2 Máy dev

- **Repo:** `/mnt/ssd_1tb/Downloads-Ubuntu/Miscellaneous/NextKloud`. Ổ này **không cho `sed -i`**; dùng Edit/Write hoặc python để sửa file.
- **Node:** `source ~/.nvm/nvm.sh` (Node 24) trước khi dùng `npx`/`npm`.
- **Container test (đang chạy):**

  | Container | Kết nối |
  |---|---|
  | `nk-pg-test` | `postgres://nk:nk@127.0.0.1:15432/nk` |
  | `nk-rustfs-test2` | `http://127.0.0.1:19000`, bucket `nktest`; key: `docker exec nk-rustfs-test2 printenv RUSTFS_ACCESS_KEY` (tương tự `RUSTFS_SECRET_KEY`). Dữ liệu nằm trong scratchpad nên mất khi reboot: tạo lại thư mục với quyền uid 1000 rồi chạy `rclone mkdir nk:nktest`. Container cũ `nk-rustfs-test` hỏng vì thư mục dữ liệu bị Docker tạo lại dưới quyền root. |

- **Test backend:**
  ```bash
  K=$(docker exec nk-rustfs-test2 printenv RUSTFS_ACCESS_KEY); S=$(docker exec nk-rustfs-test2 printenv RUSTFS_SECRET_KEY)
  NK_TEST_DB=postgres://nk:nk@127.0.0.1:15432/nk NK_TEST_S3=http://127.0.0.1:19000,nktest,$K,$S \
    cargo test -p nextkloud-backend
  ```
  - Hiện có 35 test, tất cả pass (`ops_s3` mất khoảng 1 phút). Test git cần lệnh `git` và `ssh-keygen` trên máy.
  - Thiếu `NK_TEST_DB`/`NK_TEST_S3` thì các test cần DB/S3 tự `return`. CI chạy theo cách này, nên **CI không phủ phần DB và S3**: phải chạy đầy đủ ở local trước khi push.
- **Test frontend:** `cd apps/desktop && npx tsc --noEmit -p . && npm run build`
- **Chạy backend local** (web tự serve từ `apps/desktop/dist`):
  ```bash
  BIND=127.0.0.1:18014 DATABASE_URL=postgres://nk:nk@127.0.0.1:15432/nk STORAGE_DRIVER=s3 \
  S3_ENDPOINT=http://127.0.0.1:19000 S3_BUCKET=nktest S3_ACCESS_KEY=$K S3_SECRET_KEY=$S \
  STAGING_DIR=/tmp/nk-staging NEXTKLOUD_WEB=apps/desktop/dist JWT_SECRET=localtest target/debug/nextkloud
  ```
  - DB test có user `admin` (đã tắt must_reset) và một thư mục `Big` gồm 200.000 dòng giả trong chỉ mục (không có object thật).
  - Mật khẩu admin local: hỏi người dùng hoặc đặt lại hash trực tiếp trong DB test.
  - Dừng backend: `kill $(pgrep -x nextkloud)`. **Không dùng `pkill -f`**: pattern có thể khớp chính shell đang chạy lệnh và kill nó.
- **Test UI:** puppeteer-core (đã cài trong scratchpad, thư mục `pp/`) + `/usr/bin/google-chrome --no-sandbox` headless.
  - Chụp 2 khổ: 1400×820 và 390×780 @2x.
  - Ghi script JS bằng file, không dùng heredoc (safety-check chặn).
  - Các bước đã kiểm: đăng nhập → mở thư mục 200k → nhảy giữa danh sách → đo ms/frame → grid → tìm kiếm → mobile; admin: tạo / trùng tên / quota / buộc đổi mật khẩu / xoá.

### 3.3 Biến môi trường của backend

| Biến | Mặc định | Ý nghĩa |
|---|---|---|
| `BIND` | `0.0.0.0:8014` | địa chỉ listen (Dockerfile đặt `NEXTKLOUD_ADDR` nhưng **code không đọc** biến đó; compose đặt `BIND`) |
| `DATABASE_URL` | bắt buộc | Postgres |
| `JWT_SECRET` | `dev-insecure-change-me` (có cảnh báo) | ký JWT |
| `ADMIN_PASSWORD` | `admin` | chỉ dùng khi bảng users rỗng; seed admin với must_reset |
| `STORAGE_DRIVER` | `fs` | `s3` hoặc `fs` |
| `S3_ENDPOINT` / `S3_BUCKET` / `S3_ACCESS_KEY` / `S3_SECRET_KEY` | rustfs:9000 / nextkloud / bắt buộc | RustFS |
| `FILE_DIR` | `/data/files` | gốc dữ liệu khi `fs` |
| `STAGING_DIR` | `/data/staging` | chunk upload |
| `NEXTKLOUD_WEB` | — | thư mục web để serve (SPA fallback) |
| `TTS_URL` | `http://localhost:8016` | proxy TTS |
| `ONLYOFFICE_URL`, `REDIS_URL`, `ALLOWED_ORIGINS` | — | có trong compose, **code chưa dùng** |

### 3.4 CI

- **`build.yml`** (push/PR lên main):
  - web: `npm ci`, `npm audit --audit-level=high`, build.
  - backend: `cargo test --locked`.
  - tauri: check trên Ubuntu.
  - docker backend + tts: multi-arch, push theo digest rồi gộp tag.
- **`release.yml`:** khi version đổi thì tạo branch `release/vX.Y.Z`, tag, installer cho desktop (Linux/macOS/Windows).
- **CI:** push `main` chạy `build.yml` (web + backend + Docker); version đổi thì `release.yml` tạo branch/tag + installer desktop.
- **Secrets cần có:** `DOCKERHUB_USERNAME`, `DOCKERHUB_TOKEN`, cùng các secret ký Android/iOS (xem workflow).

## 4. Bản đồ code

```
crates/backend/
  migrations/0001_init.sql        bảng users
  migrations/0002_file_index.sql  pg_trgm, collation nk_natural (ICU, kn=true), bảng files + 4 index
  migrations/0003_profile.sql     users.display_name, users.avatar_v
  src/main.rs          đọc env, migrate, seed admin, tạo Store, `reindex` CLI, tự dựng chỉ mục khi khởi động, serve web
  src/routes.rs        bảng route; content_type(); serve_web() (SPA, chặn ..)
  src/state.rs         AppState{db, store, enc, dec, uploads(RAM), http, tts_url}; UserRow; AppError → HTTP; extractor AuthUser (Bearer JWT)
  src/auth.rs          argon2 hash (tối thiểu 8 ký tự), JWT {sub, adm, exp}
  src/quota.rs         Quota thuần + unit test (limit 0 = không giới hạn; thu hẹp dưới mức đang dùng → over_quota)
  src/files.rs         Store (OpenDAL): clean_path, key, scan (đệ quy + HEAD lấy mtime), stat, write_chunk (append đúng offset), complete (multipart, user metadata mtime, quota trên bản sao), delete
  src/index.rs         chỉ mục Postgres: list (2 truy vấn dir/file, offset), search (ILIKE trigram), add_file (upsert + thư mục cha), remove_file, rebuild
  src/file_handlers.rs HTTP: list, search, download (Range/206/416), delete, init/put/complete upload
  src/users.rs         login, me, change_password, create/list users, set_quota, reset_password, delete_user + test tích hợp
  src/tts.rs           proxy voices/detect/synthesize sang service TTS
crates/migrate/        stub (6 dòng); migrate thật đã làm bằng rclone
apps/desktop/
  src/api.ts           client: req() (Bearer, 401 → logout), files/search/upload(8MB tuần tự, gửi mtime)/admin API, speak()
  src/App.tsx          toàn bộ shell: login, reset, header, sidebar, file browser (danh sách ảo), RowMenu, LookDialog, chuyển trang files/users
  src/UsersAdmin.tsx   màn quản lý user + dialog (create, quota có slider phi tuyến, reset, delete, secret)
  src/Settings.tsx     trang cài đặt: Profile, Security, LookPanel, UsersAdmin, System
  src/avatar.tsx       <Avatar> (tải blob có token, cache theo id+v) + squareAvatar() cắt 256px webp
  src/look.tsx         LookPanel / LookDialog (theme + nền)
  src/forms.tsx        ResetForm (mật khẩu mới + nhập lại; ẩn ô hiện tại khi đã biết)
  src/background.ts    theme sáng/tối/auto + ảnh nền, lưu ở `.nextkloud/settings.json` và `.nextkloud/background.webp` (resize ≤ 2560px)
  src/icons.tsx        icon SVG inline + kindOf(name) → icon/màu
  src/format.ts        fmtSize
  src/viewers/MarkdownView.tsx  tách md thành block, nút TTS từng đoạn (code/math bỏ qua)
  src/i18n/*.ts        stub, chưa dùng
  src/style.css        token màu :root / dark / prefers-color-scheme; blur chỉ bật trên desktop
  src-tauri/           shell rỗng (lib.rs, main.rs 6 dòng)
docker/server/         Dockerfile (web → rust → debian slim), docker-compose.yml
docker/tts/            server.py (edge-tts + cache mp3 theo hash)
scripts/               set-version.sh
```

## 5. API hiện có

Mọi API (trừ login và health) cần header `Authorization: Bearer <jwt>`. Lỗi trả về `{"error": "..."}`.

| Method | Path | Body / Query | Trả về | Ghi chú |
|---|---|---|---|---|
| GET | `/api/health` | — | `{status}` | |
| POST | `/api/auth/login` | `{username,password}` | `{token,must_reset,is_admin}` | |
| POST | `/api/auth/password` | `{old,new}` | 204 | xoá cờ must_reset |
| GET | `/api/me` | — | UserRow | |
| PATCH | `/api/me` | `{display_name}` | UserRow | rỗng = bỏ tên hiển thị; tối đa 64 ký tự |
| PUT / DELETE | `/api/me/avatar` | ảnh webp/png/jpeg ≤ 1MB (client cắt vuông 256px) | UserRow | lưu ở `<uuid>/.nextkloud/avatar`; `avatar_v` tăng mỗi lần đổi |
| GET | `/api/users/:id/avatar?v=` | — | ảnh | mọi user đã đăng nhập; cache immutable; client tải blob vì `<img>` không gửi được Bearer |
| GET | `/api/admin/stats` | — | `{version,storage,users,files,dirs,bytes}` | admin |
| POST | `/api/admin/reindex/:id` | — | `{files,bytes}` | admin; như lệnh CLI `reindex` |
| GET | `/api/users` | — | UserRow[] | admin |
| POST | `/api/users` | `{username,password,quota_limit?,is_admin?}` | 201 UserRow | admin; trùng tên → 409; mật khẩu < 8 ký tự → 400 |
| DELETE | `/api/users/:id` | — | 204 | admin; tự xoá mình → 409; xoá object, chỉ mục (cascade), staging |
| POST | `/api/users/:id/quota` | `{quota_limit}` | UserRow | admin; 0 = không giới hạn |
| POST | `/api/users/:id/password` | `{password}` | 204 | admin; bật lại must_reset |
| GET | `/api/files` | `prefix, sort=name\|size\|modified, desc, offset, limit≤1000 (mặc định 200)` | `{entries:[{path,name,is_dir,size,modified}], dirs, files, bytes}` | thư mục đứng trước; ẩn tên bắt đầu bằng `.` |
| GET | `/api/search` | `q, limit≤1000` | entries[] | toàn bộ cây; bỏ qua đường dẫn có `/.` |
| GET | `/api/files/*path` | header Range | stream | 206/416; không nạp cả file vào RAM |
| DELETE | `/api/files/*path` | — | TrashItem | file hoặc thư mục → thùng rác (`.nextkloud/trash/<id>/<name>`); quota giữ nguyên |
| POST | `/api/dl` | `{path}` | `{token}` | token tải 1 file, 10 phút; client mở `/api/files/<path>?dl=<token>` (stream thẳng xuống đĩa, `Content-Disposition: attachment`). Token này không dùng làm token đăng nhập được |
| GET / POST | `/api/annotations` | GET `?path=` (1 file) hoặc `?state=` (mọi ghi chú); POST `{path, kind: bookmark\|note, anchor, quote, body}` | Annotation[] / 201 | anchor: `b<block>` (md), `p<trang>` (pdf); đi theo file khi move/trash, xoá khi purge |
| PATCH / DELETE | `/api/annotations/:id` | `{body?, state?: todo\|doing\|done}` | Annotation / 204 | |
| POST | `/api/mkdir` | `{path}` | 201 | chỉ ghi chỉ mục; đã có → 409; cha là file → 409 |
| POST | `/api/move` | `{from,to}` | `{files}` | đổi tên + di chuyển file/thư mục; đích có rồi → 409; vào chính nó → 400; đồng bộ, copy+delete song song 64 |
| GET / POST | `/api/git` | POST `{url, branch?, folder?, key: generate\|paste\|none, private_key?, interval_h?}` | Repo[] / 201 Repo | mirror git 1 chiều; URL chỉ https/http/ssh/scp (chặn `ext::`, `file://`, `-` đầu); `key=generate` → state `pending` tới khi bấm sync; không bao giờ trả `ssh_key`, URL bị che token |
| PATCH / DELETE | `/api/git/:id` | `{interval_h?, branch?, url?}` / `?files=true` | Repo / 204 | đổi nhánh → clone lại; `files=true` chuyển thư mục vào thùng rác |
| POST | `/api/git/:id/sync` | — | 202 Repo | đang sync → 409 |
| GET / DELETE | `/api/trash` | — | TrashItem[] / 204 | DELETE = dọn sạch |
| POST | `/api/trash/:id/restore` | — | TrashItem | chỗ cũ đã có thứ khác → 409 |
| DELETE | `/api/trash/:id` | — | 204 | xoá hẳn, hoàn quota bằng SUM(size) trong chỉ mục |
| POST | `/api/files/uploads` | `{path,size,mtime?}` | `{id}` | kiểm quota trước; tối đa 100GiB |
| PUT | `/api/files/uploads/:id?offset=N` | bytes (≤64MiB) | `{received}` | offset phải khớp kích thước đã stage, sai → 409 |
| POST | `/api/files/uploads/:id/complete` | — | 201 | multipart lên S3, cập nhật quota và chỉ mục |
| GET | `/api/tts/voices`, `/api/tts/detect` | | | proxy |
| POST | `/api/tts/synthesize` | `{text}` | audio/mpeg | proxy |

## 6. Mô hình dữ liệu

```sql
users(id uuid pk, username text unique, pass_hash, must_reset bool, quota_limit int8 (0 = không giới hạn),
      quota_used int8, is_admin bool, created_at, display_name text null, avatar_v int /*0 = chưa có*/)
files(user_id uuid → users ON DELETE CASCADE, path text, parent text, name text COLLATE nk_natural,
      is_dir bool, size int8, modified int8 /*unix giây*/, PRIMARY KEY(user_id, path))
  index (user_id,parent,is_dir,name) | (…,size,name) | (…,modified,name) | GIN(name gin_trgm_ops) | (user_id, path text_pattern_ops)
trash(id uuid pk, user_id → users CASCADE, orig_path, is_dir, size, files, deleted_at int8)
git_repos(id, user_id → users CASCADE, url, branch, folder, ssh_key, ssh_pub, interval_h, state pending|idle|syncing|error,
          last_sync, last_commit, last_error, files, created_at)
```

- **Mirror git** (`git.rs`): git CLI trong container. Bản clone nông ở `<STAGING_DIR>/../git/<id>`, khoá ở `<id>.key` (0600), `known_hosts` chung (accept-new). Lần đầu (hoặc mất bản clone) thì đồng bộ toàn bộ: ghi mọi file và dọn file lạ trong thư mục. Các lần sau `fetch --depth 1` + `diff --name-status` giữa commit cũ và mới, chỉ ghi và xoá phần thay đổi. Bỏ qua symlink/submodule. mtime = thời điểm commit HEAD (clone nông nên không có giờ từng file). Scheduler kiểm tra 5 phút một lần, chạy lần lượt. Khởi động lại khi đang sync → state `error`. ssh cần uid có trong `/etc/passwd`: image tạo user 1000, **PUID khác 1000 thì ssh lỗi** (https + token vẫn chạy).

- **Thùng rác:** object nằm ở `<uuid>/.nextkloud/trash/<id>/<tên>`, dòng chỉ mục cũng dời theo (bị ẩn vì tên bắt đầu bằng `.`). Dòng `trash` chỉ được ghi **sau khi** chuyển xong, để "dọn sạch" không chạy đè lúc đang copy. Task nền mỗi giờ dọn mục quá `TRASH_DAYS` (mặc định 30). User không được ghi/di chuyển vào `.nextkloud` (`ops::user_path`).

- **Object key:** `<user_uuid>/<path>`. Path đã chuẩn hoá: không có `..`, không có `/` ở đầu, không có `\`.
- **Thư mục:** chỉ tồn tại trong bảng `files` (`is_dir`), **không** có object marker trên S3. `modified` của thư mục = mtime lớn nhất của các con.
- **mtime gốc:** lưu trong S3 user metadata `mtime`. rclone ghi dạng RFC3339 hoặc số giây; `parse_mtime` đọc được cả hai.
- **Cài đặt giao diện:** file `.nextkloud/settings.json` `{bg, dim, v, theme}` trong không gian file của user (bị ẩn khỏi listing).

## 7. Quyết định đã chốt và lý do

| Quyết định | Lý do |
|---|---|
| RustFS thay MinIO | MinIO community đã rời Docker Hub; RustFS là Apache-2.0. Đổi khi chưa có dữ liệu. |
| OpenDAL `services-s3` + `services-fs` | 1 API dùng cho cả prod (S3) lẫn test (fs). |
| Chỉ mục metadata trong Postgres | S3 list tối đa 1000 object mỗi lần và không phân trang theo sort được. Thư mục 1 triệu file bắt buộc phải dùng DB. |
| Phân trang offset (chưa dùng keyset) | UI cho kéo thanh cuộn tới vị trí bất kỳ, cần truy cập ngẫu nhiên. Đo trên 1 triệu dòng: 0,4ms (trang đầu) – 110ms (giữa) – 430ms (sort theo size, cuối danh sách). |
| Danh sách ảo tự viết (không dùng @tanstack/solid-virtual) | Cần dạng grid theo hàng và cache theo trang server; tự viết gọn hơn. Package vẫn nằm trong package.json, **chưa gỡ**. |
| Chunk upload stage ra đĩa local rồi mới multipart lên S3 | S3 không append được; resume dựa vào offset. |
| Migrate Nextcloud thủ công bằng rclone, không làm trong app | Người dùng yêu cầu. |
| Không có trusted_domains, CORS permissive | Người dùng yêu cầu; siết ở reverse proxy nếu cần. |
| Theme: token màu, light/dark/auto | Người dùng chê UI cũ xấu; muốn tông cam, trắng, cam kiểu Claude. |

## 8. Đã xong (lịch sử)

- **M0:** workspace, quota + unit test, compose, CI 3 workflow, i18n stub, script resign.
- **M1 backend:** JWT, buộc đổi mật khẩu, users/quota API, upload chunk, download stream có Range, axum body limit 64MB cho route chunk.
- **UI:**
  - Làm lại toàn bộ: logo squircle cam, favicon, ảnh nền như Nextcloud (preset + upload ảnh, độ tối, đồng bộ server), theme sáng/tối.
  - Responsive ≤ 860px (sidebar thành drawer) và ≤ 600px.
  - Sửa: menu user bị panel che (stacking context), dropdown tràn màn hình.
- **Storage:** chuyển sang RustFS qua OpenDAL (`0135687`).
- **Migrate:** 5.815 file / 15,39 GiB của `datlt4`, `rclone check --one-way` khớp 5.815, 0 khác biệt. Chưa chuyển: trash (854M), versions (1,7G). Log Nextcloud nặng 15GB, không cần chuyển.
- **Chỉ mục file + danh sách ảo + tìm kiếm toàn cục + giữ mtime gốc** (`80aeed5`): mtime thật đã khôi phục (đa số rơi vào 2022–2025).
- **Màn admin quản lý user** (`147e850`). Cùng commit này sửa 2 lỗi:
  - Class `.ghost` va với `.btn.ghost`, làm mọi nút ghost bấm không ăn (lỗi do commit `80aeed5` gây ra).
  - Nút đóng dialog nằm trong `<form>` mà thiếu `type="button"`, bấm vào là submit form.
- **Đổi mật khẩu lần đầu và slider quota** (`d33cee5`):
  - Màn đặt mật khẩu mới sau khi admin reset không còn ô "mật khẩu hiện tại" (dùng luôn mật khẩu vừa đăng nhập), có ô nhập lại. Người dùng từng báo lỗi "change password failed", khả năng cao do trình duyệt tự điền mật khẩu cũ.
  - Slider quota phi tuyến 0–5–10–50–100–500–1TB–2TB, có vạch đánh dấu dung lượng đang dùng.
- **9.1 Thao tác file** (`ops.rs`, `fileops.tsx`): thư mục mới, đổi tên, di chuyển (dialog duyệt cây), xoá thư mục, thùng rác (khôi phục, xoá hẳn, dọn sạch, tự dọn 30 ngày), toast "Hoàn tác"; chọn nhiều (checkbox, Ctrl-click, Shift-click dải, Ctrl+A ≤ 5.000 mục, Esc, phím Delete, nhấn giữ trên mobile); kéo thả vào thư mục/breadcrumb (chỉ chuột). Đo local (RustFS, bản debug): đổi tên thư mục 300 file mất 1,5 giây. **Chưa làm:** job nền có tiến độ cho thư mục rất lớn (request đang chạy đồng bộ, ước 10.000 file khoảng 50 giây), merge khi đích trùng tên.
- **Mirror git cho vault Obsidian** (tab Cài đặt → Đồng bộ Git, `gitsync.tsx`): thêm repo (tạo khoá ed25519 / dán private key / không khoá), hiện public key để thêm vào Deploy keys, đồng bộ ngay, chu kỳ 1–24 giờ (mặc định 8), gỡ repo (tuỳ chọn chuyển thư mục vào thùng rác). Đo local: clone + đẩy `octocat/Hello-World` mất 1,9 giây.
- **Sửa lỗi** (`869fdfb`): 9.1 chặn ghi vào `.nextkloud` làm hỏng lưu giao diện/ảnh nền; giờ cho phép riêng `settings.json` và `background.webp` (`ops::upload_path`).
- **Trình đọc (22/11):** sidebar mục lục + bookmark + ghi chú 3 trạng thái cho Markdown và PDF (`annot.tsx`, `notes.rs`, migration 0007); tải file trực tiếp bằng token ngắn hạn; quota upload cộng phần chênh; `.whiteboard` mở bằng Excalidraw; thư mục `Obsidian Vaults` (đã chuyển 14.261 mục của datlt4 từ `Vaults`, mất 7 phút); nút loa ngang dòng đầu.
- **Obsidian hoàn thiện (22/11):** trang Ghi chú (`NotesPage` trong `annot.tsx`, `GET /api/annotations?state=`); KaTeX lazy (chunk riêng), callout, frontmatter, #tag trong `MarkdownView.tsx`; tìm theo nội dung (`fulltext.rs`, bảng `file_text` + trigram, `GET /api/search/content`, chỉ file `.md/.markdown/.txt` ≤ 2MB; ghi khi upload và git sync, dời theo move, xoá khi purge, khởi động thì bù file còn thiếu; tìm có JOIN `files` nên dòng mồ côi không hiện); cache TTS LRU (`TTS_CACHE_MAX_MB`, mặc định 5120). **23/11:** sửa inline + lưu ngược (cảnh báo vault mirror từ git, `api.saveText`, parent tải lại), embed `![[note]]` hiện nội dung inline 1 cấp (40 dòng, ảnh trong note nhúng vẫn lazy-load), backlinks `GET /api/vault/backlinks` (regex khớp chính xác stem/tên/path).
- **Sao lưu** (`docker/server/backup.sh`, service `backup`): 3:00 mỗi đêm, `pg_dump -Fc` + `rclone sync` toàn bộ bucket ra `/mnt/hdd_1tb/backup/nextkloud/<ngày_giờ>/` (file thường, đọc được không cần RustFS), hardlink với bản trước nên chỉ tốn phần thay đổi, giữ 2 bản. Khôi phục: `pg_restore -d nextkloud db.dump` + `rclone copy files/ nk:nextkloud`. Bản sao lưu **chưa mã hoá** và có chứa SSH key của repo git (trong DB).
- **Mobile: đã bỏ** (xem 9.8).
- **Hồ sơ & Cài đặt:** `Settings.tsx` có các tab Hồ sơ (ảnh đại diện, tên hiển thị, dung lượng), Mật khẩu, Giao diện, và cho admin: Người dùng, Hệ thống (thống kê + nút dựng lại chỉ mục). Menu user mới; header và danh sách user hiện ảnh đại diện.

- **22–23/11 (0.1.7 → 0.1.14):**
  - **Deck (Kanban)** `kanban.rs`, `deck.rs`, `kanban.tsx`: board/cột/card, subtask 1 cấp, kéo thả, cột xong và **cột huỷ** (`is_cancel`, cột Delete) chỉ tải N card mới nhất; subtask của task trong cột huỷ hiện ✕ gạch ngang; ô chọn task cha có tìm kiếm (`GET /api/boards/:id/parents`); file đính kèm card (`cards.attachments`, nhập từ share Deck); xem trước link trong mô tả (`linkpreview.rs`, bảng `link_previews`); nhập từ Nextcloud Deck (`scripts/deck_export.py` → `/data/import/deck.json`, chủ dữ liệu hoặc admin được nhập; đã nhập board Personal deck:3, board deck:1 chưa nhập).
  - **Secret Vault** `secret.rs`, `secret.tsx` (thiết kế `SECRET_VAULT.md`): PIN 6 số + mã khôi phục, AES-256-GCM khối 64KB, tên mã hoá, tự khoá 10 phút, thùng rác riêng, xem/thumbnail/Office chỉ xem; chuyển/sao chép vào và ra khỏi vault (`/api/secret/import|export|copy`). `SECRET_PEPPER` nằm trong `.env` mini PC.
  - **File:** sao chép (`/api/copy`), tải lên cả thư mục (kéo thả + nút chọn thư mục), bảng tải lên 3 file song song + tốc độ/ETA + chỉ liệt kê file lỗi để thử lại.
  - **Viewer:** PDF chọn/copy chữ (text layer), màu đọc Vàng cam/Nền đen (chừa ảnh), 4 chế độ trang, nhảy trang chính xác; ảnh markdown qua thumbnail 1920px; xem ảnh phóng to có ‹ › ; OnlyOffice qua proxy `/oo` (`ooproxy.rs`); URL giữ vị trí khi F5 (`#/f`, `#/o`, `#/secret`, `#/kanban`); server trả 404 cho file build cũ, app tự tải lại 1 lần khi lỗi tải chunk.

## 9. Backlog chi tiết

### 9.0 Việc còn lại (tổng hợp 23/11, ưu tiên từ trên xuống)

1. **Chia sẻ** (người dùng chốt làm ngay sau Kanban): chia sẻ file/thư mục giữa user (quyền xem/sửa) + link công khai có mật khẩu và hạn, như Google Drive. File trong Bí mật không chia sẻ được. — ✅ xong 0.1.17 (`shares.rs`, `share.tsx`, migration `0014`): share theo username, link public (chỉ xem, MK + hạn), duyệt/tải/upload/sửa trong "Được chia sẻ", "Lưu bản sao" về file mình, link dùng được khi chưa đăng nhập. Chưa làm: xem inline/thumbnail/zip/office/search/TTS trên file được chia sẻ (tải về để xem), re-share.
2. **Task tracker** (danh sách việc theo hạn, lấy từ card có `due_at`) và **Habit tracker** (lưới ngày, streak) — ✅ xong 0.1.15 (`tracker.rs`, `tracker.tsx`, migration `0013`): task lọc Quá hạn/Hôm nay/7 ngày/Tất cả/Đã xong; habit chấm theo ngày, streak tính tới hôm nay, lưới 12 tuần. Chưa làm: lịch weekly, nhắc hạn (đợi SSE 9.7).
3. **Upload/download lớn** (9.2): resume sau rớt mạng/F5 (trạng thái upload lưu Postgres), tạm dừng, tải thư mục thành zip stream, API upload hàng loạt file nhỏ, job nền có tiến độ cho move/rename thư mục rất lớn, merge khi đích trùng tên. **Hỏi người dùng trước:** bỏ qua file trùng (cùng tên + dung lượng) thay vì ghi đè. — ✅ một phần ở 0.1.18: resume upload (bảng `uploads`, GET active/status, client tự chạy tiếp + retry chunk, sống qua F5 khi gửi lại cùng file và qua restart server), tải thư mục thành zip stream (`GET /api/zip/download`, store-method + ZIP64, RAM cố định, giới hạn 200k file/100GB), ảnh md không còn blob RAM (HEAD dò + `<img>` stream trực tiếp), nút "Tải thư mục (zip)" trong menu thư mục. Chưa làm: tạm dừng/chạy tiếp trong panel (hiện dừng + thử lại là resume), IndexedDB giữ hàng đợi qua F5, API batch file nhỏ, job nền move/rename + merge, Range cho zip.
4. **Obsidian:** sửa + lưu ngược về file (cảnh báo khi vault mirror từ git), embed `![[note]]`, backlinks.
5. **Lịch sử phiên bản** khi ghi đè; chuyển trash (854MB) và versions (1,7GB) từ Nextcloud.
6. **Desktop sync 2 chiều / mobile / auto-update / thông báo** (9.6, 9.7).
7. **Đa ngôn ngữ** vi/en/ja/zh (9.5). — ✅ xong v1 ở 0.1.20 (`i18n.ts`: dict 4 ngôn ngữ, `t()` fallback vi, `lang` lưu localStorage + đồng bộ server qua `UiSettings.lang`, chọn trong Giao diện & nền; chuyển toàn bộ chuỗi App/Settings/UsersAdmin/fileops/forms/look/gitsync; ngày/số theo locale). ✅ xong v2 ở 0.1.22: nốt Deck/Ghi chú/Theo dõi/Chia sẻ/Bí mật + toàn bộ viewer (markdown/pdf/office/image/lightbox/drawio/epub/zip). Chưa làm: lỗi backend vẫn message tiếng Việt.
8. **Vận hành:** giới hạn đăng nhập sai; dọn staging và multipart S3 bỏ dở; CI chạy test DB + S3 (service container); kiểm tra định kỳ bản sao lưu đêm (`backup.sh`) còn chạy; Docker Hub login trong CI dễ dính 429 khi push dày.

**Thứ tự đề xuất:** 9.1 → 9.2 → 9.3 → 9.4 → 9.5 → các mục còn lại. Sau mỗi mục: viết test, chạy test đầy đủ, test UI 2 khổ màn hình, commit, push, deploy, báo người dùng bằng tiếng Việt kèm số đo.

### 9.1 Thao tác file cơ bản ✅ (phần lớn, xem mục 8; còn job nền + merge)

**Backend**

- **`POST /api/files/mkdir {path}`**
  - Gọi `index::add_dir`: upsert dòng thư mục cùng các thư mục cha. Không ghi gì lên S3.
  - Path đã tồn tại dưới dạng file → 409.
- **`POST /api/files/move {from, to}`** (dùng chung cho đổi tên và di chuyển)
  - **File:** `op.copy(from_key, to_key)` → `op.delete(from_key)` → `UPDATE files SET path, parent, name` trong 1 transaction.
  - **Thư mục:**
    - Liệt kê con từ chỉ mục bằng `path LIKE 'from/%'`. **Cần thêm index** `(user_id, path text_pattern_ops)` trong migration `0003`.
    - Copy rồi delete từng object, song song khoảng 16 (`buffer_unordered`).
    - Sau đó cập nhật chỉ mục bằng một câu: `UPDATE files SET path = $to || substr(path, length($from)+1), parent = …`.
    - Thư mục lớn chạy dạng **job nền**: bảng `jobs(id, user_id, kind, total, done, state, error)` + `GET /api/jobs/:id`; UI hiện tiến độ.
  - **Kiểm tra:**
    - Đích nằm trong chính nguồn (`a` → `a/b`) → 400.
    - Đích đã tồn tại → 409 (chưa làm merge).
  - **Quota:** không đổi.
- **Xoá thư mục:** `DELETE /api/files/*path` khi path là thư mục.
  - Xoá đệ quy bằng `op.delete_with(prefix).recursive(true)`.
  - Hoàn quota bằng đúng `SUM(size)` lấy từ chỉ mục; dùng `quota_used = quota_used - $n` ngay trong SQL.
  - Xoá các dòng chỉ mục `path = p OR path LIKE 'p/%'`.
- **Thùng rác** (nên làm cùng lúc, để xoá nhầm còn cứu được):
  - Xoá thì chuyển vào `.trash/<ts>/<path>`; `.trash` bị ẩn khỏi listing.
  - API `GET /api/trash`, `POST /api/trash/restore`, `DELETE /api/trash` (dọn hết).
  - Job dọn tự động sau 30 ngày (để cấu hình được).
  - **Quota vẫn tính file trong thùng rác** cho tới khi bị dọn.
- **Test:**
  - mkdir lồng nhau; rename file; move thư mục 1.000 file (fs và S3); chặn move vào chính nó.
  - Xoá thư mục thì hoàn đúng quota.
  - Restore từ thùng rác.

**Frontend**

- **RowMenu:** thêm Đổi tên (sửa inline), Di chuyển (dialog chọn thư mục; cây thư mục lazy dùng `/api/files` với filter `is_dir`), Xoá thư mục (hỏi xác nhận, ghi rõ số file và dung lượng).
- **Toolbar:** nút "Thư mục mới".
- **Chọn nhiều:**
  - Checkbox hiện khi hover hoặc nhấn giữ trên mobile; shift-click chọn một dải; Ctrl/Cmd+A.
  - Lưu tập đang chọn theo `path`, không theo chỉ số: danh sách là ảo, không phải dòng nào cũng nằm trong DOM.
- **Kéo thả** file vào thư mục để di chuyển (desktop).
- **Nghiệm thu:**
  - Đổi tên thư mục chứa 10.000 file xong trong < 30 giây trên mini PC, UI có tiến độ.
  - Không còn object hoặc dòng chỉ mục mồ côi: kiểm bằng `reindex`, số file trước và sau phải bằng nhau.

### 9.2 Upload và download lớn (yêu cầu cốt lõi) ← làm tiếp theo

**Upload**

- **Kéo thả cả thư mục** (DataTransferItem.webkitGetAsEntry; dự phòng bằng `<input webkitdirectory>`).
  - Duyệt cây trong **Web Worker**, gửi về luồng chính từng batch.
  - Không giữ hàng triệu object `File` trên luồng chính; chỉ giữ hàng đợi `{path, size, handle}`.
- **Hàng đợi upload** (module `uploader.ts`):
  - Song song 4 file nhỏ; file > 64MB thì gửi 4 chunk song song.
  - Backend hiện yêu cầu **append tuần tự theo offset** → cần đổi sang ghi chunk theo vị trí:
    - Dùng `pwrite`/`seek` vào file `.part`, kèm bitmap các chunk đã nhận.
    - Hoặc ghi mỗi chunk ra một file `.partN` rồi nối lại khi `complete`.
  - **Resume:**
    - Bản ghi upload đang dở phải chuyển khỏi RAM (`state.rs::uploads`) vào Postgres, bảng `uploads(id, user_id, path, size, mtime, received, created_at)`.
    - Thêm `GET /api/files/uploads/:id` trả về `received` để client chạy tiếp sau khi reload hoặc mất mạng.
    - Client lưu hàng đợi vào IndexedDB.
  - Retry có backoff khi lỗi 5xx hoặc mất mạng. Lỗi 409 offset → hỏi server `received` rồi gửi tiếp từ đó.
- **Nhiều file nhỏ** (1 triệu file × 3 request mỗi file là quá chậm):
  - **API batch:** `POST /api/files/batch` nhận multipart hoặc tar stream gồm nhiều file < 1MB.
  - Backend ghi thẳng lên S3 (không qua staging), rồi **insert chỉ mục dạng bulk** (UNNEST, như `rebuild`).
  - Mục tiêu: ≥ 300 file/giây qua LAN.
- **Quota:** trừ trước (reserve) lúc init hoặc batch để nhiều upload song song không cùng vượt quota; hoàn lại khi huỷ hoặc hết hạn.
- **Dọn rác:**
  - Job xoá `staging/*.part` và bản ghi upload quá 7 ngày.
  - Job dọn multipart S3 dở dang (OpenDAL abort).

**Download**

- **Tải thư mục thành zip stream:** `GET /api/zip?path=…` (hoặc POST danh sách path khi chọn nhiều).
  - Dùng crate `async_zip`, chế độ store (không nén) + ZIP64, stream trực tiếp từ S3 vào response.
  - Không có file tạm, RAM giữ cố định bất kể thư mục to cỡ nào.
  - `Content-Disposition` đặt tên `<folder>.zip`.
- **Tải file đơn:** hiện `api.downloadUrl` tải hết thành **blob trong RAM rồi mới lưu**, nên file vài GB sẽ làm sập tab.
  - **Phải đổi:** dùng link trực tiếp kèm token ngắn hạn, ví dụ `GET /api/files/*path?dl=<signed token>` + `<a download>`, hoặc presigned URL.
  - Đây là **lỗi nghiêm trọng cần sửa sớm**.

**UI**

- Panel hàng đợi (thay toast): tổng tiến độ, tốc độ, ETA, tạm dừng/tiếp tục, huỷ, thử lại các file lỗi, danh sách lỗi.
- Không re-render cả danh sách mỗi lần nhận progress: throttle cập nhật khoảng 4 lần/giây.

**Nghiệm thu** (đo trên mini PC qua LAN; ghi số đo vào báo cáo):

- Upload 100.000 file nhỏ không treo tab; RAM tab < 500MB.
- Upload 1 file 50GB, rút mạng giữa chừng, cắm lại thì chạy tiếp.
- Tải zip thư mục 15GB của `datlt4` thành công; RAM backend không tăng theo kích thước thư mục.

### 9.3 Viewer, thumbnail, OnlyOffice

- **Thumbnail:**
  - `GET /api/thumb/*path?s=256`: tạo webp phía server bằng crate `image` (+ `webp`), cache vào S3 dưới `<uuid>/.nextkloud/thumbs/<hash>.webp`, hoặc ra đĩa.
  - Video: lấy 1 frame bằng ffmpeg (cần thêm ffmpeg vào image).
  - Có `ETag` và `Cache-Control`.
  - Grid hiển thị thumbnail thật, lazy theo danh sách ảo; giới hạn số request song song.
- **Viewer** (lazy import theo loại file):

  | Loại | Cách làm |
  |---|---|
  | Ảnh | lightbox, vuốt trái/phải giữa các ảnh cùng thư mục |
  | Video/audio | `<video src>` với URL có token (Range đã hỗ trợ) |
  | PDF | pdf.js |
  | Zip | API liệt kê nội dung (đọc central directory bằng Range) + tải 1 file trong zip |
  | Epub | epub.js |
  | drawio, excalidraw | nhúng editor, lưu ngược lại về file |
  | Office | OnlyOffice: `POST /api/office/config` trả config + JWT; `POST /api/office/callback` nhận file đã lưu và ghi lại vào store và chỉ mục; JWT_ENABLED trên DocumentServer |

- **Markdown kiểu Obsidian:**
  - KaTeX, wikilink `[[...]]`, callout, sửa và xem trước.
  - **TTS:** mỗi đoạn có 1 nút đọc; bỏ qua code fence và math.
  - Ấn render sẽ lưu mp3 kèm hash nội dung. Sửa md mà chưa render lại thì hiện **banner "âm thanh đã cũ"** (so hash). Xem quy tắc trong `ARCHITECTURE.md`.
- **Nghiệm thu:** bundle chính không phình (viewer nằm ở chunk riêng); mở ảnh 20MB trên A8 Star không bị đơ.

### 9.4 Kanban, Task tracker, Habit tracker và nhập Deck

**Dữ liệu Deck** (đọc từ backup, chỉ ghi số liệu):

| | Board 1 "Personal" (archived) | Board 3 "Personal" (đang dùng) |
|---|---|---|
| Bigtask | 22 | 28 |
| Subtask | 30 | 30 |
| To do | 9 | 12 |
| Doing | 6 | 6 |
| Pending | 4 | 8 |
| Done | 74 | 137 |
| Write doc | 0 | 0 |
| Delete | 11 | 22 |

- Tổng cộng 399 card chưa xoá. **223 card có cờ `done`**: Deck ẩn card có cờ này, đây có thể chính là lý do "kéo vào Done thì card biến mất".
- 17 label, 40 lượt gán label, không có file đính kèm.
- 30 card có checklist `- [ ]` trong description.
- Bảng `oc_deck_cards` có các cột: `id, title, description, stack_id, type, last_modified, created_at, owner, order, archived, duedate, done, startdate, color, deleted_at…`.
- **Người dùng đã tự mô phỏng subtask** bằng 2 cột "Bigtask" và "Subtask".
- Người dùng có nhắc tới cột **"Done2"**, nhưng **trong bản backup không có cột tên như vậy**. Cần hỏi lại: Done2 nằm ở board nào, hay chính là cột "Done" hiện tại?

**Mong muốn của người dùng:**

1. Gom card nhỏ thành **subtask** của task lớn.
   - Importer **đề xuất** cách gom để người dùng **duyệt trước khi ghi**: card ở cột Subtask → tìm cha ở cột Bigtask theo label chung, tên giống nhau, hoặc thời gian tạo gần nhau.
   - Cho người dùng sửa lại cách gom trên một màn xem trước.
2. Card đã xong **không được biến mất**, nhưng cột Done cũng không được dài vô hạn.
   - Hiện N card xong gần nhất (mặc định 10), phần còn lại gom thành "Đã xong cũ hơn (127)", bấm vào mới mở và tải dạng phân trang.
   - Có lọc theo tuần hoặc tháng.
   - Khi nhập: gộp Done2 vào Done; card có cờ `done` vào Done; **giữ `done_at`**.
3. Checklist trong description: chuyển thành subtask, hoặc giữ nguyên dạng checklist (hỏi người dùng).

**Schema đề xuất** (migration `0004`):

```sql
boards(id, owner, title, created_at, archived)
board_members(board_id, user_id, role)                          -- chia sẻ: owner | edit | view
columns(id, board_id, title, position, kind)                    -- kind: normal | done
cards(id, board_id, column_id, parent_id NULL, title, description_md, position,
      due_at, start_at, done_at, color, created_by, created_at, updated_at, archived)
labels(id, board_id, title, color)
card_labels(card_id, label_id)
card_links(card_id, url, title, image_url, fetched_at)          -- unfurl Open Graph
```

- **position:** dùng fractional index dạng chuỗi để kéo thả không phải đánh lại thứ tự cả cột.
- **API:** CRUD board, column, card; `POST /api/cards/:id/move {column_id, before, after}`; `GET /api/boards/:id?done_limit=10`; `GET /api/columns/:id/cards?offset=` cho phần card xong đã thu gọn.
- **Unfurl:** server lấy title và og:image của URL trong card. Có timeout 5 giây, tối đa 1MB, **chặn IP nội bộ (SSRF)**. Ảnh lưu cache.
- **UI:**
  - Kéo thả bằng pointer events (chạy trên mobile, A8 Star mượt).
  - Card cha hiện thanh tiến độ subtask (3/5).
  - Mở card thành drawer chi tiết, sửa markdown.
- **Task tracker:** lọc toàn bộ card có `due_at` theo hôm nay, tuần này, quá hạn.
- **Habit tracker:** `habits(id, user_id, title, schedule)` + `habit_logs(habit_id, day)`; xem dạng lưới 7×N giống GitHub, có streak.
- **Importer:** `nextkloud import-deck <sqlite> <username> --dry-run` in ra phương án gom subtask. Chỉ ghi DB khi người dùng duyệt và chạy với `--apply`.
- **Nghiệm thu:** nhập đúng 399 card (trừ cột Delete nếu người dùng đồng ý bỏ); không có card nào biến mất; cột Done hiển thị gọn.

### 9.5 Đa ngôn ngữ (i18n)

- Viết `t(key, params)` dạng signal; dictionary nằm ở `src/i18n/{vi,en,ja,zh}.ts` (hiện chỉ có 3 key).
- Chuyển toàn bộ chuỗi đang hardcode trong `App.tsx` và `UsersAdmin.tsx` sang key.
- Lựa chọn ngôn ngữ lưu vào `.nextkloud/settings.json` (thêm trường `lang`, theo cơ chế của `background.ts`) để đồng bộ giữa các thiết bị; mặc định theo `navigator.language`.
- Định dạng ngày và số theo locale đang chọn (hiện đang cố định `vi-VN`).
- Thông báo lỗi từ backend: chuyển sang dạng mã (`{code, message}`) để client dịch.

### 9.6 Desktop sync (Tauri)

- Shell Tauri hiện trống; app desktop đang load web build.
- **Engine sync** (Rust, chạy trong Tauri):
  - Người dùng chọn thư mục cục bộ tuỳ ý; cấu hình lưu theo từng máy.
  - Delta: thêm cột `etag` (hoặc `version`) + `updated_at` vào `files`, cùng API `GET /api/changes?since=<cursor>`.
  - Watcher cục bộ (`notify`) để upload thay đổi. Xung đột: giữ cả hai bản, đặt tên `name (conflict ...)`.
  - **Free-up space:** file ở dạng placeholder (Windows Cloud Files API / macOS File Provider / Linux FUSE hoặc file stub). Đây là phần khó nhất; nên làm bản đơn giản trước: "chỉ giữ metadata, mở thì tải về".
  - **Tự giải phóng:** file không mở sau N ngày thì đưa về placeholder.
- **Mobile** (Android/iOS qua Tauri mobile): chỉ duyệt, upload, download và Kanban; không sync.

### 9.7 Notifications và auto-update

- Tauri updater plugin dùng `latest.json` trong GitHub release; ký bằng updater key (CI secret). — ✅ xong 0.1.23: `tauri-plugin-updater` + `process` (Rust + JS), `plugins.updater.active` + capabilities, tự kiểm tra khi mở app (Tauri), banner + nút "Tải & cập nhật" (hiện % theo chunk), "Khởi động lại" sau khi cài; mục "Cập nhật phần mềm" trong Cài đặt → Hệ thống (check tay). Mobile: updater bỏ qua (không phải desktop).
- **Thông báo:**
  - Server dùng SSE `GET /api/events`.
  - Sự kiện: upload xong, được chia sẻ board hoặc file, card được giao hoặc đến hạn.
  - Desktop dùng notification hệ điều hành; web dùng Notification API.

### 9.8 Khác

- **Redis:** dùng cho rate-limit đăng nhập, trạng thái job, cache thumbnail và unfurl. Hiện container chạy nhưng không được dùng.
- **Chia sẻ file/thư mục** (giữa user và link công khai có mật khẩu/hạn): chưa có trong spec chi tiết, **hỏi người dùng trước**.
- **Versions:** lưu phiên bản cũ khi ghi đè; cân nhắc migrate 1,7G versions từ Nextcloud.
- **Mobile (bỏ từ 23/11, 0.1.24):** chỉ hỗ trợ PC Linux/macOS/Windows. Xoá `mobile.yml`, `scripts/resign-ios.sh`, job mobile trong `release.yml`; không build APK/IPA nữa. Lý do: iOS crash khi mở app, icon Android bị crop; nhu cầu mobile chưa có.
- **Migrate trash** (854M) từ Nextcloud, sau khi đã có thùng rác ở 9.1.
- Dọn thư mục MinIO cũ `/mnt/das_8tb/nextkloud_volume/s3` (rỗng): **hỏi người dùng trước khi xoá**.

## 10. Nợ kỹ thuật và rủi ro

Ưu tiên giảm dần.

1. **Download tải cả file vào RAM của trình duyệt** (backend đã stream; vấn đề nằm ở client) (`api.downloadUrl` → blob). File lớn làm sập tab. Xem 9.2.
2. **Race trên `quota_used`:**
   - `complete_upload`, `delete_file` đọc quota, tính trong Rust, rồi ghi đè giá trị tuyệt đối.
   - Hai request đồng thời sẽ mất một cập nhật.
   - **Sửa:** `UPDATE users SET quota_used = quota_used + $d WHERE id=$1 AND (quota_limit=0 OR quota_used + $d <= quota_limit) RETURNING …`, hoặc `SELECT … FOR UPDATE` trong transaction.
3. **Upload tuần tự từng chunk 8MB, bản ghi upload đang dở nằm trong RAM** (mất khi restart), không dọn staging mồ côi.
4. **Store và chỉ mục không atomic:**
   - Ghi S3 thành công nhưng ghi chỉ mục lỗi thì file bị ẩn (chỉ cứu được bằng `reindex`).
   - Xoá user: xoá S3 trước, DB sau.
   - Cân nhắc job đối soát định kỳ (so số lượng và dung lượng, báo lệch).
5. ~~**`reindex` xoá các thư mục rỗng**~~ (đã sửa): `rebuild` giờ giữ lại các thư mục rỗng do user tạo (không có file con trong lần quét, bỏ qua thư mục hệ thống `.`); scan bỏ qua cache `.nextkloud/thumbs/` nên thumbnail không tính vào quota.
6. **JWT mang cờ `adm`:** thu hồi quyền admin không có hiệu lực tới khi token hết hạn. Nên kiểm tra `is_admin` từ DB ở các API admin.
7. **CORS `permissive`;** `ALLOWED_ORIGINS` có trong compose nhưng không được dùng. Chưa có rate-limit đăng nhập. JWT để trong localStorage (rủi ro XSS; markdown viewer phải sanitize HTML).
8. **Offset phân trang sâu khi sort theo size** mất khoảng 430ms trên 1 triệu dòng; `COUNT` mỗi request mất khoảng 70ms. Có thể cache count (Redis) hoặc chỉ count khi offset = 0.
9. **Kích thước thư mục luôn bằng 0;** footer chỉ cộng file nằm trực tiếp trong thư mục. Muốn có tổng đệ quy thì cần cột tổng hợp hoặc truy vấn `LIKE prefix`.
10. **Lặt vặt:**
    - `users::set_quota` còn `let _ = q;` thừa.
    - `tts.rs` có import thừa `IntoResponse`.
    - Dockerfile đặt `NEXTKLOUD_ADDR` mà code không đọc.
    - `@tanstack/solid-virtual` thừa trong package.json.
    - `ARCHITECTURE.md` lỗi thời ở mục virtual list và Redis.
11. **CI không chạy test DB/S3** (không có service container). Nên thêm services Postgres và RustFS vào `build.yml` rồi set `NK_TEST_DB`/`NK_TEST_S3`.
12. **`search` dùng `ILIKE '%q%'`:** truy vấn < 3 ký tự không dùng được trigram, phải seq scan (khoảng 300ms trên 1 triệu dòng). UI chỉ gửi khi q ≥ 2 ký tự.

## 11. Bẫy đã gặp

- **Solid** delegate click ở `document`, nên `stopPropagation` không chặn được listener `document` khác. Phải kiểm tra `e.target.closest(".menu-wrap")`.
- **CSS:**
  - `content-visibility: auto` cắt mất menu bị tràn ra ngoài.
  - `backdrop-filter` tạo stacking context mới → header phải có `z-index`.
  - **Tên class chung** (`.ghost`) va với component khác; dùng tên riêng (`.skel`).
- **Nút trong `<form>`** mặc định là submit → luôn ghi `type="button"`.
- **Danh sách ảo:** đổi prefix phải `batch()` cùng lúc với cache. Nếu không, effect tải trang kế sẽ dùng prefix cũ và ghi đè số đếm.
- **OpenDAL 0.59:**
  - Phải gọi `opendal::install_default()` thì mới có HTTP transport.
  - Phải bật feature `executors-tokio` mới ghi multipart song song được.
  - Không có `.finish()`.
  - `remove_all` đã deprecated → dùng `delete_with(p).recursive(true)`.
  - Kiểm tra capability bằng `op.info().capability()`.
- **axum** `DefaultBodyLimit` mặc định 2MB → route chunk được nâng lên 64MB.
- **Route** `/api/search` đặt ngoài `/api/files/*` để không đụng file tên "search".
- **RustFS 1.0.4:** biến môi trường `RUSTFS_ACCESS_KEY` / `RUSTFS_SECRET_KEY` / `RUSTFS_ADDRESS` / `RUSTFS_CONSOLE_*`, chỉ hỗ trợ path-style. Tạo bucket bằng rclone, curl sigv4 không được.
- **Shell:** `pkill -f` có thể kill chính shell đang chạy lệnh; `sed -i` không chạy trên ổ repo; heredoc chứa JS bị safety-check chặn.

## 12. Checklist review

- [ ] `cargo test -p nextkloud-backend` pass **khi có** `NK_TEST_DB` + `NK_TEST_S3` (không chỉ bản CI).
- [ ] `npx tsc --noEmit` + `npm run build` sạch, không có warning mới.
- [ ] API admin kiểm tra quyền; mọi path từ client đều qua `clean_path` / `Store::key`.
- [ ] Thay đổi quota là atomic và có test cho trường hợp vượt quota.
- [ ] Không có secret hay dữ liệu cá nhân trong diff (repo public). Commit không ghi tác giả AI.
- [ ] UI ở 390px: không cuộn ngang, nút đủ to để chạm. Dark mode đọc rõ.
- [ ] Thư mục lớn: DOM chỉ khoảng vài chục dòng; không tải toàn bộ danh sách; không giữ blob lớn trong RAM.
- [ ] Sau deploy: health 200, log backend không có `ERROR`, đăng nhập và duyệt thư mục của `datlt4` vẫn chạy.
- [ ] Báo cáo cho người dùng bằng tiếng Việt, ngắn gọn, kèm số đo thật (không ước lượng).
