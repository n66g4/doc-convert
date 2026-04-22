#!/usr/bin/env node
/**
 * 清除 CARGO_TARGET_DIR 后执行命令，使 Cargo 使用项目内 target/（见 src-tauri/.cargo/config.toml），
 * 避免 Cursor 等环境将产物写入沙箱缓存目录。
 */
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

const argv = process.argv.slice(2);
if (argv.length === 0) {
  console.error(
    "Usage: node scripts/run_without_cargo_target_dir.mjs <command> [args...]"
  );
  console.error("Example: node scripts/run_without_cargo_target_dir.mjs npx tauri build");
  process.exit(1);
}

const env = { ...process.env };
delete env.CARGO_TARGET_DIR;

const cmd = argv[0];
const args = argv.slice(1);

const r = spawnSync(cmd, args, {
  stdio: "inherit",
  cwd: root,
  env,
  shell: process.platform === "win32",
});

process.exit(r.status ?? 1);
