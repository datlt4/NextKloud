# NextKloud — Handoff

Tài liệu bàn giao cho agent tiếp theo (review hoặc tiếp quản). Đọc file này trước, sau đó đến `docs/ARCHITECTURE.md`.
Cập nhật: 2026-11-20 · commit gần nhất `147e850`.

---

## 1. Mục tiêu sản phẩm (tóm tắt yêu cầu gốc)

NextKloud thay thế Nextcloud, **không dùng PHP**: Tauri v2 + Rust (Axum) + TypeScript + SolidJS.

- **File** kiểu Google Drive. Phải chịu được thư mục **1 triệu file** và upload/download **hàng trăm GB** mà không treo. Đây là nỗi đau chính của người dùng với Nextcloud.
- **Kanban** thay Deck (giống JIRA): có subtask, chia sẻ, thumbnail cho URL. Kèm **Task tracker** và **Habit tracker**.
- **Viewer:**
  - Office: OnlyOffice có sẵn trên mini PC, port 8027.
  - Media, zip, ebook, drawio, excalidraw.
  - Markdown kiểu Obsidian (có math) và chế độ **TTS**. Sửa md mà chưa render lại thì hiện banner "âm thanh đã cũ".
- **Máy yếu (Samsung A8 Star) phải mượt.**
- **Multi-user:**
  - Admin tạo user với mật khẩu mặc định, user bị buộc đổi mật khẩu lần đầu.
  - Admin nới hoặc thu hẹp quota.
  - **Bắt buộc có unit test.**
- **Không có trusted-domains** (khác Nextcloud).
- **i18n** EN / JA / ZH / VI, lựa chọn đồng bộ giữa các thiết bị.
- **Desktop sync** kiểu OneDrive: chọn thư mục, free-up space, tự giải phóng sau N ngày.
- **Mobile:** chỉ duyệt, upload, download và Kanban.
- Notifications và auto-update.
- **CI** theo mẫu Kermius/Kellyfin. iOS build theo chuẩn job-spec của ipa-resign-dashboard.
- **MIT license.**
- **Self-host bằng Docker** trên mini PC (Ryzen 3400GE, không có GPU NVIDIA).

## 2. Quy tắc làm việc (bắt buộc)

- **Trả lời người dùng bằng tiếng Việt.** Mặc định trả lời ngắn.
- Sửa có chủ đích, chỉ đụng vào phần cần thiết; không refactor ngoài yêu cầu. Comment trong code ngắn, tối đa 1–2 dòng.
- **Làm xong mỗi thay đổi thì tự commit và push lên `main`, không cần hỏi.** Commit message **không ghi tác giả là AI**.
- **Không bao giờ commit secret** (`.env`, mật khẩu, key). Không đổi mật khẩu admin của người dùng.
- Giá trị nào chỉ suy ra mà chưa kiểm chứng (version, path, config) thì phải nói rõ là suy đoán.
- **Release:** `release.yml` chỉ publish khi version trong `apps/desktop/package.json` thay đổi.
  - Bump bằng `scripts/set-version.sh X.Y.Z`.
  - Người dùng muốn bump **major** khi ra bản lớn. Hiện đang ở 0.1.6 và 2 commit gần nhất chưa bump.

## 3. Hạ tầng và cách chạy

### Mini PC (production)

- SSH: `ssh -o BatchMode=yes mini-wg`.
- Thư mục compose: `/home/mini/Software/NextKloud` (có `.env` chứa S3 key và mật khẩu; **không in nội dung ra**).
- **Ports:**

  | Port | Dịch vụ |
  |---|---|
  | 8011 | backend (trong container là 8014) |
  | 9000 / 9001 | RustFS (S3 API / console) |
  | 8016 | TTS |
  | 8027 | OnlyOffice |
  | 8015 | LibreTranslate |

- **Dữ liệu** (trên `/mnt/das_8tb/nextkloud_volume/`):
  - `app/`: staging upload.
  - `pgdata/`.
  - `rustfs/`: object.
  - `redis/`.
  - `tts/`.
  - `migration/`: backup Deck và file mật khẩu tạm.
- **Deploy:** push lên `main` → CI `build.yml` build image `hienanh/nextkloud` → trên mini PC chạy:
  ```bash
  cd /home/mini/Software/NextKloud && docker compose pull -q backend && docker compose up -d backend
  ```
- **Theo dõi CI** (máy local không có `gh`):
  `curl -s "https://api.github.com/repos/datlt4/NextKloud/actions/runs?per_page=5&head_sha=<SHA>"`
- **Dựng lại chỉ mục file:**
  ```bash
  docker compose exec backend nextkloud reindex [username]
  ```
  Khi khởi động, backend tự dựng chỉ mục cho user chưa có.

