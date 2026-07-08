// src/renderer/portrait/__tests__/cropMath.test.ts
import { describe, expect, it } from "vitest";
import {
  initialCoverView,
  viewToSourceRect,
  zoomAt,
  panBy,
} from "../cropMath";

// 프레임: 3:4 (예: 240x320).
const FW = 240;
const FH = 320;

describe("initialCoverView", () => {
  it("center-covers a wide image so the frame is fully filled", () => {
    // 400x200 이미지를 240x320 프레임에 커버: scale = max(240/400, 320/200) = 1.6
    const v = initialCoverView(400, 200, FW, FH);
    expect(v.scale).toBeCloseTo(1.6, 6);
    // 가로가 넘쳐 중앙 정렬: offsetX = (240 - 400*1.6)/2 = -200
    expect(v.offsetX).toBeCloseTo(-200, 6);
    expect(v.offsetY).toBeCloseTo(0, 6);
  });

  it("cover view maps the frame to a source rect fully inside the image", () => {
    const v = initialCoverView(400, 200, FW, FH);
    const r = viewToSourceRect(v, FW, FH);
    expect(r.sx).toBeGreaterThanOrEqual(0);
    expect(r.sy).toBeCloseTo(0, 6);
    expect(r.sx + r.sw).toBeLessThanOrEqual(400 + 1e-6);
    expect(r.sh).toBeCloseTo(200, 6);
  });
});

describe("viewToSourceRect", () => {
  it("returns the exact image region visible in the frame", () => {
    // scale 2, 이미지 좌상단이 프레임 (-40,-60)에 위치.
    const v = { scale: 2, offsetX: -40, offsetY: -60 };
    const r = viewToSourceRect(v, FW, FH);
    expect(r.sx).toBeCloseTo(20, 6); // -(-40)/2
    expect(r.sy).toBeCloseTo(30, 6); // -(-60)/2
    expect(r.sw).toBeCloseTo(120, 6); // 240/2
    expect(r.sh).toBeCloseTo(160, 6); // 320/2
  });
});

describe("zoomAt", () => {
  it("keeps the image point under the anchor fixed", () => {
    const v = { scale: 2, offsetX: -40, offsetY: -60 };
    const before = viewToSourceRect(v, FW, FH);
    // 프레임 중앙(120,160)에서 확대해도, 그 지점이 가리키는 원본 좌표는 불변.
    const imgXBefore = before.sx + (120 / FW) * before.sw;
    const v2 = zoomAt(v, 1.5, 120, 160);
    const after = viewToSourceRect(v2, FW, FH);
    const imgXAfter = after.sx + (120 / FW) * after.sw;
    expect(imgXAfter).toBeCloseTo(imgXBefore, 6);
    expect(v2.scale).toBeCloseTo(3, 6);
  });
});

describe("panBy", () => {
  it("shifts the source rect opposite to the drag direction", () => {
    const v = { scale: 2, offsetX: -40, offsetY: -60 };
    const r0 = viewToSourceRect(v, FW, FH);
    // 이미지를 오른쪽으로 20px 드래그하면 보이는 소스 왼쪽이 줄어든다.
    const r1 = viewToSourceRect(panBy(v, 20, 0), FW, FH);
    expect(r1.sx).toBeCloseTo(r0.sx - 10, 6); // -20/scale
  });
});
