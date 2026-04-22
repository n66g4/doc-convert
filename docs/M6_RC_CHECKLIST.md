# M6 发布候选（RC）检查清单

对应《需求规格说明书》**§10.2 验收标准**与《实施方案》**M6：发布候选（RC）**。本文档用于研发与发布前核对；**「内部用户验收」**须由产品/业务在线下签字或等效记录。

| 属性 | 内容 |
| --- | --- |
| 版本 | 1.0 |
| 最近更新 | 2026-04-18 |
| 关联 | [SRS §10.2](../../文档转换工具需求规格说明书.md)、[实施方案 §5](../../文档转换工具实施方案.md) |

---

## 1. 功能与需求追溯（MVP）

| 条目 | 口径 | 证据 / 位置 |
| --- | --- | --- |
| SRS §1.8 能力矩阵（Must） | 端到端可验证 | [M4_ACCEPTANCE.md](./M4_ACCEPTANCE.md)、`tests/fixtures/m4/`、CI `m4-pandoc` / `m4-regression` |
| FR-014 路由与 `preferred_plugins` | 可解释、平局可指定 | `core/router.rs`、API `preferred_plugins` 字段、[API.md](./API.md) |
| FR-015 免部署交付 | 本机 Core + 壳 | Tauri 打包、[CONFIGURATION.md](./CONFIGURATION.md) |
| REQ-PLG-001～004 | 发现、启用、安装扩展 | `plugin_host/`、[API.md](./API.md) `plugins` |
| SEC-001（MVP） | 见达成声明 | [SECURITY_MVP.md](./SECURITY_MVP.md) |

**Should / 裁剪**：须在 [MVP_LIMITATIONS.md](./MVP_LIMITATIONS.md) 与发布说明中一致披露。

---

## 2. 跨平台与构建（TEST-004 / NFR-008～010）

| 检查项 | 证据 |
| --- | --- |
| Rust Core 多平台编译 | CI `rust` matrix：ubuntu / macOS / Windows |
| 前端构建 | CI `frontend` |
| Tauri 壳 release 编译（Linux 冒烟） | CI `tauri-build-smoke` |
| 目标安装介质：macOS DMG、Windows NSIS | CI `tauri-bundle-macos`、`tauri-bundle-windows` |
| 最低系统版本与 WebView2 | [README.md](../README.md)「桌面壳构建」 |

---

## 3. 性能与稳定性（NFR-001 等）

| 检查项 | 说明 |
| --- | --- |
| 默认并发与单文件上限 | 见 `/health`、`/api/v1/tools/status` 与 [CONFIGURATION.md](./CONFIGURATION.md) |
| 正式数值压测 | 可作为版本基线后续补强；当前见 [PERFORMANCE_AND_TESTING.md](./PERFORMANCE_AND_TESTING.md) |

---

## 4. 单机与隐私（NFR-011～013）

| 检查项 | 说明 |
| --- | --- |
| 默认本机处理 | 产品文案：关于页、MVP_LIMITATIONS |
| 临时文件与任务 TTL | Core GC、`task_result_ttl_secs`（tools/status） |
| 日志与文件名脱敏 | 实现见任务 `input_filename_hint` 等；细节见架构文档 |

---

## 5. 文档齐全（§10.2 最后一项）

| 文档 | 状态 |
| --- | --- |
| SRS | 仓库根目录 `文档转换工具需求规格说明书.md` |
| 架构设计说明书 | 仓库根目录 `文档转换工具架构设计说明书.md` |
| 实施方案 | 仓库根目录 `文档转换工具实施方案.md` |
| 接口说明 | [API.md](./API.md) + [openapi.yaml](./openapi.yaml)（可选用 OpenAPI） |
| 运维与配置 | [CONFIGURATION.md](./CONFIGURATION.md) |
| 已知问题 | [KNOWN_ISSUES.md](./KNOWN_ISSUES.md) |
| 限制说明 | [MVP_LIMITATIONS.md](./MVP_LIMITATIONS.md) |

---

## 6. 内部用户验收（线下）

- [ ] 典型转换场景（Must 格式）走通
- [ ] 插件列表、扩展安装、关于页诊断信息可用
- [ ] 发布说明已含版本号、平台、限制与上游组件版本（DEP-004 口径）

**签核**：__________　日期：__________

---

## 7. RC 通过判定

同时满足：**§1 ～ §5 可追溯证据齐备**、**§6 内部验收完成**、**无阻塞级未关闭缺陷**（或已记入 KNOWN_ISSUES 并获产品认可），可标记 **M6 RC 关闭**并进入正式发布流程。
