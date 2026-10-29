import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import process from "node:process";

// CLASSICMAIL_VITE_PORT lets several dev instances run side by side (scripts/dev.mjs --slot N).
const port = Number(process.env.CLASSICMAIL_VITE_PORT || 1420);

export default defineConfig(() => ({
  plugins: [react()],
  // the day this build was made (Help › About)
  define: { __BUILD_DATE__: JSON.stringify(new Date().toISOString().slice(0, 10)) },
  // Vite's dependency cache lives outside Google Drive (Drive locks files it is syncing)
  cacheDir: process.env.CLASSICMAIL_VITE_CACHE || "C:/dev/outlook_build/vite-cache",
  clearScreen: false,
  server: {
    port,
    strictPort: true,
    host: false,
    hmr: { port: port + 100 },
    watch: { ignored: ["**/src-tauri/**", "**/release/**", "**/.devdata/**"] },
  },
  build: { target: "es2022", chunkSizeWarningLimit: 2000 },
  test: { environment: "jsdom", include: ["src/**/*.test.ts", "src/**/*.test.tsx"] },
}));
