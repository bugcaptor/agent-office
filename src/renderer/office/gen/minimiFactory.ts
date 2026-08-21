// src/renderer/office/gen/minimiFactory.ts
//
// 서브에이전트 미니미용 커스텀 픽셀아트 → Pixi 텍스처. `characterFactory`의
// 커스텀 시트 경로(이슈 #47의 S-적응 프리필터)를 단일 프레임에 그대로 적용한
// 축소판이다.
//
// 겉보기 크기는 부모 캐릭터의 절반인 MINIMI_CELL(8px)로 고정한다 — 커스텀이
// 없을 때의 `MiniAgentsOverlay`(부모 spriteScale × 0.5)와 같은 크기여야
// 커스텀 지정 전후로 미니미가 커졌다 작아졌다 하지 않는다.
//
// D = min(N, 8·S)로 area 프리필터한 뒤 sprite.scale = 8/D을 쓰면, 월드 스케일 S가
// 곱해져 텍셀:물리픽셀 = 1:1이 되어 지글거림이 사라진다(characterFactory와 동일 원리).
// D >= N(축소 불필요)이면 프리필터 없이 nearest 텍스처를 그대로 쓴다.
import { Texture } from "pixi.js";

import { defaultCanvasFactory, CELL, type CanvasFactory } from "./compositor";
import { downscaledFrameTexture } from "./characterFactory";
import { MINIMI_CELL, minimiDetailCellSize } from "./spriteResample";
import { getMinimiOverride } from "./minimiOverrides";

export interface MinimiAssets {
  /** 미니미 한 프레임 텍스처(nearest). */
  texture: Texture;
  /** `Sprite.scale`에 그대로 넣을 배율 = MINIMI_CELL / cellSize. */
  scale: number;
  /** 렌더 셀 픽셀 크기 D. renderScale 변경 시 재생성 판단 키. */
  cellSize: number;
  /** 다운스케일 경로에서 만든 텍스처/소스를 해제한다(교체·파괴 시).
   *  패스스루 경로는 미설정 — 소스가 `minimiOverrides`의 공유 캔버스라
   *  개별 해제하면 안 된다(`CharacterAssets.dispose`와 같은 규약). */
  dispose?: () => void;
}

/**
 * 디코드된 단일 N×N 미니미 프레임 → `MinimiAssets`.
 *
 * `renderScale`(카메라 정수 스케일 S) 미지정이면 프리필터 없이 원본 해상도를
 * 그대로 쓴다(레거시/테스트 경로 — `assetsFromCustomSheet`과 같은 규약).
 */
export function assetsFromMinimiFrame(
  frame: CanvasImageSource,
  renderScale?: number,
  factory: CanvasFactory = defaultCanvasFactory,
): MinimiAssets {
  const n = (frame as { height?: number }).height ?? CELL;
  const d = renderScale == null ? n : minimiDetailCellSize(n, renderScale);

  if (d >= n) {
    const texture = Texture.from(frame as any);
    texture.source.scaleMode = "nearest";
    return { texture, scale: MINIMI_CELL / n, cellSize: n };
  }

  // 단일 프레임이므로 아틀라스 인덱스는 항상 0.
  const texture = downscaledFrameTexture(frame, 0, n, d, factory);
  return {
    texture,
    scale: MINIMI_CELL / d,
    cellSize: d,
    dispose: () => texture.destroy(true),
  };
}

/**
 * 이 에이전트의 미니미 오버라이드가 있으면 텍스처화해 반환, 없으면 null
 * (호출부는 null일 때 현행대로 부모 idle0 축소판을 쓴다).
 */
export function createMinimiAssets(agentId: string, renderScale?: number): MinimiAssets | null {
  const override = getMinimiOverride(agentId);
  if (!override) return null;
  return assetsFromMinimiFrame(override, renderScale);
}
