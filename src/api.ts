// API：开发环境通过 Vite 代理访问 Core（相对路径）；生产可设 VITE_CORE_API_BASE。

/** 是否在 Tauri 壳内（用于启用原生对话框等） */
export function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function apiBase(): string {
  if (import.meta.env.DEV) {
    return "";
  }
  const b = import.meta.env.VITE_CORE_API_BASE as string | undefined;
  if (b && String(b).trim() !== "") {
    return String(b).replace(/\/$/, "");
  }
  // 桌面版与 main.rs 默认 DOCCONVERT_BIND_PORT=17300 对齐；避免未注入 .env 时请求到错误 origin（关于页/API 全黑或失败）
  if (isTauriRuntime()) {
    return "http://127.0.0.1:17300";
  }
  return "";
}

async function apiUrl(path: string): Promise<string> {
  const base = apiBase();
  const p = path.startsWith("/") ? path : `/${path}`;
  return `${base}${p}`;
}

/**
 * Tauri 生产包页面多为 https 自定义协议，WebView 对 `http://127.0.0.1` 的 `fetch` 常被拦截（报 Load failed）。
 * 经插件走 Rust 侧 HTTP 客户端可稳定访问本机 Core；开发模式仍走 Vite 代理，用浏览器 fetch 即可。
 */
let coreFetchImpl: typeof fetch | undefined;

async function getFetchForCore(): Promise<typeof fetch> {
  if (coreFetchImpl !== undefined) return coreFetchImpl;
  if (import.meta.env.DEV || !isTauriRuntime()) {
    coreFetchImpl = fetch;
    return fetch;
  }
  const base = apiBase();
  if (!base || !/^http:\/\//i.test(base)) {
    coreFetchImpl = fetch;
    return fetch;
  }
  const { fetch: tauriFetch } = await import("@tauri-apps/plugin-http");
  coreFetchImpl = tauriFetch;
  return tauriFetch;
}

/** 桌面版或显式指向本机 Core 时，首屏可能早于 Core 监听端口，需容忍短暂连接失败 */
function coreFetchShouldWarmupRetry(): boolean {
  if (import.meta.env.DEV) return false;
  if (isTauriRuntime()) return true;
  const b = (import.meta.env.VITE_CORE_API_BASE as string | undefined)?.trim();
  if (!b) return false;
  return /^https?:\/\/(127\.0\.0\.1|localhost)(:|\/)/i.test(b);
}

function isTransientCoreFetchFailure(e: unknown): boolean {
  if (!(e instanceof Error)) return false;
  const m = e.message.toLowerCase();
  return (
    e.name === "TypeError" ||
    m.includes("failed to fetch") ||
    m.includes("load failed") ||
    m.includes("networkerror") ||
    m.includes("network request failed") ||
    m.includes("error sending request") ||
    m.includes("connection refused") ||
    m.includes("timed out") ||
    m.includes("timeout")
  );
}

const CORE_FETCH_RETRY_MS = 250;
const CORE_FETCH_MAX_TRIES = 60;

/**
 * 访问本机 Core 的 fetch：在桌面等场景下对「Core 尚未就绪」做短重试，避免首屏任务/插件/健康检查报 Load failed。
 */
async function fetchCore(input: string | URL, init?: RequestInit): Promise<Response> {
  const fetchImpl = await getFetchForCore();
  const max = coreFetchShouldWarmupRetry() ? CORE_FETCH_MAX_TRIES : 1;
  let last: unknown;
  for (let i = 0; i < max; i++) {
    try {
      return await fetchImpl(input, init);
    } catch (e) {
      last = e;
      const canRetry =
        i < max - 1 && coreFetchShouldWarmupRetry() && isTransientCoreFetchFailure(e);
      if (!canRetry) throw e;
      await new Promise((r) => setTimeout(r, CORE_FETCH_RETRY_MS));
    }
  }
  throw last;
}

/** 与 Core `ErrorResponse` 对齐（架构 §9.3） */
export class ApiError extends Error {
  readonly status: number;
  readonly errorCode?: string;
  readonly details?: unknown;

  constructor(
    message: string,
    status: number,
    errorCode?: string,
    details?: unknown
  ) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.errorCode = errorCode;
    this.details = details;
  }
}

export function isApiError(e: unknown): e is ApiError {
  return e instanceof ApiError;
}

async function throwIfNotOk(res: Response): Promise<void> {
  if (res.ok) return;
  const status = res.status;
  let message = res.statusText;
  let errorCode: string | undefined;
  let details: unknown;
  try {
    const body = (await res.json()) as Record<string, unknown>;
    if (typeof body.message === "string") message = body.message;
    if (typeof body.error_code === "string") errorCode = body.error_code;
    if ("details" in body) details = body.details;
  } catch {
    /* 非 JSON */
  }
  throw new ApiError(message, status, errorCode, details);
}

