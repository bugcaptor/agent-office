// src/renderer/mascot/position.ts
//
// 마스코트 창(이슈 #72)의 위치 기억/복원. 위치는 순수한 UI 선호라 프로필
// 파일이 아니라 localStorage에 둔다(theme/terminalViewMode와 같은 관례).
//
// 좌표는 전부 **물리 픽셀**로 다룬다: Tauri의 `Monitor.position/size`,
// `outerPosition()`, `outerSize()`가 모두 물리 픽셀이라 스케일 팩터를 섞지
// 않는 편이 DPI가 다른 모니터 사이에서 안전하다.
//
// 어려운 부분은 "저장할 때는 있었지만 지금은 없는 모니터": 외장 모니터를
// 떼고 재시작하면 저장된 좌표가 어느 화면에도 없어 마스코트가 보이지 않는
// 곳에 뜬다. 복원 좌표가 현재 모니터 중 하나에 (여유를 두고) 걸칠 때만 쓰고,
// 아니면 주 모니터 기본 위치로 되돌린다.

export const MASCOT_POS_KEY = "agent-office.mascot.pos";
/** 기본 위치 여백(물리 px 기준으로 스케일해 쓴다) — 우하단에서 이만큼 띄운다.
 *  하단 여백이 큰 것은 macOS Dock / Windows 작업표시줄을 피하기 위해서다. */
export const MASCOT_MARGIN_RIGHT = 24;
export const MASCOT_MARGIN_BOTTOM = 80;
/** 모니터 포함 판정 허용 오차(px). 창이 화면 경계에 살짝 걸친 상태를 살린다. */
export const MONITOR_SLACK_PX = 8;

export interface Point {
  x: number;
  y: number;
}

export interface Size {
  width: number;
  height: number;
}

export interface MonitorRect extends Point, Size {
  /** 이 모니터의 배율 — 여백을 물리 픽셀로 환산하는 데 쓴다. */
  scaleFactor: number;
}

/** 창(좌상단 pos, 크기 size)이 이 모니터에 걸치는가. 순수. */
export function isOnMonitor(
  pos: Point,
  size: Size,
  m: MonitorRect,
  slack = MONITOR_SLACK_PX,
): boolean {
  return (
    pos.x + size.width >= m.x - slack &&
    pos.y + size.height >= m.y - slack &&
    pos.x <= m.x + m.width + slack &&
    pos.y <= m.y + m.height + slack
  );
}

/** 모니터 우하단 기본 위치. 여백은 해당 모니터 배율로 환산한다. 순수. */
export function defaultPosition(m: MonitorRect, size: Size): Point {
  const s = m.scaleFactor > 0 ? m.scaleFactor : 1;
  return {
    x: Math.round(m.x + m.width - size.width - MASCOT_MARGIN_RIGHT * s),
    y: Math.round(m.y + m.height - size.height - MASCOT_MARGIN_BOTTOM * s),
  };
}

/**
 * 복원 위치 결정 — 저장값이 어느 모니터에도 걸치지 않으면(모니터 해제 등)
 * 주 모니터 기본 위치로 폴백한다. 모니터 목록이 비면 저장값을 그대로 믿는다
 * (모니터 조회 실패보다 사용자가 마지막에 둔 자리가 낫다). 순수.
 */
export function resolvePosition(
  saved: Point | null,
  size: Size,
  monitors: ReadonlyArray<MonitorRect>,
  primary: MonitorRect | null,
): Point | null {
  if (saved !== null) {
    if (monitors.length === 0) return saved;
    if (monitors.some((m) => isOnMonitor(saved, size, m))) return saved;
  }
  const base = primary ?? monitors[0] ?? null;
  return base === null ? null : defaultPosition(base, size);
}

/** localStorage에서 저장 위치 읽기. 없거나 깨졌으면 null. */
export function readSavedPosition(storage: Pick<Storage, "getItem"> | null): Point | null {
  if (!storage) return null;
  try {
    const raw = storage.getItem(MASCOT_POS_KEY);
    if (!raw) return null;
    const parsed: unknown = JSON.parse(raw);
    if (
      typeof parsed === "object" &&
      parsed !== null &&
      Number.isFinite((parsed as Point).x) &&
      Number.isFinite((parsed as Point).y)
    ) {
      return { x: (parsed as Point).x, y: (parsed as Point).y };
    }
  } catch {
    /* 손상값은 없는 것과 같이 취급 */
  }
  return null;
}

export function writeSavedPosition(storage: Pick<Storage, "setItem"> | null, pos: Point): void {
  try {
    storage?.setItem(
      MASCOT_POS_KEY,
      JSON.stringify({ x: Math.round(pos.x), y: Math.round(pos.y) }),
    );
  } catch {
    /* 저장 실패(프라이빗 모드 등)는 무시 — 위치는 편의 기능이다 */
  }
}
