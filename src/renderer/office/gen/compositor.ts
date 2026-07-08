// src/renderer/office/gen/compositor.ts
//
// 스프라이트 시트 합성기.
//
// 문자 → 색상 해석 후 오프스크린 캔버스에 픽셀을 찍는다. 레이어 순서:
// body → clothes → hair → accessory. 프레임 구성: idle 2프레임(bob 0px / 1px),
// walk 2프레임(다리 스왑 + bob). 스프라이트시트는 가로로 나열.
//
// 픽셀 아트 제약: nearest-neighbor 전제 — 안티앨리어싱을 유발하는 API(예: 서브픽셀
// 좌표의 drawImage 스케일링, blur 필터 등)는 여기서 사용하지 않는다. `fillRect`로
// 정수 좌표에 1x1 사각형을 찍는 것만 사용.
import type { CharacterPalette } from './palette';
import type { PixelRows } from './parts';

export const CELL = 16;                 // 셀(캐릭터) 픽셀 크기
export type FrameName = 'idle0' | 'idle1' | 'walk0' | 'walk1';
export const FRAME_ORDER: FrameName[] = ['idle0', 'idle1', 'walk0', 'walk1'];

/** 캔버스 생성 추상화 — 브라우저는 OffscreenCanvas, 테스트는 주입 가능. */
export type CanvasFactory = (w: number, h: number) => {
  ctx: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;
  canvas: HTMLCanvasElement | OffscreenCanvas;
};

export const defaultCanvasFactory: CanvasFactory = (w, h) => {
  const canvas = typeof OffscreenCanvas !== 'undefined'
    ? new OffscreenCanvas(w, h)
    : Object.assign(document.createElement('canvas'), { width: w, height: h });
  const ctx = (canvas as any).getContext('2d')!;
  (ctx as any).imageSmoothingEnabled = false;
  return { ctx, canvas: canvas as any };
};

/**
 * 문자 → 0xRRGGBB 또는 null(투명).
 *
 * 이 함수는 원래 모듈 비공개로 설계됐으나, "resolveChar 매핑 테스트"를 명시적
 * 산출물로 요구하므로(바인딩 요구사항) 테스트가 직접 임포트할 수 있도록
 * export로 변경했다 — 로직/케이스는 원래 설계와 동일, 가시성만 다름.
 */
export function resolveChar(ch: string, pal: CharacterPalette): number | null {
  switch (ch) {
    case '.': return null;
    case 'o': return pal.outline;
    case 'S': return pal.skin.shadow; case 's': return pal.skin.base; case 'H': return pal.skin.light;
    case 'A': return pal.hair.shadow; case 'a': return pal.hair.base;  case 'B': return pal.hair.light;
    case 'C': return pal.shirt.shadow;case 'c': return pal.shirt.base; case 'D': return pal.shirt.light;
    case 'P': return pal.pants.shadow;case 'p': return pal.pants.base;
    case 'e': return pal.outline;      // 눈
    case 'W': return 0xffffff;
    default:  return null;
  }
}

/** 한 레이어(PixelRows)를 (dx,dy) 오프셋으로 ctx에 찍음. */
function blitLayer(
  ctx: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D,
  rows: PixelRows, pal: CharacterPalette, dx: number, dy: number,
): void {
  for (let y = 0; y < rows.length; y++) {
    const row = rows[y];
    for (let x = 0; x < row.length; x++) {
      const c = resolveChar(row[x], pal);
      if (c === null) continue;
      ctx.fillStyle = `#${c.toString(16).padStart(6, '0')}`;
      ctx.fillRect(dx + x, dy + y, 1, 1);
    }
  }
}

export interface CharacterLayers {
  body: PixelRows;
  clothes: PixelRows;
  hair: PixelRows;
  accessory: PixelRows;
  legsWalkA: PixelRows;
  legsWalkB: PixelRows;
  underlay?: PixelRows; // body 뒤에 그림 (예: 수인 꼬리)
  overlay?: PixelRows;  // accessory 위에 그림 (예: 귀/엄니/이음선)
}

/** 한 프레임을 (frameX*CELL, 0)에 그림. bob과 다리 스왑을 프레임별로 적용. */
function drawFrame(
  ctx: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D,
  frame: FrameName, layers: CharacterLayers, pal: CharacterPalette, ox: number,
): void {
  const bob = (frame === 'idle1' || frame === 'walk1') ? 1 : 0; // 1px 흔들림
  if (layers.underlay) blitLayer(ctx, layers.underlay, pal, ox, bob);
  blitLayer(ctx, layers.body, pal, ox, bob);
  if (frame === 'walk0') blitLayer(ctx, layers.legsWalkA, pal, ox, bob);
  if (frame === 'walk1') blitLayer(ctx, layers.legsWalkB, pal, ox, bob);
  blitLayer(ctx, layers.clothes, pal, ox, bob);
  blitLayer(ctx, layers.hair, pal, ox, bob);
  blitLayer(ctx, layers.accessory, pal, ox, bob);
  if (layers.overlay) blitLayer(ctx, layers.overlay, pal, ox, bob);
}

export interface SpriteSheetResult {
  canvas: HTMLCanvasElement | OffscreenCanvas;
  cell: number;
  frames: FrameName[];        // 인덱스 = frameX
  frameRects: Record<FrameName, { x: number; y: number; w: number; h: number }>;
}

/** 4프레임 가로 스프라이트시트 생성 (64x16). 순수. */
export function composeSpriteSheet(
  layers: CharacterLayers, pal: CharacterPalette, factory: CanvasFactory = defaultCanvasFactory,
): SpriteSheetResult {
  const { ctx, canvas } = factory(CELL * FRAME_ORDER.length, CELL);
  const frameRects = {} as SpriteSheetResult['frameRects'];
  FRAME_ORDER.forEach((f, i) => {
    drawFrame(ctx, f, layers, pal, i * CELL);
    frameRects[f] = { x: i * CELL, y: 0, w: CELL, h: CELL };
  });
  return { canvas, cell: CELL, frames: FRAME_ORDER, frameRects };
}

export type FrameGrids = Record<FrameName, PixelRows>;

/** 비휴머노이드용: 프레임별 완성 그리드를 dy=0으로 그대로 blit(자체 bob/변형 내장). 순수. */
export function composeFramesSheet(
  frames: FrameGrids, pal: CharacterPalette, factory: CanvasFactory = defaultCanvasFactory,
): SpriteSheetResult {
  const { ctx, canvas } = factory(CELL * FRAME_ORDER.length, CELL);
  const frameRects = {} as SpriteSheetResult['frameRects'];
  FRAME_ORDER.forEach((f, i) => {
    blitLayer(ctx, frames[f], pal, i * CELL, 0);
    frameRects[f] = { x: i * CELL, y: 0, w: CELL, h: CELL };
  });
  return { canvas, cell: CELL, frames: FRAME_ORDER, frameRects };
}
