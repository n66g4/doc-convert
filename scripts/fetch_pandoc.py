#!/usr/bin/env python3
"""
从 GitHub 下载当前平台对应的 Pandoc 二进制到 src-tauri/resources/pandoc/，
并校验 SHA256（见 build/pandoc.json）。构建 / 打包前执行：npm run fetch-pandoc
GPL 说明见 resources/pandoc/NOTICE.txt
"""
from __future__ import annotations

import hashlib
import http.client
import json
import platform
import shutil
import stat
import subprocess
import sys
import tempfile
import tarfile
import time
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "build" / "pandoc.json"
OUT_DIR = ROOT / "src-tauri" / "resources" / "pandoc"


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def pick_target_key() -> str:
    s = platform.system()
    m = platform.machine().lower()
    if s == "Darwin":
        arch = "aarch64" if m in ("arm64", "aarch64") else "x86_64"
        return f"macos_{arch}"
    if s == "Windows":
        return "windows_x86_64"
    if s == "Linux":
        arch = "aarch64" if m in ("aarch64", "arm64") else "x86_64"
        return f"linux_{arch}"
    print(f"Unsupported platform: {s} {m}", file=sys.stderr)
    sys.exit(1)


def load_entry() -> dict:
    data = json.loads(MANIFEST.read_text(encoding="utf-8"))
    key = pick_target_key()
    try:
        entry = data["targets"][key]
    except KeyError as e:
        print(f"No pandoc entry for {key} in build/pandoc.json", file=sys.stderr)
        raise SystemExit(1) from e
    entry["_version"] = data.get("version", "?")
    return entry


def download(url: str) -> bytes:
    import urllib.error
    import urllib.request

    print(f"Downloading: {url}")
    req = urllib.request.Request(url, headers={"User-Agent": "DocConvert-fetch-pandoc/1"})
    last_err: BaseException | None = None
    for attempt in range(1, 4):
        try:
            with urllib.request.urlopen(req, timeout=600) as resp:
                chunks: list[bytes] = []
                while True:
                    block = resp.read(1024 * 1024)
                    if not block:
                        break
                    chunks.append(block)
                return b"".join(chunks)
        except (
            urllib.error.URLError,
            http.client.HTTPException,
            http.client.IncompleteRead,
            OSError,
        ) as e:
            last_err = e
            print(f"Download attempt {attempt}/3 failed: {e}", file=sys.stderr)
            if attempt < 3:
                time.sleep(2 * attempt)
    assert last_err is not None
    raise last_err


def find_binary(root: Path, win: bool) -> Path:
    name = "pandoc.exe" if win else "pandoc"
    for p in root.rglob(name):
        if p.is_file():
            return p
    raise FileNotFoundError(f"Could not find {name} under {root}")


def main() -> None:
    if not MANIFEST.is_file():
        print(f"Missing {MANIFEST}", file=sys.stderr)
        sys.exit(1)

    entry = load_entry()
    url = entry["url"]
    expect = entry.get("sha256") or ""
    win = platform.system() == "Windows"
    dst_name = "pandoc.exe" if win else "pandoc"
    dst = OUT_DIR / dst_name
    version_file = OUT_DIR / "VERSION"
    want_ver = entry["_version"]
    if (
        dst.is_file()
        and version_file.is_file()
        and version_file.read_text(encoding="utf-8").strip() == want_ver
    ):
        print(f"Pandoc {want_ver} already present at {dst}, skip download")
        try:
            out = subprocess.run(
                [str(dst), "--version"],
                capture_output=True,
                text=True,
                timeout=30,
                check=True,
            )
            print(out.stdout.splitlines()[0] if out.stdout else "pandoc ok")
        except (subprocess.CalledProcessError, FileNotFoundError, OSError) as e:
            print(f"Warning: could not run pandoc --version: {e}", file=sys.stderr)
        print(f"Using: {dst}")
        return

    data = download(url)
    digest = sha256_bytes(data)
    if expect and digest.lower() != expect.lower():
        print(
            f"SHA256 mismatch: got {digest}, expected {expect}",
            file=sys.stderr,
        )
        sys.exit(1)
    if not expect:
        print(f"Warning: no sha256 in manifest; computed {digest}", file=sys.stderr)

    is_zip = url.endswith(".zip")
    is_tgz = ".tar.gz" in url or url.endswith(".tgz")

    OUT_DIR.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory() as tmp:
        tdir = Path(tmp)
        archive = tdir / ("a.zip" if is_zip else "a.tar.gz")
        archive.write_bytes(data)

        if is_zip:
            with zipfile.ZipFile(archive, "r") as zf:
                zf.extractall(tdir)
        elif is_tgz:
            with tarfile.open(archive, "r:gz") as tf:
                tf.extractall(tdir)
        else:
            print("Unknown archive type", file=sys.stderr)
            sys.exit(1)

        src = find_binary(tdir, win)
        shutil.copy2(src, dst)
        if not win:
            dst.chmod(dst.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

    (OUT_DIR / "VERSION").write_text(entry["_version"] + "\n", encoding="utf-8")

    try:
        out = subprocess.run(
            [str(OUT_DIR / ("pandoc.exe" if win else "pandoc")), "--version"],
            capture_output=True,
            text=True,
            timeout=30,
            check=True,
        )
        print(out.stdout.splitlines()[0] if out.stdout else "pandoc ok")
    except (subprocess.CalledProcessError, FileNotFoundError, OSError) as e:
        print(f"Warning: could not run pandoc --version: {e}", file=sys.stderr)

    print(f"Installed: {OUT_DIR / dst_name}")


if __name__ == "__main__":
    main()
