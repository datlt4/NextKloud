# NextKloud: kế hoạch hoàn thiện

Bản để người dùng review trước khi làm. Chi tiết kỹ thuật và tiêu chí nghiệm thu từng mục nằm ở `HANDOFF.md` mục 9; file này chốt **thứ tự, phạm vi và các câu hỏi cần trả lời**.

Ước lượng tính theo "đợt" (1 đợt ≈ 1 lần giao: code + test + deploy). Đây là ước lượng, chưa đo.

## 0. Hiện trạng (22/11/2026)

Đã chạy trên mini PC:

- File: chỉ mục Postgres (1 triệu file vẫn mượt), danh sách ảo, tìm kiếm, giữ mtime gốc. Thư mục mới, đổi tên, di chuyển, chọn nhiều, kéo thả, thùng rác (khôi phục, hoàn tác, tự dọn).
- Viewer: ảnh, video/audio, PDF, zip, epub, drawio, Excalidraw, Office (OnlyOffice), text/code. Thumbnail ảnh/video.
- Obsidian: wikilink, ảnh nhúng, TTS từng đoạn + đọc cả bài, Edge TTS có Google dự phòng, mirror git cho vault.
- Tài khoản: admin quản lý user và quota, hồ sơ, avatar, đổi mật khẩu, giao diện và ảnh nền.

## 1. Việc gấp (P0): 1 đợt

| # | Việc | Lý do |
|---|---|---|
| 1.1 | **Tải file lớn không qua RAM**: link tải trực tiếp có token ngắn hạn, thay cho blob | Hiện tải file vài GB làm sập tab, nhất là trên A8 Star |
| 1.2 | **Quota khi upload/xoá cộng trừ phần chênh**, không ghi đè giá trị (`file_handlers.rs:274`, `index.rs:231` giữ nguyên vì rebuild cần ghi tuyệt đối) | Upload song song đang làm lệch dung lượng |
| 1.3 | **File `.whiteboard`**: mở bằng Excalidraw, đọc/ghi đúng định dạng Nextcloud Whiteboard | Xem ghi chú bên dưới |
| 1.4 | Kiểm tra lại trên màn thật các việc Muse Spark đã làm: nút loa ngang dòng đầu, icon mới, Google TTS dự phòng | Người dùng đã yêu cầu, chưa ai đối chiếu trên vault thật |

**Ghi chú về `viznav.whiteboard`:** file này **rỗng (0 byte) ngay trong Nextcloud cũ** (`data/datlt4/files/viznav.whiteboard`). Whiteboard 2.1.2 giữ nội dung trên server cộng tác của nó, không ghi vào file, mà trên mini PC không có server đó. Nên không có dữ liệu để mở; NextKloud chỉ mở được thành bảng trắng. Nếu bản vẽ quan trọng: mở lại trong Nextcloud cũ, export ra `.excalidraw`.

## 2. Upload và download lớn (P1): 2–3 đợt

Theo `HANDOFF.md` 9.2:

- Kéo thả cả thư mục; hàng đợi upload song song; resume sau khi rớt mạng hoặc tải lại trang (lưu trạng thái upload vào Postgres thay vì RAM).
- API upload hàng loạt file nhỏ (mục tiêu ≥ 300 file/giây qua LAN).
- Tải thư mục thành zip stream (không file tạm).
- Panel hàng đợi: tốc độ, ETA, tạm dừng, thử lại.
- Job nền có tiến độ cho đổi tên/di chuyển thư mục rất lớn (còn nợ từ 9.1).

**Nghiệm thu:** 100.000 file nhỏ không treo tab; file 50GB rút mạng giữa chừng vẫn chạy tiếp; tải zip thư mục 15GB, RAM backend không tăng.

## 3. Obsidian hoàn thiện (P1): 2 đợt

