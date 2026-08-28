// src/renderer/office/scenes/defineScene.ts
//
// 테마 씬(office를 제외한 전부)의 공통 조립 절차.
//
// office 씬만 테마 팔레트(theme.pixi)를 그대로 쓰고, 나머지 씬은 전부 똑같은
// 순서를 밟는다: 저작된 원색 팔레트에 잔무늬 죽이기(quietPalette) → 채도
// 감쇠(desaturatePalette) → 테마 변환(adaptPalette)을 걸고, 레터박스 배경에는
// 같은 감쇠·변환을 색 하나짜리로 먹인다. 씬마다 다른 것은 그 절차에 넣는
// **값**(원색 팔레트, 죽일 무늬, 남길 예외, 배경색)과 픽셀아트뿐이다.
//
// 씬이 아홉 개가 되면서 이 절차가 아홉 벌로 복사돼 있었다 — 감쇠 순서를
// 바꾸거나 변환 단계를 하나 더 끼우려면 아홉 군데를 똑같이 고쳐야 했고, 한
// 군데를 빠뜨려도 그 씬만 조용히 다른 색이 될 뿐 아무 데서도 걸리지 않는다.
// 여기 한 곳으로 모아 두면 절차는 하나이고 씬은 값만 낸다.
import {
  SCENE_CHROMA_CUT,
  adaptColor,
  adaptPalette,
  desaturateColor,
  desaturatePalette,
  quietPalette,
  sceneColorMode,
} from "./sceneColor";
import type { KeepGroup, QuietGroup } from "./sceneColor";
import type { OfficeMap } from "../map/mapData";
import type { SceneDef, SceneId, TileDrawFn } from "./sceneTypes";

export interface SceneSpec<P extends Record<string, number>> {
  id: SceneId;
  /** 픽커 드롭다운 라벨의 **번역 키**(`office` 네임스페이스). */
  labelKey: string;
  map: OfficeMap;
  /** 저작된 "한낮의 원색" 팔레트. 여기 값이 곧 씬의 색 저작물이다. */
  raw: P;
  /** 바탕에 묻힐 잔무늬 규칙 — 채도 감쇠보다 **앞에** 온다. */
  quiet: readonly QuietGroup<P>[];
  /** 감쇠 예외(빛나는 것, 맵 밖 경치). 없으면 전 색이 기본 감쇠를 받는다. */
  keep?: readonly KeepGroup<P>[];
  /** 레터박스(맵 밖) 배경의 **원색** — 감쇠·테마 변환은 여기서 먹인다. */
  background: number;
  /** 이 씬만 다른 채도 감쇠량. 기본은 `SCENE_CHROMA_CUT`. */
  chromaCut?: number;
  /** 확정된 팔레트를 묶어 16px 한 칸을 그리는 함수를 낸다. */
  draw: (pal: P) => TileDrawFn;
}

/**
 * 씬 정의 하나를 만든다.
 *
 * 팔레트 조립(잔무늬·감쇠)은 모듈 로드 때 **한 번**만 하고, 테마 변환은
 * `resolve` 안에서 매번 한다 — 앞은 테마와 무관한 저작 결정이고 뒤는 테마에
 * 딸린 후처리라, 테마를 바꿀 때 다시 계산해야 하는 것은 뒤쪽뿐이다.
 */
export function defineScene<P extends Record<string, number>>(spec: SceneSpec<P>): SceneDef {
  const cut = spec.chromaCut ?? SCENE_CHROMA_CUT;
  const palette = desaturatePalette(quietPalette(spec.raw, spec.quiet), cut, spec.keep ?? []);
  const background = desaturateColor(spec.background, cut);

  return {
    id: spec.id,
    labelKey: spec.labelKey,
    map: spec.map,
    resolve: (theme) => {
      const mode = sceneColorMode(theme.id);
      return {
        background: adaptColor(background, mode),
        drawTile: spec.draw(adaptPalette(palette, mode)),
      };
    },
  };
}
