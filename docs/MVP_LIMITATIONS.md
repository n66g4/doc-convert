# MVP 能力与限制说明（Should / 环境）

对应 SRS §1.8：**Should** 未达标须在发布说明列明。下列为当前工程基线下的典型限制，随版本迭代应更新。

## 1. 输入（Should 或环境依赖）

| 项 | 说明 |
| --- | --- |
| 扫描 PDF / OCR | **Should**；质量依赖 Docling/MarkItDown 及本机算力；不设通用 OCR 数值线。 |
| 图像 PNG/JPG/TIFF | **Should**；与 OCR 策略一致或「尽力而为」。 |
| HTML/XML | HTML 列为 **Should**；本版本通过 Pandoc 与路由覆盖常见用例。 |
| 旧版 Word `.doc`（97–2004） | 先转为 `.docx` 再走 Pandoc：**macOS** 使用系统 `textutil`；**Windows / Linux** 需在 `PATH` 中提供 LibreOffice（`soffice` / `libreoffice`）以完成该预处理。 |
| RapidOCR（Docling 占位图 OCR） | 首次使用会从网络拉取 ONNX；模型缓存默认写入 **用户数据目录** `…/DocConvert/cache/rapidocr/`（macOS 即 `~/Library/Application Support/DocConvert/cache/rapidocr`），不再写入应用包内 `site-packages`。启动 Core 时可用 `DOCCONVERT_RAPIDOCR_MODEL_DIR` 覆盖路径。 |

## 2. 输出（Should）

| 项 | 说明 |
| --- | --- |
| JSON/XML 结构化导出 | **Should**；可通过 Docling 插件链或后续版本增强。 |
| Word / LaTeX | **Should**；Pandoc 路径可覆盖部分场景，未单独做矩阵验收。 |

## 3. 运行时依赖

| 项 | 说明 |
| --- | --- |
| Microsoft MarkItDown（PyPI `markitdown`） | **XLSX / PPTX / PDF→Markdown** 等路径需要 **Python ≥ 3.11** 与已安装 MarkItDown；与系统 Python 3.9 等不兼容。 |
| 随包 Python | 发布介质可通过 `npm run bundle-python` 将 `python/.venv` 打入 `resources/python/`（见实施方案 P3）。 |
| Pandoc | 随包或 `fetch-pandoc`；纯文本输入在 Pandoc 中映射为 `markdown` 输入格式（见 `workers.rs`）。 |

## 4. 离线声明

默认转换在本机 Core 完成；若某插件或模型需首次下载或联网，须在用户可见说明中**逐项列明**（SRS §1.9）。
