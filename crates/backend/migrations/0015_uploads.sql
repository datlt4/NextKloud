-- Upload dở dang lưu ở Postgres (thay vì chỉ RAM) để resume sau rớt mạng/F5/restart server.
CREATE TABLE IF NOT EXISTS uploads (
    id TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    owner UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    size BIGINT NOT NULL,
    mtime BIGINT,
    received BIGINT NOT NULL DEFAULT 0,
    created_at BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS uploads_lookup ON uploads (owner, path, size);
