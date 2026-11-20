-- Prefix scan (path LIKE 'a/%') cho move/xoá thư mục.
CREATE INDEX IF NOT EXISTS files_by_path_prefix ON files (user_id, path text_pattern_ops);

-- Mục trong thùng rác: object nằm ở <uuid>/.nextkloud/trash/<id>/<name>, quota vẫn tính tới khi dọn.
CREATE TABLE IF NOT EXISTS trash (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    orig_path TEXT NOT NULL,
    is_dir BOOLEAN NOT NULL,
    size BIGINT NOT NULL DEFAULT 0,
    files BIGINT NOT NULL DEFAULT 0,
    deleted_at BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS trash_by_user ON trash (user_id, deleted_at);
