// src/renderer/sprite/__tests__/spriteNormalize.test.ts
//
// 업로드 정규화 순수 로직 TDD. 캔버스는 @napi-rs/canvas 팩토리 주입으로
// DOM 없이 검증한다 (office/gen 테스트와 동일 관례).
import { describe, expect, it } from "vitest";
import { createCanvas } from "@napi-rs/canvas";
import {
  SHEET_W,
  SHEET_H,
  SHEET_COLS,
  CELL_MIN,
  CELL_MAX,
  isSheetSize,
  detectSheet,
  cropCellSize,
  bobOffset,
  drawNearest,
  expandFrameToSheet,
  normalizeCrop,
  normalizeSheet,
  dataUrlToBase64,
  applyBackgroundKey,
  isFullyOpaque,
  BG_KEY_TOLERANCE,
  type SpriteCanvasFactory,
} from "../spriteNormalize";
import { CELL } from "../../office/gen/compositor";

const napiFactory: SpriteCanvasFactory = (w, h) => {
  const canvas = createCanvas(w, h);
  return {
    ctx: canvas.getContext("2d") as unknown as CanvasRenderingContext2D,
    canvas: canvas as unknown as ReturnType<SpriteCanvasFactory>["canvas"],
  };
};

/** (x,y) 픽셀의 [r,g,b,a]. */
function px(canvas: unknown, x: number, y: number): number[] {
  const ctx = (canvas as ReturnType<typeof createCanvas>).getContext("2d");
  return Array.from(ctx.getImageData(x, y, 1, 1).data);
}

describe("detectSheet", () => {
  it("w===4h && h>=16 이면 시트로 인식하고 N=min(h,256)을 준다", () => {
    expect(detectSheet(64, 16)).toEqual({ kind: "sheet", n: 16 });
    expect(detectSheet(192, 48)).toEqual({ kind: "sheet", n: 48 });
    expect(detectSheet(1024, 256)).toEqual({ kind: "sheet", n: 256 });
    // h가 16의 배수가 아니어도 비율만 맞으면 시트다(구 규칙 폐지).
    expect(detectSheet(96, 24)).toEqual({ kind: "sheet", n: 24 });
  });

  it("256 초과 시트는 N=256으로 클램프(다운스케일 대상)한다", () => {
    expect(detectSheet(4096, 1024)).toEqual({ kind: "sheet", n: 256 });
  });

  it("비율 불일치·h<16·AI 원본은 crop 모드다", () => {
    expect(detectSheet(16, 16)).toEqual({ kind: "crop" }); // 단일 프레임
    expect(detectSheet(64, 32)).toEqual({ kind: "crop" }); // 비율 불일치
    expect(detectSheet(1024, 1024)).toEqual({ kind: "crop" }); // AI 생성물
    expect(detectSheet(32, 8)).toEqual({ kind: "crop" }); // h<16
  });

  it("isSheetSize는 detectSheet 위임 boolean이다", () => {
    expect(isSheetSize(64, 16)).toBe(true);
    expect(isSheetSize(96, 24)).toBe(true);
    expect(isSheetSize(1024, 1024)).toBe(false);
    expect(isSheetSize(32, 8)).toBe(false);
  });

  it("셀 상수와 열 개수를 노출한다", () => {
    expect(CELL_MIN).toBe(16);
    expect(CELL_MAX).toBe(256);
    expect(SHEET_COLS).toBe(4);
  });
});

describe("cropCellSize", () => {
  it("크롭 원본 픽셀 크기를 반올림 후 [16,256]로 클램프한다", () => {
    expect(cropCellSize(23.6)).toBe(24);
    expect(cropCellSize(10)).toBe(16); // 하한
    expect(cropCellSize(900)).toBe(256); // 상한
    expect(cropCellSize(128)).toBe(128);
  });
});

describe("bobOffset", () => {
  it("숨쉬기 밥 오프셋은 max(1, round(N/16))px", () => {
    expect(bobOffset(16)).toBe(1);
    expect(bobOffset(32)).toBe(2);
    expect(bobOffset(48)).toBe(3);
    expect(bobOffset(256)).toBe(16);
    expect(bobOffset(20)).toBe(1); // round(1.25)=1
  });
});

