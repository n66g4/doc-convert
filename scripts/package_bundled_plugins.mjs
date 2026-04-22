/**
 * 将仓库 plugins/bundled/<id>/ 各自打成独立 zip，与主程序安装包分开分发。
 * 安装：应用「插件」页 → 从 zip 安装 → 填写该 zip 的绝对路径。
 *
 * 依赖系统 tar（macOS / Linux / Windows 10+ 自带 bsdtar 均支持 -acf 写 zip）。
 */
import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";
import { spawnSync } from "child_process";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.join(__dirname, "..");
const bundledRoot = path.join(root, "plugins", "bundled");
const outDir = path.join(root, "dist-plugins");

function zipPluginDir(pluginId) {
  const src = path.join(bundledRoot, pluginId);
  const toml = path.join(src, "plugin.toml");
  if (!fs.existsSync(toml)) {
    return false;
  }
  fs.mkdirSync(outDir, { recursive: true });
  const outZip = path.join(outDir, `${pluginId}.zip`);
  if (fs.existsSync(outZip)) {
    fs.rmSync(outZip);
  }
  const r = spawnSync("tar", ["-acf", outZip, "-C", src, "."], {
    stdio: "inherit",
    encoding: "utf8",
  });
  if (r.status !== 0) {
    console.error(`package-plugins: 打包失败 ${pluginId}（tar -acf …）`);
    process.exit(1);
  }
  console.log("package-plugins:", pluginId, "→", outZip);
  return true;
}

if (!fs.existsSync(bundledRoot)) {
  console.error("package-plugins: 未找到目录 plugins/bundled");
  process.exit(1);
}

const dirs = fs
  .readdirSync(bundledRoot, { withFileTypes: true })
  .filter((e) => e.isDirectory())
  .map((e) => e.name);

let n = 0;
for (const id of dirs.sort()) {
  if (zipPluginDir(id)) n += 1;
}

if (n === 0) {
  console.error("package-plugins: plugins/bundled 下无有效插件（需含 plugin.toml）");
  process.exit(1);
}

console.log(`package-plugins: 完成，共 ${n} 个 zip → ${outDir}`);