- **Hiển thị:** KaTeX (công thức), callout `> [!note]`, tag, frontmatter, checkbox, embed note `![[note]]` hiện nội dung thay vì link, backlinks.
- **Sửa:** chế độ sửa + xem trước, lưu ngược về file. Với vault mirror từ git, khi sửa thì cảnh báo "sẽ bị ghi đè ở lần sync sau" (hoặc làm git 2 chiều, xem câu hỏi Q3).
- **TTS:**
  - Đã có cache theo hash (giọng + tốc độ + nội dung) ở `/mnt/das_8tb/nextkloud_volume/tts` trên mini PC (hiện 2,1MB): bấm lần 2 không gọi Edge/Google nữa, giữ qua các lần deploy.
  - Cần thêm: giới hạn dung lượng (LRU, ví dụ 5GB), nút "tạo sẵn giọng cả bài" chạy nền.
  - **Ước lượng dung lượng:** mp3 Edge khoảng 48 kbps ≈ 360KB/phút đọc. Một ghi chú 1.000 từ ≈ 6–7 phút ≈ 2,5MB. 1.000 ghi chú như vậy ≈ 2,5GB. Đề xuất **giữ cache ở server, không ghi vào `.nextkloud` trong vault** (tránh lẫn vào repo git và không tính vào quota).
- **Tìm kiếm nội dung** (không chỉ tên file) trong vault: chỉ mục full-text Postgres cho file `.md`.

## 4. Kanban, Task, Habit + nhập Deck (P2): 3 đợt

Theo `HANDOFF.md` 9.4: board/cột/card có subtask, card xong không biến mất mà thu gọn ("Đã xong cũ hơn (127)"), nhập 399 card từ backup Deck với màn duyệt cách gom subtask trước khi ghi. Task tracker theo hạn; Habit tracker dạng lưới có streak.

## 5. Chia sẻ và lịch sử phiên bản (P2): 2 đợt, cần chốt phạm vi

- Chia sẻ file/thư mục giữa user; link công khai có mật khẩu và hạn.
- Lịch sử phiên bản khi ghi đè; migrate thùng rác (854MB) và versions (1,7GB) từ Nextcloud.

## 6. Ứng dụng desktop và mobile (P3): 4+ đợt

- Desktop (Tauri): đồng bộ thư mục 2 chiều, xử lý xung đột, "giải phóng dung lượng" bản đơn giản (chỉ giữ metadata, mở thì tải).
- Mobile: duyệt, upload, download, Kanban, nghe TTS; không đồng bộ.
- Auto-update qua GitHub release; thông báo (upload xong, card đến hạn) qua SSE.

## 7. Đa ngôn ngữ (P3): 1 đợt

Tách toàn bộ chuỗi tiếng Việt ra file dịch (vi, en, ja, zh), lưu lựa chọn theo tài khoản.

## 8. Vận hành và an toàn (làm xen giữa các đợt)

- **Sao lưu:** Postgres (dump hằng đêm) + RustFS (rclone sang ổ khác). Hiện **chưa có backup**.
- CI chạy cả test DB và S3 (thêm service container).
- Giới hạn số lần đăng nhập sai; API admin kiểm `is_admin` từ DB thay vì token; thu hẹp CORS.
- Dọn file staging và multipart S3 bỏ dở.
- Đo hiệu năng trên A8 Star (bundle, thời gian mở thư mục, cuộn).

## Thứ tự đề xuất

1. P0 (mục 1) → 2. Upload/download lớn → 3. Obsidian → 4. Kanban + nhập Deck → 5. Sao lưu (nên chen sớm, ngay sau P0) → 6. Chia sẻ/versions → 7. Desktop/mobile → 8. i18n.

## Câu hỏi cần người dùng trả lời

- **Q1.** Thứ tự trên được chưa? Nhất là: làm Kanban trước hay upload/download lớn trước?
- **Q2.** Cột "Done2" trong Deck nằm ở board nào? (backup không có cột tên này.)
- **Q3.** Vault Obsidian: chỉ đọc/nghe trên NextKloud (giữ mirror 1 chiều), hay muốn sửa trên NextKloud rồi **đẩy ngược lên git**?
- **Q4.** Chia sẻ: có cần link công khai cho người ngoài không, hay chỉ chia sẻ giữa các user?
- **Q5.** Sao lưu ra đâu: ổ khác trên mini PC, hay máy/cloud khác?
- **Q6.** Có cần migrate thùng rác (854MB) và versions (1,7GB) từ Nextcloud không?
- **Q7.** Bản vẽ `viznav` có quan trọng không? (cần export từ Nextcloud cũ.)
