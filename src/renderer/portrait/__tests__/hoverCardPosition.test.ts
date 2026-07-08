// src/renderer/portrait/__tests__/hoverCardPosition.test.ts
import { describe, expect, it } from "vitest";
import { clampCardPosition } from "../AgentHoverCard";

// 뷰포트: 800x600 (앱 창 크기). 카드: 대략 200x172.
const VW = 800;
const VH = 600;
const MARGIN = 8;

describe("clampCardPosition", () => {
  it("fits as-is when there is room (no clamp)", () => {
    const p = clampCardPosition(100, 100, 200, 172, VW, VH, MARGIN);
    expect(p).toEqual({ x: 100, y: 100 });
  });

  it("clamps at the right edge", () => {
    // rawX + cardW(200) > VW(800) - margin(8) => 792 한계
    const p = clampCardPosition(750, 100, 200, 172, VW, VH, MARGIN);
    expect(p.x).toBeCloseTo(VW - 200 - MARGIN, 6); // 592
    expect(p.y).toBeCloseTo(100, 6);
  });

  it("clamps at the bottom edge", () => {
    const p = clampCardPosition(100, 550, 200, 172, VW, VH, MARGIN);
    expect(p.x).toBeCloseTo(100, 6);
    expect(p.y).toBeCloseTo(VH - 172 - MARGIN, 6); // 420
  });

  it("clamps both edges when hovering bottom-right", () => {
    const p = clampCardPosition(780, 570, 200, 172, VW, VH, MARGIN);
    expect(p.x).toBeCloseTo(VW - 200 - MARGIN, 6);
    expect(p.y).toBeCloseTo(VH - 172 - MARGIN, 6);
  });

  it("floors at margin when the card is larger than the viewport", () => {
    // 카드(900)가 뷰포트(800)보다 커서, 우측 한계가 margin보다 왼쪽에 위치 -> margin으로 바닥.
    const p = clampCardPosition(50, 50, 900, 700, VW, VH, MARGIN);
    expect(p.x).toBeCloseTo(MARGIN, 6);
    expect(p.y).toBeCloseTo(MARGIN, 6);
  });
});
