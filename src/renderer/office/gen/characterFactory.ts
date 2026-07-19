// src/renderer/office/gen/characterFactory.ts
//
// Character factory: assembles the seeded PRNG,
// palette, part data, and compositor into the profile -> Pixi-animation
// pipeline. This is the end of the signature chain — only this module
// depends on Pixi (texture-ification). The pure part (`generateSheet` /
// `selectLayers`) stays DOM/Pixi-free so it's directly vitest-able.
import { Texture, Rectangle } from "pixi.js";

import { makeRng, hashStringToSeed } from "./prng";
import {
  defaultCanvasFactory,
  CELL,
  FRAME_ORDER,
  type FrameName,
  type CanvasFactory,
} from "./compositor";
import { getSpriteOverride } from "./spriteOverrides";
import { getArchetype, resolveArchetype, composeArchetypeSheet } from "./archetypes";
import { detailCellSize, areaDownscalePremul } from "./spriteResample";
import type { AgentProfile } from "../types";

export interface CharacterAssets {
  base: Texture; // 시트 전체 (nearest). 다운스케일 경로에선 idle0 프레임 텍스처.
  frames: Record<FrameName, Texture>; // 프레임별 서브텍스처
  idle: Texture[]; // [idle0, idle1]
  walk: Texture[]; // [walk0, walk1]
  cellSize: number; // 렌더 셀 픽셀 크기. 절차 생성=16, 커스텀=min(N, 16·S).
  descriptor: { archetype: string; hair: string; clothes: string; accessory: string }; // 디버그/프로필 표시
  /** 다운스케일 경로에서 생성한 프레임별 텍스처/소스를 해제한다(교체·파괴 시).
   *  아틀라스/절차 경로는 미설정(공유 소스라 개별 해제 안 함). */
  dispose?: () => void;
}

const CUSTOM_DESCRIPTOR = {
  archetype: "custom",
  hair: "custom",
  clothes: "custom",
  accessory: "custom",
} as const;

/** seed(+archetype) → 결정적 팔레트/시트 스펙. archetype 기본 "human"(레거시 호환). */
export function selectLayers(seed: string, archetype: string = "human") {
  const arch = getArchetype(archetype);
  const rng = makeRng(hashStringToSeed(seed));
  const pal = arch.generatePalette(rng);
  const built = arch.buildFrames(rng, pal);
  return {
    pal,
    descriptor: { archetype: arch.id, ...built.descriptor },
    build: built.sheet,
  };
}

/** 순수 시트 생성(테스트 픽셀 비교용). factory 2번째/archetype 3번째(레거시 호출 호환). */
export function generateSheet(
  seed: string,
  factory: CanvasFactory = defaultCanvasFactory,
  archetype: string = "human",
) {
  const { pal, build, descriptor } = selectLayers(seed, archetype);
  return { sheet: composeArchetypeSheet(build, pal, factory), descriptor };
}

/** 한 프레임(소스 아틀라스의 i번째 N×N 영역)을 D×D로 area 다운스케일한 nearest
 *  텍스처. 프레임별 개별 소스라 시트 seam bleed가 없다. 순수 리샘플 + 캔버스 I/O. */
function downscaledFrameTexture(
  sheet: CanvasImageSource,
  frameIdx: number,
  n: number,
  d: number,
  factory: CanvasFactory,
): Texture {
  // 1) 소스 프레임 픽셀 추출(nearest, N×N).
  const srcC = factory(n, n);
  (srcC.ctx as { imageSmoothingEnabled: boolean }).imageSmoothingEnabled = false;
  (srcC.ctx as CanvasRenderingContext2D).drawImage(sheet, frameIdx * n, 0, n, n, 0, 0, n, n);
  const srcImg = (srcC.ctx as CanvasRenderingContext2D).getImageData(0, 0, n, n);
  // 2) area(box) 다운스케일.
  const dst = areaDownscalePremul({ data: srcImg.data, width: n, height: n }, d, d);
  // 3) D×D 캔버스에 써서 텍스처화.
  const dstC = factory(d, d);
  const dctx = dstC.ctx as CanvasRenderingContext2D;
  const imgData = dctx.createImageData(d, d);
  imgData.data.set(dst.data);
  dctx.putImageData(imgData, 0, 0);
  const tex = Texture.from(dstC.canvas as any);
  tex.source.scaleMode = "nearest";
  return tex;
}

/**
 * 디코드된 4N×N 커스텀 시트 → CharacterAssets.
 *
 * renderScale(S)이 주어지면 D = min(N, 16·S)로 area 프리필터해(고해상 시트의
 * 지글거림 제거, 이슈 #47) 프레임별 개별 텍스처를 만든다. D === N(축소 불필요,
 * 예: 저해상 시트 또는 큰 창)이거나 renderScale 미지정(레거시/테스트)이면 기존
 * 아틀라스 슬라이스(공유 소스, nearest) 경로를 그대로 쓴다.
 */
