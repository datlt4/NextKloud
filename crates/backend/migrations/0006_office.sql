CREATE TABLE IF NOT EXISTS office_docs (
    key TEXT PRIMARY KEY,
    user_id UUID NOT NULL,
    path TEXT NOT NULL,
    created_at BIGINT NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS office_docs_user ON office_docs (user_id);