describe("drawNearest", () => {
  it("소스 사각형을 nearest로 목적 크기에 그린다 (블렌딩 없음)", () => {
    // 2×2 소스: 좌상 빨강, 우상 초록, 좌하 파랑, 우하 흰색.
    const src = createCanvas(2, 2);
    const sctx = src.getContext("2d");
    sctx.fillStyle = "#ff0000"; sctx.fillRect(0, 0, 1, 1);
    sctx.fillStyle = "#00ff00"; sctx.fillRect(1, 0, 1, 1);
    sctx.fillStyle = "#0000ff"; sctx.fillRect(0, 1, 1, 1);
    sctx.fillStyle = "#ffffff"; sctx.fillRect(1, 1, 1, 1);

    const out = drawNearest(
      src as unknown as CanvasImageSource,
      { sx: 0, sy: 0, sw: 2, sh: 2 },
      16,
      16,
      napiFactory
    );
    // 각 사분면 중심은 원색 그대로여야 한다.
    expect(px(out, 4, 4)).toEqual([255, 0, 0, 255]);
    expect(px(out, 12, 4)).toEqual([0, 255, 0, 255]);
    expect(px(out, 4, 12)).toEqual([0, 0, 255, 255]);
    expect(px(out, 12, 12)).toEqual([255, 255, 255, 255]);
  });
});

describe("expandFrameToSheet", () => {
  it("16×16 프레임을 4프레임 64×16 시트로 만들고 idle1/walk1에 1px 밥을 준다", () => {
    const frame = createCanvas(CELL, CELL);
    const fctx = frame.getContext("2d");
    fctx.fillStyle = "#ff0000"; fctx.fillRect(0, 0, 1, 1);
    fctx.fillStyle = "#0000ff"; fctx.fillRect(0, 15, 1, 1);

    const sheet = expandFrameToSheet(frame as unknown as CanvasImageSource, CELL, napiFactory);
    expect(px(sheet, 0, 0)).toEqual([255, 0, 0, 255]);
    expect(px(sheet, 0, 15)).toEqual([0, 0, 255, 255]);
    expect(px(sheet, CELL, 0)[3]).toBe(0);
    expect(px(sheet, CELL, 1)).toEqual([255, 0, 0, 255]);
    expect(px(sheet, CELL * 2, 0)).toEqual([255, 0, 0, 255]);
    expect(px(sheet, CELL * 3, 1)).toEqual([255, 0, 0, 255]);
  });

  it("N=32 프레임은 128×32 시트로 확장되고 밥은 2px", () => {
    const N = 32;
    const frame = createCanvas(N, N);
    const fctx = frame.getContext("2d");
    fctx.fillStyle = "#ff0000"; fctx.fillRect(0, 0, 1, 1); // 좌상단 마커

    const sheet = expandFrameToSheet(frame as unknown as CanvasImageSource, N, napiFactory);
    const c = sheet as unknown as ReturnType<typeof createCanvas>;
    expect(c.width).toBe(SHEET_COLS * N); // 128
    expect(c.height).toBe(N); // 32
    // idle0(셀0): 원위치. idle1(셀1): 2px 아래.
    expect(px(sheet, 0, 0)).toEqual([255, 0, 0, 255]);
    expect(px(sheet, N, 0)[3]).toBe(0);
    expect(px(sheet, N, 1)[3]).toBe(0);
    expect(px(sheet, N, 2)).toEqual([255, 0, 0, 255]);
  });

  it("절차 생성 시트 크기 상수는 64×16으로 남는다", () => {
    expect(SHEET_W).toBe(64);
    expect(SHEET_H).toBe(16);
  });
});

describe("dataUrlToBase64", () => {
  it("data: 헤더를 벗긴다", () => {
    expect(dataUrlToBase64("data:image/png;base64,QUJD")).toBe("QUJD");
    expect(dataUrlToBase64("garbage")).toBe("");
  });
});

describe("normalizeSheet (시트 패스스루)", () => {
  it("4N×N 시트를 원본 셀 해상도로 보존한다", () => {
    const N = 48;
    const src = createCanvas(4 * N, N);
    const sctx = src.getContext("2d");
    sctx.fillStyle = "#123456"; sctx.fillRect(0, 0, 4 * N, N);
    const { sheet, n } = normalizeSheet(src as unknown as CanvasImageSource, 4 * N, N, napiFactory);
    expect(n).toBe(48);
    const c = sheet as unknown as ReturnType<typeof createCanvas>;
    expect(c.width).toBe(4 * 48);
    expect(c.height).toBe(48);
  });

  it("256 초과 시트는 1024×256으로 다운스케일한다", () => {
    const src = createCanvas(4 * 512, 512);
    const { sheet, n } = normalizeSheet(src as unknown as CanvasImageSource, 4 * 512, 512, napiFactory);
    expect(n).toBe(256);
    const c = sheet as unknown as ReturnType<typeof createCanvas>;
    expect(c.width).toBe(1024);
    expect(c.height).toBe(256);
  });
});