export function assetsFromCustomSheet(
  sheet: CanvasImageSource,
  renderScale?: number,
  factory: CanvasFactory = defaultCanvasFactory,
): CharacterAssets {
  const n = (sheet as { height?: number }).height ?? CELL;
  const d = renderScale == null ? n : detailCellSize(n, renderScale);

  if (d >= n) {
    const base = Texture.from(sheet as any);
    base.source.scaleMode = "nearest";
    const frames = {} as Record<FrameName, Texture>;
    FRAME_ORDER.forEach((f, i) => {
      frames[f] = new Texture({
        source: base.source,
        frame: new Rectangle(i * n, 0, n, n),
      });
    });
    return {
      base,
      frames,
      cellSize: n,
      descriptor: { ...CUSTOM_DESCRIPTOR },
      idle: [frames.idle0, frames.idle1],
      walk: [frames.walk0, frames.walk1],
    };
  }

  const frames = {} as Record<FrameName, Texture>;
  FRAME_ORDER.forEach((f, i) => {
    frames[f] = downscaledFrameTexture(sheet, i, n, d, factory);
  });
  const list = FRAME_ORDER.map((f) => frames[f]);
  return {
    base: frames.idle0,
    frames,
    cellSize: d,
    descriptor: { ...CUSTOM_DESCRIPTOR },
    idle: [frames.idle0, frames.idle1],
    walk: [frames.walk0, frames.walk1],
    dispose: () => list.forEach((t) => t.destroy(true)),
  };
}

/** Pixi 텍스처까지. 렌더러 컨텍스트 필요. renderScale(카메라 정수 스케일 S)이
 *  주어지면 커스텀 시트를 S-적응 프리필터한다(이슈 #47). 절차 생성 경로는 항상
 *  16px이라 renderScale과 무관하게 불변. */
export function createCharacterAssets(profile: AgentProfile, renderScale?: number): CharacterAssets {
  const override = getSpriteOverride(profile.id);
  if (override) return assetsFromCustomSheet(override, renderScale);
  const seed = profile.seed || profile.id;
  const archetype = resolveArchetype(profile.archetype as string | undefined, seed);
  const { sheet, descriptor } = generateSheet(seed, defaultCanvasFactory, archetype);
  const base = Texture.from(sheet.canvas as any);
  base.source.scaleMode = "nearest"; // Pixi v8: 픽셀 선명도
  const frames = {} as Record<FrameName, Texture>;
  for (const f of sheet.frames) {
    const r = sheet.frameRects[f];
    frames[f] = new Texture({ source: base.source, frame: new Rectangle(r.x, r.y, r.w, r.h) });
  }
  return {
    base,
    frames,
    cellSize: CELL,
    descriptor,
    idle: [frames.idle0, frames.idle1],
    walk: [frames.walk0, frames.walk1],
  };
}

/**
 * 미리보기용 확대 캔버스 생성 추상화 — 기본 동작(브라우저 `document`
 * 기반)은 그대로 두되, 합성 시트와 동일하게 주입 시임을 유지한다. 테스트는
 * `@napi-rs/canvas` 팩토리를 주입해 `document` 없이 결정성을 검증한다
 * (`gen/`은 DOM 비의존이라는 원칙과 일치).
 */
export type PreviewCanvasFactory = (
  w: number,
  h: number,
) => {
  ctx: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;
  canvas: { toDataURL(type?: string): string };
};

const defaultPreviewCanvasFactory: PreviewCanvasFactory = (w, h) => {
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d")!;
  return { ctx, canvas };
};

/**
 * (정합화: C의 프로필 다이얼로그 라이브 프리뷰용 — 순수, Pixi 비의존)
 * idle0 프레임만 잘라 scale배 확대한 PNG dataURL 반환.
 *
 * 동결(frozen): 동기 함수, dataURL을 반환한다. `generateSpritePreview(seed)`
 * 형태로 호출 가능해야 하므로 `scale`/캔버스 팩토리는 전부 기본값을 가진다.
 */
export function generateSpritePreview(
  seed: string,
  scale = 6,
  sheetFactory: CanvasFactory = defaultCanvasFactory,
  outFactory: PreviewCanvasFactory = defaultPreviewCanvasFactory,
  archetype: string = "human",
): string {
  const { sheet } = generateSheet(seed, sheetFactory, archetype);
  const size = CELL * scale;
  const { ctx, canvas } = outFactory(size, size);
  (ctx as { imageSmoothingEnabled: boolean }).imageSmoothingEnabled = false;
  (ctx as CanvasRenderingContext2D).drawImage(
    sheet.canvas as CanvasImageSource,
    0,
    0,
    CELL,
    CELL,
    0,
    0,
    size,
    size,
  );
  return canvas.toDataURL("image/png");
}
