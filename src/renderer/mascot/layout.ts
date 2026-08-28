// src/renderer/mascot/layout.ts
//
// 신호등 strip을 붙였을 때 마스코트 창이 가져야 할 크기와, 칸이 넘칠 때
// 어떻게 접을지를 계산하는 순수 함수(docs/mascot-lights-design.md §5.1).
// 렌더 관심사(몇 칸까지 그릴지)는 여기 mascot 쪽에 두고, main 렌더러(store/
// mascotLights.ts)는 항상 전체 목록을 보낸다 — 프로토콜을 단순하게 유지한다.
//
// 치수는 전부 논리 px. 창 실제 적용(물리 px 환산·set_mascot_layout 호출)은
// MascotApp.tsx의 몫이라 여기서는 손대지 않는다.

import { LIGHT_GAP, LIGHT_PX, LIGHT_STRIP_PAD, MASCOT_WINDOW_H, MASCOT_WINDOW_W, MAX_LIGHTS } from "./protocol";

/** 램프가 나열되는 방향의 strip 길이(논리 px). 칸이 없으면 0(strip 자체가 없다). */
function stripLength(count: number): number {
  if (count <= 0) return 0;
  return LIGHT_STRIP_PAD * 2 + LIGHT_PX * count + LIGHT_GAP * (count - 1);
}

/** strip의 직교(두께) 방향 길이(논리 px). 칸이 없으면 0. */
function stripThickness(count: number): number {
  return count <= 0 ? 0 : LIGHT_PX + LIGHT_STRIP_PAD * 2;
}

/**
 * 마스코트 창이 가져야 할 크기(논리 px) — 스프라이트 유무·칸 수·배열 방향으로
 * 결정한다. 가로 모드는 strip이 폭 방향으로 늘어나고 두께(30px)가 스프라이트
 * 아래에 얹히며, 세로 모드는 반대다.
 */
export function computeMascotLayout(input: {
  lightCount: number;
  vertical: boolean;
  hasSprite: boolean;
}): { width: number; height: number } {
  const { lightCount, vertical, hasSprite } = input;
  const spriteW = hasSprite ? MASCOT_WINDOW_W : 0;
  const spriteH = hasSprite ? MASCOT_WINDOW_H : 0;
  if (vertical) {
    return {
      width: Math.max(spriteW, stripThickness(lightCount)),
      height: spriteH + stripLength(lightCount),
    };
  }
  return {
    width: Math.max(spriteW, stripLength(lightCount)),
    height: spriteH + stripThickness(lightCount),
  };
}

/**
 * 넘치는 칸을 접는다(결정 8) — max를 넘으면 앞 (max-1)칸만 보이고 나머지는
 * `+k` 오버플로 칩 하나로 뭉친다. max 이하면 전부 보인다(overflowCount=0).
 */
export function foldOverflow<T>(
  lights: readonly T[],
  max: number = MAX_LIGHTS,
): { shown: T[]; overflowCount: number } {
  if (lights.length <= max) {
    return { shown: [...lights], overflowCount: 0 };
  }
  const shownCount = max - 1;
  return {
    shown: lights.slice(0, shownCount),
    overflowCount: lights.length - shownCount,
  };
}
