-- Chia sẻ file/thư mục: giữa các user (view|edit) + link công khai (token, mật khẩu, hạn).
-- File trong Bí mật nằm dưới .nextkloud nên user_path đã chặn, không chia sẻ được.
CREATE TABLE IF NOT EXISTS shares (
    id UUID PRIMARY KEY,
    owner UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    is_dir BOOLEAN NOT NULL DEFAULT FALSE,
    -- chia sẻ cho user cụ thể (NULL khi là link công khai)
    target_user UUID REFERENCES users(id) ON DELETE CASCADE,
    -- link công khai (NULL khi chia sẻ cho user cụ thể)
    token TEXT UNIQUE,
    pass_hash TEXT,
    permission TEXT NOT NULL DEFAULT 'view',
    expires_at BIGINT,
    created_at BIGINT NOT NULL,
    CONSTRAINT shares_kind CHECK (
        (target_user IS NOT NULL AND token IS NULL) OR
        (target_user IS NULL AND token IS NOT NULL)
    ),
    CONSTRAINT shares_perm CHECK (permission IN ('view', 'edit'))
);
CREATE INDEX IF NOT EXISTS shares_owner ON shares (owner);
CREATE INDEX IF NOT EXISTS shares_target ON shares (owner, target_user) WHERE target_user IS NOT NULL;