export interface Task {
  task_id: string;
  status: "pending" | "processing" | "completed" | "failed" | "cancelled";
  progress: number;
  input_format?: string;
  output_format: string;
  plugin_chain: Array<{ plugin_id: string; version: string }>;
  created_at: string;
  updated_at: string;
  result_url?: string;
  error?: { code: string; message: string; details?: unknown };
  /** 原始上传文件名（basename），用于任务列表区分多任务 */
  input_filename_hint?: string;
  /** 与上传文件 basename 关联的下载文件名（如 `a.docx` → `a.md`） */
  result_download_filename?: string;
}

/** 与 Core `RouteStep` 一致，用于路由预览 */
export interface RoutePreviewStep {
  plugin_id: string;
  in_format: string;
  out_format: string;
  step_index: number;
}

/** `POST /api/v1/plugins/{id}/test` 自检结果 */
export interface PluginSmokeTestResult {
  /** `smoke`：轻量；`deep`：深度（最小样例真实转换） */
  depth: "smoke" | "deep";
  ok: boolean;
  plugin_id: string;
  runtime_type: string;
  message: string;
  detail?: unknown;
}

export interface Plugin {
  id: string;
  name: string;
  version: string;
  author: string;
  description: string;
  enabled: boolean;
  supported_formats: { input: string[]; output: string[] };
  status: "active" | "inactive" | "update_available";
}

export interface HealthResponse {
  status: string;
  schema?: number;
  service: string;
  host_api_version?: string;
  pid: number;
  started_at_unix_ms?: number;
  uptime_ms: number;
  /** 本机诊断：当前选用的解释器与 Pandoc 路径 */
  python_executable?: string;
  pandoc_executable?: string;
  /** 数据根目录（日志、任务、扩展插件等） */
  data_root?: string;
  logs_directory?: string;
  bind_port?: number;
}

/** `GET /api/v1/tools/status` — Core 与插件快照（M5 诊断） */
export interface ToolsStatusResponse {
  platform: string;
  arch: string;
  host_api_version?: string;
  auto_update_enabled: boolean;
  core: {
    bind_port: number;
    max_concurrent_tasks: number;
    task_result_ttl_secs: number;
    max_file_size_bytes: number;
    data_root: string;
    plugins_extra_dir: string;
    runtime_dir: string;
  };
  plugins: Array<{
    id: string;
    name: string;
    version: string;
    enabled: boolean;
    status: string;
    last_checked: string;
  }>;
}

/**
 * 在系统文件管理器中打开路径（Tauri 桌面版）。
 * 浏览器开发模式会尝试将路径复制到剪贴板。
 */
