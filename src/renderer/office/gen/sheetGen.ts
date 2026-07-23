// src/renderer/office/gen/sheetGen.ts
//
// 시드 → 스프라이트 시트 생성의 **순수** 부분. characterFactory.ts에서 떼어냈다.
//
// 왜 나눴나: characterFactory는 Pixi Texture를 만드는 것이 일이라 `pixi.js`를
// import한다. 마스코트 창(이슈 #72)은 캐릭터 한 명의 idle 2프레임만 2D 캔버스로
// 그리므로 Pixi 렌더러를 두 번째 창 번들에 끌고 들어올 이유가 없다 — 그러려면
// 시트 생성이 Pixi 비의존 모듈에 있어야 한다. 로직은 그대로 옮겼고,
// characterFactory가 두 함수를 그대로 재수출하므로 기존 호출부는 영향이 없다.
import { makeRng, hashStringToSeed } from "./prng";
import { defaultCanvasFactory, type CanvasFactory } from "./compositor";
import { getArchetype, composeArchetypeSheet } from "./archetypes";

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
