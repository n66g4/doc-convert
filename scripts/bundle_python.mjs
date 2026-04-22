/**
 * 将仓库 python/.venv 复制到 src-tauri/resources/python，供安装包随包。
 * 复制后向该 venv 安装 MarkItDown、Docling（Docling 体积较大）。
 */
import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";
import { spawnSync } from "child_process";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.join(__dirname, "..");
const venv = path.join(root, "python", ".venv");
const dest = path.join(root, "src-tauri", "resources", "python");

const PYPI_INDEX = "https://pypi.org/simple";

function resolvePythonInVenv(venvRoot) {
  if (process.platform === "win32") {
    const p = path.join(venvRoot, "Scripts", "python.exe");
    return fs.existsSync(p) ? p : null;
  }
  for (const name of ["python3", "python"]) {
    const p = path.join(venvRoot, "bin", name);
    if (fs.existsSync(p)) return p;
  }
  return null;
}

/** @returns {string|null} 如 "3.11" */
function venvPythonVersion(venvRoot) {
  const py = resolvePythonInVenv(venvRoot);
  if (!py) return null;
  const r = spawnSync(
    py,
    [
      "-c",
      "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')",
    ],
    { encoding: "utf8" }
  );
  if (r.status !== 0) return null;
  return r.stdout.trim();
}

function parseVersion(v) {
  const [a, b] = v.split(".").map(Number);
  return { major: a, minor: b };
}

function meetsMinPython(versionStr, minMajor, minMinor) {
  const { major, minor } = parseVersion(versionStr);
  return major > minMajor || (major === minMajor && minor >= minMinor);
}

function pipInstallPythonDeps(venvRoot) {
  const py = resolvePythonInVenv(venvRoot);
  if (!py) {
    console.error("bundle-python: 未找到 venv 内 python");
    process.exit(1);
  }
  console.log(
    "bundle-python: pip install markitdown[all] + docling（PyPI 官方索引）→",
    venvRoot
  );
  const r = spawnSync(
    py,
    [
      "-m",
      "pip",
      "install",
      "--upgrade",
      "pip",
      `markitdown[all]>=0.1`,
      "docling>=2.0.0,<3.0",
      "-i",
      PYPI_INDEX,
    ],
    { stdio: "inherit", env: process.env }
  );
  if (r.status !== 0) {
    console.error(
      "bundle-python: pip install 失败。请使用 Python 3.10+ 创建 python/.venv，例如：python3.12 -m venv python/.venv && python/.venv/bin/pip install -e '.[markitdown]'"
    );
    process.exit(1);
  }
}

if (!fs.existsSync(venv)) {
  console.log(
    "bundle-python: 跳过（无 python/.venv）。需要 MarkItDown 时：python3.12 -m venv python/.venv && cd python && .venv/bin/pip install -e '.[markitdown]'"
  );
  process.exit(0);
}

const ver = venvPythonVersion(venv);
if (!ver) {
  console.error("bundle-python: 无法检测 venv 的 Python 版本");
  process.exit(1);
}
if (!meetsMinPython(ver, 3, 10)) {
  console.error(
    `bundle-python: 当前 venv 为 Python ${ver}，Microsoft MarkItDown 需要 ≥3.10。请删除 python/.venv 后重建：\n` +
      `  python3.12 -m venv python/.venv && cd python && .venv/bin/pip install -e ".[markitdown]"`
  );
  process.exit(1);
}

fs.rmSync(dest, { recursive: true, force: true });
// venv 的 python3 常是指向 Homebrew 的符号链接；Node fs.cpSync 即使 dereference 也可能保留外链。
// 使用 cp -RL 彻底跟随链接复制真实解释器，否则 .app 内断链 → Core 退回 PATH 上 python3（常见 3.9）→ ImportError: markitdown
if (process.platform === "win32") {
  fs.cpSync(venv, dest, { recursive: true, dereference: true });
} else {
  const r = spawnSync("cp", ["-RL", venv, dest], {
    encoding: "utf8",
    stdio: ["ignore", "inherit", "inherit"],
  });
  if (r.status !== 0) {
    console.error("bundle-python: cp -RL 失败，无法生成可移植 venv");
    process.exit(1);
  }
}

pipInstallPythonDeps(dest);

fs.writeFileSync(
  path.join(dest, "README.txt"),
  `随包 Python（Python ${ver}）：由 npm run bundle-python 从 python/.venv 复制，并已 pip install markitdown[all]、docling（PyPI）。\n`,
  "utf8"
);
console.log("bundle-python: 已复制并安装 markitdown + docling →", dest);
