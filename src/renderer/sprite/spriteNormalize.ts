// src/renderer/sprite/spriteNormalize.ts
//
// 커스텀 픽셀 아트 업로드 정규화(설계 C). 크기 판별 → nearest 리샘플 →
// 4프레임 시트 합성. 시트 셀 크기 N ∈ [16,256]을 원본 해상도 그대로 보존한다
// (겉보기 크기는 렌더 단계에서 16px로 스케일). 순수(캔버스 팩토리 주입) —
// vitest에서 @napi-rs/canvas로 DOM 없이 검증.
import { CELL, FRAME_ORDER } from "../office/gen/compositor";
import type { SourceRect } from "../portrait/cropMath";

/** 시트 프레임 열 개수(idle0/idle1/walk0/walk1 = 4). */
export const SHEET_COLS = FRAME_ORDER.length; // 4
/** 셀 크기 하한/상한(설계 C: N ∈ [16,256]). */
export const CELL_MIN = CELL; // 16
export const CELL_MAX = 256;

/** 절차 생성(seed) 전용 레거시 시트 크기 — decodeSheet 방어 폴백에서만 사용. */
export const SHEET_W = SHEET_COLS * CELL; // 64
export const SHEET_H = CELL; // 16

/** drawImage 소스로도, toDataURL 출력으로도 쓸 수 있는 캔버스. */
export type SpriteCanvas = CanvasImageSource & {
  toDataURL(type?: string): string;
};

export type SpriteCanvasFactory = (
  w: number,
  h: number
) => { ctx: CanvasRenderingContext2D; canvas: SpriteCanvas };

export const defaultSpriteCanvasFactory: SpriteCanvasFactory = (w, h) => {
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  return { ctx: canvas.getContext("2d")!, canvas };
};

/** 업로드 판별: 4N×N(N≥16) 시트인가, 아니면 크롭 모드인가. */
export type SheetDetect = { kind: "sheet"; n: number } | { kind: "crop" };

/**
 * `w === 4h && h >= 16`이면 시트. N = min(h, 256)으로 클램프(256 초과는
 * 다운스케일 대상). 그 외(h<16, 비율 불일치, 정사각 AI 원본)는 크롭 모드.
 */
export function detectSheet(w: number, h: number): SheetDetect {
  if (w === SHEET_COLS * h && h >= CELL_MIN) {
    return { kind: "sheet", n: Math.min(h, CELL_MAX) };
  }
  return { kind: "crop" };
}

/** `detectSheet`의 boolean 위임 래퍼. SpriteEditor 마이그레이션은 완료되어 프로덕션
 * 코드에서는 더 이상 쓰이지 않고, 테스트 전용 호환 헬퍼로만 남아 있다. */
export function isSheetSize(w: number, h: number): boolean {
  return detectSheet(w, h).kind === "sheet";
}

/** 크롭 영역의 원본 픽셀 크기를 셀 크기 N으로: round 후 [16,256] 클램프. */
export function cropCellSize(cropSourcePx: number): number {
  return Math.max(CELL_MIN, Math.min(CELL_MAX, Math.round(cropSourcePx)));
}

/** 숨쉬기(밥) 오프셋을 셀 크기에 비례 스케일: max(1, round(N/16))px. */
export function bobOffset(n: number): number {
  return Math.max(1, Math.round(n / CELL));
}

/** src의 rect 영역을 nearest로 (dw×dh) 캔버스에 그려 반환. */
export function drawNearest(
  src: CanvasImageSource,
  rect: SourceRect,
  dw: number,
  dh: number,
  factory: SpriteCanvasFactory = defaultSpriteCanvasFactory
): SpriteCanvas {
  const { ctx, canvas } = factory(dw, dh);
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(src, rect.sx, rect.sy, rect.sw, rect.sh, 0, 0, dw, dh);
  return canvas;
}

/**
 * N×N 단일 프레임 → 4N×N 4프레임 시트. idle1/walk1은 compositor의 절차 생성과
 * 동일하게 아래로 밥(숨쉬기) — 밥 크기는 bobOffset(N). 소스 아래 bob줄은 셀
 * 밖으로 나가므로 잘라 넣는다.
 */
export function expandFrameToSheet(
  frame: CanvasImageSource,
  n: number = CELL,
  factory: SpriteCanvasFactory = defaultSpriteCanvasFactory
): SpriteCanvas {
  const { ctx, canvas } = factory(SHEET_COLS * n, n);
  ctx.imageSmoothingEnabled = false;
  const bob = bobOffset(n);
  FRAME_ORDER.forEach((f, i) => {
    const b = f === "idle1" || f === "walk1" ? bob : 0;
    ctx.drawImage(frame, 0, 0, n, n - b, i * n, b, n, n - b);
  });
  return canvas;
}

