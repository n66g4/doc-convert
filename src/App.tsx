import { useState, useEffect, useCallback, useRef } from "react";
import {
  api,
  openLocalPath,
  type HealthResponse,
  type ToolsStatusResponse,
  type RoutePreviewStep,
  type PluginSmokeTestResult,
  Task,
  Plugin,
  isApiError,
} from "./api";
import {
  Upload,
  FileText,
  Puzzle,
  Activity,
  X,
  Download,
  RefreshCw,
  AlertCircle,
  Clock,
  Loader2,
  ToggleLeft,
  ToggleRight,
  Zap,
  Info,
  FolderOpen,
  ClipboardCopy,
  Trash2,
  ChevronUp,
  ChevronDown,
  TestTube2,
} from "lucide-react";

/** 与 package.json version 保持一致 */
const APP_VERSION = "0.1.0";

/** 从文件名推断输入格式（与 Core normalize 对齐，用于路由预览） */
function inferInputFormatFromFilename(filename: string): string {
  const i = filename.lastIndexOf(".");
  if (i < 0 || i === filename.length - 1) return "";
  const ext = filename.slice(i + 1).toLowerCase();
  if (ext === "md" || ext === "markdown" || ext === "gfm") return "markdown";
  if (ext === "txt" || ext === "text") return "plain";
  if (ext === "htm") return "html";
  return ext;
}

function pluginsSupportingHop(
  plugins: Plugin[],
  inFmt: string,
  outFmt: string
): Plugin[] {
  return plugins.filter(
    (p) =>
      p.enabled &&
      p.supported_formats.input.includes(inFmt) &&
      p.supported_formats.output.includes(outFmt)
  );
}

function formatErrorForUi(e: unknown): { summary: string; details?: string } {
  if (isApiError(e)) {
    const parts = [
      e.errorCode ? `[${e.errorCode}]` : `[HTTP ${e.status}]`,
      e.message,
    ].filter(Boolean);
    const summary = parts.join(" ");
    let details: string | undefined;
    if (e.details !== undefined && e.details !== null) {
      try {
        details = JSON.stringify(e.details, null, 2);
      } catch {
        details = String(e.details);
      }
    }
    return { summary, details };
  }
  return { summary: e instanceof Error ? e.message : String(e) };
}

