-- Ghim thư mục ở sidebar (đồng bộ đa thiết bị, thay localStorage nk-bookmarks).
CREATE TABLE IF NOT EXISTS bookmarks (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (user_id, path)
);