### Máy dev local

- **Container test có sẵn:**
  - `nk-pg-test`: Postgres 16, `127.0.0.1:15432`, user/pass/db đều là `nk`.
  - `nk-rustfs-test`: RustFS, `127.0.0.1:19000`, bucket `nktest`. Key đọc bằng `docker exec nk-rustfs-test printenv RUSTFS_ACCESS_KEY` (tương tự với `RUSTFS_SECRET_KEY`).
- **Test đầy đủ** (thiếu 2 biến môi trường thì các test cần DB/S3 tự bỏ qua):
  ```bash
  NK_TEST_DB=postgres://nk:nk@127.0.0.1:15432/nk \
  NK_TEST_S3=http://127.0.0.1:19000,nktest,$KEY,$SECRET \
  cargo test -p nextkloud-backend          # hiện tại 27 test pass
  ```
- **Frontend:**
  ```bash
  cd apps/desktop && npx tsc --noEmit -p . && npm run build
  ```
  Cần `source ~/.nvm/nvm.sh` trước.
- **Chạy backend local:**
  - Biến môi trường: `BIND=127.0.0.1:18014 DATABASE_URL=… STORAGE_DRIVER=s3 S3_ENDPOINT=http://127.0.0.1:19000 S3_BUCKET=nktest S3_ACCESS_KEY=… S3_SECRET_KEY=… STAGING_DIR=… NEXTKLOUD_WEB=apps/desktop/dist`.
  - Lệnh: `target/debug/nextkloud`.
  - Tài khoản admin trong DB test: `admin` / `adminpass123`. Chỉ dùng ở local.
- **Test UI:** puppeteer-core + `/usr/bin/google-chrome` headless. Script mẫu nằm trong scratchpad của session trước (`virt.mjs`, `admin.mjs`).
  Viết script bằng file, không dùng heredoc chứa JS vì bị safety-check chặn.

## 4. Đã xong

| Hạng mục | Ghi chú |
|---|---|
| Khung repo, CI build/release/mobile, quota (unit test), Docker | M0 |
| Auth JWT, buộc đổi mật khẩu lần đầu, API users/quota | `users.rs`, `auth.rs` |
| Storage RustFS qua OpenDAL 0.59 | `files.rs`. Upload chunk 8MB → staging → multipart 16MB×4 lên S3. Download stream có Range. |
| Migrate Nextcloud (làm thủ công bằng rclone) | 5.815 file / 15,39 GiB của `datlt4`, `rclone check` khớp 100%. Chưa migrate trash (854M) và versions (1,7G). |
| **Chỉ mục file trong Postgres** (`index.rs`, migration `0002`) | Phân trang offset, thư mục luôn đứng trước, sort tự nhiên (ICU), tìm kiếm bằng trigram, giữ mtime gốc (S3 user metadata `mtime`). 1 triệu file: trang đầu ~70ms, giữa danh sách ~110ms. |
| Danh sách ảo (list và grid) | `App.tsx`: chỉ render dòng đang thấy, tải từng trang 200 dòng, debounce 80ms khi kéo thanh cuộn. |
| Giao diện: theme cam/kem sáng/tối, logo, hình nền đồng bộ qua `.nextkloud/` | `background.ts` |
| **Màn hình admin quản lý user** | `UsersAdmin.tsx`: tạo user (sinh mật khẩu), quota (preset, cảnh báo khi thấp hơn mức đang dùng), reset mật khẩu, xoá (gõ lại tên để xác nhận; xoá luôn object và chỉ mục). |

## 5. Việc cần làm (theo thứ tự ưu tiên)

Mỗi mục đều cần: unit test cho backend, typecheck và build frontend, kiểm tra UI bằng Chrome headless (desktop 1400px và mobile 390px), sau đó commit, push và deploy.

### 5.1 Thao tác file cơ bản ← **làm tiếp theo**

- **Tạo thư mục:**
  - Backend: thêm dòng dir vào chỉ mục. Không cần object marker trên S3, vì chỉ mục là nguồn sự thật cho cây thư mục.
  - API gợi ý: `POST /api/files/mkdir {path}`.
- **Đổi tên và di chuyển** file/thư mục:
  - S3 không có rename, phải copy rồi delete từng object.
  - Thư mục lớn chạy dạng job nền có tiến độ; cập nhật chỉ mục bằng `UPDATE … SET path = $new || substr(path, …)` trong 1 transaction.
  - Cần thêm index `text_pattern_ops` cho truy vấn kiểu `path LIKE 'a/b/%'`.
- **Xoá thư mục** (đệ quy):
  - Hiện `delete` từ chối thư mục.
  - Cần hoàn quota đúng bằng `SUM(size)` từ chỉ mục.
