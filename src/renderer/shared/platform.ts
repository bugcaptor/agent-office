// src/renderer/shared/platform.ts
//
// 렌더러가 쓰는 플랫폼 판별. 단축키 관례(macOS는 Cmd, 그 외는 Ctrl/F11)와
// IME 우회 코드가 갈라지는 지점이 여러 곳이라, 같은 3줄을 각자 복제하지
// 않도록 한 군데로 모았다.

/** 지금 렌더러가 macOS에서 돌고 있는가. */
export const IS_MAC =
  typeof navigator !== "undefined" &&
  /mac/i.test(navigator.platform || navigator.userAgent || "");

/** 지금 렌더러가 Windows에서 돌고 있는가. 파일 탐색기 이름("탐색기" vs
 *  "Finder")처럼 OS별 표기가 갈리는 곳에서 쓴다. */
export const IS_WINDOWS =
  typeof navigator !== "undefined" &&
  /win/i.test(navigator.platform || navigator.userAgent || "");
