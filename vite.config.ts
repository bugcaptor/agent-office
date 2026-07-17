import { fileURLToPath, URL } from "node:url";
import { createRequire } from "node:module";

import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// createRequire (not a JSON import) -- this config file is loaded by Vite's
// own esbuild-based transform, which doesn't reliably support `import ...
// with { type: "json" }` yet; require() of JSON is universally supported.
const pkg = createRequire(import.meta.url)("./package.json") as { version: string };

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],

  // AboutDialog reads this to show the running app version without a
  // separate IPC round-trip to the Tauri side -- package.json's `version`
  // is the single source of truth (kept in sync with src-tauri via
  // scripts/bump-version.mjs).
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },

  resolve: {
    // Kept in sync with tsconfig.json "paths"
    alias: {
      "@renderer": fileURLToPath(new URL("./src/renderer", import.meta.url)),
      "@shared": fileURLToPath(new URL("./src/shared", import.meta.url)),
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
