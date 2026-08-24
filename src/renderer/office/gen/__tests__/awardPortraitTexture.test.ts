// containSize 순수 함수 검증 — 초상 프리필터 목표 해상도 계산.
// (텍스처 생성 자체는 Pixi 렌더러 컨텍스트가 필요해 여기서 다루지 않는다.)
import { describe, it, expect } from "vitest";
import { containSize } from "../awardPortraitTexture";

describe("containSize", () => {
  it("비율을 유지하며 상자 안에 넣는다(세로가 긴 초상)", () => {
    // 240×320을 14×14 사진칸 × S=3 = 42×42에
    expect(containSize(240, 320, 42, 42)).toEqual({ w: 32, h: 42 });
  });

  it("가로가 긴 원본은 너비가 상자에 닿는다", () => {
    expect(containSize(320, 240, 42, 42)).toEqual({ w: 42, h: 32 });
  });

  it("상자가 원본보다 크면 확대하지 않는다", () => {
    expect(containSize(240, 320, 480, 640)).toEqual({ w: 240, h: 320 });
  });

  it("극단적 축소에서도 최소 1px", () => {
    expect(containSize(240, 320, 1, 1)).toEqual({ w: 1, h: 1 });
  });

  it("빈 원본은 1×1로 방어한다", () => {
    expect(containSize(0, 0, 42, 42)).toEqual({ w: 1, h: 1 });
  });
});
