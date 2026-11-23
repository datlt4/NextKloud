-- Task tracker: lọc card theo due_at (không cần bảng mới, dùng cards).
-- Habit tracker: thói quen + log theo ngày.
CREATE TABLE IF NOT EXISTS habits (
    id UUID PRIMARY KEY,
    owner UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    -- v1 chỉ hỗ trợ 'daily'; giữ cột để sau này thêm 'weekly'
    schedule TEXT NOT NULL DEFAULT 'daily',
    archived BOOLEAN NOT NULL DEFAULT FALSE,
    created_at BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS habits_owner ON habits (owner, archived);

CREATE TABLE IF NOT EXISTS habit_logs (
    habit_id UUID NOT NULL REFERENCES habits(id) ON DELETE CASCADE,
    -- ngày YYYY-MM-DD (ngày địa phương của client)
    day TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (habit_id, day)
);

-- Lọc task theo hạn: card có due_at của user.
CREATE INDEX IF NOT EXISTS cards_due ON cards (board_id, due_at) WHERE due_at IS NOT NULL;
