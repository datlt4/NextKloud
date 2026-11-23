-- Cột "huỷ" (vd Delete của Deck): card cũ thu gọn như cột xong, subtask của task trong cột này hiện là đã huỷ.
ALTER TABLE board_columns ADD COLUMN is_cancel BOOLEAN NOT NULL DEFAULT FALSE;
UPDATE board_columns SET is_cancel = TRUE
 WHERE lower(trim(title)) IN ('delete', 'deleted', 'cancel', 'cancelled', 'canceled', 'huỷ', 'hủy', 'đã huỷ', 'đã hủy', 'xoá', 'xóa');

-- Xem trước link (tiêu đề, ảnh, tên trang) lấy từ thẻ og: của trang, lưu lại để khỏi tải lại.
CREATE TABLE link_previews (
    url TEXT PRIMARY KEY,
    title TEXT,
    image TEXT,
    site TEXT,
    fetched_at BIGINT NOT NULL
);
