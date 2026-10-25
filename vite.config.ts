import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

function localPort(name: string, fallback: number): number {
  const value = process.env[name];
  if (value === undefined) return fallback;
  if (!/^\d+$/.test(value) || Number(value) < 1024 || Number(value) > 65535) {
    throw new Error(`${name} must be a local development port from 1024 to 65535.`);
  }
  return Number(value);
}

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    host: "127.0.0.1",
    port: localPort("FOLIO_DEV_PORT", 1420),
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**", "**/dist/**"] },
    proxy: {
      "/__folio": {
        target: `http://127.0.0.1:${localPort("FOLIO_PREVIEW_PORT", 19148)}`,
        rewrite: (path) => path.replace(/^\/__folio/, ""),
      },
    },
  },
  build: {
    target: "safari16",
    chunkSizeWarningLimit: 1200,
  },
});
