# DocConvert（文档转换工具）

Tauri 2 桌面壳 + Rust Core（Axum）+ React 前端；插件化路由（MarkItDown / Docling / Pandoc）。

## 环境要求

- **Rust**（stable）、**Node 20+**、**Python 3.11+**（完整格式/MarkItDown 路径推荐；仅 Pandoc 路径可用系统 Python）
- 首次构建前：`npm run fetch-pandoc`（或依赖 `beforeBuildCommand` 自动执行）

## 常用命令

| 命令 | 说明 |
| --- | --- |
| `npm run dev:stack` | 拉取 Pandoc（若缺）→ 后台 Core :17300 → Vite |
| `npm run dev:core` | 仅 Core（`DOCCONVERT_BIND_PORT=17300`） |
| `npm run dev` | 仅前端（需 Core 已由上一步启动） |
| `npm run build` | 前端生产构建 |
| `npm run tauri:dev` | Tauri 开发模式 |
| `npm run regression:m4:pandoc` | M4 回归（Pandoc 子集，**需先自行启动 Core**） |
| `npm run regression:m4:local` | 本地一键：随机端口起 Core → `M4_SUITE=pandoc` 回归 → 退出（避免占用 :17300） |

## 文档（仓库内）

| 文档 | 说明 |
| --- | --- |
| `docs/M6_RC_CHECKLIST.md` | **M6 RC**：SRS §10.2 验收核对与证据索引 |
| `docs/API.md` | Core HTTP 接口说明 |
| `docs/openapi.yaml` | 同上（OpenAPI 3.0，可导入 Swagger/Redoc） |
| `docs/CONFIGURATION.md` | 环境变量、数据目录、运维与路由 |
| `docs/SECURITY_MVP.md` | SEC-001（MVP）达成声明与边界 |
| `docs/PERFORMANCE_AND_TESTING.md` | 性能与测试矩阵口径（NFR / TEST-004） |
| `docs/KNOWN_ISSUES.md` | 已知问题模板（发布时填写） |
| `docs/M4_ACCEPTANCE.md` | M4 验收与全量回归环境 |
| `docs/MVP_LIMITATIONS.md` | Should 项与运行时限制 |

根目录另含 SRS、架构设计说明书、实施方案（产品/研发评审基线）。

## 样本与回归

```bash
python3 scripts/gen_m4_fixtures.py   # 生成 tests/fixtures/m4/
bash scripts/run_m4_local.sh           # 默认 pandoc 子集
M4_SUITE=full bash scripts/run_m4_local.sh  # 全量（需 venv + pip install markitdown）
```

## 配置

- 数据目录：`DOCCONVERT_DATA_DIR` 或默认应用数据目录下的 `DocConvert/`
- 解释器：`DOCCONVERT_PYTHON`；Pandoc：`DOCCONVERT_PANDOC`
- 路由：`config/routes.toml`，用户覆盖：`DATA_ROOT/config/routes.user.toml`

## 依赖升级维护（MarkItDown / Docling / Pandoc）

本项目里这三类能力来源不同，升级方式也不同：

- `markitdown`、`docling`：由 `scripts/bundle_python.mjs` 在打包时通过 pip 安装到随包 Python。
- `pandoc`：由 `scripts/fetch_pandoc.py` 按 `build/pandoc.json` 下载并校验 SHA256。

建议每次只升级一个组件，分别验证后再合并，避免定位困难。

### 1) 升级 MarkItDown

1. 修改 `scripts/bundle_python.mjs` 里的 pip 包约束（当前是 `markitdown[all]>=0.1`）。
2. 执行：

```bash
npm run bundle-python
```

3. 验证：
   - `src-tauri/resources/python/` 已刷新。
   - 能通过最小转换流程（建议用 Office/PDF 样本跑一轮）。
4. 打包前再执行一次：

```bash
npm run tauri:build
```

> 注意：MarkItDown 要求 Python >= 3.10。仓库内建议使用 `python/.venv`（3.11/3.12）。

### 2) 升级 Docling

1. 修改 `scripts/bundle_python.mjs` 里的 Docling 版本约束（当前是 `docling>=2.0.0,<3.0`）。
2. 执行：

```bash
npm run bundle-python
```

