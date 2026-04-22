import { defineConfig, loadEnv } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const corePort = env.VITE_DEV_CORE_PORT || "17300";
  const coreTarget = `http://127.0.0.1:${corePort}`;

  return {
    plugins: [react()],
    clearScreen: false,
    server: {
      port: 5173,
      strictPort: true,
      host: host || false,
      hmr: host
        ? {
            protocol: "ws",
            host,
            port: 5183,
          }
        : undefined,
      watch: {
        ignored: ["**/src-tauri/**"],
      },
      proxy: {
        "/api": {
          target: coreTarget,
          changeOrigin: true,
        },
        "/health": {
          target: coreTarget,
          changeOrigin: true,
        },
      },
    },
    envPrefix: ["VITE_", "TAURI_ENV_*"],
    build: {
      target:
        process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari16",
      minify: !process.env.TAURI_ENV_DEBUG ? "esbuild" : false,
      sourcemap: !!process.env.TAURI_ENV_DEBUG,
    },
  };
});
