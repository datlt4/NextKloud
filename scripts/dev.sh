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
export JWT_SECRET=localtest
mkdir -p /tmp/nk-staging
exec "$ROOT/target/debug/nextkloud"
