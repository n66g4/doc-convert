#!/usr/bin/env bash
# 创建 python/.venv 并安装 MarkItDown、Docling（需 Python 3.11+）
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/python"

if command -v python3.11 &>/dev/null; then
  PY=python3.11
elif command -v python3.12 &>/dev/null; then
  PY=python3.12
else
  PY=python3
fi

echo "Using: $($PY --version 2>&1)"

"$PY" -m venv .venv
# shellcheck disable=SC1091
source .venv/bin/activate
python -m pip install -U pip
python -m pip install -r requirements.txt
python -m pip install -e ".[all]"

echo ""
echo "Done. Activate with: source python/.venv/bin/activate"
echo "Or run Core with: export DOCCONVERT_PYTHON=\"$ROOT/python/.venv/bin/python\""
