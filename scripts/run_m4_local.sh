#!/usr/bin/env bash
# 本地一键：启动 Core → 跑 M4 回归 → 结束 Core。
# 用法：
#   bash scripts/run_m4_local.sh              # M4_SUITE=pandoc（默认，无需 MarkItDown）
#   M4_SUITE=full bash scripts/run_m4_local.sh # 全量（需 Python≥3.11 + pip install markitdown）
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ ! -f src-tauri/resources/pandoc/pandoc ]] && [[ ! -f src-tauri/resources/pandoc/pandoc.exe ]]; then
  echo "==> 拉取 Pandoc..."
  npm run fetch-pandoc
fi

# 未指定端口时选随机空闲端口，避免与本地已在跑的 :17300 Core 冲突
if [[ -z "${DOCCONVERT_BIND_PORT:-}" ]]; then
  export DOCCONVERT_BIND_PORT="$(python3 -c "import socket; s=socket.socket(); s.bind(('127.0.0.1',0)); print(s.getsockname()[1]); s.close()")"
  echo "==> 使用随机端口: ${DOCCONVERT_BIND_PORT}"
fi
export DOCCONVERT_DATA_DIR="${DOCCONVERT_DATA_DIR:-$(mktemp -d /tmp/docconvert-m4.XXXXXX)}"
if [[ -f "$ROOT/src-tauri/resources/pandoc/pandoc" ]]; then
  export DOCCONVERT_PANDOC="$ROOT/src-tauri/resources/pandoc/pandoc"
elif [[ -f "$ROOT/src-tauri/resources/pandoc/pandoc.exe" ]]; then
  export DOCCONVERT_PANDOC="$ROOT/src-tauri/resources/pandoc/pandoc.exe"
else
  echo "未找到 pandoc 二进制，请先: npm run fetch-pandoc" >&2
  exit 1
fi

echo "==> data_root: $DOCCONVERT_DATA_DIR"

cleanup() {
  if [[ -n "${CORE_PID:-}" ]]; then
    kill "$CORE_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

echo "==> 启动 Core (:${DOCCONVERT_BIND_PORT})..."
cargo run --manifest-path src-tauri/Cargo.toml --bin doc-convert-core -- --core-only &
CORE_PID=$!

for _ in $(seq 1 120); do
  if curl -sf "http://127.0.0.1:${DOCCONVERT_BIND_PORT}/health" >/dev/null 2>&1; then
    echo "==> Core 就绪（端口 ${DOCCONVERT_BIND_PORT}）。"
    break
  fi
  sleep 0.25
done

if ! curl -sf "http://127.0.0.1:${DOCCONVERT_BIND_PORT}/health" | grep -q '"status"'; then
  echo "Core 未在端口 ${DOCCONVERT_BIND_PORT} 响应 /health" >&2
  exit 1
fi

export DOCCONVERT_BASE="http://127.0.0.1:${DOCCONVERT_BIND_PORT}"
export M4_SUITE="${M4_SUITE:-pandoc}"

echo "==> M4 回归 (M4_SUITE=${M4_SUITE})..."
bash scripts/regression_m4.sh

echo "==> 完成。"
