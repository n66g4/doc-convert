#!/usr/bin/env bash
# 一键开发：拉取 Pandoc（若缺失）→ 后台启动 Core:17300 → 启动 Vite。
# Python 插件依赖请先执行: ./scripts/setup_python.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ ! -f src-tauri/resources/pandoc/pandoc ]] && [[ ! -f src-tauri/resources/pandoc/pandoc.exe ]]; then
  echo "==> Fetching Pandoc..."
  npm run fetch-pandoc
fi

echo "==> Starting Core on :17300 (background)..."
npm run dev:core &
CORE_PID=$!
cleanup() { kill "$CORE_PID" 2>/dev/null || true; }
trap cleanup EXIT INT TERM

for _ in $(seq 1 80); do
  if curl -sf "http://127.0.0.1:17300/health" >/dev/null 2>&1; then
    echo "==> Core ready."
    break
  fi
  sleep 0.15
done

echo "==> Starting Vite..."
npm run dev
