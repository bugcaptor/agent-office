// src/renderer/office/gen/__tests__/minimiFactory.test.ts
//
// 미니미 커스텀 픽셀아트 → 텍스처 변환 TDD. characterFactory.custom.test.ts와
// 같은 이유로 pixi.js를 최소 페이크로 목킹하고(Texture.from은 실제 캔버스가
// 필요) 분기 선택·배율 계산만 검증한다.
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("pixi.js", () => {
  class Rectangle {
    constructor(
      public x: number,
      public y: number,
      public w: number,
      public h: number,
    ) {}
  }
  class Texture {
    source: { scaleMode: string; src?: unknown };
    frame?: Rectangle;
    destroyed = false;
    static from(src: unknown) {
      const t = new Texture();
      t.source = { scaleMode: "linear", src };
      return t;
    }
    constructor(opts?: { source: Texture["source"]; frame: Rectangle }) {
      this.source = opts?.source ?? { scaleMode: "linear" };
      this.frame = opts?.frame;
    }
    destroy() {
      this.destroyed = true;
    }
  }
  return { Texture, Rectangle };
});

import { setMinimiOverride, resetMinimiOverrides } from "../minimiOverrides";
import { assetsFromMinimiFrame, createMinimiAssets } from "../minimiFactory";
import { MINIMI_CELL } from "../spriteResample";

afterEach(() => resetMinimiOverrides());

/** 다운스케일 경로 배선만 확인하는 캔버스 팩토리(실제 픽셀 없음). */
const stubFactory = () => {
  let made = 0;
  const makeCtx = () => ({
    imageSmoothingEnabled: false,
    drawImage: () => {},
    getImageData: (_x: number, _y: number, w: number, h: number) => ({
      data: new Uint8ClampedArray(w * h * 4),
    }),
    createImageData: (w: number, h: number) => ({ data: new Uint8ClampedArray(w * h * 4) }),
    putImageData: () => {},
  });
  return (w: number, h: number) => ({ ctx: makeCtx(), canvas: { id: made++, w, h } }) as never;
};

const frameOf = (n: number) => ({ width: n, height: n }) as unknown as CanvasImageSource;

describe("assetsFromMinimiFrame", () => {
  it("renderScale 미지정이면 원본 해상도를 그대로 쓰고 배율은 8/N", () => {
    const a = assetsFromMinimiFrame(frameOf(32));
    expect(a.cellSize).toBe(32);
    expect(a.scale).toBeCloseTo(8 / 32);
    expect(a.texture.source.scaleMode).toBe("nearest");
    expect(a.dispose).toBeUndefined(); // 공유 캔버스 소스 → 개별 해제 금지
  });

  it("D >= N이면(작은 프레임/큰 창) 프리필터 없이 패스스루", () => {
    // N=16, S=4 → D=min(16, 8·4=32)=16 = N
    const a = assetsFromMinimiFrame(frameOf(16), 4);
    expect(a.cellSize).toBe(16);
    expect(a.scale).toBeCloseTo(8 / 16);
    expect(a.dispose).toBeUndefined();
  });

  it("D < N이면 D=min(N,8·S)로 area 프리필터하고 배율은 8/D", () => {
    // N=64, S=2 → D=min(64, 16)=16
    const a = assetsFromMinimiFrame(frameOf(64), 2, stubFactory());
    expect(a.cellSize).toBe(16);
    expect(a.scale).toBeCloseTo(8 / 16);
    expect(a.texture.source.scaleMode).toBe("nearest");
    expect(typeof a.dispose).toBe("function"); // 자체 소유 텍스처 → 해제 훅 필요
  });

  it("겉보기 크기는 D와 무관하게 8px로 유지된다(cellSize × scale = 8)", () => {
    for (const [n, s] of [
      [16, 1],
      [64, 1],
      [64, 4],
      [128, 8],
      [256, 3],
    ] as const) {
      const a = assetsFromMinimiFrame(frameOf(n), s, stubFactory());
      expect(a.cellSize * a.scale).toBeCloseTo(MINIMI_CELL);
    }
  });
});

describe("createMinimiAssets", () => {
  it("오버라이드가 없으면 null(현행 부모 idle0 축소판 유지)", () => {
    expect(createMinimiAssets("ghost", 3)).toBeNull();
  });

  it("오버라이드가 있으면 그 프레임으로 에셋을 만든다", () => {
    const frame = frameOf(16);
    setMinimiOverride("a1", frame);
    const a = createMinimiAssets("a1", 4);
    expect(a).not.toBeNull();
    expect((a!.texture.source as { src?: unknown }).src).toBe(frame);
  });
});
