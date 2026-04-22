# DocConvert Core HTTP API 说明

本机 **127.0.0.1** 上的 Axum HTTP 服务（与 Tauri 壳或 `--core-only` 启动一致）。生产构建中前端通过 `VITE_CORE_API_BASE` 指向该基址。

**机器可读契约**：同目录 [openapi.yaml](./openapi.yaml)（OpenAPI 3.0）。

## 通用约定

- **Content-Type**：JSON 接口使用 `application/json`；上传使用 `multipart/form-data`。
- **错误体**：HTTP 状态码 + JSON `ErrorResponse`：
  - `error_code`：稳定枚举字符串（如 `NO_ROUTE`、`FILE_TOO_LARGE`）
  - `message`：人类可读说明
  - `details`：可选，JSON（如 `NO_ROUTE` 时含 `candidates`）
- **CORS**：开发时放宽；默认仍仅本机访问。

## 端点一览

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/health` | 健康检查与诊断路径 |
| POST | `/api/v1/convert` | 提交转换任务（multipart） |
| GET | `/api/v1/tasks` | 任务列表 |
| GET | `/api/v1/tasks/{task_id}` | 任务详情 |
| POST | `/api/v1/tasks/{task_id}/cancel` | 取消任务 |
| GET | `/api/v1/tasks/{task_id}/download` | 下载结果文件 |
| GET | `/api/v1/plugins` | 插件列表 |
| PUT | `/api/v1/plugins/{plugin_id}/enable` | 启用/禁用插件 |
| GET | `/api/v1/tools/status` | 运行时与插件快照（诊断） |

---

## GET /health

**响应 200**：`HealthResponse`

- `status`：固定 `"ok"`
- `schema`：整数，随契约演进递增
- `service`：`docconvert-core`
- `host_api_version`：宿主 API 版本字符串（当前 `"1"`）
- `pid`、`started_at_unix_ms`、`uptime_ms`
- `python_executable`、`pandoc_executable`
- `data_root`、`logs_directory`、`bind_port`

---

## POST /api/v1/convert

**请求**：`multipart/form-data`

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `file` | 是 | 上传文件 |
| `output_format` | 是 | 目标格式（如 `markdown`、`html`） |
| `input_format` | 否 | 覆盖输入格式推断 |
| `preferred_plugins` | 否 | 路由平局时优先插件 id（FR-014） |
| `options` | 否 | JSON 字符串，透传插件链 |

**响应 202**：`ConvertResponse`

- `status`：`accepted`
- `task_id`：UUID 字符串
- `message`：如 `Task queued`

**错误**：413 `FILE_TOO_LARGE`、422 `NO_ROUTE` / `INVALID_OPTIONS` 等。

---

## GET /api/v1/tasks/{task_id}

**响应 200**：任务对象（见 OpenAPI schema `ConvertTask`）

- `status`：`pending` | `processing` | `completed` | `failed` | `cancelled`
- `plugin_chain`：固化后的插件与版本
- `result_url`：完成后可配合 `download` 使用

---

## GET /api/v1/tasks/{task_id}/download

任务成功完成后返回结果文件流；否则 4xx/5xx。

---

## PUT /api/v1/plugins/{plugin_id}/enable

**请求体**：`{ "enabled": true | false }`

---

## GET /api/v1/tools/status

**响应**：平台、`core`（端口、配额、路径）、`plugins` 数组等，详见 OpenAPI。

---

## 错误码与 HTTP 状态（摘要）

| error_code | 典型 HTTP |
| --- | --- |
| NO_ROUTE | 422 |
| FILE_TOO_LARGE | 413 |
| TASK_NOT_FOUND | 404 |
| PLUGIN_NOT_FOUND | 404 |
| INVALID_OPTIONS | 422 |
| PLUGIN_FAILED / INTERNAL_ERROR | 500 |
| TIMEOUT | 504 |

完整映射以 `AppError::http_status()` 为准（源码 `src-tauri/src/infra/error.rs`）。
