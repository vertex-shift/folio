import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**", "**/dist/**"] },
    proxy: {
      "/__folio": {
        target: "http://127.0.0.1:19148",
        rewrite: (path) => path.replace(/^\/__folio/, ""),
      },
    },
  },
  build: {
    target: "safari16",
    chunkSizeWarningLimit: 1200,
  },
});
