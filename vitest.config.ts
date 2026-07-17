import { fileURLToPath, URL } from "node:url";
import { createRequire } from "node:module";

import { defineConfig, configDefaults } from "vitest/config";
import react from "@vitejs/plugin-react";

// Kept in sync with vite.config.ts -- AboutDialog.test.tsx asserts on
// `__APP_VERSION__`, so the test runner needs the same define the app build
// gets.
const pkg = createRequire(import.meta.url)("./package.json") as { version: string };

// https://vitest.dev/config/
export default defineConfig({
  plugins: [react()],
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },
  resolve: {
    // Kept in sync with tsconfig.json "paths" / vite.config.ts
    alias: {
      "@renderer": fileURLToPath(new URL("./src/renderer", import.meta.url)),
      "@shared": fileURLToPath(new URL("./src/shared", import.meta.url)),
    },
  },
  test: {
    // Default environment is plain Node (fast) for pure logic (e.g. src/shared).
    // Renderer component tests opt into jsdom per-file via a leading
    // `// @vitest-environment jsdom` docblock comment.
    environment: "node",
    // Scaffold has no tests yet — treat that as a pass instead of a failure.
    passWithNoTests: true,
    exclude: [...configDefaults.exclude, ".claude/**", ".superpowers/**"],
  },
});
