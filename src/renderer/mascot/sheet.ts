// src/renderer/mascot/sheet.ts
//
// 마스코트 창(이슈 #72)의 스프라이트 확보 — Pixi를 쓰지 않는 순수 2D 캔버스 경로.
//
// 오피스 씬은 Pixi 텍스처가 필요해 `createCharacterAssets`를 쓰지만, 마스코트는
// 한 캐릭터의 idle 2프레임만 그리면 되므로 Pixi 렌더러(와 그에 딸린 eval-free
// 셰이더 셋업)를 두 번째 창에 끌고 들어올 이유가 없다. 절차 생성은 `generateSheet`
// (gen/은 DOM 전용·Pixi 비의존), 커스텀은 `load_sprite`를 직접 디코드한다.
//
// 고해상 커스텀 시트는 이슈 #47과 같은 지글거림을 피하기 위해 `areaDownscalePremul`
// 로 프레임별 프리필터를 거친다 — 오피스뷰와 동일한 알고리즘이라 외형이 일치한다.
import { CELL, defaultCanvasFactory } from "../office/gen/compositor";
import { generateSheet } from "../office/gen/sheetGen";
import { resolveArchetype } from "../office/gen/archetypes";
import { areaDownscalePremul, detailCellSize } from "../office/gen/spriteResample";
import { detectSheet, SHEET_COLS, SHEET_H, SHEET_W } from "../sprite/spriteNormalize";
import { tauriApi } from "../ipc/tauriApi";
import { MASCOT_SPRITE_PX, type MascotState } from "./protocol";

/** 마스코트가 그릴 idle 2프레임(각 cell×cell 캔버스). */
export interface MascotFrames {
  idle: CanvasImageSource[];
  /** 프레임 한 변 픽셀 수(절차 생성=16, 커스텀=프리필터된 D). */
  cell: number;
}

/**
 * 디코드 캔버스 크기 — `spriteCache.sheetCanvasDims`와 같은 규약이다. 그 함수를
 * import하지 않는 이유는 spriteCache가 zustand 스토어를 끌고 오기 때문(마스코트
 * 창은 스토어를 두 번째로 만들지 않는다는 원칙, 설계 §1).
 */
export function mascotSheetDims(w: number, h: number): { w: number; h: number } {
  const det = detectSheet(w, h);
  return det.kind === "sheet" ? { w: SHEET_COLS * det.n, h: det.n } : { w: SHEET_W, h: SHEET_H };
}

/**
 * 커스텀 시트 셀 N을 마스코트 표시 크기에 맞춰 프리필터할 목표 해상도 D.
 * 렌더 스케일 = (표시 px × dpr) / CELL — 물리 픽셀 기준이라 레티나에서도
 * 텍셀:물리픽셀이 1:1에 수렴한다. N이 이미 작으면 D === N(확대 경로, nearest).
 */
export function mascotDetailCell(n: number, dpr = 1, spritePx = MASCOT_SPRITE_PX): number {
  return detailCellSize(n, (spritePx * Math.max(1, dpr)) / CELL);
}

/** 이 상태가 커스텀 시트를 써야 하는가(= 저장된 스프라이트가 있는가). 순수. */
export function usesCustomSheet(state: Pick<MascotState, "agentId" | "spriteUpdatedAt">): boolean {
  return state.agentId !== null && state.spriteUpdatedAt !== null;
}

function makeCanvas(w: number, h: number): HTMLCanvasElement {
  const c = document.createElement("canvas");
  c.width = w;
  c.height = h;
  return c;
}

/** 시트의 i번째 셀을 잘라낸 n×n 캔버스(nearest). */
function sliceFrame(sheet: CanvasImageSource, index: number, n: number): HTMLCanvasElement {
  const c = makeCanvas(n, n);
  const ctx = c.getContext("2d")!;
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(sheet, index * n, 0, n, n, 0, 0, n, n);
  return c;
}

/** n×n 프레임을 d×d로 area 다운스케일한 캔버스. d >= n이면 원본 슬라이스 그대로. */
function frameAt(sheet: CanvasImageSource, index: number, n: number, d: number): HTMLCanvasElement {
  const src = sliceFrame(sheet, index, n);
  if (d >= n) return src;
  const img = src.getContext("2d")!.getImageData(0, 0, n, n);
  const out = areaDownscalePremul({ data: img.data, width: n, height: n }, d, d);
  const dst = makeCanvas(d, d);
  const dctx = dst.getContext("2d")!;
  const dImg = dctx.createImageData(d, d);
  dImg.data.set(out.data);
  dctx.putImageData(dImg, 0, 0);
  return dst;
}

/** base64 PNG(4N×N 정규화 시트) → 캔버스. spriteCache.decodeSheet와 같은 규약. */
function decodeSheet(b64: string): Promise<CanvasImageSource> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => {
      const { w, h } = mascotSheetDims(img.naturalWidth, img.naturalHeight);
      const canvas = makeCanvas(w, h);
      const ctx = canvas.getContext("2d")!;
      ctx.imageSmoothingEnabled = false;
      ctx.drawImage(img, 0, 0, w, h);
      resolve(canvas);
    };
    img.onerror = () => reject(new Error("mascot: 스프라이트 시트 디코드 실패"));
    img.src = `data:image/png;base64,${b64}`;
  });
}

/**
 * 상태에 해당하는 idle 프레임 2장을 만든다. 커스텀 시트 로드/디코드가 실패하면
 * 절차 생성으로 조용히 폴백한다(마스코트가 통째로 사라지는 것보다 낫다).
 */
export async function loadMascotFrames(
  state: MascotState,
  dpr = typeof window === "undefined" ? 1 : window.devicePixelRatio || 1,
): Promise<MascotFrames | null> {
  if (state.agentId === null) return null;

  if (usesCustomSheet(state)) {
    try {
      const b64 = await tauriApi.loadSprite(state.agentId);
      if (b64) {
        const sheet = await decodeSheet(b64);
        const n = (sheet as { height?: number }).height ?? CELL;
        const d = mascotDetailCell(n, dpr);
        return { idle: [frameAt(sheet, 0, n, d), frameAt(sheet, 1, n, d)], cell: d };
      }
    } catch (err) {
      console.warn("mascot: 커스텀 스프라이트 로드 실패 — 절차 생성으로 폴백", err);
    }
  }

  const seed = state.seed || state.agentId;
  const { sheet } = generateSheet(
    seed,
    defaultCanvasFactory,
    resolveArchetype(state.archetype ?? undefined, seed),
  );
  const src = sheet.canvas as CanvasImageSource;
  return { idle: [sliceFrame(src, 0, CELL), sliceFrame(src, 1, CELL)], cell: CELL };
}