/** 供 `POST /api/v1/convert`：避免 Tauri `plugin-http` 对 `FormData` 再序列化时破坏 multipart，改为显式 boundary + 原始字节。 */
async function buildConvertMultipartBody(
  file: File,
  outputFormat: string,
  options?: {
    inputFormat?: string;
    preferredPlugins?: string;
    options?: string;
  }
): Promise<{ body: Uint8Array; contentType: string }> {
  const boundary =
    "docconvert-" +
    (globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random()}`);
  const enc = new TextEncoder();
  const crlf = enc.encode("\r\n");

  const safeFilename = file.name
    .replace(/^.*[/\\]/, "")
    .replace(/"/g, "_");

  const chunks: Uint8Array[] = [];

  const pushText = (s: string) => chunks.push(enc.encode(s));

  // file
  pushText(`--${boundary}\r\n`);
  pushText(
    `Content-Disposition: form-data; name="file"; filename="${safeFilename}"\r\n`
  );
  pushText("Content-Type: application/octet-stream\r\n\r\n");
  chunks.push(new Uint8Array(await file.arrayBuffer()));
  chunks.push(crlf);

  // output_format
  pushText(`--${boundary}\r\n`);
  pushText('Content-Disposition: form-data; name="output_format"\r\n\r\n');
  pushText(`${outputFormat}\r\n`);

  if (options?.inputFormat) {
    pushText(`--${boundary}\r\n`);
    pushText('Content-Disposition: form-data; name="input_format"\r\n\r\n');
    pushText(`${options.inputFormat}\r\n`);
  }
  if (options?.preferredPlugins) {
    pushText(`--${boundary}\r\n`);
    pushText(
      'Content-Disposition: form-data; name="preferred_plugins"\r\n\r\n'
    );
    pushText(`${options.preferredPlugins}\r\n`);
  }
  if (options?.options) {
    pushText(`--${boundary}\r\n`);
    pushText('Content-Disposition: form-data; name="options"\r\n\r\n');
    pushText(`${options.options}\r\n`);
  }

  pushText(`--${boundary}--\r\n`);

  let total = 0;
  for (const c of chunks) total += c.length;
  const body = new Uint8Array(total);
  let o = 0;
  for (const c of chunks) {
    body.set(c, o);
    o += c.length;
  }
  // 使用引号包裹 boundary，与 RFC 一致；Content-Type 须与正文 delimiter 完全一致
  return {
    body,
    contentType: `multipart/form-data; boundary="${boundary}"`,
  };
}

/**
 * 转换上传：multipart 体积可能很大。Tauri `plugin-http` 会把整段 body 经 IPC 交给 Rust，
 * 大请求易触发底层 reqwest「error sending request for url」。
 * 对本机 Core（127.0.0.1 / localhost）改用系统 `fetch`，不经 IPC；并对 Core 未就绪做短重试。
 */
async function fetchConvertUpload(
  url: string,
  init: RequestInit
): Promise<Response> {
  const u = typeof url === "string" ? url : String(url);
  // 任意环境：只要目标是本机 Core 绝对 URL，都用系统 fetch（避免 plugin-http IPC 与错误 Content-Type）
  const isLocalCore = /^https?:\/\/(127\.0\.0\.1|localhost)(:\d+)?\//i.test(
    u
  );
  if (!isLocalCore) {
    return fetchCore(u, init);
  }
  const max = coreFetchShouldWarmupRetry() ? CORE_FETCH_MAX_TRIES : 1;
  let last: unknown;
  for (let i = 0; i < max; i++) {
    try {
      return await fetch(u, init);
    } catch (e) {
      last = e;
      const canRetry =
        i < max - 1 &&
        coreFetchShouldWarmupRetry() &&
        isTransientCoreFetchFailure(e);
      if (!canRetry) throw e;
      await new Promise((r) => setTimeout(r, CORE_FETCH_RETRY_MS));
    }
  }
  throw last;
}

export async function openLocalPath(path: string): Promise<{
  ok: boolean;
  hint?: string;
}> {
  const p = path.trim();
  if (!p) return { ok: false, hint: "路径为空" };
  if (isTauriRuntime()) {
    try {
      const { openPath } = await import("@tauri-apps/plugin-opener");
      await openPath(p);
      return { ok: true };
    } catch (e) {
      return {
        ok: false,
        hint: e instanceof Error ? e.message : String(e),
      };
    }
  }
  try {
    await navigator.clipboard.writeText(p);
    return {
      ok: true,
      hint: "已复制路径到剪贴板（浏览器开发模式无法打开文件夹）",
    };
  } catch {
    return {
      ok: false,
      hint: "无法打开文件夹；请手动在访达或资源管理器中打开上述路径",
    };
  }
}

export const api = {
  async health(): Promise<HealthResponse> {
    const url = await apiUrl("/health");
    const res = await fetchCore(url);
    await throwIfNotOk(res);
    return res.json();
  },

  async toolsStatus(): Promise<ToolsStatusResponse> {
    const url = await apiUrl("/api/v1/tools/status");
    const res = await fetchCore(url);
    await throwIfNotOk(res);
    return res.json();
  },

  /**
   * 解析路由链（与转换时一致）。`preferredPlugins` 为 JSON 数组字符串时校验自定义链。
   */
  async previewRoute(
    inputFormat: string,
    outputFormat: string,
    preferredPlugins?: string | null
  ): Promise<{
    steps: RoutePreviewStep[];
    /** 单跳平局时，首选失败或空输出后将自动尝试的插件 id（与 Core 一致） */
    fallback_plugin_ids?: string[];
  }> {
    const url = await apiUrl("/api/v1/convert/preview-route");
    const body: Record<string, string> = {
      input_format: inputFormat.trim(),
      output_format: outputFormat.trim(),
    };
    if (preferredPlugins && preferredPlugins.trim()) {
      body.preferred_plugins = preferredPlugins.trim();
    }
    const res = await fetchCore(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
    await throwIfNotOk(res);
    return res.json();
  },

  /**
   * 桌面版：由 Tauri Rust 从磁盘流式 multipart 提交本机 Core，避免 WebView/JS 整文件读入内存。
   * 仅在 `isTauriRuntime()` 下可用。
   */
  async convertLocalPath(
    localPath: string,
    outputFormat: string,
    options?: {
      inputFormat?: string;
      preferredPlugins?: string;
      options?: string;
    }
  ): Promise<{ task_id: string; status: string }> {
    const { invoke } = await import("@tauri-apps/api/core");
    const body = {
      path: localPath.trim(),
      outputFormat,
      inputFormat: options?.inputFormat,
      preferredPlugins: options?.preferredPlugins,
      options: options?.options,
    };
    return invoke<{ task_id: string; status: string }>(
      "convert_submit_local_file",
      { body }
    );
  },

  async convert(
    file: File,
    outputFormat: string,
    options?: {
      inputFormat?: string;
      preferredPlugins?: string;
      options?: string;
    }
  ): Promise<{ task_id: string; status: string }> {
    const url = await apiUrl("/api/v1/convert");
    const { body, contentType } = await buildConvertMultipartBody(
      file,
      outputFormat,
      options
    );
    // Blob 必须带 type，否则 WebView 可能把请求标成 application/octet-stream，服务端无法按 multipart 解析
    const res = await fetchConvertUpload(url, {
      method: "POST",
      body: new Blob([new Uint8Array(body)], { type: contentType }),
    });
    await throwIfNotOk(res);
    return res.json();
  },

  async getTask(taskId: string): Promise<Task> {
    const url = await apiUrl(`/api/v1/tasks/${taskId}`);
    const res = await fetchCore(url);
    await throwIfNotOk(res);
    return res.json();
  },

  async listTasks(): Promise<{ tasks: Task[] }> {
    const url = await apiUrl("/api/v1/tasks");
    const res = await fetchCore(url);
    await throwIfNotOk(res);
    return res.json();
  },

  async cancelTask(taskId: string): Promise<void> {
    const url = await apiUrl(`/api/v1/tasks/${taskId}/cancel`);
    const res = await fetchCore(url, { method: "POST" });
    await throwIfNotOk(res);
  },

  /** 删除已结束任务（已完成 / 失败 / 已取消）及磁盘缓存（POST，避免 DELETE 在部分环境 405） */
  async deleteTask(taskId: string): Promise<void> {
    const url = await apiUrl(`/api/v1/tasks/${encodeURIComponent(taskId)}/remove`);
    const res = await fetchCore(url, { method: "POST" });
    await throwIfNotOk(res);
  },

  /** 清空所有已结束任务 */
  async clearFinishedTasks(): Promise<{ removed: number }> {
    const url = await apiUrl("/api/v1/tasks/clear-finished");
    const res = await fetchCore(url, { method: "POST" });
    await throwIfNotOk(res);
    const j = (await res.json()) as { removed?: number };
    return { removed: j.removed ?? 0 };
  },

  async listPlugins(): Promise<{ plugins: Plugin[] }> {
    const url = await apiUrl("/api/v1/plugins");
    const res = await fetchCore(url);
    await throwIfNotOk(res);
    return res.json();
  },

  async setPluginEnabled(pluginId: string, enabled: boolean): Promise<void> {
    const url = await apiUrl(`/api/v1/plugins/${pluginId}/enable`);
    const res = await fetchCore(url, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ enabled }),
    });
    await throwIfNotOk(res);
  },

  /**
   * 插件自检：最小样例走与任务相同的 worker 转换路径（`depth=deep`）。
   */
  async testPlugin(pluginId: string): Promise<PluginSmokeTestResult> {
    const path = `/api/v1/plugins/${encodeURIComponent(pluginId)}/test?depth=deep`;
    const url = await apiUrl(path);
    const res = await fetchCore(url, { method: "POST" });
    await throwIfNotOk(res);
    return res.json();
  },

  downloadUrl(taskId: string): Promise<string> {
    return apiUrl(`/api/v1/tasks/${taskId}/download`);
  },

  /** 桌面生产包内 WebView 对 `http://127.0.0.1` 直链下载不可靠，先经 fetchCore 取 Blob 再保存。 */
  async downloadTaskResult(
    taskId: string,
    outputFormat: string,
    downloadFilename?: string
  ): Promise<void> {
    const fname = downloadFilename ?? `result.${outputFormat}`;
    const url = await apiUrl(`/api/v1/tasks/${encodeURIComponent(taskId)}/download`);
    const useBlob =
      isTauriRuntime() &&
      !import.meta.env.DEV &&
      /^http:\/\//i.test(apiBase());
    if (useBlob) {
      const res = await fetchCore(url);
      await throwIfNotOk(res);
      const blob = await res.blob();
      const obj = URL.createObjectURL(blob);
      try {
        const a = document.createElement("a");
        a.href = obj;
        a.download = fname;
        a.click();
      } finally {
        URL.revokeObjectURL(obj);
      }
      return;
    }
    const a = document.createElement("a");
    a.href = url;
    a.download = fname;
    a.click();
  },
};