/** 4N×N 시트를 셀 해상도 보존으로 정규화(패스스루). detectSheet가 N(≤256)을 결정,
 *  256 초과는 nearest로 1024×256 다운스케일. */
export function normalizeSheet(
  src: CanvasImageSource,
  w: number,
  h: number,
  factory: SpriteCanvasFactory = defaultSpriteCanvasFactory
): { sheet: SpriteCanvas; n: number } {
  const det = detectSheet(w, h);
  const n = det.kind === "sheet" ? det.n : CELL;
  const sheet = drawNearest(src, { sx: 0, sy: 0, sw: w, sh: h }, SHEET_COLS * n, n, factory);
  return { sheet, n };
}

/** 크롭 영역(원본 픽셀 rect)을 셀 N=cropCellSize(rect.sw)로 보존해 4N×N 4프레임 시트로. */
export function normalizeCrop(
  src: CanvasImageSource,
  rect: SourceRect,
  factory: SpriteCanvasFactory = defaultSpriteCanvasFactory
): { sheet: SpriteCanvas; n: number } {
  const n = cropCellSize(rect.sw);
  const frame = drawNearest(src, rect, n, n, factory);
  const sheet = expandFrameToSheet(frame, n, factory);
  return { sheet, n };
}

/** getImageData/putImageData가 필요한 픽셀 편집용 캔버스(브라우저·napi 공통). */
type PixelCanvas = {
  width: number;
  height: number;
  getContext(type: "2d"): {
    getImageData(sx: number, sy: number, sw: number, sh: number): ImageData;
    putImageData(data: ImageData, dx: number, dy: number): void;
  } | null;
};

/** 배경 투명화 허용 오차(RGB 유클리드 거리). AI 생성물 배경 그라데이션·디더 흡수용. */
export const BG_KEY_TOLERANCE = 32;

/** 캔버스가 완전 불투명(모든 alpha=255)인가. 투명화 기본 체크 판별용. */
export function isFullyOpaque(canvas: PixelCanvas): boolean {
  const ctx = canvas.getContext("2d")!;
  const { data } = ctx.getImageData(0, 0, canvas.width, canvas.height);
  for (let i = 3; i < data.length; i += 4) {
    if (data[i] < 255) return false;
  }
  return true;
}

/**
 * 가장자리 flood fill 배경 투명화. 좌상단 (0,0) 픽셀 색을 기준색으로, 테두리
 * 픽셀 중 기준색과 RGB 유클리드 거리 오차(tolerance) 내인 지점에서 시작해 4방
 * 연결된 영역만 alpha 0으로 만든다. 캐릭터 내부의 동일 색(흰 셔츠 등)은 배경과
 * 단절되어 있으면 보존된다. in-place.
 */
export function applyBackgroundKey(
  canvas: PixelCanvas,
  tolerance: number = BG_KEY_TOLERANCE
): void {
  const w = canvas.width;
  const h = canvas.height;
  if (w === 0 || h === 0) return;
  const ctx = canvas.getContext("2d")!;
  const img = ctx.getImageData(0, 0, w, h);
  const data = img.data;

  const kr = data[0];
  const kg = data[1];
  const kb = data[2];
  const tol2 = tolerance * tolerance;
  const within = (p: number): boolean => {
    const i = p * 4;
    const dr = data[i] - kr;
    const dg = data[i + 1] - kg;
    const db = data[i + 2] - kb;
    return dr * dr + dg * dg + db * db <= tol2;
  };

  const visited = new Uint8Array(w * h);
  const stack: number[] = [];
  const seed = (x: number, y: number): void => {
    const p = y * w + x;
    if (!visited[p] && within(p)) {
      visited[p] = 1;
      stack.push(p);
    }
  };
  for (let x = 0; x < w; x++) {
    seed(x, 0);
    seed(x, h - 1);
  }
  for (let y = 0; y < h; y++) {
    seed(0, y);
    seed(w - 1, y);
  }

  while (stack.length) {
    const p = stack.pop()!;
    data[p * 4 + 3] = 0; // alpha 0
    const x = p % w;
    const y = (p / w) | 0;
    if (x > 0) seed(x - 1, y);
    if (x < w - 1) seed(x + 1, y);
    if (y > 0) seed(x, y - 1);
    if (y < h - 1) seed(x, y + 1);
  }

  ctx.putImageData(img, 0, 0);
}

/** "data:image/png;base64,XXXX" -> "XXXX" (백엔드는 헤더 없는 base64 기대). */
export function dataUrlToBase64(dataUrl: string): string {
  return dataUrl.split(",", 2)[1] ?? "";
}