3. 验证重点（建议最少覆盖）：
   - PDF -> Markdown 主流程；
   - 图片 OCR（RapidOCR）；
   - `picture_item` 与 `page_image` 两种取图模式；
   - `docling_dump_extracted_images` 落盘是否正常。

### 3) 升级 Pandoc

1. 编辑 `build/pandoc.json`：
   - 更新顶层 `version`；
   - 更新各平台 `url`；
   - 更新各平台 `sha256`。
2. 执行下载与校验：

```bash
npm run fetch-pandoc
```

3. 验证：
   - `src-tauri/resources/pandoc/pandoc`（或 `pandoc.exe`）可执行；
   - `src-tauri/resources/pandoc/VERSION` 与目标版本一致；
   - 至少跑一条 Pandoc 路径的端到端转换。

### 4) 推荐的升级验收顺序

```bash
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
npm run tauri:build:smoke
# 无误后再
npm run tauri:build
```

### 5) 回滚策略（出问题时）

- Python 依赖回滚：把 `scripts/bundle_python.mjs` 的版本约束改回上一个可用值，重新 `npm run bundle-python`。
- Pandoc 回滚：把 `build/pandoc.json` 回退到上一版本并重新 `npm run fetch-pandoc`。
- 永远保留一个“已验证通过”的打包产物，便于快速回退发布。

### 随包 Python 与 MarkItDown（XLSX / PPTX / PDF 等）

**Microsoft MarkItDown 要求 Python ≥ 3.10**。请用 3.11/3.12 创建仓库内 venv（勿用系统自带的 3.9）：

```bash
cd /path/to/doc-convert/python
rm -rf .venv
python3.12 -m venv .venv   # 或 python3.11
.venv/bin/pip install -U pip
.venv/bin/pip install -e ".[markitdown]"
cd .. && npm run bundle-python
```

打包时 `bundle-python` 会：校验版本 → 复制 `python/.venv` → 向副本执行 `pip install "markitdown[all]>=0.1"`（使用 **pypi.org**，需网络）。若未创建 `python/.venv` 或 Python 低于 3.10，脚本会跳过或报错，**安装包内可能没有可用的 MarkItDown**。

**已安装的应用**若仍报错：可安装 MarkItDown 后，用环境变量指定解释器（示例）：

```bash
export DOCCONVERT_PYTHON="$HOME/你的路径/python3.12"
# 该解释器需已 pip install "markitdown[all]"
```

然后再从终端启动 `DocConvert.app`，或把 `DOCCONVERT_PYTHON` 写入 `launchctl`/桌面快捷方式的环境（视需求而定）。

## 桌面壳构建（Tauri）

**目标平台**：**macOS**（安装包为 **`.dmg`**）与 **Windows 10 及以上 x64**（安装包为 **NSIS `.exe`**）。`tauri.conf.json` 中 `bundle.targets` 已设为 `["dmg","nsis"]`，不包含 Linux 安装包。

| 系统要求 | 说明 |
| --- | --- |
| macOS | 最低 **11.0 (Big Sur)**（见 `bundle.macOS.minimumSystemVersion`）；在 Apple Silicon / Intel 上分别在本机构建。 |
| Windows | **Windows 10/11** x64；需 **WebView2** 运行时（安装包可通过 `downloadBootstrapper` 引导安装）。 |

- 仅验证编译、不生成安装包：`npm run tauri:build:smoke`（`tauri build --no-bundle --ci`）。
- 完整打包：`npm run tauri:build`（执行 `beforeBuildCommand`）。
- 在 **macOS** 上仅打 DMG：`npm run tauri:build:dmg`。
- 在 **Windows** 上仅打 NSIS：`npm run tauri:build:nsis`。

> 安装包需在对应系统上构建（DMG 在 macOS，NSIS 安装程序在 Windows）。Linux 上可用 `tauri:build:smoke` 做编译冒烟。

**Linux（仅开发/CI 冒烟）** 需 WebKitGTK 等，见 [Tauri 前置条件](https://v2.tauri.app/start/prerequisites/)。

**图标**：源文件为 `src-tauri/icons/app-icon.svg`。更新后执行 `npm run tauri:icon`，再提交生成文件。
