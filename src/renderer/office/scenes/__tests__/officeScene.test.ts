// src/renderer/office/scenes/__tests__/officeScene.test.ts
//
// `awardFrameRectPx()` 지오메트리 단위 테스트 — "이 달의 우수사원" 벽 사진을
// 타일 드로잉에서 빼내 오버레이(`entities/AwardFrameOverlay.ts`)로 옮긴 뒤,
// 이 함수가 그 배치(위치+카드 크기)의 단일 출처가 됐다. 값 자체가 설계
// 근거이므로 회귀를 못 박는다: 정중앙(tx8-9)을 **피한** 오른쪽 간격 tx12-13,
// 그 안에서도 중앙정렬이 아닌 왼쪽 4px, 작은 폴라로이드 18×21, 상단 벽 ty0~ty1.
// 정중앙 단독 배치 + 정사각 액자가 "영정사진 같다"는 눈검증을 받았던 자리다.

import { describe, expect, it } from "vitest";
import { awardFrameRectPx } from "../officeScene";

describe("awardFrameRectPx", () => {
  it("폴라로이드 18×21을 정중앙이 아닌 tx12-13 간격의 왼쪽 4px, 상단 벽(ty0~ty1)에 꽂는다", () => {
    const rect = awardFrameRectPx();
    expect(rect).toEqual({ x: 12 * 16 + 4, y: 4, w: 18, h: 21 });
  });

  it("정중앙(tx8-9 간격)에 놓지 않는다 — 단독 정중앙 배치가 신격화 도상이었다", () => {
    // GRID 폭 20칸 → 정중앙은 tx9.5(=x 152). 카드가 그 근처에 없어야 한다.
    const rect = awardFrameRectPx();
    expect(rect.x).toBeGreaterThan(9.5 * 16);
  });

  it("호출마다 같은 값을 낸다(순수 함수, 상태 없음)", () => {
    expect(awardFrameRectPx()).toEqual(awardFrameRectPx());
  });
});