/** 路由平局且未指定 preferred_plugins 时，Core 返回 NO_ROUTE + details.candidates（FR-014） */
function extractNoRouteCandidates(e: unknown): string[] | undefined {
  if (!isApiError(e) || e.errorCode !== "NO_ROUTE" || e.details == null) {
    return undefined;
  }
  if (typeof e.details !== "object") return undefined;
  const raw = (e.details as { candidates?: unknown }).candidates;
  if (!Array.isArray(raw)) return undefined;
  const ids = raw.filter((x): x is string => typeof x === "string");
  return ids.length > 0 ? ids : undefined;
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function formatTtlHours(secs: number): string {
  return `${Math.round(secs / 3600)} 小时`;
}

function ErrorBanner({ error }: { error: { summary: string; details?: string } }) {
  return (
    <div
      style={{
        padding: "12px 16px",
        background: "#7f1d1d20",
        border: "1px solid #7f1d1d",
        borderRadius: 8,
        color: "var(--danger)",
        fontSize: 13,
      }}
    >
      <AlertCircle size={14} style={{ display: "inline", marginRight: 6, verticalAlign: "middle" }} />
      <span>{error.summary}</span>
      {error.details && (
        <pre
          style={{
            marginTop: 10,
            padding: 10,
            background: "var(--surface2)",
            borderRadius: 6,
            fontSize: 11,
            overflow: "auto",
            maxHeight: 160,
            color: "var(--text-muted)",
            whiteSpace: "pre-wrap",
            wordBreak: "break-word",
          }}
        >
          {error.details}
        </pre>
      )}
    </div>
  );
}

// ─── Types ────────────────────────────────────────────────────────────────────

const OUTPUT_FORMATS = [
  { value: "markdown", label: "Markdown (.md)" },
  { value: "html", label: "HTML (.html)" },
  { value: "plain", label: "纯文本 (.txt)" },
  { value: "docx", label: "Word (.docx)" },
  { value: "json", label: "JSON (.json)" },
  { value: "latex", label: "LaTeX (.tex)" },
  { value: "rst", label: "reStructuredText (.rst)" },
];

type Tab = "convert" | "tasks" | "plugins" | "about";

// ─── Helpers ──────────────────────────────────────────────────────────────────

function statusTag(status: Task["status"]) {
  switch (status) {
    case "completed":
      return <span className="tag tag-success">已完成</span>;
    case "failed":
      return <span className="tag tag-danger">失败</span>;
    case "processing":
      return <span className="tag tag-info">转换中</span>;
    case "pending":
      return <span className="tag tag-warning">等待中</span>;
    case "cancelled":
      return <span className="tag tag-muted">已取消</span>;
  }
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function timeAgo(iso: string): string {
  const diff = (Date.now() - new Date(iso).getTime()) / 1000;
  if (diff < 60) return "刚刚";
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`;
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`;
  return `${Math.floor(diff / 86400)} 天前`;
}

// ─── DropZone ─────────────────────────────────────────────────────────────────

/** macOS WKWebView 上 HTML5 dataTransfer.files 常为空；桌面版用原生 drop 拿路径，仅 stat 元数据（大文件不经 JS 读入内存）。 */
type ConvertQueueItem =
  | { kind: "path"; path: string; displayName: string; size: number }
  | { kind: "file"; file: File };

function queueItemFilename(item: ConvertQueueItem | undefined): string {
  if (!item) return "";
  return item.kind === "path" ? item.displayName : item.file.name;
}

async function tauriPathsToQueueItems(
  paths: string[]
): Promise<ConvertQueueItem[]> {
  const { stat } = await import("@tauri-apps/plugin-fs");
  const out: ConvertQueueItem[] = [];
  for (const p of paths) {
    try {
      const s = await stat(p);
      if (!s.isFile) continue;
      const name = p.replace(/^.*[/\\]/, "") || "file";
      const size = typeof s.size === "bigint" ? Number(s.size) : s.size;
      out.push({ kind: "path", path: p, displayName: name, size });
    } catch (e) {
      console.error("Tauri stat 失败:", p, e);
    }
  }
  return out;
}

function isTauriApp(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function DropZone({
  onQueueAdd,
}: {
  onQueueAdd: (items: ConvertQueueItem[]) => void;
}) {
  const [dragging, setDragging] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const htmlDragDepth = useRef(0);

  useEffect(() => {
    if (!isTauriApp()) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void (async () => {
      const { getCurrentWebview } = await import("@tauri-apps/api/webview");
      const u = await getCurrentWebview().onDragDropEvent((event) => {
        if (cancelled) return;
        const p = event.payload;
        if (p.type === "enter" || p.type === "over") {
          setDragging(true);
        } else if (p.type === "leave") {
          setDragging(false);
        } else if (p.type === "drop" && p.paths.length > 0) {
          setDragging(false);
          void (async () => {
            try {
              const items = await tauriPathsToQueueItems(p.paths);
              if (items.length) onQueueAdd(items);
            } catch (e) {
              console.error("Tauri 拖放处理路径失败:", e);
            }
          })();
        }
      });
      if (!cancelled) unlisten = u;
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [onQueueAdd]);

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      htmlDragDepth.current = 0;
      setDragging(false);
      const dropped = Array.from(e.dataTransfer.files);
      if (dropped.length) {
        onQueueAdd(dropped.map((file) => ({ kind: "file" as const, file })));
        return;
      }
      // macOS 等环境下 files 常为空；路径由上方 onDragDropEvent + stat 处理
    },
    [onQueueAdd]
  );

  return (
    <div
      className="drop-zone"
      style={{
        border: `2px dashed ${dragging ? "var(--accent)" : "var(--border)"}`,
        borderRadius: "var(--radius)",
        padding: "40px 24px",
        textAlign: "center",
        cursor: "pointer",
        background: dragging ? "#6366f110" : "var(--surface2)",
        transition: "all 0.2s",
      }}
      onDragEnter={(e) => {
        e.preventDefault();
        htmlDragDepth.current += 1;
        setDragging(true);
      }}
      onDragOver={(e) => {
        e.preventDefault();
        e.dataTransfer.dropEffect = "copy";
        setDragging(true);
      }}
      onDragLeave={(e) => {
        e.preventDefault();
        htmlDragDepth.current -= 1;
        if (htmlDragDepth.current <= 0) {
          htmlDragDepth.current = 0;
          setDragging(false);
        }
      }}
      onDrop={handleDrop}
      onClick={() => {
        if (isTauriApp()) {
          void (async () => {
            try {
              const { open } = await import("@tauri-apps/plugin-dialog");
              const selected = await open({ multiple: true, directory: false });
              if (selected === null) return;
              const paths = Array.isArray(selected) ? selected : [selected];
              const items = await tauriPathsToQueueItems(paths);
              if (items.length) onQueueAdd(items);
            } catch (e) {
              console.error("选择文件失败:", e);
            }
          })();
        } else {
          inputRef.current?.click();
        }
      }}
    >
      <Upload
        size={36}
        style={{ color: dragging ? "var(--accent)" : "var(--text-muted)", marginBottom: 12 }}
      />
      <p style={{ color: "var(--text)", fontWeight: 600, marginBottom: 6 }}>
        拖放文件到此处，或点击选择
      </p>
      <p style={{ color: "var(--text-muted)", fontSize: 12 }}>
        支持 PDF、Word、Excel、PPT、HTML、TXT、图像等格式
      </p>
      <input
        ref={inputRef}
        type="file"
        multiple
        style={{ display: "none" }}
        onChange={(e) => {
          const picked = Array.from(e.target.files ?? []);
          if (picked.length) {
            onQueueAdd(picked.map((file) => ({ kind: "file" as const, file })));
          }
          e.target.value = "";
        }}
      />
    </div>
  );
}

// ─── ConvertTab ───────────────────────────────────────────────────────────────

function ConvertTab({ onTaskCreated }: { onTaskCreated: (id: string) => void }) {
  const [queue, setQueue] = useState<ConvertQueueItem[]>([]);
  const [outputFormat, setOutputFormat] = useState("markdown");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<{
    summary: string;
    details?: string;
    noRouteCandidates?: string[];
  } | null>(null);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [inputFormatOverride, setInputFormatOverride] = useState("");
  const [ocrImageSource, setOcrImageSource] = useState<"picture_item" | "page_image">("picture_item");
  const [ocrPostprocessMode, setOcrPostprocessMode] = useState<"strict" | "lenient">("strict");
  const [dumpExtractedImages, setDumpExtractedImages] = useState(false);
  const [pluginsForRoute, setPluginsForRoute] = useState<Plugin[]>([]);
  const [routeAutoSteps, setRouteAutoSteps] = useState<RoutePreviewStep[] | null>(null);
  /** 单跳平局时 Core 在首选失败/空输出后自动尝试的插件 */
  const [routeAutoFallbacks, setRouteAutoFallbacks] = useState<string[]>([]);
  const [routeAutoError, setRouteAutoError] = useState<string | null>(null);
  const [routeExplicitSteps, setRouteExplicitSteps] = useState<RoutePreviewStep[] | null>(null);
  const [routeExplicitError, setRouteExplicitError] = useState<string | null>(null);
  const [routeChainIds, setRouteChainIds] = useState<string[]>([]);
  const [routeCustomized, setRouteCustomized] = useState(false);
  const [routeLoadingAuto, setRouteLoadingAuto] = useState(false);
  const [routeLoadingExplicit, setRouteLoadingExplicit] = useState(false);

  const inputForRoute =
    inputFormatOverride.trim() ||
    (queue[0] ? inferInputFormatFromFilename(queueItemFilename(queue[0])) : "");

  const chainSig = JSON.stringify(routeChainIds);
  const autoChainIds = routeAutoSteps?.map((s) => s.plugin_id) ?? [];
  const effectiveChainIds = routeChainIds.length > 0 ? routeChainIds : autoChainIds;
  const routeChainHasDocling = effectiveChainIds.includes("docling_adapter");
  const routeChainMatchesAuto =
    autoChainIds.length > 0 &&
    routeChainIds.length === autoChainIds.length &&
    routeChainIds.every((id, i) => id === autoChainIds[i]);
  const routeCustomizedEffective = routeCustomized && !routeChainMatchesAuto;

  useEffect(() => {
    setRouteCustomized(false);
  }, [outputFormat, inputFormatOverride, queue[0] ? queueItemFilename(queue[0]) : ""]);

  useEffect(() => {
    if (routeCustomized && routeChainMatchesAuto) {
      setRouteCustomized(false);
    }
  }, [routeCustomized, routeChainMatchesAuto]);

  useEffect(() => {
    if (!showAdvanced) {
      setPluginsForRoute([]);
      return;
    }
    let alive = true;
    void api
      .listPlugins()
      .then(({ plugins }) => {
        if (alive) setPluginsForRoute(plugins);
      })
      .catch(() => {
        if (alive) setPluginsForRoute([]);
      });
    return () => {
      alive = false;
    };
  }, [showAdvanced]);

  useEffect(() => {
    if (!showAdvanced) {
      setRouteAutoSteps(null);
      setRouteAutoFallbacks([]);
      setRouteAutoError(null);
      setRouteLoadingAuto(false);
      return;
    }
    if (!inputForRoute) {
      setRouteAutoSteps(null);
      setRouteAutoFallbacks([]);
      setRouteAutoError(
        "请选择文件，或在「输入格式」中填写格式（如 doc、docx、pdf），以预览插件链。"
      );
      setRouteLoadingAuto(false);
      return;
    }

    let cancelled = false;
    setRouteLoadingAuto(true);
    setRouteAutoError(null);
    void api
      .previewRoute(inputForRoute, outputFormat, null)
      .then((r) => {
        if (cancelled) return;
        setRouteAutoSteps(r.steps);
        setRouteAutoFallbacks(r.fallback_plugin_ids ?? []);
        setRouteAutoError(null);
        if (!routeCustomized) {
          setRouteChainIds((prev) => {
            const next = r.steps.map((s) => s.plugin_id);
            if (prev.length === next.length && prev.every((p, i) => p === next[i])) {
              return prev;
            }
            return next;
          });
        }
      })
      .catch((e) => {
        if (cancelled) return;
        setRouteAutoSteps(null);
        setRouteAutoFallbacks([]);
        setRouteAutoError(formatErrorForUi(e).summary);
        if (!routeCustomized) setRouteChainIds([]);
      })
      .finally(() => {
        if (!cancelled) setRouteLoadingAuto(false);
      });

    return () => {
      cancelled = true;
    };
  }, [showAdvanced, inputForRoute, outputFormat, routeCustomized]);

  useEffect(() => {
    if (!showAdvanced || !inputForRoute) {
      setRouteExplicitSteps(null);
      setRouteExplicitError(null);
      setRouteLoadingExplicit(false);
      return;
    }
    if (!routeCustomizedEffective || routeChainIds.length === 0) {
      setRouteExplicitSteps(null);
      setRouteExplicitError(null);
      setRouteLoadingExplicit(false);
      return;
    }

    let cancelled = false;
    setRouteLoadingExplicit(true);
    setRouteExplicitError(null);
    void api
      .previewRoute(inputForRoute, outputFormat, JSON.stringify(routeChainIds))
      .then((r) => {
        if (cancelled) return;
        setRouteExplicitSteps(r.steps);
        setRouteExplicitError(null);
      })
      .catch((e) => {
        if (cancelled) return;
        setRouteExplicitSteps(null);
        setRouteExplicitError(formatErrorForUi(e).summary);
      })
      .finally(() => {
        if (!cancelled) setRouteLoadingExplicit(false);
      });

    return () => {
      cancelled = true;
    };
  }, [showAdvanced, inputForRoute, outputFormat, routeCustomizedEffective, chainSig]);

  const convertFiles = async (preferredOverride?: string) => {
    if (!queue.length) return;
    setLoading(true);
    setError(null);
    const ppEffective =
      preferredOverride !== undefined
        ? preferredOverride.trim() || undefined
        : routeCustomizedEffective && routeChainIds.length > 0
          ? JSON.stringify(routeChainIds)
          : undefined;
    if (preferredOverride !== undefined && preferredOverride.trim()) {
      setRouteCustomized(true);
      try {
        const arr = JSON.parse(preferredOverride.trim()) as unknown;
        if (Array.isArray(arr) && arr.every((x) => typeof x === "string")) {
          setRouteChainIds(arr as string[]);
        }
      } catch {
        /* 保持链由后续预览同步 */
      }
      setShowAdvanced(true);
    }
    try {
      const pluginOptions: Record<string, unknown> = {};
      if (outputFormat === "markdown" && routeChainHasDocling) {
        pluginOptions.docling_ocr_image_source = ocrImageSource;
        pluginOptions.docling_ocr_postprocess_mode = ocrPostprocessMode;
        if (dumpExtractedImages) {
          pluginOptions.docling_dump_extracted_images = true;
        }
      }
      const opts = {
        inputFormat: inputFormatOverride.trim() || undefined,
        preferredPlugins: ppEffective,
        options: Object.keys(pluginOptions).length
          ? JSON.stringify(pluginOptions)
          : undefined,
      };
      for (const item of queue) {
        if (item.kind === "path") {
          if (!isTauriApp()) {
            throw new Error("本地路径转换仅在桌面版可用");
          }
          const res = await api.convertLocalPath(item.path, outputFormat, opts);
          onTaskCreated(res.task_id);
        } else {
          const res = await api.convert(item.file, outputFormat, opts);
          onTaskCreated(res.task_id);
        }
      }
      setQueue([]);
    } catch (e: unknown) {
      const base = formatErrorForUi(e);
      const cand = extractNoRouteCandidates(e);
      setError(
        cand?.length ? { ...base, noRouteCandidates: cand } : { ...base }
      );
    } finally {
      setLoading(false);
    }
  };

  const handleConvert = () => {
    void convertFiles();
  };

  const displayRouteSteps: RoutePreviewStep[] | null = routeCustomizedEffective
    ? routeExplicitSteps ?? routeAutoSteps
    : routeAutoSteps;

  const routeChainInvalid =
    routeCustomizedEffective &&
    (routeChainIds.length === 0 || !!routeExplicitError || routeLoadingExplicit);
  const ocrOptionsLockedByRoute = !routeChainHasDocling;

  const moveChainStep = (index: number, delta: number) => {
    setRouteCustomized(true);
    setRouteChainIds((ids) => {
      const j = index + delta;
      if (j < 0 || j >= ids.length) return ids;
      const n = [...ids];
      [n[index], n[j]] = [n[j], n[index]];
      return n;
    });
  };

  const replaceChainStep = (index: number, pluginId: string) => {
    setRouteCustomized(true);
    setRouteChainIds((ids) => {
      const n = [...ids];
      n[index] = pluginId;
      return n;
    });
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 20 }}>
      <DropZone
        onQueueAdd={(items) => setQueue((prev) => [...prev, ...items])}
      />

      {queue.length > 0 && (
        <div className="card" style={{ padding: "12px 16px" }}>
          <p style={{ color: "var(--text-muted)", fontSize: 12, marginBottom: 10 }}>
            已选择 {queue.length} 个文件
          </p>
          <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
            {queue.map((item, i) => (
              <div
                key={`${item.kind}-${i}-${item.kind === "path" ? item.path : item.file.name}`}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 10,
                  padding: "8px 12px",
                  background: "var(--surface2)",
                  borderRadius: 6,
                }}
              >
                <FileText size={16} style={{ color: "var(--accent)", flexShrink: 0 }} />
                <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {item.kind === "path" ? item.displayName : item.file.name}
                </span>
                <span style={{ color: "var(--text-muted)", fontSize: 11 }}>
                  {formatFileSize(item.kind === "path" ? item.size : item.file.size)}
                </span>
                <button
                  className="btn-ghost"
                  style={{ padding: "2px 6px" }}
                  onClick={() => setQueue((prev) => prev.filter((_, j) => j !== i))}
                >
                  <X size={13} />
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      <div className="card" style={{ display: "flex", alignItems: "center", gap: 16, padding: "14px 20px" }}>
        <label style={{ color: "var(--text-muted)", fontSize: 13, minWidth: 80 }}>输出格式</label>
        <select
          value={outputFormat}
          onChange={(e) => setOutputFormat(e.target.value)}
          style={{ flex: 1 }}
        >
          {OUTPUT_FORMATS.map((f) => (
            <option key={f.value} value={f.value}>
              {f.label}
            </option>
          ))}
        </select>
      </div>

      <div className="card" style={{ padding: "12px 16px" }}>
        <button
          type="button"
          className="btn-ghost"
          onClick={() => setShowAdvanced((v) => !v)}
          style={{ padding: "4px 8px", fontSize: 12 }}
        >
          {showAdvanced ? "隐藏高级选项" : "高级选项"}
        </button>
        <span style={{ fontSize: 11, color: "var(--text-muted)", marginLeft: 8 }}>
          输入格式覆盖、路由插件链预览与指定
        </span>
        {showAdvanced && (
          <div style={{ marginTop: 14, display: "flex", flexDirection: "column", gap: 12 }}>
            <div>
              <label style={{ fontSize: 12, color: "var(--text-muted)", display: "block" }}>
                输入格式（可选）
              </label>
              <input
                value={inputFormatOverride}
                onChange={(e) => setInputFormatOverride(e.target.value)}
                placeholder="留空则按扩展名推断，例如 pdf、doc、docx、html、plain"
                style={{
                  width: "100%",
                  marginTop: 6,
                  padding: "8px 10px",
                  borderRadius: 6,
                  border: "1px solid var(--border)",
                  background: "var(--surface2)",
                  color: "var(--text)",
                  fontSize: 13,
                }}
              />
            </div>

            <div style={{ order: 3 }}>
              <label style={{ fontSize: 12, color: "var(--text-muted)", display: "block" }}>
                PDF 转图片方式（用于图片 OCR）
              </label>
              <select
                value={ocrImageSource}
                onChange={(e) =>
                  setOcrImageSource(e.target.value as "picture_item" | "page_image")
                }
                disabled={ocrOptionsLockedByRoute}
                style={{
                  width: "100%",
                  marginTop: 6,
                  padding: "8px 10px",
                  borderRadius: 6,
                  border: "1px solid var(--border)",
                  background: "var(--surface2)",
                  color: "var(--text)",
                  fontSize: 13,
                }}
              >
                <option value="picture_item">PictureItem（默认，按文档图片槽位）</option>
                <option value="page_image">整页图片（每页渲染后 OCR）</option>
              </select>
              {ocrOptionsLockedByRoute && (
                <p style={{ fontSize: 11, color: "var(--text-muted)", marginTop: 6 }}>
                  当前插件链未使用 docling_adapter，OCR 设置不会生效。
                </p>
              )}
            </div>

            <div style={{ order: 4 }}>
              <label style={{ fontSize: 12, color: "var(--text-muted)", display: "block" }}>
                图片 OCR 后处理模式
              </label>
              <select
                value={ocrPostprocessMode}
                onChange={(e) =>
                  setOcrPostprocessMode(e.target.value as "strict" | "lenient")
                }
                disabled={ocrOptionsLockedByRoute}
                style={{
                  width: "100%",
                  marginTop: 6,
                  padding: "8px 10px",
                  borderRadius: 6,
                  border: "1px solid var(--border)",
                  background: "var(--surface2)",
                  color: "var(--text)",
                  fontSize: 13,
                }}
              >
                <option value="strict">严格（过滤孤立单字母噪声）</option>
                <option value="lenient">宽松（保留疑似单字结果）</option>
              </select>
            </div>

            <div style={{ order: 5 }}>
              <label
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 8,
                  fontSize: 12,
                  color: "var(--text)",
                  cursor: routeChainHasDocling ? "pointer" : "not-allowed",
                }}
              >
                <input
                  type="checkbox"
                  checked={dumpExtractedImages}
                  disabled={!routeChainHasDocling}
                  onChange={(e) => setDumpExtractedImages(e.target.checked)}
                />
                保存提取图片用于核对（Docling）
              </label>
              <p style={{ fontSize: 11, color: "var(--text-muted)", margin: "6px 0 0 24px" }}>
                保存到 Application Support/DocConvert/debug/docling_images/
              </p>
            </div>

            <div style={{ borderTop: "1px dashed var(--border)", paddingTop: 10, order: 2 }}>
              <div style={{ fontSize: 12, fontWeight: 600, color: "var(--text)", marginBottom: 6 }}>
                路由插件链
              </div>
              <p style={{ fontSize: 11, color: "var(--text-muted)", margin: "0 0 8px", lineHeight: 1.55 }}>
                根据当前「输入格式 + 输出格式」向 Core 请求与转换一致的路由解析。勾选「使用下方插件链」后，可调整顺序或逐步更换插件；无需手写 JSON。
              </p>
              {routeLoadingAuto && (
                <p style={{ fontSize: 11, color: "var(--text-muted)" }}>
                  <Loader2
                    size={12}
                    style={{ display: "inline", marginRight: 4, animation: "spin 1s linear infinite" }}
                  />
                  正在解析路由…
                </p>
              )}
              {!routeLoadingAuto && routeAutoError && (
                <p style={{ fontSize: 12, color: "var(--danger)", marginBottom: 8 }}>{routeAutoError}</p>
              )}
              {!routeLoadingAuto && !routeAutoError && routeAutoSteps && routeAutoSteps.length > 0 && (
                <div
                  style={{
                    fontSize: 11,
                    color: "var(--text-muted)",
                    marginBottom: 8,
                    padding: "8px 10px",
                    background: "var(--surface2)",
                    borderRadius: 6,
                    border: "1px solid var(--border)",
                  }}
                >
                  <span style={{ color: "var(--text-muted)" }}>自动路由（参考）：</span>
                  {routeAutoSteps.map((s, i) => (
                    <span key={i}>
                      {i > 0 ? " → " : " "}
                      <code style={{ fontSize: 11 }}>{s.plugin_id}</code>
                      <span style={{ opacity: 0.85 }}>
                        （{s.in_format}→{s.out_format}）
                      </span>
                    </span>
                  ))}
                </div>
              )}
              {!routeLoadingAuto &&
                !routeAutoError &&
                routeAutoFallbacks.length > 0 && (
                  <p
                    style={{
                      fontSize: 11,
                      color: "var(--text-muted)",
                      marginBottom: 8,
                      lineHeight: 1.55,
                    }}
                  >
                    若首选失败或产出空文件，将按顺序自动尝试：{" "}
                    {routeAutoFallbacks.map((id, i) => (
                      <span key={id}>
                        {i > 0 ? " → " : ""}
                        <code style={{ fontSize: 11 }}>{id}</code>
                      </span>
                    ))}
                  </p>
                )}

              <p style={{ fontSize: 11, color: "var(--text-muted)", marginBottom: 10 }}>
                默认展示自动路由；你可直接编辑下方插件链，修改后将按自定义链提交。
              </p>

              <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                  {routeLoadingExplicit && (
                    <p style={{ fontSize: 11, color: "var(--text-muted)" }}>
                      <Loader2
                        size={12}
                        style={{ display: "inline", marginRight: 4, animation: "spin 1s linear infinite" }}
                      />
                      校验自定义链…
                    </p>
                  )}
                  {routeExplicitError && (
                    <p style={{ fontSize: 12, color: "var(--danger)" }}>{routeExplicitError}</p>
                  )}
                  {routeChainIds.map((pid, i) => {
                    const step = displayRouteSteps?.[i];
                    const hopPlugins =
                      step && pluginsForRoute.length
                        ? pluginsSupportingHop(
                            pluginsForRoute,
                            step.in_format,
                            step.out_format
                          )
                        : [];
                    const selectOptions =
                      hopPlugins.length && !hopPlugins.some((p) => p.id === pid)
                        ? [
                            {
                              id: pid,
                              name: "",
                              version: "",
                              author: "",
                              description: "",
                              enabled: true,
                              supported_formats: { input: [], output: [] },
                              status: "active" as const,
                            },
                            ...hopPlugins,
                          ]
                        : hopPlugins;
                    return (
                      <div
                        key={`${i}-${pid}`}
                        style={{
                          display: "flex",
                          flexWrap: "wrap",
                          alignItems: "center",
                          gap: 8,
                          padding: "10px 12px",
                          background: "var(--surface2)",
                          borderRadius: 6,
                          border: "1px solid var(--border)",
                        }}
                      >
                        <span
                          style={{
                            fontSize: 12,
                            fontWeight: 600,
                            color: "var(--accent)",
                            minWidth: 22,
                          }}
                        >
                          {i + 1}
                        </span>
                        {selectOptions.length > 0 ? (
                          <select
                            value={pid}
                            onChange={(e) => replaceChainStep(i, e.target.value)}
                            style={{
                              flex: "1 1 200px",
                              minWidth: 160,
                              padding: "6px 8px",
                              borderRadius: 6,
                              border: "1px solid var(--border)",
                              background: "var(--surface)",
                              color: "var(--text)",
                              fontSize: 13,
                            }}
                          >
                            {selectOptions.map((p) => {
                              const inHop = hopPlugins.some((x) => x.id === p.id);
                              return (
                                <option key={p.id} value={p.id}>
                                  {p.id}
                                  {inHop && p.name && p.name !== p.id ? ` — ${p.name}` : ""}
                                  {!inHop ? "（当前，请改选为兼容本跳的插件）" : ""}
                                </option>
                              );
                            })}
                          </select>
                        ) : (
                          <code style={{ fontSize: 12, flex: 1 }}>{pid}</code>
                        )}
                        {step && (
                          <span style={{ fontSize: 11, color: "var(--text-muted)", flex: "1 1 100%" }}>
                            {step.in_format} → {step.out_format}
                          </span>
                        )}
                        <div style={{ display: "flex", gap: 4, marginLeft: "auto" }}>
                          <button
                            type="button"
                            className="btn-ghost"
                            style={{ padding: "4px 6px" }}
                            title="上移"
                            disabled={i === 0}
                            onClick={() => moveChainStep(i, -1)}
                          >
                            <ChevronUp size={16} />
                          </button>
                          <button
                            type="button"
                            className="btn-ghost"
                            style={{ padding: "4px 6px" }}
                            title="下移"
                            disabled={i === routeChainIds.length - 1}
                            onClick={() => moveChainStep(i, 1)}
                          >
                            <ChevronDown size={16} />
                          </button>
                        </div>
                      </div>
                    );
                  })}
                  {!routeChainIds.length && (
                    <p style={{ fontSize: 11, color: "var(--danger)" }}>插件链为空，请等待自动路由解析成功。</p>
                  )}
                </div>
            </div>
          </div>
        )}
      </div>

      {error && (
        <ErrorBanner
          error={{ summary: error.summary, details: error.details }}
        />
      )}

      {error?.noRouteCandidates && error.noRouteCandidates.length > 0 && (
        <div
          className="card"
          style={{
            padding: "14px 16px",
            border: "1px solid var(--border)",
            background: "var(--surface2)",
          }}
        >
          <p style={{ fontSize: 13, color: "var(--text)", marginBottom: 10 }}>
            多个插件均可处理该转换，请选一个插件后重试（FR-014）：
          </p>
          <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
            {error.noRouteCandidates.map((id) => (
              <button
                key={id}
                type="button"
                className="btn-primary"
                style={{ fontSize: 13, padding: "8px 14px" }}
                disabled={loading}
                onClick={() => void convertFiles(JSON.stringify([id]))}
              >
                使用「{id}」
              </button>
            ))}
          </div>
        </div>
      )}

      <button
        className="btn-primary"
        style={{ padding: "10px 24px", fontSize: 14, fontWeight: 600 }}
        onClick={handleConvert}
        disabled={!queue.length || loading || routeChainInvalid}
        title={
          routeChainInvalid
            ? "请在高级选项中修正插件链（或取消「使用下方插件链」）"
            : undefined
        }
      >
        {loading ? (
          <>
            <Loader2 size={15} style={{ display: "inline", marginRight: 6, animation: "spin 1s linear infinite" }} />
            转换中…
          </>
        ) : (
          <>
            <Zap size={15} style={{ display: "inline", marginRight: 6 }} />
            开始转换 {queue.length > 0 ? `(${queue.length} 个文件)` : ""}
          </>
        )}
      </button>
    </div>
  );
}

// ─── TaskItem ─────────────────────────────────────────────────────────────────

function canDeleteTask(task: Task): boolean {
  return (
    task.status === "completed" ||
    task.status === "failed" ||
    task.status === "cancelled"
  );
}

function TaskItem({
  task,
  onRefresh,
  onError,
}: {
  task: Task;
  onRefresh: () => void;
  onError?: (e: unknown) => void;
}) {
  const isActive = task.status === "processing" || task.status === "pending";
  const [deleting, setDeleting] = useState(false);
  const fileLabel =
    task.input_filename_hint?.trim() ||
    (task.input_format ? `*.${task.input_format}` : null) ||
    "未知文件";

  const handleDownload = async () => {
    try {
      await api.downloadTaskResult(
        task.task_id,
        task.output_format,
        task.result_download_filename
      );
    } catch (e: unknown) {
      onError?.(e);
    }
  };

  return (
    <div className="card" style={{ padding: "14px 18px" }}>
      <div style={{ display: "flex", alignItems: "flex-start", gap: 12, marginBottom: 8 }}>
        <FileText
          size={18}
          style={{ color: "var(--accent)", flexShrink: 0, marginTop: 2 }}
          aria-hidden
        />
        <div style={{ flex: 1, minWidth: 0 }}>
          <div
            style={{
              fontSize: 14,
              fontWeight: 600,
              color: "var(--text)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
              lineHeight: 1.35,
            }}
            title={fileLabel}
          >
            {fileLabel}
          </div>
          <div style={{ fontSize: 12, color: "var(--text-muted)", marginTop: 4 }}>
            输出 {task.output_format.toUpperCase()}
            {task.input_format ? ` · 输入格式 ${task.input_format}` : ""}
          </div>
        </div>
        {statusTag(task.status)}
        <span style={{ fontSize: 11, color: "var(--text-muted)" }}>
          {timeAgo(task.created_at)}
        </span>
      </div>

      {(isActive || task.status === "completed") && (
        <div style={{ marginBottom: 10 }}>
          <div className="progress-bar">
            <div
              className={`progress-fill${task.status === "completed" ? " complete" : task.status === "failed" ? " failed" : ""}`}
              style={{ width: `${task.progress}%` }}
            />
          </div>
        </div>
      )}

      {task.plugin_chain.length > 0 && (
        <div style={{ marginBottom: 8 }}>
          <div
            style={{
              fontSize: 12,
              fontWeight: 600,
              color: "var(--text)",
              marginBottom: 6,
            }}
          >
            插件链
          </div>
          <div style={{ fontSize: 11, color: "var(--text-muted)", marginBottom: 6 }}>
            按序号依次执行：
          </div>
          <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
            {task.plugin_chain.map((p, i) => (
              <span key={i} className="tag tag-muted" style={{ fontSize: 11 }}>
                {i + 1}. {p.plugin_id}
                {p.version !== "unknown" ? ` v${p.version}` : ""}
              </span>
            ))}
          </div>
        </div>
      )}

      {task.error && (
        <div style={{ fontSize: 12, color: "var(--danger)", background: "#7f1d1d15", padding: "8px 12px", borderRadius: 6, marginBottom: 8 }}>
          <strong style={{ marginRight: 6 }}>{task.error.code}</strong>
          {task.error.message}
        </div>
      )}

      <div style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
        {isActive && (
          <button className="btn-ghost" onClick={onRefresh} style={{ padding: "4px 10px" }}>
            <RefreshCw size={13} style={{ display: "inline", marginRight: 4 }} />
            刷新
          </button>
        )}
        {task.status === "completed" && (
          <button className="btn-primary" onClick={handleDownload} style={{ padding: "4px 12px" }}>
            <Download size={13} style={{ display: "inline", marginRight: 4 }} />
            下载结果
          </button>
        )}
        {isActive && (
          <button
            className="btn-danger"
            style={{ padding: "4px 10px" }}
            onClick={() => api.cancelTask(task.task_id).then(onRefresh)}
          >
            <X size={13} style={{ display: "inline", marginRight: 4 }} />
            取消
          </button>
        )}
        {canDeleteTask(task) && (
          <button
            type="button"
            className="btn-ghost"
            style={{ padding: "4px 10px", color: "var(--danger)" }}
            disabled={deleting}
            title="从列表中移除并删除本机缓存的结果文件"
            onClick={() => {
              if (
                !window.confirm(
                  "确定删除该任务记录并清理本机结果缓存？此操作不可恢复。"
                )
              ) {
                return;
              }
              setDeleting(true);
              void api
                .deleteTask(task.task_id)
                .then(onRefresh)
                .catch((e: unknown) => onError?.(e))
                .finally(() => setDeleting(false));
            }}
          >
            <Trash2 size={13} style={{ display: "inline", marginRight: 4 }} />
            {deleting ? "删除中…" : "删除"}
          </button>
        )}
      </div>
    </div>
  );
}

// ─── TasksTab ─────────────────────────────────────────────────────────────────

function TasksTab({ newTaskIds }: { newTaskIds: string[] }) {
  const [tasks, setTasks] = useState<Task[]>([]);
  const [loading, setLoading] = useState(false);
  const [listError, setListError] = useState<{ summary: string; details?: string } | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const { tasks } = await api.listTasks();
      setTasks(tasks);
      setListError(null);
    } catch (e: unknown) {
      setListError(formatErrorForUi(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh, newTaskIds.length]);

  // 自动轮询进行中的任务
  useEffect(() => {
    const hasActive = tasks.some(
      (t) => t.status === "processing" || t.status === "pending"
    );
    if (!hasActive) return;
    const timer = setInterval(refresh, 1500);
    return () => clearInterval(timer);
  }, [tasks, refresh]);

  if (loading && !tasks.length && !listError) {
    return (
      <div style={{ textAlign: "center", padding: "60px 0", color: "var(--text-muted)" }}>
        <Loader2 size={40} style={{ marginBottom: 12, animation: "spin 1s linear infinite" }} />
        <p>正在加载任务列表…</p>
      </div>
    );
  }

  if (!tasks.length && !listError) {
    return (
      <div style={{ textAlign: "center", padding: "60px 0", color: "var(--text-muted)" }}>
        <Clock size={40} style={{ marginBottom: 12, opacity: 0.4 }} />
        <p>暂无转换任务</p>
      </div>
    );
  }

  const finishedCount = tasks.filter(canDeleteTask).length;

  const handleClearFinished = async () => {
    if (finishedCount === 0) return;
    if (
      !window.confirm(
        `将删除 ${finishedCount} 条已结束任务（已完成、失败或已取消）的记录，并清理本机结果缓存。进行中的任务不受影响。确定？`
      )
    ) {
      return;
    }
    setLoading(true);
    try {
      await api.clearFinishedTasks();
      setListError(null);
      await refresh();
    } catch (e: unknown) {
      setListError(formatErrorForUi(e));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
      {listError && <ErrorBanner error={listError} />}
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", flexWrap: "wrap", gap: 8 }}>
        <span style={{ color: "var(--text-muted)", fontSize: 13 }}>
          共 {tasks.length} 个任务
          {finishedCount > 0 && (
            <span style={{ marginLeft: 8 }}>· 可清理 {finishedCount} 条已结束</span>
          )}
        </span>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          {finishedCount > 0 && (
            <button
              type="button"
              className="btn-ghost"
              onClick={() => void handleClearFinished()}
              disabled={loading}
              style={{ padding: "5px 12px", color: "var(--danger)" }}
            >
              <Trash2 size={13} style={{ display: "inline", marginRight: 5 }} />
              清空已结束任务
            </button>
          )}
          <button className="btn-ghost" onClick={refresh} disabled={loading} style={{ padding: "5px 12px" }}>
            <RefreshCw size={13} style={{ display: "inline", marginRight: 5 }} />
            刷新
          </button>
        </div>
      </div>
      {tasks.map((task) => (
        <TaskItem
          key={task.task_id}
          task={task}
          onRefresh={refresh}
          onError={(e) => setListError(formatErrorForUi(e))}
        />
      ))}
    </div>
  );
}

// ─── PluginsTab ───────────────────────────────────────────────────────────────

function PluginsTab() {
  const [plugins, setPlugins] = useState<Plugin[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<{ summary: string; details?: string } | null>(null);
  const [toggleError, setToggleError] = useState<string | null>(null);
  const [testBusyId, setTestBusyId] = useState<string | null>(null);
  const [bulkTesting, setBulkTesting] = useState(false);
  const [pluginTestById, setPluginTestById] = useState<Record<string, PluginSmokeTestResult>>({});
  const testBusy = bulkTesting || testBusyId !== null;

  const load = async () => {
    setLoading(true);
    setError(null);
    try {
      const { plugins } = await api.listPlugins();
      setPlugins(plugins);
    } catch (e: unknown) {
      setError(formatErrorForUi(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, []);

  const toggle = async (id: string, enabled: boolean) => {
    setToggleError(null);
    try {
      await api.setPluginEnabled(id, !enabled);
      setPlugins((prev) =>
        prev.map((p) => (p.id === id ? { ...p, enabled: !enabled } : p))
      );
    } catch (e: unknown) {
      setToggleError(formatErrorForUi(e).summary);
    }
  };

  const probeOne = async (id: string) => {
    setTestBusyId(id);
    try {
      const r = await api.testPlugin(id);
      setPluginTestById((m) => ({ ...m, [id]: r }));
    } catch (e: unknown) {
      setPluginTestById((m) => ({
        ...m,
        [id]: {
          depth: "deep",
          ok: false,
          plugin_id: id,
          runtime_type: "unknown",
          message: formatErrorForUi(e).summary,
        },
      }));
    } finally {
      setTestBusyId(null);
    }
  };

  const probeAll = async () => {
    setBulkTesting(true);
    const next: Record<string, PluginSmokeTestResult> = {};
    try {
      for (const p of plugins) {
        setTestBusyId(p.id);
        try {
          next[p.id] = await api.testPlugin(p.id);
        } catch (e: unknown) {
          next[p.id] = {
            depth: "deep",
            ok: false,
            plugin_id: p.id,
            runtime_type: "unknown",
            message: formatErrorForUi(e).summary,
          };
        }
      }
      setPluginTestById((prev) => ({ ...prev, ...next }));
    } finally {
      setTestBusyId(null);
      setBulkTesting(false);
    }
  };

  if (loading) {
    return (
      <div style={{ textAlign: "center", padding: "40px", color: "var(--text-muted)" }}>
        <Loader2 size={28} style={{ animation: "spin 1s linear infinite" }} />
      </div>
    );
  }

  if (error) {
    return (
      <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        <ErrorBanner error={error} />
        <div>
          <button className="btn-ghost" type="button" onClick={() => void load()} style={{ padding: "6px 14px" }}>
            重试
          </button>
        </div>
      </div>
    );
  }

  if (!plugins.length) {
    return (
      <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        <div style={{ textAlign: "center", padding: "40px 0", color: "var(--text-muted)" }}>
          <Puzzle size={40} style={{ marginBottom: 12, opacity: 0.4 }} />
          <p>未发现任何插件</p>
          <p style={{ fontSize: 12, marginTop: 8, maxWidth: 420, marginLeft: "auto", marginRight: "auto", lineHeight: 1.5 }}>
            仅使用随应用内置的插件；当前版本不再支持从本地 zip 安装第三方扩展。
          </p>
        </div>
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
      <div
        className="card"
        style={{
          padding: "12px 16px",
          display: "flex",
          flexWrap: "wrap",
          alignItems: "center",
          gap: 12,
          justifyContent: "space-between",
        }}
      >
        <div style={{ flex: "1 1 240px" }}>
          <div style={{ fontWeight: 600, marginBottom: 4 }}>插件自检</div>
          <p style={{ fontSize: 12, color: "var(--text-muted)", margin: 0, lineHeight: 1.5 }}>
            在临时目录写入最小样例，走与正式任务相同的 worker 转换路径；Docling 等首次可能较慢。
          </p>
        </div>
        <button
          type="button"
          className="btn-primary"
          style={{ padding: "8px 16px", fontSize: 13, flexShrink: 0 }}
          disabled={testBusy}
          onClick={() => void probeAll()}
        >
          {bulkTesting ? (
            <>
              <Loader2 size={14} style={{ display: "inline", marginRight: 6, animation: "spin 1s linear infinite" }} />
              自检中…
            </>
          ) : (
            <>
              <TestTube2 size={14} style={{ display: "inline", marginRight: 6 }} />
              自检全部插件
            </>
          )}
        </button>
      </div>
      {toggleError && (
        <div
          style={{
            padding: "10px 14px",
            background: "#7f1d1d15",
            borderRadius: 6,
            color: "var(--danger)",
            fontSize: 12,
          }}
        >
          {toggleError}
        </div>
      )}
      {plugins.map((p) => (
        <div
          key={p.id}
          className="card"
          style={{ opacity: p.enabled ? 1 : 0.6, transition: "opacity 0.2s" }}
        >
          <div style={{ display: "flex", alignItems: "flex-start", gap: 12 }}>
            <Puzzle size={20} style={{ color: "var(--accent)", marginTop: 2, flexShrink: 0 }} />
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 4 }}>
                <span style={{ fontWeight: 600 }}>{p.name}</span>
                <span className="tag tag-muted">v{p.version}</span>
                <span className={`tag ${p.enabled ? "tag-success" : "tag-muted"}`}>
                  {p.enabled ? "启用" : "禁用"}
                </span>
              </div>
              <p style={{ color: "var(--text-muted)", fontSize: 12, marginBottom: 8 }}>
                {p.description}
              </p>
              <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
                <div>
                  <span style={{ fontSize: 11, color: "var(--text-muted)" }}>输入：</span>
                  {p.supported_formats.input.slice(0, 8).map((f) => (
                    <span key={f} className="tag tag-info" style={{ marginRight: 4, fontSize: 11 }}>
                      {f}
                    </span>
                  ))}
                  {p.supported_formats.input.length > 8 && (
                    <span className="tag tag-muted" style={{ fontSize: 11 }}>
                      +{p.supported_formats.input.length - 8}
                    </span>
                  )}
                </div>
                <div>
                  <span style={{ fontSize: 11, color: "var(--text-muted)" }}>输出：</span>
                  {p.supported_formats.output.map((f) => (
                    <span key={f} className="tag tag-success" style={{ marginRight: 4, fontSize: 11 }}>
                      {f}
                    </span>
                  ))}
                </div>
              </div>
            </div>
            <div style={{ display: "flex", flexDirection: "column", alignItems: "flex-end", gap: 8 }}>
              <button
                type="button"
                className="btn-ghost"
                style={{ padding: "6px 12px", fontSize: 12 }}
                disabled={testBusy}
                title="最小样例走与任务相同的转换路径"
                onClick={() => void probeOne(p.id)}
              >
                {testBusyId === p.id ? (
                  <Loader2 size={14} style={{ animation: "spin 1s linear infinite" }} />
                ) : (
                  <>
                    <TestTube2 size={14} style={{ display: "inline", marginRight: 6 }} />
                    自检
                  </>
                )}
              </button>
              <button
                style={{ background: "none", padding: "4px" }}
                onClick={() => toggle(p.id, p.enabled)}
                title={p.enabled ? "禁用插件" : "启用插件"}
              >
                {p.enabled ? (
                  <ToggleRight size={24} style={{ color: "var(--accent)" }} />
                ) : (
                  <ToggleLeft size={24} style={{ color: "var(--text-muted)" }} />
                )}
              </button>
            </div>
          </div>
          {pluginTestById[p.id] && (
            <div
              style={{
                marginTop: 10,
                padding: "10px 12px",
                borderRadius: 6,
                fontSize: 12,
                background: pluginTestById[p.id].ok ? "#14532d18" : "#7f1d1d15",
                color: pluginTestById[p.id].ok ? "var(--text)" : "var(--danger)",
                border: `1px solid ${pluginTestById[p.id].ok ? "var(--border)" : "transparent"}`,
              }}
            >
              <strong style={{ marginRight: 8 }}>{pluginTestById[p.id].ok ? "通过" : "失败"}</strong>
              <span style={{ color: pluginTestById[p.id].ok ? "var(--text-muted)" : "inherit" }}>
                {pluginTestById[p.id].message}
              </span>
              {pluginTestById[p.id].detail != null && (
                <pre
                  style={{
                    marginTop: 8,
                    fontSize: 11,
                    overflow: "auto",
                    maxHeight: 120,
                    color: "var(--text-muted)",
                  }}
                >
                  {typeof pluginTestById[p.id].detail === "string"
                    ? (pluginTestById[p.id].detail as string)
                    : JSON.stringify(pluginTestById[p.id].detail, null, 2)}
                </pre>
              )}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}

// ─── AboutTab ─────────────────────────────────────────────────────────────────

function AboutTab() {
  const [diagLoading, setDiagLoading] = useState(true);
  const [openHint, setOpenHint] = useState<string | null>(null);
  const [copyHint, setCopyHint] = useState<string | null>(null);
  const [diag, setDiag] = useState<{
    dataRoot?: string;
    logsDir?: string;
    bindPort?: number;
    err?: string;
    healthFull?: HealthResponse;
    tools?: ToolsStatusResponse;
    toolsErr?: string;
  }>({});

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const [hRes, tRes] = await Promise.allSettled([
        api.health(),
        api.toolsStatus(),
      ]);
      if (cancelled) return;
      const next: {
        dataRoot?: string;
        logsDir?: string;
        bindPort?: number;
        err?: string;
        healthFull?: HealthResponse;
        tools?: ToolsStatusResponse;
        toolsErr?: string;
      } = {};
      if (hRes.status === "fulfilled") {
        const h = hRes.value;
        next.dataRoot = h.data_root;
        next.logsDir = h.logs_directory;
        next.bindPort = h.bind_port;
        next.healthFull = h;
      } else {
        next.err = formatErrorForUi(hRes.reason).summary;
      }
      if (tRes.status === "fulfilled") {
        next.tools = tRes.value;
      } else {
        next.toolsErr = formatErrorForUi(tRes.reason).summary;
      }
      setDiag(next);
      setDiagLoading(false);
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: 20,
        color: "var(--text, #e2e8f0)",
      }}
    >
      <div
        className="card"
        style={{ padding: "20px 22px", color: "var(--text, #e2e8f0)" }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 16 }}>
          <Info size={22} style={{ color: "var(--accent)", flexShrink: 0 }} />
          <div>
            <h2 style={{ fontSize: 16, fontWeight: 700, margin: 0, color: "var(--text)" }}>
              关于文档转换工具
            </h2>
            <p style={{ fontSize: 12, color: "var(--text-muted, #64748b)", margin: "4px 0 0" }}>
              版本 {APP_VERSION}
            </p>
          </div>
        </div>
        {diagLoading && (
          <p style={{ fontSize: 13, color: "var(--text-muted, #64748b)", margin: "0 0 12px" }}>
            正在加载诊断信息…
          </p>
        )}
        {(diag.dataRoot || diag.err) && (
          <section style={{ marginBottom: 18 }}>
            <h3 style={{ fontSize: 13, fontWeight: 600, marginBottom: 8, color: "var(--text)" }}>
              本机数据与日志（诊断）
            </h3>
            {diag.err ? (
              <p style={{ fontSize: 12, color: "var(--danger)", margin: 0 }}>{diag.err}</p>
            ) : (
              <div
                style={{
                  fontSize: 12,
                  color: "var(--text-muted)",
                  lineHeight: 1.6,
                  fontFamily: "ui-monospace, monospace",
                  wordBreak: "break-all",
                }}
              >
                {diag.bindPort !== undefined && (
                  <div style={{ marginBottom: 6 }}>
                    Core 端口：<span style={{ color: "var(--text)" }}>{diag.bindPort}</span>
                  </div>
                )}
                {diag.dataRoot && (
                  <div style={{ marginBottom: 6 }}>
                    数据目录：<span style={{ color: "var(--text)" }}>{diag.dataRoot}</span>
                  </div>
                )}
                {diag.logsDir && (
                  <div style={{ marginBottom: 8 }}>
                    日志目录：<span style={{ color: "var(--text)" }}>{diag.logsDir}</span>
                  </div>
                )}
                {(diag.dataRoot || diag.logsDir) && (
                  <div style={{ display: "flex", flexWrap: "wrap", gap: 8, marginTop: 4 }}>
                    {diag.dataRoot && (
                      <button
                        type="button"
                        className="btn-ghost"
                        style={{
                          fontSize: 12,
                          padding: "6px 10px",
                          display: "inline-flex",
                          alignItems: "center",
                          gap: 6,
                        }}
                        onClick={async () => {
                          setOpenHint(null);
                          const r = await openLocalPath(diag.dataRoot!);
                          setOpenHint(
                            r.ok
                              ? r.hint ?? null
                              : r.hint ?? "打开失败"
                          );
                        }}
                      >
                        <FolderOpen size={14} />
                        打开数据目录
                      </button>
                    )}
                    {diag.logsDir && (
                      <button
                        type="button"
                        className="btn-ghost"
                        style={{
                          fontSize: 12,
                          padding: "6px 10px",
                          display: "inline-flex",
                          alignItems: "center",
                          gap: 6,
                        }}
                        onClick={async () => {
                          setOpenHint(null);
                          const r = await openLocalPath(diag.logsDir!);
                          setOpenHint(
                            r.ok
                              ? r.hint ?? null
                              : r.hint ?? "打开失败"
                          );
                        }}
                      >
                        <FolderOpen size={14} />
                        打开日志目录
                      </button>
                    )}
                  </div>
                )}
                {openHint && (
                  <p
                    style={{
                      fontSize: 11,
                      color: openHint.includes("失败") || openHint.includes("无法")
                        ? "var(--danger)"
                        : "var(--text-muted)",
                      margin: "8px 0 0",
                      fontFamily: "inherit",
                    }}
                  >
                    {openHint}
                  </p>
                )}
              </div>
            )}
          </section>
        )}
        {(diag.tools || diag.toolsErr) && (
          <section style={{ marginBottom: 18 }}>
            <div
              style={{
                display: "flex",
                alignItems: "center",
                justifyContent: "space-between",
                gap: 12,
                flexWrap: "wrap",
                marginBottom: 8,
              }}
            >
              <h3 style={{ fontSize: 13, fontWeight: 600, margin: 0, color: "var(--text)" }}>
                Core 与运行时配额
              </h3>
              {(diag.healthFull || diag.tools) && (
                <button
                  type="button"
                  className="btn-ghost"
                  style={{
                    fontSize: 12,
                    padding: "6px 10px",
                    display: "inline-flex",
                    alignItems: "center",
                    gap: 6,
                  }}
                  onClick={async () => {
                    setCopyHint(null);
                    try {
                      const payload = {
                        app_version: APP_VERSION,
                        generated_at: new Date().toISOString(),
                        health: diag.healthFull ?? null,
                        tools: diag.tools ?? null,
                      };
                      await navigator.clipboard.writeText(
                        JSON.stringify(payload, null, 2)
                      );
                      setCopyHint("已复制到剪贴板（可粘贴到工单或议题）");
                    } catch (e: unknown) {
                      setCopyHint(formatErrorForUi(e).summary);
                    }
                  }}
                >
                  <ClipboardCopy size={14} />
                  复制诊断 JSON
                </button>
              )}
            </div>
            {diag.toolsErr ? (
              <p style={{ fontSize: 12, color: "var(--danger)", margin: 0 }}>{diag.toolsErr}</p>
            ) : diag.tools ? (
              <div
                style={{
                  fontSize: 12,
                  color: "var(--text-muted)",
                  lineHeight: 1.65,
                  fontFamily: "ui-monospace, monospace",
                  wordBreak: "break-all",
                }}
              >
                <div style={{ marginBottom: 6 }}>
                  平台：<span style={{ color: "var(--text)" }}>{diag.tools.platform}</span> /{" "}
                  <span style={{ color: "var(--text)" }}>{diag.tools.arch}</span>
                  {diag.tools.host_api_version && (
                    <>
                      {" "}
                      · host_api <span style={{ color: "var(--text)" }}>{diag.tools.host_api_version}</span>
                    </>
                  )}
                </div>
                <div style={{ marginBottom: 6 }}>
                  并发上限{" "}
                  <span style={{ color: "var(--text)" }}>
                    {diag.tools.core.max_concurrent_tasks}
                  </span>
                  ，单文件上限{" "}
                  <span style={{ color: "var(--text)" }}>
                    {formatBytes(diag.tools.core.max_file_size_bytes)}
                  </span>
                  ，任务列表保留{" "}
                  <span style={{ color: "var(--text)" }}>
                    {formatTtlHours(diag.tools.core.task_result_ttl_secs)}
                  </span>
                </div>
                <div style={{ marginBottom: 6 }}>
                  扩展插件目录：
                  <span style={{ color: "var(--text)" }}>{diag.tools.core.plugins_extra_dir}</span>
                </div>
                <div style={{ marginBottom: 6 }}>
                  runtime：
                  <span style={{ color: "var(--text)" }}>{diag.tools.core.runtime_dir}</span>
                </div>
                <div>
                  已注册插件{" "}
                  <span style={{ color: "var(--text)" }}>{diag.tools.plugins.length}</span> 个
                </div>
              </div>
            ) : null}
            {copyHint && (
              <p
                style={{
                  fontSize: 11,
                  color: copyHint.includes("失败") || copyHint.includes("无法") || copyHint.includes("拒绝")
                    ? "var(--danger)"
                    : "var(--text-muted)",
                  margin: "8px 0 0",
                  fontFamily: "inherit",
                }}
              >
                {copyHint}
              </p>
            )}
          </section>
        )}
        <section style={{ marginBottom: 18 }}>
          <h3 style={{ fontSize: 13, fontWeight: 600, marginBottom: 8, color: "var(--text)" }}>
            本地与隐私
          </h3>
          <p style={{ fontSize: 13, color: "var(--text-muted)", lineHeight: 1.65, margin: 0 }}>
            转换任务由本机 Core 服务处理，默认不会将您的文档静默上传至公网。实际能力取决于已启用的插件及其依赖（部分格式或模型可能使用本机以外的资源）；请以各插件说明与当前版本发布说明为准。
          </p>
        </section>
        <section style={{ marginBottom: 18 }}>
          <h3 style={{ fontSize: 13, fontWeight: 600, marginBottom: 8, color: "var(--text)" }}>
            Markdown 与输出格式
          </h3>
          <p style={{ fontSize: 13, color: "var(--text-muted)", lineHeight: 1.65, margin: 0 }}>
            选择 Markdown 输出时，以 GFM 常见子集为参考边界；表格、代码块与公式的保留程度随源文档与插件链而异。HTML、纯文本等输出由对应插件与 Pandoc 能力共同决定。
          </p>
        </section>
        <section>
          <h3 style={{ fontSize: 13, fontWeight: 600, marginBottom: 8, color: "var(--text)" }}>
            运行时与诊断
          </h3>
          <p style={{ fontSize: 13, color: "var(--text-muted)", lineHeight: 1.65, margin: 0 }}>
            窗口底部状态栏显示 Core 连接状态及当前选用的 Python、Pandoc 路径。桌面版默认将 Core 固定在本机 17300 端口，与生产构建的前端一致。高级用户可通过环境变量{" "}
            <code style={{ fontSize: 12, color: "var(--accent)" }}>DOCCONVERT_DATA_DIR</code>、
            <code style={{ fontSize: 12, color: "var(--accent)" }}> DOCCONVERT_PYTHON</code>、
            <code style={{ fontSize: 12, color: "var(--accent)" }}> DOCCONVERT_PANDOC</code>{" "}
            覆盖数据目录与可执行文件路径；发布安装包前可在本机执行 <code style={{ fontSize: 12 }}>npm run bundle-python</code>{" "}
            将 <code style={{ fontSize: 12 }}>python/.venv</code> 打入随包资源。
          </p>
        </section>
      </div>
    </div>
  );
}

// ─── StatusBar ────────────────────────────────────────────────────────────────

function StatusBar() {
  const [health, setHealth] = useState<{
    ok: boolean;
    uptime?: number;
    python?: string;
    pandoc?: string;
    dataRoot?: string;
    bindPort?: number;
    lastError?: string;
  } | null>(null);

  useEffect(() => {
    const check = async () => {
      try {
        const h = await api.health();
        setHealth({
          ok: h.status === "ok",
          uptime: h.uptime_ms,
          python: h.python_executable,
          pandoc: h.pandoc_executable,
          dataRoot: h.data_root,
          bindPort: h.bind_port,
        });
      } catch (e: unknown) {
        setHealth({ ok: false, lastError: formatErrorForUi(e).summary });
      }
    };
    check();
    const t = setInterval(check, 10000);
    return () => clearInterval(t);
  }, []);

  const shortPath = (p: string | undefined) => {
    if (!p) return "";
    return p.length > 42 ? `…${p.slice(-40)}` : p;
  };

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: 4,
        padding: "8px 16px",
        borderTop: "1px solid var(--border)",
        fontSize: 11,
        color: "var(--text-muted)",
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <Activity
          size={13}
          style={{ color: health?.ok ? "var(--success)" : "var(--danger)" }}
        />
        <span>
          Core {health === null ? "检测中…" : health.ok ? "运行中" : "未连接"}
        </span>
        {health?.ok && health.bindPort !== undefined && (
          <span style={{ color: "var(--text-muted)" }}>:{health.bindPort}</span>
        )}
        {health?.ok && health.uptime !== undefined && (
          <span>· {Math.floor(health.uptime / 1000)}s</span>
        )}
      </div>
      {health && !health.ok && health.lastError && (
        <div style={{ color: "var(--danger)", lineHeight: 1.35 }} title={health.lastError}>
          {health.lastError.length > 120 ? `${health.lastError.slice(0, 118)}…` : health.lastError}
        </div>
      )}
      {health?.ok && (health.python || health.pandoc || health.dataRoot) && (
        <div
          style={{
            fontFamily: "ui-monospace, monospace",
            lineHeight: 1.35,
            opacity: 0.9,
          }}
          title={[health.dataRoot, health.python, health.pandoc].filter(Boolean).join("\n")}
        >
          {health.dataRoot && <div>Data: {shortPath(health.dataRoot)}</div>}
          {health.python && <div>Python: {shortPath(health.python)}</div>}
          {health.pandoc && <div>Pandoc: {shortPath(health.pandoc)}</div>}
        </div>
      )}
    </div>
  );
}

// ─── App ──────────────────────────────────────────────────────────────────────

export default function App() {
  const [tab, setTab] = useState<Tab>("convert");
  const [newTaskIds, setNewTaskIds] = useState<string[]>([]);

  const handleTaskCreated = (id: string) => {
    setNewTaskIds((prev) => [...prev, id]);
    setTab("tasks");
  };

  const tabs: { key: Tab; label: string; icon: React.ReactNode }[] = [
    { key: "convert", label: "转换", icon: <Upload size={16} /> },
    { key: "tasks", label: "任务", icon: <FileText size={16} /> },
    { key: "plugins", label: "插件", icon: <Puzzle size={16} /> },
    { key: "about", label: "关于", icon: <Info size={16} /> },
  ];

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100vh",
        background: "var(--bg)",
      }}
    >
      {/* Header */}
      <header
        style={{
          padding: "16px 24px 0",
          borderBottom: "1px solid var(--border)",
          background: "var(--surface)",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 16 }}>
          <div
            style={{
              width: 32,
              height: 32,
              background: "var(--accent)",
              borderRadius: 8,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
            }}
          >
            <FileText size={18} color="#fff" />
          </div>
          <h1 style={{ fontSize: 16, fontWeight: 700, color: "var(--text)" }}>
            文档转换工具
          </h1>
          <span className="tag tag-muted" style={{ marginLeft: 4 }}>
            MVP {APP_VERSION}
          </span>
        </div>

        {/* Tabs */}
        <nav style={{ display: "flex", gap: 4 }}>
          {tabs.map((t) => (
            <button
              key={t.key}
              onClick={() => setTab(t.key)}
              style={{
                background: tab === t.key ? "var(--surface2)" : "transparent",
                color: tab === t.key ? "var(--text)" : "var(--text-muted)",
                border: "none",
                borderBottom: tab === t.key ? "2px solid var(--accent)" : "2px solid transparent",
                borderRadius: "6px 6px 0 0",
                padding: "8px 16px",
                display: "flex",
                alignItems: "center",
                gap: 7,
                fontSize: 13,
                fontWeight: tab === t.key ? 600 : 400,
                transition: "all 0.15s",
              }}
            >
              {t.icon}
              {t.label}
            </button>
          ))}
        </nav>
      </header>

      {/* Content */}
      <main
        style={{
          flex: 1,
          overflow: "auto",
          padding: "24px",
          maxWidth: 760,
          width: "100%",
          margin: "0 auto",
        }}
      >
        {tab === "convert" && <ConvertTab onTaskCreated={handleTaskCreated} />}
        {tab === "tasks" && <TasksTab newTaskIds={newTaskIds} />}
        {tab === "plugins" && <PluginsTab />}
        {tab === "about" && <AboutTab />}
      </main>

      {/* Status bar */}
      <StatusBar />

      <style>{`
        @keyframes spin {
          from { transform: rotate(0deg); }
          to { transform: rotate(360deg); }
        }
      `}</style>
    </div>
  );
}
