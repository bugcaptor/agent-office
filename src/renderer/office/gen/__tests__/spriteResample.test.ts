// src/renderer/office/gen/__tests__/spriteResample.test.ts
//
// S-적응 프리필터(이슈 #47)의 순수 코어 검증. detailCellSize의 D=min(N,16·S)
// 산식과, areaDownscalePremul의 area 평균·premultiplied alpha(프린지 방지)를
// 픽셀 단위로 못 박는다. DOM/Pixi 비의존이라 node 환경에서 결정적으로 돈다.
import { describe, expect, it } from "vitest";

import { detailCellSize, areaDownscalePremul, type Rgba } from "../spriteResample";

const rgba = (w: number, h: number, px: number[][]): Rgba => ({
  data: Uint8ClampedArray.from(px.flat()),
  width: w,
  height: h,
});

describe("detailCellSize", () => {
  it("D = min(N, 16·S)", () => {
    expect(detailCellSize(256, 3)).toBe(48); // 고해상 시트, 중간 창
    expect(detailCellSize(256, 1)).toBe(16); // 작은 창 → 강한 축소
    expect(detailCellSize(100, 2)).toBe(32);
    expect(detailCellSize(32, 4)).toBe(32); // 16·4=64 > N → 원본 유지(축소 안 함)
    expect(detailCellSize(16, 5)).toBe(16); // 절차 생성급 저해상은 항상 N
  });

  it("S는 정수로 반올림하고 최소 1로 클램프", () => {
    expect(detailCellSize(256, 2.4)).toBe(32); // round(2.4)=2 → 32
    expect(detailCellSize(256, 2.6)).toBe(48); // round(2.6)=3 → 48
    expect(detailCellSize(256, 0)).toBe(16); // max(1,·) → 16
  });
});

describe("areaDownscalePremul", () => {
  it("단색 불투명 블록은 그대로 보존한다", () => {
    const src = rgba(2, 2, [
      [40, 80, 120, 255], [40, 80, 120, 255],
      [40, 80, 120, 255], [40, 80, 120, 255],
    ]);
    const out = areaDownscalePremul(src, 1, 1);
    expect([...out.data]).toEqual([40, 80, 120, 255]);
  });

  it("불투명 4픽셀의 area 평균을 낸다", () => {
    const src = rgba(2, 2, [
      [0, 0, 0, 255], [255, 255, 255, 255],
      [255, 255, 255, 255], [255, 255, 255, 255],
    ]);
    const out = areaDownscalePremul(src, 1, 1);
    // (0+255+255+255)/4 = 191.25 → ClampedArray 반올림 191
    expect([...out.data.slice(0, 3)]).toEqual([191, 191, 191]);
    expect(out.data[3]).toBe(255);
  });

  it("premultiplied alpha: 투명 텍셀의 RGB가 색 평균을 오염시키지 않는다", () => {
    // 불투명 빨강 + 완전 투명(검정). 나이브 평균이면 R=127로 프린지가 생기지만
    // premult면 색은 순수 빨강, 알파만 절반이어야 한다.
    const src = rgba(2, 1, [
      [255, 0, 0, 255], [0, 0, 0, 0],
    ]);
    const out = areaDownscalePremul(src, 1, 1);
    expect([...out.data.slice(0, 3)]).toEqual([255, 0, 0]); // 프린지 없음
    expect(out.data[3]).toBeGreaterThanOrEqual(127); // 알파 ≈ 50%
    expect(out.data[3]).toBeLessThanOrEqual(128);
  });

  it("부분 겹침(비정수 비율)도 가중 평균한다", () => {
    // 3→1 다운스케일: 세 텍셀을 동일 가중(각 1.0)으로 평균.
    const src = rgba(3, 1, [
      [0, 0, 0, 255], [90, 90, 90, 255], [180, 180, 180, 255],
    ]);
    const out = areaDownscalePremul(src, 1, 1);
    expect([...out.data.slice(0, 3)]).toEqual([90, 90, 90]); // (0+90+180)/3
    expect(out.data[3]).toBe(255);
  });

  it("완전 투명 영역은 색 0·알파 0으로 남긴다", () => {
    const src = rgba(2, 1, [
      [10, 20, 30, 0], [40, 50, 60, 0],
    ]);
    const out = areaDownscalePremul(src, 1, 1);
    expect([...out.data]).toEqual([0, 0, 0, 0]);
  });
});
