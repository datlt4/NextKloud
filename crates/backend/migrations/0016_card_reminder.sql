-- Nhắc task đến hạn: đánh dấu đã nhắc để không spam mỗi phút.
ALTER TABLE cards ADD COLUMN reminded_at BIGINT;
