-- Hồ sơ: tên hiển thị + ảnh đại diện (avatar_v = 0: chưa có; tăng mỗi lần đổi để client bỏ cache).
ALTER TABLE users ADD COLUMN IF NOT EXISTS display_name TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS avatar_v INT NOT NULL DEFAULT 0;
