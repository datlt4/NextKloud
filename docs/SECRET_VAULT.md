# Secret Vault: thiết kế (đã làm, v0.1.13)

Mục tiêu: một khu "Bí mật" riêng cho mỗi user. Khu này mở bằng mã PIN, không hiện ở đâu khác trong app, không chia sẻ được, và được mã hoá trên đĩa. Mã hoá nhẹ để mở file nhanh.

## 1. Chống được gì, không chống được gì

| Tình huống | Có bảo vệ không |
|---|---|
| Ai đó đọc thẳng ổ RustFS, thư mục `/mnt/das_8tb/...`, hoặc bản sao lưu trên HDD | **Có**: chỉ thấy dữ liệu đã mã hoá, tên file cũng bị mã hoá |
| Mất/bị lấy ổ cứng, hoặc bản sao lưu | **Có** |
| Người khác cầm máy đang đăng nhập NextKloud của bạn | **Có**: phải nhập PIN; tự khoá sau một thời gian không dùng |
| Admin xem database / S3 | **Có**: không có PIN thì không giải mã được |
| Kẻ chiếm được server **trong lúc bạn đang mở khoá** | **Không**: lúc đó khoá giải mã nằm trong RAM server |
| Kẻ sửa code server rồi chờ bạn nhập PIN | **Không** (giới hạn chung của mã hoá phía server) |

## 2. Các lớp khoá

```
PIN (6 số) ──Argon2id(salt, 64MB, 3 vòng)──┐
                                          ├─HKDF──> KEK (khoá bọc, chỉ tồn tại lúc nhập PIN)
SECRET_PEPPER (trong .env, không nằm trong DB) ┘
                                                     │ AES-256-GCM
VK (Vault Key, 256 bit ngẫu nhiên, tạo 1 lần) <──────┘   lưu trong DB dạng đã bọc: wrapped_vk
   │ AES-256-GCM
   ├──> DEK của từng file (256 bit ngẫu nhiên): lưu ở đầu object, dạng đã bọc
   └──> tên file / thư mục: mã hoá, lưu trong DB
```

- **Vì sao có pepper:** PIN 6 số chỉ có 1 triệu khả năng. Nếu chỉ có DB/bản sao lưu, kẻ tấn công có thể thử hết trong vài phút. Pepper nằm ở `.env` của server, **không có trong DB lẫn bản sao lưu**, nên lấy được ổ đĩa/bản sao lưu vẫn không thử PIN offline được.
- **Thử PIN online** qua web: sai 5 lần khoá 15 phút, sai tiếp thì thời gian khoá tăng gấp đôi.
- **Đổi PIN:** chỉ bọc lại VK (vài mili giây), không phải mã hoá lại file nào.
- **Mã khôi phục:** lúc tạo vault, server hiện 1 lần một chuỗi 24 ký tự. Chuỗi này là cách thứ hai để mở VK, dùng khi quên PIN. Mã đủ mạnh (~120 bit) nên **không trộn pepper**: mất `.env` thì vẫn mở được bằng mã khôi phục rồi đặt PIN mới. Quên cả PIN lẫn mã khôi phục thì **dữ liệu mất vĩnh viễn**, admin cũng không cứu được (đây là chủ đích).
- **Nên sao lưu `SECRET_PEPPER`** (password manager / giấy). Mất pepper thì PIN không dùng được nữa, phải mở bằng mã khôi phục.

## 3. Mã hoá file (nhanh, mở được giữa file)

- **AES-256-GCM:** CPU của mini PC có AES-NI. Tốc độ ước tính vài GB/giây mỗi core (chưa đo), nhanh hơn nhiều so với tốc độ đọc ổ và mạng.
- **Cắt file thành khối 64KB**, mỗi khối mã hoá riêng: nonce = 8 byte ngẫu nhiên của file + số thứ tự khối. Mỗi khối thêm 16 byte tag, tốn thêm khoảng 0,025% dung lượng.
- **Đọc từ giữa file được:** khối thứ i nằm ở vị trí cố định, nên tua video, xem PDF trang 300, hay tải tiếp sau khi đứt mạng chỉ cần giải mã vài khối cần thiết.
- **Object trên S3:** `<uuid>/.secret/<id ngẫu nhiên>`, đầu object là header gồm phiên bản, DEK đã bọc và nonce gốc. Tên file và cấu trúc thư mục không nằm trong key S3.

