-- Chỉ mục metadata file: list/phân trang/tìm kiếm không phải hỏi S3 (S3 list tối đa 1000/lần).
CREATE EXTENSION IF NOT EXISTS pg_trgm;
-- Sắp xếp tự nhiên: "file 2" < "file 10".
CREATE COLLATION IF NOT EXISTS nk_natural (provider = icu, locale = 'und-u-kn-true');

CREATE TABLE IF NOT EXISTS files (
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    parent TEXT NOT NULL,
    name TEXT COLLATE nk_natural NOT NULL,
    is_dir BOOLEAN NOT NULL,
    size BIGINT NOT NULL DEFAULT 0,
    modified BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (user_id, path)
);
CREATE INDEX IF NOT EXISTS files_by_name ON files (user_id, parent, is_dir, name);
CREATE INDEX IF NOT EXISTS files_by_size ON files (user_id, parent, is_dir, size, name);
CREATE INDEX IF NOT EXISTS files_by_modified ON files (user_id, parent, is_dir, modified, name);
CREATE INDEX IF NOT EXISTS files_name_trgm ON files USING gin (name gin_trgm_ops);
