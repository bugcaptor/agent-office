/// <reference types="vite/client" />

// vite.config.ts / vitest.config.ts inject this via `define` from
// package.json's `version` -- see AboutDialog.tsx.
declare const __APP_VERSION__: string;
