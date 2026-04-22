# M4 里程碑验收说明（SRS §1.8 MVP）

## 1. 范围

对应《实施方案》**M4：MVP 格式**——在版本受控样本上完成 **Must** 能力矩阵的端到端验证；**Should** 项未达标时须在「限制说明」中列明（见同目录 `MVP_LIMITATIONS.md`）。

## 2. 固定样本库

路径：`tests/fixtures/m4/`。生成方式：

```bash
python3 scripts/gen_m4_fixtures.py
```

| 文件 | 覆盖 SRS |
| --- | --- |
| `must_plain.txt` | Must：纯文本 TXT |
| `must_minimal.rtf` | Must：RTF |
| `must_word.docx` | Must：Word |
| `must_excel.xlsx` | Must：Excel |
| `must_slides.pptx` | Must：PowerPoint |
| `must_pdf_text.pdf` | Must：文本型 PDF |
| `must_simple.html` | Should：HTML 输入（防回归） |

## 3. 自动化回归

前置：已构建 Core、`npm run fetch-pandoc`、本机 `curl`/`jq`；Core 已监听（默认 `http://127.0.0.1:17300`）。

### 3.1 Pandoc 子集（推荐用于 CI / 无 Python 3.11 环境）

覆盖 **Must** 中可由 **Pandoc** 独立完成的路径（TXT/RTF/DOCX + Must 输出 HTML/plain + Should HTML），**不依赖** Microsoft MarkItDown 的 PyPI 包：

```bash
export DOCCONVERT_PANDOC="$PWD/src-tauri/resources/pandoc/pandoc"
export DOCCONVERT_BIND_PORT=17300
# 启动 Core 后：
M4_SUITE=pandoc bash scripts/regression_m4.sh
```

npm：`npm run regression:m4:pandoc`（需自行先启动 Core，与 `scripts/dev_stack.sh` 或 `cargo run ... --core-only` 配合）。

### 3.2 全量（Must 含 XLSX / PPTX / PDF）

需 **Python ≥ 3.11**（MarkItDown 官方包要求），并安装：

```bash
python3.12 -m venv .venv-m4
.venv-m4/bin/pip install -U pip markitdown
export DOCCONVERT_PYTHON="$PWD/.venv-m4/bin/python"
```

然后启动 Core，执行：

```bash
M4_SUITE=full bash scripts/regression_m4.sh
```

npm：`npm run regression:m4`（全量；同样需先启动 Core）。

## 4. M4 关闭口径

- **最低发布线**：`M4_SUITE=pandoc` 回归通过，且 `MVP_LIMITATIONS.md` 中如实写明对 **XLSX/PPTX/PDF** 等路径的依赖（Python 版本、随包 venv、`npm run bundle-python`）。
- **完整 Must 矩阵自动化**：在同一发布线基础上，`M4_SUITE=full` 在 CI 或发布流水线上通过。

## 5. 追溯

- 路由：`config/routes.toml`（Pandoc 与 MarkItDown 平局消解）。
- 实现：`src-tauri/src/workers.rs`（Pandoc `--from`/`--to` 对 `plain` 输入/输出的区分）。

## 6. 与 M6 RC 的关系

M4 关闭是 **M6 发布候选**的前提之一；完整验收条目与文档索引见 [M6_RC_CHECKLIST.md](./M6_RC_CHECKLIST.md)。
