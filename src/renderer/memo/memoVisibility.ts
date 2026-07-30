// src/renderer/memo/memoVisibility.ts
//
// 포스트잇 위젯(#79) 열림 상태의 순수 영속 계층. terminalViewMode.ts와 같은
// 관례 — 타입/키/load/persist만 두고 스토어·Tauri 의존이 없다(그래서 노드
// 환경에서도 안전하고, memoStore가 부팅 시 동기적으로 초기값을 읽을 수 있다).
//
// 열림 상태는 **에이전트별이 아니라 전역**이다: 위젯은 늘 활성 탭의 장을
// 보여주므로, "포스트잇을 띄워 둔다"는 사용자의 작업 습관이지 캐릭터의 속성이
// 아니다.

export const MEMO_VISIBLE_STORAGE_KEY = "agent-office.memo-visible";

/** 저장된 열림 상태를 읽는다. 없거나 알 수 없으면 false(기본 닫힘). */
export function loadStoredMemoVisible(): boolean {
  try {
    return localStorage.getItem(MEMO_VISIBLE_STORAGE_KEY) === "true";
  } catch {
    return false;
  }
}

/** 열림 상태를 localStorage에 영속한다. 저장 불가 환경에서는 조용히 무시. */
export function persistMemoVisible(visible: boolean): void {
  try {
    localStorage.setItem(MEMO_VISIBLE_STORAGE_KEY, visible ? "true" : "false");
  } catch {
    // 프라이빗 모드/노드 환경 등 저장 불가 — 적용 자체는 유효하므로 무시.
  }
}
