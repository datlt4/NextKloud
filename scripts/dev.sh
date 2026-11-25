#!/usr/bin/env bash
# Dev server local: backend + web dist mới build, dùng DB/S3 test.
# Không có secret thật — toàn bộ credential chỉ dùng cho test local.
# Yêu cầu: nk-pg-test (15432), nk-rustfs-dev (19000) đang chạy, bucket nktest đã tạo.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export BIND=0.0.0.0:18014
export DATABASE_URL=postgres://nk:nk@127.0.0.1:15432/nk
export STORAGE_DRIVER=s3
export S3_ENDPOINT=http://127.0.0.1:19000
export S3_BUCKET=nktest
export S3_ACCESS_KEY=nktest
export S3_SECRET_KEY=nktest-secret-123
export STAGING_DIR=/tmp/nk-staging
export NEXTKLOUD_WEB="$ROOT/apps/desktop/dist"
# bind 0.0.0.0 (LAN) nên JWT secret không được công khai: ngẫu nhiên mỗi lần chạy, hoặc đặt trong dev.local.env
export JWT_SECRET="${JWT_SECRET:-$(head -c 24 /dev/urandom | base64)}"
# secret that (OnlyOffice...): doc tu scripts/dev.local.env (gitignore), khong commit
if [ -f "$ROOT/scripts/dev.local.env" ]; then
  set -a; . "$ROOT/scripts/dev.local.env"; set +a
fi
mkdir -p /tmp/nk-staging
exec "$ROOT/target/debug/nextkloud"
