-- Chia sẻ lại: người nhận chia sẻ tiếp cho user khác (quyền ≤ quyền của mình).
-- granted_by: người tạo share (NULL = chủ file); parent_id: share cấp quyền cho người đó.
-- Xoá share cha -> các share con (và cháu) mất theo.
ALTER TABLE shares ADD COLUMN IF NOT EXISTS granted_by UUID REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE shares ADD COLUMN IF NOT EXISTS parent_id UUID REFERENCES shares(id) ON DELETE CASCADE;
CREATE INDEX IF NOT EXISTS shares_parent ON shares (parent_id) WHERE parent_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS shares_granted_by ON shares (granted_by) WHERE granted_by IS NOT NULL;
