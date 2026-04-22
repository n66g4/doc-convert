/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_CORE_API_BASE?: string;
  readonly VITE_DEV_CORE_PORT?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
