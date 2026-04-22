"""
markitdown_adapter/plugin_main.py
薄适配层：将 JSON-RPC ctx 映射为 markitdown 官方 API 调用。
"""
from __future__ import annotations
import os
import sys
from pathlib import Path


def health(ctx: dict | None = None) -> dict:
    """供 Core/UI 轻量自检：仅检测包是否可见，不加载转换栈（避免误报与耗时）。"""
    import importlib.util

    if importlib.util.find_spec("markitdown") is None:
        return {"status": "error", "message": "未安装 markitdown 包"}
    return {
        "status": "ok",
        "message": "MarkItDown 包已安装（完整链路请用深度自检）",
    }


def run(ctx: dict) -> dict:
    """
    JSON-RPC 入口。ctx 字段：
      input_path, output_path, in_format, out_format, temp_dir
    """
    input_path = Path(ctx["input_path"])
    output_path = Path(ctx["output_path"])
    out_format = ctx.get("out_format", "markdown")
    input_sz = input_path.stat().st_size if input_path.is_file() else 0

    if out_format not in ("markdown",):
        raise ValueError(f"markitdown_adapter: unsupported output format '{out_format}'")

    try:
        from markitdown import MarkItDown
    except ImportError as e:
        raise RuntimeError(
            "未安装 Microsoft MarkItDown 或 Python 版本过低（需 ≥3.10）。"
            "请使用 Python 3.11+ 的 venv：pip install 'markitdown[all]>=0.1'，并设置环境变量 DOCCONVERT_PYTHON 指向该解释器；"
            "或按 README「随包 Python 与 MarkItDown」用 python3.12 重建 python/.venv 后重新 npm run bundle-python 再打包。"
        ) from e

    md = MarkItDown()
    result = md.convert(str(input_path))
    text = result.text_content or ""

    output_path.parent.mkdir(parents=True, exist_ok=True)
    if not text and input_sz > 0:
        raise ValueError(
            "MarkItDown 未提取到文本，但输入文件非空。请确认依赖齐全（如 PDF 引擎），"
            "或换用 Docling 等插件对比；详见 Core 日志。"
        )
    output_path.write_text(text, encoding="utf-8")

    return {
        "status": "ok",
        "output_path": str(output_path),
        "char_count": len(text),
    }


if __name__ == "__main__":
    # 独立测试用
    import json
    req = json.loads(sys.stdin.read())
    try:
        result = run(req["params"])
        print(json.dumps({"jsonrpc": "2.0", "id": req.get("id", 1), "result": result}))
    except Exception as exc:
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": req.get("id", 1),
            "error": {"code": -32000, "message": str(exc)},
        }))
