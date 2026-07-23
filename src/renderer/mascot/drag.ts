// src/renderer/mascot/drag.ts
//
// 마스코트(이슈 #72)의 클릭 ↔ 드래그 판정. 순수 상태기라 vitest로 검증한다.
//
// `data-tauri-drag-region`을 쓰지 않는 이유: 그 속성은 mousedown 즉시 OS 창
// 드래그로 넘어가며 클릭 이벤트를 통째로 삼킨다. 마스코트는 "누르면 터미널을
// 연다"가 주 기능이라 클릭이 살아 있어야 하므로, 포인터 이동 거리로 직접
// 구분하고 임계를 넘은 순간에만 `startDragging()`을 부른다.
//
// 임계를 넘어 OS 드래그가 시작되면 웹뷰에는 pointerup이 오지 않는다(정상) —
// 그래서 "드래그가 시작됐다"는 사실 자체가 클릭 취소의 신호다.

/** 클릭으로 인정하는 최대 이동 거리(CSS px). 손떨림은 흡수하고 의도적 이동은 잡는다. */
export const DRAG_THRESHOLD_PX = 4;

export type DragOutcome = "none" | "start-drag" | "click";

export interface DragDetector {
  /** pointerdown. */
  down(x: number, y: number): void;
  /** pointermove — 임계를 처음 넘는 호출에서만 "start-drag". */
  move(x: number, y: number): DragOutcome;
  /** pointerup — 임계를 넘지 않았으면 "click". */
  up(): DragOutcome;
  /** pointercancel/창 이탈. */
  cancel(): void;
}

export function createDragDetector(threshold = DRAG_THRESHOLD_PX): DragDetector {
  let origin: { x: number; y: number } | null = null;
  let dragging = false;

  return {
    down(x, y) {
      origin = { x, y };
      dragging = false;
    },
    move(x, y) {
      if (origin === null || dragging) return "none";
      const dx = x - origin.x;
      const dy = y - origin.y;
      if (Math.hypot(dx, dy) <= threshold) return "none";
      dragging = true;
      return "start-drag";
    },
    up() {
      const wasClick = origin !== null && !dragging;
      origin = null;
      dragging = false;
      return wasClick ? "click" : "none";
    },
    cancel() {
      origin = null;
      dragging = false;
    },
  };
}