- **Chọn nhiều** (checkbox/shift-click) rồi xoá, di chuyển hoặc tải về hàng loạt.
- **Thùng rác** (khôi phục, tự dọn sau N ngày). Sau đó cân nhắc migrate trash cũ từ Nextcloud.

### 5.2 Upload và download lớn (yêu cầu cốt lõi)

- **Kéo thả cả thư mục** (`webkitGetAsEntry`):
  - Duyệt cây trong **Web Worker**, xếp hàng đợi, không giữ hàng triệu `File` trên luồng chính.
- **Upload song song:**
  - 3–4 file cùng lúc; file lớn gửi các chunk song song.
  - Tự chạy tiếp sau khi mất mạng. Backend đã có `?offset=` và kiểm tra offset.
  - **Vấn đề:** `uploads` đang nằm trong RAM (`state.rs`), nên mất khi restart. Cần lưu vào Postgres hoặc Redis.
- **Nhiều file nhỏ:** 1 triệu file × 3 request mỗi file là quá chậm. Đề xuất API batch cho file < 1MB: gộp nhiều file trong 1 request, hoặc upload dạng tar stream.
- **Tải cả thư mục** thành **zip stream**:
  - Tạo zip ngay khi gửi, không tạo file tạm (crate `async_zip`, chế độ store không nén).
  - Không giới hạn kích thước.
- **UI hàng đợi** (tốc độ, ETA, tạm dừng, huỷ, thử lại), thay toast hiện tại.
- **Kiểm thử:** 100k file nhỏ và 1 file 50GB trên mini PC. Đo RAM của backend và trình duyệt.

### 5.3 Viewer và OnlyOffice

- Ảnh: thumbnail 256px dạng webp, tạo phía server và cache lại. Grid hiển thị thumbnail thật thay cho icon.
- Video/audio: phát bằng Range, đã hỗ trợ sẵn.
- PDF.
- Zip: liệt kê nội dung, giải nén 1 file.
- Ebook: epub.
- drawio, excalidraw.
- OnlyOffice: callback lưu file, có JWT. OnlyOffice có sẵn ở `ONLYOFFICE_URL`.
- Markdown kiểu Obsidian:
  - Đã có `viewers/MarkdownView.tsx` và TTS proxy `/api/tts/*`.
  - Còn thiếu: math (KaTeX), wikilink, sửa trực tiếp, banner stale-audio theo quy tắc trong ARCHITECTURE.md.
- Lazy-load viewer theo loại file để giữ bundle nhỏ cho A8 Star.

### 5.4 Kanban, Task và Habit tracker

- Board/column/card/subtask, kéo thả mượt trên mobile. Chia sẻ board cho user khác. Card có thumbnail cho URL (unfurl Open Graph phía server).
- **Nhập Deck** từ `/mnt/das_8tb/nextkloud_volume/migration/nextcloud-owncloud-2026-11-20.db` (SQLite, 429 card).
- **Mong muốn của người dùng khi nhập Deck:**
  - Deck không có subtask nên người dùng đã tạo nhiều card nhỏ thay cho task lớn. Importer cần **gom các card nhỏ thành subtask** của task cha.
    → **Đề xuất cách gom cho người dùng duyệt trước khi nhập.**
  - Cột **"Done2"** được dùng để giữ card đã xong, vì kéo vào "Done" thì card biến mất (bị archive). Card xong **không được biến mất** nhưng cột cũng không được dài vô hạn.
    → Gợi ý: hiện N card gần nhất, phần cũ thu gọn hoặc lọc theo thời gian. Gộp Done2 vào Done khi nhập.
- Task tracker và Habit tracker (streak, lịch).

### 5.5 Đa ngôn ngữ (i18n)

- Đã có sẵn `src/i18n/{en,ja,vi,zh}.ts` **nhưng chưa nối vào UI**: chuỗi tiếng Việt đang hardcode trong `App.tsx` và `UsersAdmin.tsx`.
- Lựa chọn ngôn ngữ lưu vào `.nextkloud/settings.json` để đồng bộ giữa các thiết bị, giống cách lưu theme/nền.

### 5.6 Desktop sync (Tauri)

- `src-tauri` hiện chỉ là khung rỗng.
- **Engine sync:**
  - Chọn thư mục cục bộ tuỳ ý.
  - File chỉ là placeholder (free-up space); tự giải phóng sau N ngày không mở.
  - Delta qua chỉ mục, cần thêm cột `etag` / `version`.
- **Mobile:** chỉ duyệt, upload, download và Kanban.

### 5.7 Notifications và auto-update

- Tauri updater gắn với release trên GitHub. Thông báo khi upload xong, khi được chia sẻ, khi Kanban có thay đổi.

