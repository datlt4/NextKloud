-- Kanban: board -> cột -> card (card có thể là subtask của 1 card khác cùng board).
CREATE TABLE boards (
    id UUID PRIMARY KEY,
    owner UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    -- [{"title": "...", "color": "#hex"}]
    labels JSONB NOT NULL DEFAULT '[]',
    archived BOOLEAN NOT NULL DEFAULT FALSE,
    position DOUBLE PRECISION NOT NULL DEFAULT 0,
    -- nguồn nhập (vd "deck:3"), chặn nhập trùng
    source TEXT,
    created_at BIGINT NOT NULL
);
CREATE INDEX boards_owner ON boards (owner);

CREATE TABLE board_columns (
    id UUID PRIMARY KEY,
    board_id UUID NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    position DOUBLE PRECISION NOT NULL,
    -- cột "xong": card vào đây được ghi done_at, hiện gọn N card mới nhất
    is_done BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX board_columns_board ON board_columns (board_id);

CREATE TABLE cards (
    id UUID PRIMARY KEY,
    board_id UUID NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
    column_id UUID NOT NULL REFERENCES board_columns(id) ON DELETE CASCADE,
    parent_id UUID REFERENCES cards(id) ON DELETE SET NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    position DOUBLE PRECISION NOT NULL,
    labels TEXT[] NOT NULL DEFAULT '{}',
    color TEXT,
    due_at BIGINT,
    done_at BIGINT,
    archived BOOLEAN NOT NULL DEFAULT FALSE,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);
CREATE INDEX cards_column ON cards (column_id, position);
CREATE INDEX cards_column_done ON cards (column_id, done_at DESC);
CREATE INDEX cards_parent ON cards (parent_id);
CREATE INDEX cards_board ON cards (board_id);
