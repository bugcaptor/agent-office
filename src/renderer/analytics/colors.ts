// src/renderer/analytics/colors.ts
//
// 분석 차트의 에이전트별 색. 살아있는 프로필은 스프라이트 팔레트의 대표색
// (셔츠 base — 캐릭터의 주된 옷 색)을 쓰고, 삭제된 에이전트는 중립 회색
// 계열을 순환 배정한다(docs/session-analytics-design.md §4.3).
//
// 색 파생은 office 스프라이트 파이프라인과 동일한 결정적 순수 경로
// (seed → PRNG → archetype.generatePalette)를 재사용한다. gen/ 모듈은
// DOM/Pixi 비의존이라 여기서 그대로 가져다 써도 순수성이 유지된다.
import type { AgentProfile } from "@shared/types";
import { hashStringToSeed, makeRng } from "../office/gen/prng";
import { getArchetype, resolveArchetype } from "../office/gen/archetypes";

/** 0xRRGGBB 숫자를 "#rrggbb" 문자열로. */
function toHex(rgb: number): string {
  return "#" + (rgb & 0xffffff).toString(16).padStart(6, "0");
}

/** 살아있는 프로필의 대표색(스프라이트 셔츠 base). seed에 대해 결정적. */
export function representativeColor(profile: AgentProfile): string {
  const archetypeId = resolveArchetype(profile.archetype, profile.seed);
  const archetype = getArchetype(archetypeId);
  const rng = makeRng(hashStringToSeed(profile.seed));
  const palette = archetype.generatePalette(rng);
  return toHex(palette.shirt.base);
}

/** 삭제된 에이전트에 순환 배정할 중립 회색 팔레트(명도 차로 구분). */
export const DELETED_GRAYS: readonly string[] = [
  "#8d8d8d",
  "#6f6f6f",
  "#a5a5a5",
  "#585858",
  "#bcbcbc",
];

/** 삭제된 에이전트 색: 등장 순서 인덱스로 회색을 순환 배정. */
export function grayForIndex(index: number): string {
  return DELETED_GRAYS[index % DELETED_GRAYS.length];
}
