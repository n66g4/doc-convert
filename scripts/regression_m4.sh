#!/usr/bin/env bash
# SRS §1.8 MVP 固定样本回归（M4）。
# 前置：Core 已监听 DOCCONVERT_BASE（默认 http://127.0.0.1:17300），已 fetch-pandoc。
#
# M4_SUITE=pandoc  仅跑 Pandoc 可覆盖的 Must/Should（TXT/RTF/DOCX/HTML，不装 MarkItDown 亦可）
# M4_SUITE=full    全量（含 XLSX/PPTX/PDF，需 Python≥3.11 且 pip install markitdown，见 docs/M4_ACCEPTANCE.md）
set -euo pipefail

BASE="${DOCCONVERT_BASE:-http://127.0.0.1:17300}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIX="${M4_FIXTURES_DIR:-$SCRIPT_DIR/../tests/fixtures/m4}"
STRICT="${M4_STRICT:-1}"
SUITE="${M4_SUITE:-full}"

if ! command -v curl >/dev/null || ! command -v jq >/dev/null; then
  echo "regression_m4: 需要 curl 与 jq" >&2
  exit 1
fi

if [[ ! -d "$FIX" ]]; then
  echo "regression_m4: 样本目录不存在: $FIX（请先运行: python3 scripts/gen_m4_fixtures.py）" >&2
  exit 1
fi

poll_task() {
  local tid=$1
  local i=0
  while [[ $i -lt 240 ]]; do
    local st
    st=$(curl -sS "${BASE}/api/v1/tasks/${tid}" | jq -r .status)
    if [[ "$st" == "completed" ]]; then
      return 0
    fi
    if [[ "$st" == "failed" ]]; then
      echo "regression_m4: 任务失败 $tid" >&2
      curl -sS "${BASE}/api/v1/tasks/${tid}" | jq . >&2
      return 1
    fi
    sleep 0.5
    i=$((i + 1))
  done
  echo "regression_m4: 任务超时 $tid" >&2
  return 1
}

convert_one() {
  local file=$1
  local out=$2
  local resp tid
  resp=$(curl -sS -X POST "${BASE}/api/v1/convert" \
    -F "file=@${FIX}/${file}" \
    -F "output_format=${out}")
  tid=$(echo "$resp" | jq -r .task_id)
  if [[ "$tid" == "null" || -z "$tid" ]]; then
    echo "regression_m4: 无效响应: $resp" >&2
    return 1
  fi
  poll_task "$tid"
}

run_case() {
  local label=$1
  local file=$2
  local out=$3
  echo "---- ${label} ----"
  if convert_one "$file" "$out"; then
    echo "OK  ${label}"
  else
    if [[ "$STRICT" == "1" ]]; then
      echo "FAIL ${label}" >&2
      exit 1
    else
      echo "SKIP ${label} (M4_STRICT=0)" >&2
    fi
  fi
}

curl -sfS "${BASE}/health" | jq -e '.status == "ok"' >/dev/null

# —— Must：纯文本 TXT / RTF ——
run_case "Must TXT → Markdown" "must_plain.txt" "markdown"
run_case "Must TXT → HTML" "must_plain.txt" "html"
run_case "Must TXT → plain" "must_plain.txt" "plain"
run_case "Must RTF → Markdown" "must_minimal.rtf" "markdown"

# —— Must：Word / Excel / PowerPoint ——
run_case "Must DOCX → Markdown" "must_word.docx" "markdown"
run_case "Must DOCX → HTML" "must_word.docx" "html"

if [[ "$SUITE" == "full" ]]; then
  # 依赖 MarkItDown（Python≥3.11）
  run_case "Must XLSX → Markdown" "must_excel.xlsx" "markdown"
  run_case "Must PPTX → Markdown" "must_slides.pptx" "markdown"
  run_case "Must PDF → Markdown" "must_pdf_text.pdf" "markdown"
else
  echo "---- M4_SUITE=${SUITE}：跳过 XLSX / PPTX / PDF（改用 M4_SUITE=full + Python3.11+ markitdown）----"
fi

# —— Should：HTML 输入（矩阵为 Should，仍纳入回归防回归）——
run_case "Should HTML → Markdown" "must_simple.html" "markdown"

echo ""
echo "regression_m4: 全部用例通过（STRICT=${STRICT}, SUITE=${SUITE}）。"
