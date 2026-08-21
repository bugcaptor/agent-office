import { fileURLToPath, URL } from "node:url";

import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";

// `emptyOutDir`가 dist-web를 통째로 비우면서 **git이 추적하는** `.gitkeep`까지
// 지워, 빌드만 돌려도 워킹트리가 더러워졌다(`tauri dev`의 beforeDevCommand가
// 매번 web:build를 탄다). 산출물의 일부로 다시 내보내 원상복구한다.
// .gitkeep 자체는 지울 수 없다 — rust-embed의 `#[folder = "../dist-web/"]`는
// 폴더가 없으면 컴파일 단계에서 실패하므로 새 클론에도 폴더가 있어야 한다.
function keepGitkeep(): Plugin {
  return {
    name: "agent-office:keep-gitkeep",
    apply: "build",
    generateBundle() {
      this.emitFile({ type: "asset", fileName: ".gitkeep", source: "" });
    },
  };
}

// 웹 호스팅(kbm #7m) 클라이언트 빌드.
//
// 데스크톱 렌더러(vite.config.ts)와 **별도 산출물**이다. 같은 번들을 재사용하지
// 않는 이유는 크기가 아니라 소유권이다 — 렌더러는 PersistedState의 소유자라
// (persist.ts가 agents 변경마다 전량 saveState) 같은 번들이 두 벌 뜨면 상호
// 덮어쓰기와 백그라운드 라이터(일기·턴로그·요약기) 이중 실행이 필연이다.
// 웹 클라이언트는 로컬 영속이 없고 서버 push가 유일한 진실이다.
//
// 산출물은 `dist-web/`이고 Rust가 rust-embed로 바이너리에 내장한다. debug
// 빌드에서는 런타임에 디스크를 읽으므로 `npm run web:dev`(--watch)만 띄워 두면
// 새 빌드가 즉시 서빙된다.
export default defineConfig({
  plugins: [react(), keepGitkeep()],
  // 엔트리(index.html)가 여기 있다 — 산출물이 `dist-web/index.html`이 되어야
  // Rust 쪽 정적 서빙(rust-embed)이 그대로 집어 든다.
  root: fileURLToPath(new URL("./src/web", import.meta.url)),
  // 서버가 `/web/` 아래에 마운트한다.
  base: "/web/",
  resolve: {
    alias: {
      "@shared": fileURLToPath(new URL("./src/shared", import.meta.url)),
      "@web": fileURLToPath(new URL("./src/web", import.meta.url)),
    },
  },
  build: {
    outDir: fileURLToPath(new URL("./dist-web", import.meta.url)),
    emptyOutDir: true,
    rollupOptions: {
      output: {
        // rust-embed가 폴더를 통째로 내장하므로 파일명 해시는 그대로 둔다
        // (캐시 무효화는 해시가 담당).
        entryFileNames: "assets/[name]-[hash].js",
        chunkFileNames: "assets/[name]-[hash].js",
        assetFileNames: "assets/[name]-[hash][extname]",
      },
    },
  },
});
