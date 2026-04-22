#!/usr/bin/env bash
# 最小冒烟：需已有 Core 在 DOCCONVERT_BIND_PORT（默认 17300）监听。
set -euo pipefail
BASE="${DOCCONVERT_BASE:-http://127.0.0.1:17300}"
if ! curl -sfS "${BASE}/health" | grep -q '"status"'; then
  echo "smoke_core: GET ${BASE}/health failed" >&2
  exit 1
fi
echo "smoke_core: OK ${BASE}/health"
if ! curl -sfS "${BASE}/api/v1/tools/status" | grep -q '"bind_port"'; then
  echo "smoke_core: GET ${BASE}/api/v1/tools/status failed" >&2
  exit 1
fi
echo "smoke_core: OK ${BASE}/api/v1/tools/status"