## 4. Upload, download, xem file

- **Upload:** server nhận từng chunk 8MB, mã hoá **ngay khi nhận** rồi mới ghi ra đĩa staging. Nhờ vậy bản rõ không bao giờ nằm trên đĩa, kể cả file tạm. 8MB là bội số của 64KB nên các khối khớp nhau.
- **Download / xem** (ảnh, PDF, video, markdown + TTS): server đọc các khối cần, giải mã rồi stream ra. Trình duyệt không cần làm gì thêm, nên A8 Star cũng mượt.
- **Thumbnail:** tạo lúc xem, không lưu cache (cache sẽ là bản rõ). Có thể thêm cache mã hoá sau nếu cần.
- **Office:** chỉ xem. DocumentServer tải bản rõ qua vé 10 phút (không phải token mở khoá); bản xem có thể nằm trong cache của DocumentServer.
- **Tắt cho file bí mật:** chia sẻ, mirror git, sửa bằng OnlyOffice, bookmark/ghi chú, tìm kiếm toàn app.

## 5. Mở khoá và tự khoá

- **Nhập PIN đúng:** server giữ VK trong RAM, gắn với 1 "phiên mở khoá" có token riêng. Token này nằm trong `sessionStorage`, nên đóng tab là mất.
- **Tự khoá khi:**
  - không dùng 10 phút;
  - bấm "Khoá";
  - đăng xuất;
  - server khởi động lại.

  Khi khoá, VK bị xoá khỏi RAM.
- **Mỗi request vào khu bí mật** phải có cả token đăng nhập lẫn token mở khoá.

## 6. Giao diện

- **Thanh bên:** thêm mục "Bí mật" (icon ổ khoá).
  - Lần đầu: đặt PIN, nhập lại, rồi hiện mã khôi phục (có nút chép và ô tick "Đã lưu mã").
  - Các lần sau: bàn phím PIN 6 số, to, bấm dễ trên mobile.
- **Bên trong:** dùng cùng trình duyệt file như bình thường (tải lên, thư mục mới, đổi tên, di chuyển, xoá, xem), không có nút chia sẻ. Thanh trên có nút **Khoá** và đồng hồ đếm ngược tới lúc tự khoá.
- **Cài đặt:** đổi PIN, tạo lại mã khôi phục (cần PIN), xoá vault (gõ xác nhận).

## 7. Dữ liệu (dự kiến)

```sql
secret_vaults(user_id pk, salt, wrapped_vk, recovery_salt, wrapped_vk_recovery, failed, locked_until, created_at)
secret_files(id, user_id, parent_id null, name_enc, is_dir, size, mtime, object_key, created_at)
```

Dung lượng file bí mật vẫn tính vào quota như file thường.

## 8. Phương án khác: mã hoá ngay trên trình duyệt (E2E)

Server không bao giờ thấy bản rõ, an toàn hơn trường hợp server bị chiếm. Đổi lại:

- mã hoá/giải mã bằng JS trên A8 Star chậm hơn;
- viewer PDF/video/thumbnail và TTS phải viết lại để tự giải mã;
- desktop/mobile đều phải có code mã hoá riêng.

**Đề xuất làm phương án phía server (mục 2–5) trước.** Định dạng file vẫn có thể đổi sang E2E sau, vì header object có số phiên bản.

## 9. Cần người dùng chốt

1. PIN **6 số** hay cho phép mật khẩu chữ?
2. Thời gian tự khoá: **10 phút** không dùng?
3. Xoá file bí mật: vào **thùng rác riêng** của vault (cũng mã hoá), hay xoá hẳn luôn?
4. Có cho xem trước (ảnh, PDF, video) trong vault không, hay chỉ tải về?
5. Có đồng ý **tự giữ `SECRET_PEPPER` + mã khôi phục** (mất cả hai là mất dữ liệu)?
