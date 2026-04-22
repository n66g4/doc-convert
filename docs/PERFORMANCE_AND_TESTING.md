# 性能与测试矩阵（M6 口径）

对应 SRS **NFR-001**、**DEP-001/002**、**TEST-004** 等与《实施方案》测试章节。本文档固定 **RC 阶段的声明口径**，避免与代码实现不一致。

---

## 1. 性能（NFR-001）

- **默认**：单任务、并发上限与单文件上限见 `GET /api/v1/tools/status` 中 `core` 字段（与 `AppConfig` 一致）。
- **DEP-002 推荐配置 / DEP-001 最低配置**：以 SRS 与版本发布说明为准；**不设**全局 OCR 数值验收线（§1.8 扫描 PDF 等为 Should 或样本定性）。
- **正式压测报告**：可作为发布后持续工程项；**不**作为 M6 RC 的硬性阻塞项，但须在 [M6_RC_CHECKLIST.md](./M6_RC_CHECKLIST.md) 中勾选「性能基线」说明。

---

## 2. 跨平台测试（TEST-004 / NFR-008～010）

| 层级 | 覆盖 |
| --- | --- |
| CI | Rust `fmt/clippy/test` 于 ubuntu / macOS / Windows；前端 `npm run build`；Tauri `tauri:build:smoke`（Linux）；`m4-pandoc` / `m4-regression`；DMG/NSIS 分平台构建 |
| 本地 | `npm run regression:m4:local`、`npm run tauri:dev` |

**ARM64 Windows**：目标矩阵在 SRS 中列出；当前 CI 以 Windows x64 为主，ARM64 可在版本基线中单独声明验证状态。

---

## 3. 回归与样本

- **M4**：Must 格式与固定样本见 [M4_ACCEPTANCE.md](./M4_ACCEPTANCE.md)。
- **冒烟**：`scripts/smoke_core.sh`（`health` + `tools/status`）。

---

## 4. 修订

随版本更新 `tools/status` 默认值或 CI 矩阵时，同步更新本节与发布说明。