describe("normalizeCrop (크롭 해상도 보존)", () => {
  it("크롭 원본 픽셀 크기를 셀 N으로 보존해 4N×N 시트를 만든다", () => {
    const src = createCanvas(200, 200);
    const sctx = src.getContext("2d");
    sctx.fillStyle = "#00ff00"; sctx.fillRect(0, 0, 200, 200);
    sctx.fillStyle = "#ff0000"; sctx.fillRect(0, 0, 1, 1); // 좌상단 마커
    // 원본 픽셀 100px 크롭 -> N=100.
    const { sheet, n } = normalizeCrop(
      src as unknown as CanvasImageSource,
      { sx: 0, sy: 0, sw: 100, sh: 100 },
      napiFactory
    );
    expect(n).toBe(100);
    const c = sheet as unknown as ReturnType<typeof createCanvas>;
    expect(c.width).toBe(4 * 100);
    expect(c.height).toBe(100);
    // idle0 셀 좌상단은 마커(빨강) 근처.
    expect(px(sheet, 0, 0)).toEqual([255, 0, 0, 255]);
  });

  it("16 미만 크롭은 N=16으로 클램프한다", () => {
    const src = createCanvas(50, 50);
    const { n } = normalizeCrop(
      src as unknown as CanvasImageSource,
      { sx: 0, sy: 0, sw: 8, sh: 8 },
      napiFactory
    );
    expect(n).toBe(16);
  });
});

describe("isFullyOpaque", () => {
  it("모든 픽셀 alpha=255면 true", () => {
    const c = createCanvas(4, 4);
    const x = c.getContext("2d");
    x.fillStyle = "#ff0000"; x.fillRect(0, 0, 4, 4);
    expect(isFullyOpaque(c as unknown as HTMLCanvasElement)).toBe(true);
  });
  it("반투명/투명 픽셀이 하나라도 있으면 false", () => {
    const c = createCanvas(4, 4);
    const x = c.getContext("2d");
    x.fillStyle = "#ff0000"; x.fillRect(0, 0, 4, 4);
    const img = x.getImageData(0, 0, 4, 4);
    img.data[3] = 0; // (0,0) 투명화
    x.putImageData(img, 0, 0);
    expect(isFullyOpaque(c as unknown as HTMLCanvasElement)).toBe(false);
  });
});

describe("applyBackgroundKey", () => {
  it("테두리 연결 배경만 투명화하고 내부 동일색은 보존한다", () => {
    // 8×8: 전체 빨강 배경, (2..6,2..6) 파랑 테두리 사각형, 그 안 (3..5,3..5) 빨강(내부 구멍).
    const c = createCanvas(8, 8);
    const x = c.getContext("2d");
    x.fillStyle = "#ff0000"; x.fillRect(0, 0, 8, 8);
    x.fillStyle = "#0000ff"; x.fillRect(2, 2, 4, 4);
    x.fillStyle = "#ff0000"; x.fillRect(3, 3, 2, 2); // 파랑에 둘러싸인 내부 빨강

    applyBackgroundKey(c as unknown as HTMLCanvasElement);

    const at = (px: number, py: number) =>
      Array.from(x.getImageData(px, py, 1, 1).data);
    expect(at(0, 0)[3]).toBe(0); // 가장자리 배경 빨강 -> 투명
    expect(at(7, 7)[3]).toBe(0); // 반대편 가장자리도 연결 -> 투명
    expect(at(2, 2)).toEqual([0, 0, 255, 255]); // 파랑 테두리 보존
    expect(at(3, 3)).toEqual([255, 0, 0, 255]); // 내부 빨강(구멍) 보존
  });

  it("허용 오차(BG_KEY_TOLERANCE) 밖 색은 배경으로 보지 않는다", () => {
    const c = createCanvas(4, 4);
    const x = c.getContext("2d");
    x.fillStyle = "#000000"; x.fillRect(0, 0, 4, 4);
    x.fillStyle = "#ffffff"; x.fillRect(1, 1, 2, 2); // 오차 밖(거리 441)
    applyBackgroundKey(c as unknown as HTMLCanvasElement);
    const at = (px: number, py: number) =>
      Array.from(x.getImageData(px, py, 1, 1).data);
    expect(at(0, 0)[3]).toBe(0); // 검정 배경 투명
    expect(at(1, 1)).toEqual([255, 255, 255, 255]); // 흰색 보존
    expect(BG_KEY_TOLERANCE).toBeGreaterThan(0);
  });
});