### 5.8 Khác

- **Redis:** đã chạy trong compose nhưng backend **chưa dùng**. Dùng cho trạng thái upload, rate-limit đăng nhập, cache thumbnail.
- **Chia sẻ file/thư mục** giữa user và link công khai (chưa có trong spec chi tiết; hỏi người dùng trước).
- iOS build theo job-spec của ipa-resign-dashboard: kiểm tra lại `mobile.yml` và `scripts/resign-ios.sh`.
- `crates/migrate` mới là stub. Người dùng muốn migrate thủ công, không cần nhúng vào app, nên chỉ làm nếu được yêu cầu.

## 6. Nợ kỹ thuật và rủi ro đã biết (nhờ reviewer kiểm tra)

1. **Race trên `quota_used`:**
   - `complete_upload` và `delete_file` đọc quota → tính → ghi lại. Hai request đồng thời có thể ghi đè lẫn nhau.
   - Nên dùng `UPDATE users SET quota_used = quota_used + $delta …` kèm điều kiện giới hạn, hoặc `SELECT … FOR UPDATE`.
2. **Store và chỉ mục không atomic:**
   - Ghi S3 thành công nhưng ghi chỉ mục lỗi thì file bị "ẩn".
   - Hiện chỉ có `reindex` để chữa. Cân nhắc thêm job đối soát định kỳ.
3. **Thư mục rỗng còn lại** trong chỉ mục sau khi xoá file cuối cùng. Đây là chủ ý (giống Drive), nhưng `reindex` sẽ làm chúng biến mất vì S3 không có object.
4. **Kích thước thư mục luôn bằng 0;** footer chỉ tính file nằm trực tiếp trong thư mục.
5. **Offset sâu khi sort theo size** trên 1 triệu dòng mất khoảng 430ms. Nếu cần nhanh hơn thì chuyển sang keyset pagination.
6. **JWT mang cờ `adm`:** bỏ quyền admin của ai đó không có hiệu lực cho tới khi token hết hạn.
7. **Xoá user:** xoá object trên S3 trước, xoá DB sau. Lỗi ở giữa thì còn user nhưng mất dữ liệu.
8. **CORS** đang là `permissive`; `ALLOWED_ORIGINS` khai báo trong compose **chưa được dùng**. Chưa có rate-limit đăng nhập.
9. **Bản ghi upload đang dở nằm trong RAM**, mất khi restart (xem 5.2). Staging file mồ côi không có job dọn.
10. **Lặt vặt:**
    - `users::set_quota` còn biến thừa `let _ = q;`.
    - `tts.rs` có import thừa (warning).
    - `ARCHITECTURE.md` nhắc `@tanstack/solid-virtual` nhưng thực tế không dùng; danh sách ảo tự viết.
    - Cùng file đó mô tả Redis cache nhưng thực tế chưa dùng.
11. **Hiển thị ngày:** S3 `LastModified` là ngày upload; ngày sửa gốc lấy từ metadata `mtime`. File upload bằng client khác không gửi `mtime` thì hiển thị ngày upload.
12. **Bẫy đã gặp, tránh lặp lại:**
    - Solid delegate click ở `document`, nên `stopPropagation` không chặn được listener khác. Phải kiểm tra bằng `closest()`.
    - `content-visibility` cắt menu bị tràn ra ngoài; `backdrop-filter` tạo stacking context mới.
    - Đặt tên class chung (`.ghost`) từng va với `.btn.ghost`.
    - Nút trong `<form>` phải có `type="button"`.
    - OpenDAL cần `install_default()` và feature `executors-tokio`.
    - axum `DefaultBodyLimit` mặc định chỉ 2MB.
    - Route `/api/search` đặt ngoài `/api/files/*` để không đụng file tên "search".
    - Không dùng `pkill -f` với pattern có thể khớp chính lệnh đang chạy.

## 7. Checklist cho reviewer

- [ ] `cargo test -p nextkloud-backend` pass khi có `NK_TEST_DB` và `NK_TEST_S3`.
- [ ] `npx tsc --noEmit` và `npm run build` sạch.
- [ ] Mọi API admin đều kiểm tra `admin.admin`; mọi đường dẫn đều qua `clean_path` (chặn `..`, đường dẫn tuyệt đối, `\`).
- [ ] Không có secret trong diff. Commit không ghi tác giả AI.
- [ ] UI chạy được ở 390px, không cuộn ngang; ở chế độ tối vẫn đọc rõ.
- [ ] Thư mục lớn: DOM chỉ chứa khoảng vài chục dòng; không tải toàn bộ danh sách.
- [ ] Sau khi deploy: `curl http://<mini>:8011/api/health` trả 200 và log backend không có lỗi.
