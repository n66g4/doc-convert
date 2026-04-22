# 配置与运维说明（DocConvert）

面向安装包与高级用户：数据目录、环境变量、可选配置文件与路由覆盖。

---

## 1. 数据目录布局（架构 §10.1）

默认：`{系统应用数据目录}/DocConvert/`（macOS/Windows 见各平台惯例）。

可通过 **`DOCCONVERT_DATA_DIR`** 覆盖为任意可写路径。启动时 Core 会创建（若不存在）：

| 子路径 | 用途 |
| --- | --- |
| `runtime/` | `core.json` 锁与心跳（单实例） |
| `logs/` | 滚动日志（如 `docconvert.log`） |
| `temp/` | 任务临时文件（按 `task_id` 分子目录） |
| `tasks/` | 任务结果落盘（实现细节以版本为准） |
| `plugins_extra/` | 扩展插件根（双根之一）；**不由**应用内安装，由发版或运维预置 |
| `config/` | 用户覆盖配置（如 `routes.user.toml`） |

---

## 2. 环境变量

| 变量 | 说明 |
| --- | --- |
| `DOCCONVERT_DATA_DIR` | 数据根目录 |
| `DOCCONVERT_BIND_PORT` | Core 监听端口；未设置时可为 `127.0.0.1:0`（随机端口，仅 `--core-only` 场景常见） |
| `DOCCONVERT_PYTHON` | Python 解释器路径（MarkItDown/Docling worker） |
| `DOCCONVERT_PANDOC` | Pandoc 可执行文件路径 |
| `DOCCONVERT_MAX_FILE_BYTES` | 单文件最大字节数（覆盖 `config.json`）；不设时默认约 **2 GiB**，便于数百 MB 级 PDF |

Tauri 开发模式默认将 `DOCCONVERT_BIND_PORT` 设为 `17300`，与 Vite 代理一致。

---

## 3. `config.json`（可选）

位于 `DATA_ROOT/config.json`（若存在则加载）。字段包括：

- `data_root`（通常与目录一致）
- `max_file_size_bytes`：单文件上限（默认约 **2 GiB**，以代码为准；旧版若曾保存 500 MiB 需手动改大）。**大文件**：前端会整段读入内存再上传，数百 MB～GiB 级 PDF 请保证本机可用内存明显大于文件体积，否则可能卡顿或 OOM。
- `max_concurrent_tasks`：并发任务上限
- `task_result_ttl_secs`：内存中已完成任务元数据保留时间（秒）
- `python_executable`、`pandoc_executable`

环境变量 **`DOCCONVERT_PYTHON` / `DOCCONVERT_PANDOC`** 在加载后仍可覆盖磁盘配置。

---

## 4. 路由配置

- **内置**：`config/routes.toml`（随包资源或开发仓库）
- **用户覆盖**：`DATA_ROOT/config/routes.user.toml`（深度合并）

用于 FR-014 与格式归一化；详见架构文档 §7、§10.4。

---

## 5. 可观测性

- **日志**：`DATA_ROOT/logs/`
- **HTTP**：`GET /health`、`GET /api/v1/tools/status`
- **桌面 UI**：关于页与状态栏（复制诊断 JSON）

---

## 6. 发布与构建（摘要）

| 命令 | 说明 |
| --- | --- |
| `npm run dev:stack` | 开发：Pandoc + Core + Vite |
| `npm run tauri:build` | 完整打包（执行 `beforeBuildCommand`） |
| `npm run regression:m4:local` | 本地 M4 回归（随机端口起 Core） |

详见 [README.md](../README.md)。
