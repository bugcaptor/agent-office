// src/renderer/terminal/botGuard.ts
//
// 맨 셸 가드(이슈 #57 후속). 봇 모드는 세션에 이미 떠 있는 에이전트(claude 등)의
// 프롬프트에 지시문을 타이핑 주입한다 — claude가 아니라 맨 셸이 떠 있으면 그
// 지시문이 셸 명령으로 잘못 실행돼 에러가 난다. 봇을 켜기 전에 터미널 버퍼
// 꼬리(현재 화면)를 훑어 에이전트 TUI가 떠 있는지 best-effort로 판정한다.
//
// 스크롤백 전체가 아니라 "꼬리"만 보는 이유: claude를 종료한 뒤 맨 셸로 돌아와도
// 스크롤백엔 claude 출력이 남아 있다. 지금 프롬프트가 무엇인지는 화면 맨 아래가
// 말해준다. 확신이 서지 않으면(false) 호출부가 확인 다이얼로그로 사용자에게
// 넘긴다 — 오탐(맨 셸을 에이전트로 오인)보다 미탐(한 번 더 확인)이 안전하다.

/** 화면 꼬리에서 찾는 에이전트 TUI 시그니처(소문자 비교). */
const AGENT_MARKERS = [
  "for shortcuts", // claude/codex 입력창 힌트
  "esc to interrupt", // 턴 진행 중 표시
  "claude code",
  "/help for help",
  "codex",
  "⏵⏵", // claude auto-accept 표시
];

/**
 * 터미널 버퍼 텍스트의 꼬리를 훑어 에이전트가 프롬프트를 잡고 있는지 추정한다.
 * text가 없으면(터미널 미생성) false.
 */
export function looksLikeAgentRunning(text: string | undefined): boolean {
  if (!text) return false;
  // 마지막 ~1500자(대략 현재 화면 + 여유)만 소문자로 비교.
  const tail = text.slice(-1500).toLowerCase();
  return AGENT_MARKERS.some((m) => tail.includes(m));
}
