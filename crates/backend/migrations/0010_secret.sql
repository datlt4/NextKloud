-- Secret Vault: khoá vault bọc bằng PIN (+ pepper) và bằng mã khôi phục; tên file mã hoá.
CREATE TABLE secret_vaults (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    salt BYTEA NOT NULL,
    wrapped_vk BYTEA NOT NULL,
    rec_salt BYTEA NOT NULL,
    wrapped_vk_rec BYTEA NOT NULL,
    failed INT NOT NULL DEFAULT 0,
    lockouts INT NOT NULL DEFAULT 0,
    locked_until BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE secret_files (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    parent_id UUID REFERENCES secret_files(id) ON DELETE CASCADE,
    name_enc BYTEA NOT NULL,
    name_mac BYTEA NOT NULL,
    is_dir BOOLEAN NOT NULL,
    size BIGINT NOT NULL DEFAULT 0,
    mtime BIGINT NOT NULL,
    object_key TEXT,
    stored BIGINT NOT NULL DEFAULT 0,
    trashed_at BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- Tên không trùng trong cùng thư mục (so bằng HMAC của tên), không tính mục trong thùng rác.
CREATE UNIQUE INDEX secret_files_name ON secret_files (user_id, COALESCE(parent_id, '00000000-0000-0000-0000-000000000000'::uuid), name_mac)
    WHERE trashed_at IS NULL;
CREATE INDEX secret_files_parent ON secret_files (user_id, parent_id);
CREATE INDEX secret_files_trash ON secret_files (trashed_at) WHERE trashed_at IS NOT NULL;
