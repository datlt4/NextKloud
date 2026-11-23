#!/usr/bin/env bash
# Bump the version everywhere: apps/desktop/package.json, Tauri config + shell, backend crates.
# Usage: scripts/set-version.sh 0.2.0   |   scripts/set-version.sh patch
set -euo pipefail
V="${1:?usage: set-version.sh <version>|patch}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# "patch": tăng số cuối của version hiện tại (mỗi commit lên main đều bump, xem HANDOFF mục 2)
if [ "$V" = patch ]; then
  V="$(node -p "const v=require('$ROOT/apps/desktop/package.json').version.split('.');v[2]=+v[2]+1;v.join('.')")"
fi
node -e "
const fs = require('fs');
for (const f of ['apps/desktop/package.json', 'apps/desktop/src-tauri/tauri.conf.json']) {
  const p = '$ROOT/' + f;
  const j = JSON.parse(fs.readFileSync(p, 'utf8'));
  j.version = '$V';
  fs.writeFileSync(p, JSON.stringify(j, null, 2) + '\n');
}"
sed -i "s/^version = \".*\"/version = \"$V\"/" "$ROOT/apps/desktop/src-tauri/Cargo.toml" "$ROOT/crates/backend/Cargo.toml" "$ROOT/crates/migrate/Cargo.toml"
# Keep --locked CI green: version bumps invalidate Cargo.lock.
if command -v cargo >/dev/null; then
  (cd "$ROOT" && cargo update -w >/dev/null 2>&1 || true)
  (cd "$ROOT/apps/desktop/src-tauri" && cargo update >/dev/null 2>&1 || true)
fi
echo "version -> $V"
