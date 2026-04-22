"""
pandoc_wrapper/plugin_main.py
Pandoc 包装器的 Python 健康检查入口（runtime.type = "pandoc_wrapper" 不走此路径）。
此文件仅用于 health() 探测：调用 pandoc --version 检查可执行文件是否存在。
与 Core 一致：优先使用环境变量 DOCCONVERT_PANDOC（绝对路径），否则使用 PATH 上的 pandoc。
"""
from __future__ import annotations
import json
import os
import subprocess
import sys


def health(ctx: dict | None = None) -> dict:
    try:
        pandoc_exe = os.environ.get("DOCCONVERT_PANDOC", "pandoc")
        result = subprocess.run(
            [pandoc_exe, "--version"],
            capture_output=True,
            text=True,
            timeout=10,
        )
        version_line = result.stdout.splitlines()[0] if result.stdout else "unknown"
        return {"status": "ok", "pandoc_version": version_line}
    except FileNotFoundError:
        return {
            "status": "error",
            "message": (
                "未找到 pandoc 可执行文件。可设置 DOCCONVERT_PANDOC 为随包/本机 pandoc 路径，"
                "或确保 PATH 中有 pandoc（与 Core 使用的路径一致）。"
            ),
        }
    except Exception as e:
        return {"status": "error", "message": str(e)}


def run(ctx: dict) -> dict:
    """
    pandoc_wrapper runtime 类型由 Rust worker 直接调用 pandoc 二进制，
    此 Python 入口保留以便兼容性检查或未来用途。
    """
    raise RuntimeError("pandoc_wrapper 由 Rust worker 直接调用，不经过 Python 入口")


if __name__ == "__main__":
    req = json.loads(sys.stdin.read())
    method = req.get("method", "health")
    try:
        if method == "health":
            result = health(req.get("params", {}))
        elif method == "convert":
            result = run(req["params"])
        else:
            raise ValueError(f"Unknown method: {method}")
        print(json.dumps({"jsonrpc": "2.0", "id": req.get("id", 1), "result": result}))
    except Exception as exc:
        print(json.dumps({
            "jsonrpc": "2.0",
            "id": req.get("id", 1),
            "error": {"code": -32000, "message": str(exc)},
        }))
