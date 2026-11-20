-- Mirror 1 chiều từ git repo vào thư mục của user.
CREATE TABLE IF NOT EXISTS git_repos (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    url TEXT NOT NULL,
    branch TEXT NOT NULL DEFAULT '',
    folder TEXT NOT NULL,
    ssh_key TEXT,
    ssh_pub TEXT,
    interval_h INT NOT NULL DEFAULT 8,
    -- pending (chờ thêm deploy key) | idle | syncing | error
    state TEXT NOT NULL DEFAULT 'idle',
    last_sync BIGINT,
    last_commit TEXT,
    last_error TEXT,
    files BIGINT NOT NULL DEFAULT 0,
    created_at BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS git_repos_by_user ON git_repos (user_id);
