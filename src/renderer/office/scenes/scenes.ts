// src/renderer/office/scenes/scenes.ts
//
// 풍경(scene) 레지스트리 — theme/themes.ts와 같은 모양으로 맞춘 순수 데이터
// 계층(부수효과는 sceneStorage.ts / appStore.setScene이 담당).
//
// 테마(색 축)와 직교한다: 어떤 씬 × 어떤 테마 조합도 유효하며, 씬 색은
// sceneColor.ts가 테마별로 자동 변환한다.
import { OFFICE_SCENE } from "./officeScene";
import { BEACH_SCENE } from "./beachScene";
import { VALLEY_SCENE } from "./valleyScene";
import type { SceneDef, SceneId } from "./sceneTypes";

export const SCENES: Record<SceneId, SceneDef> = {
  office: OFFICE_SCENE,
  beach: BEACH_SCENE,
  valley: VALLEY_SCENE,
};

/** 픽커의 순환 순서(= 기본 풍경이 첫 번째). */
export const SCENE_ORDER: readonly SceneId[] = ["office", "beach", "valley"];

export const DEFAULT_SCENE_ID: SceneId = "office";

export function isSceneId(v: unknown): v is SceneId {
  return typeof v === "string" && v in SCENES;
}

/** SCENE_ORDER 기준 다음 풍경. UI는 드롭다운이지만 순환 계약(레지스트리
 * 무결성 테스트)과 향후 단축키/CLI 전환을 위해 둔다 — nextThemeId와 동형. */
export function nextSceneId(id: SceneId): SceneId {
  const i = SCENE_ORDER.indexOf(id);
  return SCENE_ORDER[(i + 1) % SCENE_ORDER.length];
}

export type { SceneDef, SceneId, SceneRender, TileDrawFn } from "./sceneTypes";
