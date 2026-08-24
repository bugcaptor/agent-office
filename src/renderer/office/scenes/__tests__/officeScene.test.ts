// src/renderer/office/scenes/__tests__/officeScene.test.ts
//
// `awardFrameRectPx()` 지오메트리 단위 테스트 — "이 달의 우수사원" 액자를
// 타일 드로잉에서 빼내 오버레이(`entities/AwardFrameOverlay.ts`)로 옮긴 뒤,
// 이 함수가 그 배치(위치+외곽 크기)의 단일 출처가 됐다. 값 자체가 설계
// 근거(데스크 쌍 사이 빈 간격 tx8-9 중앙, 28×28, 상단 벽 ty0~ty1)이므로
// 회귀를 못 박는다.

import { describe, expect, it } from "vitest";
import { awardFrameRectPx } from "../officeScene";

describe("awardFrameRectPx", () => {
  it("외곽 28×28 사각형을 데스크 쌍 사이 빈 간격(tx8-9) 중앙, 상단 벽(ty0~ty1)에 배치한다", () => {
    const rect = awardFrameRectPx();
    expect(rect).toEqual({ x: 130, y: 4, w: 28, h: 28 });
  });

  it("호출마다 같은 값을 낸다(순수 함수, 상태 없음)", () => {
    expect(awardFrameRectPx()).toEqual(awardFrameRectPx());
  });
});
