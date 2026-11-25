#!/bin/sh
# Sao lưu NextKloud ra ổ khác: Postgres dump + toàn bộ file (dạng file thường, đọc được không cần RustFS).
# Mỗi bản là 1 thư mục <ngày_giờ>/ đầy đủ; file không đổi dùng chung qua hardlink nên bản mới chỉ tốn phần thay đổi.
# Giữ BACKUP_KEEP bản hoàn chỉnh gần nhất (mặc định 2). Bản đang chạy nằm ở .tmp-*, lỗi thì bỏ, không đụng bản cũ.
# Chạy tay 1 lần: docker compose run --rm backup now
set -eu
DEST=/backup
KEEP=${BACKUP_KEEP:-2}
AT=${BACKUP_AT:-03:00}
command -v pg_dump >/dev/null || apk add --no-cache postgresql16-client coreutils tzdata >/dev/null

log() { echo "$(date '+%F %T') $*" | tee -a "$DEST/backup.log"; }

run() {
  ts=$(date +%Y-%m-%d_%H%M)
  new="$DEST/.tmp-$ts"
  rm -rf "$DEST"/.tmp-*
  last=$(ls -1d "$DEST"/20* 2>/dev/null | tail -n 1 || true)
  mkdir -p "$new"
  log "bắt đầu $ts (nền: ${last:-không có})"
  # Hardlink từ bản trước; rclone ghi file đổi qua file tạm rồi rename nên bản cũ không bị sửa theo.
  if [ -n "$last" ] && [ -d "$last/files" ]; then cp -al "$last/files" "$new/files"; fi
  # multipart S3 bỏ dở (backend chết giữa chừng) > 24h: huỷ để không chiếm chỗ; lỗi thì bỏ qua
  rclone backend cleanup "nk:${BACKUP_BUCKET:-nextkloud}" -o max-age=24h --log-level ERROR || log "cảnh báo: dọn multipart lỗi"
  pg_dump -h "${PGHOST:-db}" -Fc > "$new/db.dump" # PGUSER/PGPASSWORD/PGDATABASE lấy từ env
  rclone sync "nk:${BACKUP_BUCKET:-nextkloud}" "$new/files" --exclude "*/.nextkloud/thumbs/**" --transfers 8 --log-level ERROR
  chown -R 1000:1000 "$new"
  mv "$new" "$DEST/$ts"
  # Bản mới đã hoàn chỉnh mới xoá bớt bản cũ.
  ls -1d "$DEST"/20* | head -n -"$KEEP" | while read -r old; do rm -rf "$old"; log "xoá bản cũ $old"; done
  log "xong $ts: $(du -sh "$DEST/$ts" | cut -f1) (dùng chung hardlink), tổng $(du -sh "$DEST" | cut -f1)"
  chown 1000:1000 "$DEST/backup.log"
}

mkdir -p "$DEST"
if [ "${1:-}" = "now" ]; then
  run
  exit 0
fi
while :; do
  now=$(date +%s)
  next=$(date -d "$(date +%F) $AT" +%s)
  [ "$next" -le "$now" ] && next=$((next + 86400))
  sleep $((next - now))
  run || { log "LỖI, giữ nguyên các bản cũ"; rm -rf "$DEST"/.tmp-*; }
done
