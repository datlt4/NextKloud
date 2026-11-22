-- Nội dung chữ của file .md/.txt (<= 2MB) để tìm theo nội dung. Đi theo file khi move, xoá khi purge.
CREATE TABLE IF NOT EXISTS file_text (
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    body TEXT NOT NULL,
    PRIMARY KEY (user_id, path)
);
CREATE INDEX IF NOT EXISTS file_text_trgm ON file_text USING gin (body gin_trgm_ops);
CREATE INDEX IF NOT EXISTS file_text_by_path ON file_text (user_id, path text_pattern_ops);
