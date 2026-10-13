// src/renderer/terminal/terminalViewMode.ts
//
// 터미널 오버레이 뷰 모드(이슈 #69)의 순수 로직 계층 — 타입, 순환(토글)
// 순서, localStorage 영속. 스토어/Tauri 의존이 없어 appStore가 안전하게
// import할 수 있다.
//
// - windowed: 화면 중앙 72%×72% 오버레이(기존 동작, 배경 딤 유지).
// - filled:   오버레이가 앱 창 전체를 덮음(인앱, 배경 딤 제거).

export type TerminalViewMode = "windowed" | "filled";

export const TERMINAL_VIEW_MODE_STORAGE_KEY = "agent-office.terminal-view-mode";

/** 순환(2스테이트 토글) 순서. cycleTerminalViewMode / 헤더 버튼이 이 순서로 돈다. */
const CYCLE_ORDER: TerminalViewMode[] = ["windowed", "filled"];

export function isTerminalViewMode(v: unknown): v is TerminalViewMode {
  return v === "windowed" || v === "filled";
}

/** 현재 모드의 다음 순환 모드(windowed↔filled 토글). */
export function nextTerminalViewMode(mode: TerminalViewMode): TerminalViewMode {
  const i = CYCLE_ORDER.indexOf(mode);
  return CYCLE_ORDER[(i + 1) % CYCLE_ORDER.length];
}

/** 저장된 뷰 모드를 읽는다. 없거나 알 수 없으면 windowed. localStorage 부재(node)도 안전. */
export function loadStoredTerminalViewMode(): TerminalViewMode {
  try {
    const raw = localStorage.getItem(TERMINAL_VIEW_MODE_STORAGE_KEY);
    return isTerminalViewMode(raw) ? raw : "windowed";
  } catch {
    return "windowed";
  }
}

/** 뷰 모드를 localStorage에 영속한다. 저장 불가 환경에서는 조용히 무시. */
export function persistTerminalViewMode(mode: TerminalViewMode): void {
  try {
    localStorage.setItem(TERMINAL_VIEW_MODE_STORAGE_KEY, mode);
  } catch {
    // 프라이빗 모드/노드 환경 등 저장 불가 — 적용 자체는 유효하므로 무시.
  }
}
