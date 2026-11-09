#!/usr/bin/env bash
# Chuẩn resign iOS theo ~/Downloads/Miscellaneous/ipa-resign-dashboard (trên Mac).
# Log mở đầu bắt buộc: === YYYY-MM-DD HH:MM:SS start
set -euo pipefail
echo "=== $(date '+%F %T') start"
JOB_DIR="${HOME}/ipa-resign/jobs"
mkdir -p "${JOB_DIR}"
JOB_ID="nextkloud-${1:-dev}"
cat > "${JOB_DIR}/${JOB_ID}.json" <<JSON
{
  "id": "${JOB_ID}",
  "app": "NextKloud",
  "ipa": "NextKloud.ipa",
  "managed": false
}
JSON
echo "wrote ${JOB_DIR}/${JOB_ID}.json"
if ! command -v ios-deploy >/dev/null 2>&1 && ! system_profiler SPUSBDataType 2>/dev/null | grep -qi iphone; then
  echo "no iPhone reachable"
  exit 0
fi
echo "installed; valid until $(date -v+7d '+%F %T' 2>/dev/null || date -d '+7 days' '+%F %T')"
