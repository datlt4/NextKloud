-- Bookmark + ghi chú của user trên 1 file (markdown/pdf). Đi theo file khi đổi tên/di chuyển/vào thùng rác.
CREATE TABLE IF NOT EXISTS annotations (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    kind TEXT NOT NULL,              -- bookmark | note
    anchor TEXT NOT NULL DEFAULT '', -- md: "b<chỉ số block>", pdf: "p<số trang>"
    quote TEXT NOT NULL DEFAULT '',  -- đoạn chữ được chọn, dùng để tìm lại vị trí khi file đổi
    body TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'todo', -- note: todo | doing | done
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS annotations_by_path ON annotations (user_id, path text_pattern_ops);
CREATE INDEX IF NOT EXISTS annotations_by_state ON annotations (user_id, kind, state);
