// src/renderer/i18n/backendPhrases.ts
//
// **백엔드(Rust)가 스스로 채워 보내는 고정 문구** 중 렌더러가 알아봐야 하는
// 것. 화면 문구(카탈로그)도, 프롬프트도, 사용자 입력 판정 규칙도 아니라 따로
// 둔다. 원문과 반드시 같아야 한다 — 바꿀 때는 짝이 되는 Rust 상수도 함께.
// (`src/renderer/i18n/` 아래는 하드코딩 한글 스캐너의 제외 경로다.)

/** 완료 메시지가 없을 때 hub가 채우는 폴백 본문 — `src-tauri/src/notification/
 *  hub.rs`의 `STOP_FALLBACK_KO`/`STOP_FALLBACK_EN`. Rust 쪽 언어는 이 둘뿐이다. */
export const STOP_FALLBACK_MESSAGES: ReadonlySet<string> = new Set([
  "작업이 완료되었습니다.",
  "The work is done.",
]);
